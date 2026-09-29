//! 应用程序管理 —— 把进程按 `.app` bundle 聚合，展示用户视角的「运行中的应用」
//!
//! macOS 的进程和应用是分离的：一个 Chrome「应用」可能对应 20+ 个进程
//! （main + helper + renderer + gpu + plugin ...）。用户看活动监视器只关心
//! 「我开了哪些应用、各占多少内存」，这个模块就做这个。

use crate::operation_executor::{AppQuitter, DomainFuture};
use crate::operations::{
    AppIdentity, ApplicationSnapshotRegistration, InstalledAppIdentity, OperationStore,
    ProcessIdentity, ProcessTarget,
};
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use sysinfo::System;

#[derive(Serialize, Clone, Debug)]
pub struct AppChildProcess {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub exe: String,
    pub start_time: u64,
    pub memory_mb: f64,
    pub cpu_percent: f32,
    pub ports: Vec<u16>,
    /// 是否是该应用的「主进程」（其他子进程的祖先）
    pub is_main: bool,
    /// 展示时的缩进层级（0 = 主进程；1+ = 子孙）
    pub depth: usize,
    /// 是否命中安全审计，允许用户手动终止但需要高亮提醒
    pub protected: bool,
    /// 受保护原因的 i18n key（`process.protect.*`）+ 插值参数，纯展示。
    pub protected_reason_key: Option<String>,
    pub protected_reason_params: crate::i18n_text::I18nParams,
    /// 是否在用户白名单（由上层注入）
    pub whitelisted: bool,
    pub selection_key: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AppInfo {
    /// 应用 bundle 路径，如 /Applications/Safari.app
    pub bundle_path: String,
    /// 显示名（从 Info.plist 或从路径推断）
    pub name: String,
    /// bundle id，如 com.apple.Safari（可能为空）
    pub bundle_id: String,
    /// 应用图标 base64 PNG（64x64）
    pub icon_base64: Option<String>,
    /// 主进程 PID（通常是路径最短的那个）
    pub main_pid: u32,
    /// 所有相关进程（主进程 + helper）
    pub all_pids: Vec<u32>,
    /// 每个子进程的详细（按树形结构展开后的顺序 + depth）
    pub children: Vec<AppChildProcess>,
    /// 总内存（所有相关进程求和，单位 MB）
    pub memory_mb: f64,
    /// 总 CPU（所有相关进程求和）
    pub cpu_percent: f32,
    /// 运行时长（秒，取最早启动的那个进程）
    pub uptime_secs: u64,
    /// 监听的端口（如果有）
    pub ports: Vec<u16>,
    /// 是否为系统应用（位于 /System / /Library / 等）
    pub is_system: bool,
    /// 受保护子进程数量
    pub protected_process_count: usize,
    /// 白名单子进程数量
    pub whitelisted_process_count: usize,
    pub selection_key: String,
}

/// 列出所有运行中的 .app 应用
pub fn list_running_apps(sys: &mut System) -> Vec<AppInfo> {
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(150));
    sys.refresh_cpu_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    // Step 1: 按 bundle path 分组进程
    let mut groups: HashMap<String, Vec<(u32, &sysinfo::Process)>> = HashMap::new();
    for (pid, proc) in sys.processes() {
        // 只看当前用户的（跨用户的 root 服务属于系统，前端不关心）
        if !crate::process_safety::is_same_user(proc) {
            continue;
        }
        let exe = match proc.exe() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        // 提取 .app bundle 路径：
        //   exe 形如 /Applications/Safari.app/Contents/MacOS/Safari
        //   找第一个 .app 结尾的祖先目录
        let bundle = match find_app_bundle(&exe) {
            Some(b) => b,
            None => continue,
        };
        let key = bundle.to_string_lossy().to_string();
        groups.entry(key).or_default().push((pid.as_u32(), proc));
    }

    // Step 2: 为每组生成 AppInfo
    let mut apps: Vec<AppInfo> = groups
        .into_iter()
        .filter_map(|(bundle_path, procs)| build_app_info(&bundle_path, &procs, sys))
        .collect();

    // 端口注入（应用总端口 + 每个子进程各自的端口）
    let all_pids: Vec<u32> = apps.iter().flat_map(|a| a.all_pids.clone()).collect();
    let port_map = crate::ports::ports_by_pid(&all_pids);
    for app in apps.iter_mut() {
        let mut ports: Vec<u16> = Vec::new();
        for pid in &app.all_pids {
            if let Some(list) = port_map.get(pid) {
                ports.extend(list.iter().copied());
            }
        }
        ports.sort();
        ports.dedup();
        app.ports = ports;

        // 每个子进程
        for child in app.children.iter_mut() {
            if let Some(list) = port_map.get(&child.pid) {
                child.ports = list.clone();
            }
        }
    }

    // 按内存降序
    apps.sort_by(|a, b| {
        b.memory_mb
            .partial_cmp(&a.memory_mb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    apps
}

/// 从 exe 路径往上找 .app 结尾的目录
pub(crate) fn find_app_bundle(exe: &std::path::Path) -> Option<PathBuf> {
    for ancestor in exe.ancestors() {
        if let Some(name) = ancestor.file_name() {
            if name.to_string_lossy().ends_with(".app") {
                return Some(ancestor.to_path_buf());
            }
        }
    }
    None
}

fn build_app_info(
    bundle_path: &str,
    procs: &[(u32, &sysinfo::Process)],
    sys: &System,
) -> Option<AppInfo> {
    if procs.is_empty() {
        return None;
    }
    let bundle = PathBuf::from(bundle_path);
    let info_plist = bundle.join("Contents/Info.plist");
    let (name_from_plist, bundle_id) = read_plist_metadata(&info_plist);
    let name_from_path = bundle
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".into());
    let name = name_from_plist.unwrap_or(name_from_path);

    // 主进程：路径最短的那个（MacOS/Safari 短于 XPCServices/... 的）
    let main = procs
        .iter()
        .min_by_key(|(pid, proc)| {
            let path_len = proc
                .exe()
                .map(|p| p.to_string_lossy().len())
                .unwrap_or(usize::MAX);
            (path_len, *pid)
        })
        .cloned();
    let (main_pid, _main_proc) = main?;

    let all_pids: Vec<u32> = procs.iter().map(|(p, _)| *p).collect();
    let memory_mb: f64 = procs
        .iter()
        .map(|(_, p)| p.memory() as f64 / 1024.0 / 1024.0)
        .sum();
    let cpu_percent: f32 = procs.iter().map(|(_, p)| p.cpu_usage()).sum();

    let earliest_start = procs.iter().map(|(_, p)| p.start_time()).min().unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let uptime_secs = now.saturating_sub(earliest_start);

    let is_system = bundle_path.starts_with("/System/")
        || bundle_path.starts_with("/Library/CoreServices/")
        || bundle_path.starts_with("/usr/libexec/");

    // ==== 构建父子关系的 children 列表（深度优先） ====
    let children = build_children_tree(main_pid, procs, sys);
    let protected_process_count = children.iter().filter(|c| c.protected).count();
    let whitelisted_process_count = children.iter().filter(|c| c.whitelisted).count();

    // 读取应用图标
    let icon_base64 = crate::app_scanner::read_icon_base64_for_bundle(&bundle);

    Some(AppInfo {
        bundle_path: bundle_path.to_string(),
        name,
        bundle_id: bundle_id.unwrap_or_default(),
        icon_base64,
        main_pid,
        all_pids,
        children,
        memory_mb,
        cpu_percent,
        uptime_secs,
        ports: Vec::new(),
        is_system,
        protected_process_count,
        whitelisted_process_count,
        selection_key: String::new(),
    })
}

struct DfsContext<'a> {
    main_pid: u32,
    procs: &'a [(u32, &'a sysinfo::Process)],
    children_of: &'a HashMap<u32, Vec<u32>>,
    parent_pids: &'a std::collections::HashSet<u32>,
}

/// 以主进程为根，把 procs 按父子关系展开成带 depth 的扁平有序列表
fn build_children_tree(
    main_pid: u32,
    procs: &[(u32, &sysinfo::Process)],
    sys: &System,
) -> Vec<AppChildProcess> {
    let mut result: Vec<AppChildProcess> = Vec::new();
    let pid_set: std::collections::HashSet<u32> = procs.iter().map(|(p, _)| *p).collect();
    let parent_pids = crate::process_safety::collect_parent_pids(sys);

    // 构建 parent -> children 索引（只在本应用的 pid_set 范围内）
    let mut children_of: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    for (pid, proc) in procs {
        if let Some(ppid) = proc.parent() {
            let ppid = ppid.as_u32();
            if pid_set.contains(&ppid) {
                children_of.entry(ppid).or_default().push(*pid);
            }
        }
    }
    // 排序让输出稳定（按 PID）
    for v in children_of.values_mut() {
        v.sort();
    }

    // DFS
    fn dfs(
        pid: u32,
        depth: usize,
        context: &DfsContext<'_>,
        out: &mut Vec<AppChildProcess>,
        visited: &mut std::collections::HashSet<u32>,
    ) {
        if visited.contains(&pid) {
            return;
        }
        visited.insert(pid);
        if let Some((_, proc)) = context.procs.iter().find(|(p, _)| *p == pid) {
            let name = proc.name().to_string_lossy().to_string();
            let protection =
                crate::process_safety::evaluate_protection(proc, &name, context.parent_pids);
            out.push(AppChildProcess {
                pid,
                parent_pid: proc.parent().map(|p| p.as_u32()),
                name,
                exe: proc
                    .exe()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_default(),
                start_time: proc.start_time(),
                memory_mb: proc.memory() as f64 / 1024.0 / 1024.0,
                cpu_percent: proc.cpu_usage(),
                ports: Vec::new(),
                is_main: pid == context.main_pid,
                depth,
                protected: protection.protected,
                protected_reason_key: protection.reason.as_ref().map(|text| text.key.clone()),
                protected_reason_params: protection
                    .reason
                    .as_ref()
                    .map(|text| text.params.clone())
                    .unwrap_or_default(),
                whitelisted: protection.whitelisted,
                selection_key: String::new(),
            });
        }
        if let Some(children) = context.children_of.get(&pid) {
            for &c in children {
                dfs(c, depth + 1, context, out, visited);
            }
        }
    }

    let context = DfsContext {
        main_pid,
        procs,
        children_of: &children_of,
        parent_pids: &parent_pids,
    };
    let mut visited = std::collections::HashSet::new();
    dfs(main_pid, 0, &context, &mut result, &mut visited);

    // 有些 orphan 进程父进程不在本应用范围内（比如直接从 launchd 起的 helper）
    // 按内存降序挂在 depth=0 下
    let mut orphans: Vec<u32> = procs
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| !visited.contains(p))
        .collect();
    orphans.sort_by_key(|p| {
        procs
            .iter()
            .find(|(pp, _)| pp == p)
            .map(|(_, proc)| std::cmp::Reverse(proc.memory()))
            .unwrap_or(std::cmp::Reverse(0))
    });
    for o in orphans {
        dfs(o, 0, &context, &mut result, &mut visited);
    }

    result
}

/// 读 .app/Contents/Info.plist，提取 CFBundleName / CFBundleIdentifier
/// 用最朴素的文本解析（Info.plist 多是 XML 格式；二进制 plist 我们不解析，
/// 返回 None 由 caller 走路径推断兜底）
/// 读 Info.plist 里的 `(展示名, bundle id)`。
///
/// **原生解析，不 shell 出去。** 早先的实现是：二进制 plist 先 `plutil
/// -convert xml1` 转成 XML 文本，再用正则按 key 抓 `<string>`。App Sandbox
/// 下沙箱进程只能 exec 自己 bundle 里的二进制，`plutil` 根本不可用，所以这条
/// 路径在 MAS 版上必然失效 —— 改用 `plist` crate 原地解析 XML / bplist 都行。
///
/// 顺带修掉正则的一个真 bug：正则找的是「`<key>K</key>` 之后的**第一个**
/// `<string>`」。若该 key 的值是数组/字典（例如 `CFBundleURLTypes`），
/// 正则会跨过结构直接抓到**嵌套里**的字符串，属性就串了。真正的解析器只看
/// 该 key 自己的值，类型不对就返回 None。
pub(crate) fn read_plist_metadata(path: &std::path::Path) -> (Option<String>, Option<String>) {
    let map = read_plist_map(path);
    let name =
        plist_string(&map, "CFBundleDisplayName").or_else(|| plist_string(&map, "CFBundleName"));
    let id = plist_string(&map, "CFBundleIdentifier");
    (name, id)
}

/// 读一个 plist 文件的顶层字典。解析不了就返回空 map（调用方各字段都拿不到，
/// 自然退化成「未知 app」，而不是 panic）。
pub(crate) fn read_plist_map(path: &std::path::Path) -> plist::Dictionary {
    let Ok(bytes) = std::fs::read(path) else {
        return plist::Dictionary::new();
    };
    // 一次读入内存。`Value::from_reader` 自己按魔数认 XML / bplist 两种格式，
    // 所以不需要（也不该）在这里手写分支再调 plutil。
    plist::Value::from_reader(std::io::Cursor::new(&bytes))
        .ok()
        .and_then(|value| value.into_dictionary())
        .unwrap_or_default()
}

/// 取字典里某个 key 的字符串值；**类型不是 string 就返回 None**。
///
/// 空白串（含全空格）按 None 处理 —— 显示名落空时调用方会继续往下走分支，
/// 返回 `"   "` 会让界面出现一个看不见的空白名字。
pub(crate) fn plist_string(map: &plist::Dictionary, key: &str) -> Option<String> {
    let value = map.get(key)?.as_string()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 从展示视图构造后端应用身份（bundle + 子进程 identity）
pub(crate) fn app_identities_from_infos(apps: &[AppInfo]) -> Vec<AppIdentity> {
    apps.iter().map(app_identity_from_info).collect()
}

fn app_identity_from_info(app: &AppInfo) -> AppIdentity {
    AppIdentity {
        bundle_path: app.bundle_path.clone(),
        bundle_id: app.bundle_id.clone(),
        app_name: app.name.clone(),
        processes: app
            .children
            .iter()
            .map(|child| ProcessTarget {
                identity: ProcessIdentity {
                    pid: child.pid,
                    name: child.name.clone(),
                    exe: child.exe.clone(),
                    start_time: child.start_time,
                },
                protected: child.protected,
                whitelisted: child.whitelisted,
            })
            .collect(),
    }
}

/// 注册应用快照并把 app key / child key 回填到视图
pub(crate) fn register_applications(
    store: &mut OperationStore,
    apps: &mut [AppInfo],
) -> Result<ApplicationSnapshotRegistration, UserError> {
    let registration = store.register_application_snapshot(app_identities_from_infos(apps))?;
    for (app, key) in apps.iter_mut().zip(registration.app_keys.iter()) {
        app.selection_key = key.clone();
    }
    let mut cursor = 0usize;
    for (app, key) in apps.iter_mut().zip(registration.app_keys.iter()) {
        for child in app.children.iter_mut() {
            let binding = registration.child_bindings.get(cursor).ok_or_else(|| {
                UserError::new(
                    ErrorCode::APP_CHILD_SELECTION_MISMATCH,
                    "子进程选择 key 数量与快照不一致",
                )
            })?;
            if binding.app_key != *key || binding.pid != child.pid {
                return Err(UserError::new(
                    ErrorCode::APP_CHILD_SELECTION_MISMATCH,
                    "子进程选择 key 不属于该应用",
                ));
            }
            child.selection_key = binding.child_key.clone();
            cursor += 1;
        }
    }
    Ok(registration)
}

#[derive(Serialize, Clone, Debug)]
pub struct AppGracefulQuitReport {
    pub app_name: String,
    pub bundle_id: String,
    /// 优雅退出失败原因：结构化错误，`None` 表示退出成功。
    pub quit_error: Option<UserError>,
}

pub(crate) struct SystemAppQuitter;

impl AppQuitter for SystemAppQuitter {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        let observed: Vec<InstalledAppIdentity> = bundle_paths
            .iter()
            .filter_map(|path| crate::uninstaller::read_installed_identity(Path::new(path)))
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, app: &InstalledAppIdentity) -> DomainFuture<'_, Result<(), UserError>> {
        let requested = app.app_name.clone();
        Box::pin(async move { crate::uninstaller::quit_app(&requested).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::OperationStore;

    fn child(pid: u32, name: &str, bundle: &str) -> AppChildProcess {
        AppChildProcess {
            pid,
            parent_pid: None,
            name: name.to_owned(),
            exe: format!("/Applications/{bundle}.app/Contents/MacOS/{name}"),
            start_time: 1_700_000_000 + u64::from(pid),
            memory_mb: 1.0,
            cpu_percent: 0.0,
            ports: Vec::new(),
            is_main: pid == 1,
            depth: 0,
            protected: false,
            protected_reason_key: None,
            protected_reason_params: Vec::new(),
            whitelisted: false,
            selection_key: String::new(),
        }
    }

    fn app_info(bundle: &str) -> AppInfo {
        AppInfo {
            bundle_path: format!("/Applications/{bundle}.app"),
            name: bundle.to_owned(),
            bundle_id: format!("com.example.{bundle}"),
            icon_base64: None,
            main_pid: 1,
            all_pids: vec![1, 2],
            children: vec![child(1, bundle, bundle), child(2, "Helper", bundle)],
            memory_mb: 2.0,
            cpu_percent: 0.0,
            uptime_secs: 60,
            ports: Vec::new(),
            is_system: false,
            protected_process_count: 0,
            whitelisted_process_count: 0,
            selection_key: String::new(),
        }
    }

    #[test]
    fn app_identity_from_info_keeps_backend_process_identity() {
        let info = app_info("Editor");
        let identity = app_identity_from_info(&info);

        assert_eq!(identity.bundle_path, "/Applications/Editor.app");
        assert_eq!(identity.bundle_id, "com.example.Editor");
        assert_eq!(identity.processes.len(), 2);
        assert_eq!(identity.processes[0].identity.pid, 1);
        assert_eq!(identity.processes[0].identity.exe, info.children[0].exe);
        assert_eq!(
            identity.processes[0].identity.start_time,
            info.children[0].start_time
        );
        assert_eq!(identity.processes[1].identity.name, "Helper");
        assert!(identity.processes.iter().all(|p| !p.protected));
    }

    #[test]
    fn app_identity_processes_stay_inside_the_bundle() {
        let info = app_info("Editor");
        for target in app_identities_from_infos(&[info]) {
            let prefix = format!("{}/", target.bundle_path);
            for process in &target.processes {
                assert!(
                    process.identity.exe.starts_with(&prefix),
                    "子进程 {} 不属于 bundle {}",
                    process.identity.exe,
                    target.bundle_path
                );
            }
        }
    }

    #[test]
    fn register_applications_binds_app_and_child_selection_keys() {
        let mut apps = vec![app_info("Editor"), app_info("Notes")];
        let mut store = OperationStore::new();

        let registration = register_applications(&mut store, &mut apps).unwrap();

        assert_eq!(registration.app_keys.len(), 2);
        assert_eq!(registration.child_bindings.len(), 4);
        assert_eq!(apps[0].selection_key, registration.app_keys[0]);
        assert_eq!(apps[1].selection_key, registration.app_keys[1]);
        assert_eq!(
            apps[0].children[0].selection_key,
            registration.child_bindings[0].child_key
        );
        assert_eq!(
            apps[0].children[1].selection_key,
            registration.child_bindings[1].child_key
        );
        assert_eq!(
            apps[1].children[0].selection_key,
            registration.child_bindings[2].child_key
        );
        assert_eq!(
            apps[1].children[1].selection_key,
            registration.child_bindings[3].child_key
        );
        assert_ne!(apps[0].selection_key, apps[0].children[0].selection_key);
    }

    #[test]
    fn real_apps_expose_backend_start_time_and_bundle_scoped_exe() {
        let mut sys = System::new_all();
        let apps = list_running_apps(&mut sys);
        assert!(!apps.is_empty());

        for app in &apps {
            assert!(app.selection_key.is_empty());
            let prefix = format!("{}/", app.bundle_path);
            for child in &app.children {
                assert!(
                    child.start_time > 0,
                    "PID {} 缺少后端 start_time",
                    child.pid
                );
                assert!(child.selection_key.is_empty());
                assert!(
                    child.exe.starts_with(&prefix),
                    "子进程 {} 不属于 bundle {}",
                    child.exe,
                    app.bundle_path
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "plist_native_tests.rs"]
mod plist_native_tests;
