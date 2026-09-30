use crate::operations::CacheAction;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sysinfo::System;
use walkdir::WalkDir;

/// macOS 上常见的 CLI 工具安装路径（Tauri 打包后 PATH 只有 /usr/bin:/bin）
const EXTRA_PATHS: &[&str] = &["/usr/local/bin", "/opt/homebrew/bin", "/opt/homebrew/sbin"];

pub(crate) const STALE_PROJECT_ROOTS: &[&str] = &[
    "Projects",
    "Code",
    "Developer",
    "workspace",
    "repos",
    "Repos",
    "git",
    "src",
    "Desktop",
    "Documents",
];

/// 在标准 PATH + macOS 常见路径中查找可执行文件
fn find_tool(name: &str) -> Option<PathBuf> {
    // 先用 which（继承当前 PATH）
    if let Ok(p) = which::which(name) {
        return Some(p);
    }
    // 再查 macOS 常见路径
    for dir in EXTRA_PATHS {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 创建 Command 并注入扩展 PATH（确保 Tauri 沙箱内也能找到工具）
fn tool_command(name: &str) -> Option<std::process::Command> {
    let path = find_tool(name)?;
    let mut cmd = std::process::Command::new(path);
    // 把 EXTRA_PATHS 追加到 PATH 环境变量，让子进程也能找到依赖
    let current_path = std::env::var("PATH").unwrap_or_default();
    let extra = EXTRA_PATHS.join(":");
    cmd.env("PATH", format!("{}:{}", extra, current_path));
    Some(cmd)
}

/// 是否有任何一个指定名称的进程正在运行。
/// 用于避免在用户正在 npm install / cargo build / xcodebuild 时清理缓存。
pub fn is_any_tool_busy(names: &[&str]) -> Option<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for proc in sys.processes().values() {
        let pname = proc.name().to_string_lossy().to_lowercase();
        // 读 cmdline 第一个参数，匹配更准（比如 `node` 在跑 `npm install`）
        let cmd_first = proc
            .cmd()
            .first()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        for target in names {
            let t = target.to_lowercase();
            if pname == t || pname.starts_with(&format!("{}-", t)) {
                return Some(target.to_string());
            }
            // 如果 cmdline 第一段含工具名，也算
            if cmd_first.ends_with(&format!("/{}", t)) || cmd_first == t {
                return Some(target.to_string());
            }
            // 子命令匹配：node + npm/npx/pnpm 参数
            if (pname == "node" || pname.ends_with("/node"))
                && proc.cmd().iter().any(|a| {
                    let s = a.to_string_lossy().to_lowercase();
                    s.contains(&format!("/{}/", t)) || s.ends_with(&format!("/{}", t))
                })
            {
                return Some(target.to_string());
            }
        }
    }
    None
}

/// 缓存清理项 —— 代表一个「可以被清理的东西」。
///
/// 文案一律以 i18n key + 插值参数下发，译文在前端词典（`src/i18n/*`）里。
/// 后端**不得**在这里塞可读文案：字段名带 `_key` 后缀就是这个约束的类型级表达，
/// 测试 `cache_items_declare_keys_in_the_right_namespace` 再从源码层钉一遍。
///
/// `label_key` / `description_key` 不参与任何安全判定：白名单、风险等级、
/// 选择 key（opaque、随机、单次消费）全部与它们解耦。
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CacheItem {
    pub id: String,
    pub category: CacheCategory,
    /// i18n key，形如 `cache.item.trash` / `cache.item.npmCache`
    pub label_key: String,
    /// 标签插值参数（可能为空）
    pub label_params: Vec<(String, String)>,
    /// i18n key，形如 `cache.desc.trash`
    pub description_key: String,
    /// 描述插值参数（可能为空）
    pub description_params: Vec<(String, String)>,
    pub path: Option<String>,
    pub size_bytes: u64,
    pub safety: Safety,
    pub default_select: bool,
    #[serde(skip)]
    pub(crate) action: CacheAction,
    #[serde(skip)]
    pub(crate) stale_owner_uid: Option<u32>,
    #[serde(skip)]
    pub(crate) stale_canonical_path: Option<PathBuf>,
    /// 恢复成本：清理后恢复需要做什么
    pub recover_hint: String,
}

impl<'de> serde::Deserialize<'de> for CacheItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
        Err(serde::de::Error::custom("缓存项只能由后端生成"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheCategory {
    Npm,
    Pnpm,
    Yarn,
    Docker,
    Homebrew,
    Xcode,
    Cocoapods,
    Cargo,
    Pip,
    Go,
    System,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// 100% 无风险，工具原生清理
    Safe,
    /// 低风险，通常不影响使用但需要重新下载
    Low,
    /// 中等风险，可能删除用户内容，默认不选
    Medium,
}

#[derive(Serialize, Clone, Debug)]
pub struct CacheScanResult {
    pub items: Vec<CacheItem>,
    pub total_bytes: u64,
    pub scanned_at_ms: u64,
}

type ScanFn = Box<dyn FnOnce(Arc<PathBuf>) -> Vec<CacheItem> + Send>;

/// 运行所有阶段并汇总结果，同时按真实完成顺序上报进度。
async fn run_stages(
    stages: Vec<(&'static str, ScanFn)>,
    progress: Option<crate::scan_progress::ProgressSink>,
) -> Vec<CacheItem> {
    let mut tasks = Vec::new();
    for (stage, f) in stages {
        // home 的来源分形态：
        // - 完整版：`$HOME`，本来就是用户家目录
        // - MAS：**passwd 里的真实 home**。沙箱把 `$HOME` 指向应用自己的
        //   空 container，用它拼路径会让所有 `~/Library/...` 落到那个
        //   空目录，扫出 0 B —— 实测 13.99 GB 的缓存在 App Store 版显示 0
        //   就是这么来的。改成真实 home 之后，能否读到取决于用户有没有
        //   授权过对应目录（见 folder_access::enter_granted_scopes）。
        let home = Arc::new(crate::folder_access::scanner_home());
        let sink = progress.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            // 在**这个阻塞任务内部**进入授权作用域，任务结束即释放。
            //
            // 为什么不能提到外面持着：`SecurityScope` 里是一个 ObjC NSURL，
            // 不是 `Send`；跨 `.await` 持有会让整个 future 失去 `Send`，
            // 而 Tauri 的命令宏要求 `Future + Send`。而在任务内部
            // 进入/释放，start 与 stop 依然严格配对（任务结束就析构），
            // 只是每个 stage 各自进出一次。
            let _granted_scopes = crate::folder_access::enter_granted_scopes();
            if let Some(sink) = &sink {
                sink(crate::scan_progress::StageUpdate::running(stage));
            }
            // **先过滤再上报 done**。顺序反了会引入一个很难自查的 bug：
            // 进度事件带上的是「过滤前」的 found_bytes，而最终列表是过滤后的
            // 总量，两者对不上（实测差 210 MB）。UI 上表现为「已发现 13.0 GB」
            // 但列表加起来只有 12.8 GB，用户看着像 bug —— 实际是我们上报了
            // 一批 MAS 版根本清不了的项。
            let batch = drop_uncleanable(f(home));
            if let Some(sink) = &sink {
                sink(crate::scan_progress::StageUpdate::done(stage, &batch));
            }
            batch
        }));
    }
    let mut items: Vec<CacheItem> = Vec::new();
    for t in tasks {
        if let Ok(batch) = t.await {
            items.extend(batch);
        }
    }
    items
}

/// 丢掉当前构建形态**清不掉**的条目。
///
/// MAS 形态下 `pnpm` / `yarn` / `docker` / `go` 都在沙箱里 exec 不出去
/// （`CacheAction::needs_external_cli`）。这些条目如果照常出现在列表里，
/// 用户勾上点清理只会拿到一句「权限不足 / 启动失败」，而且是**不可逆操作流程的
/// 中途**才失败 —— 比一开始就不提供更糟。
///
/// 关键取舍：**在扫描阶段就过滤，而不是在点清理时拒绝**。列表是用户对「这个产品
/// 能做什么」的判断依据；给一个清不了的选项，等于骗人。
///
/// 注意不影响 pip：`pip_cleanup` 有 `direct_cleanup` 兜底，删目录即可。
fn drop_uncleanable(items: Vec<CacheItem>) -> Vec<CacheItem> {
    if crate::flavor::CURRENT.can_exec_external_tools() {
        return items;
    }
    items
        .into_iter()
        .filter(|item| item.action.is_cleanable())
        .collect()
}

pub async fn scan(progress: Option<crate::scan_progress::ProgressSink>) -> CacheScanResult {
    let stages: Vec<(&'static str, ScanFn)> = vec![
        ("scanStage.npmCache", Box::new(move |home| scan_npm(&home))),
        (
            "scanStage.pnpmCache",
            Box::new(move |home| scan_pnpm(&home)),
        ),
        (
            "scanStage.yarnCache",
            Box::new(move |home| scan_yarn(&home)),
        ),
        (
            "scanStage.dockerImagesAndContainers",
            Box::new(|_home| scan_docker()),
        ),
        (
            "scanStage.dockerUnusedImages",
            Box::new(|_home| scan_docker_stale_images()),
        ),
        (
            "scanStage.staleNodeModules",
            Box::new(move |home| scan_stale_node_modules(&home)),
        ),
        ("scanStage.homebrewCache", Box::new(|_home| scan_homebrew())),
        (
            "scanStage.xcodeCache",
            Box::new(move |home| scan_xcode(&home)),
        ),
        (
            "scanStage.cocoapodsCache",
            Box::new(move |home| scan_cocoapods(&home)),
        ),
        (
            "scanStage.cargoCache",
            Box::new(move |home| scan_cargo(&home)),
        ),
        ("scanStage.pipCache", Box::new(move |home| scan_pip(&home))),
        (
            "scanStage.goModuleCache",
            Box::new(move |home| scan_go(&home)),
        ),
        (
            "scanStage.appCache",
            Box::new(move |home| scan_app_caches(&home)),
        ),
        (
            "scanStage.appLogs",
            Box::new(move |home| scan_app_logs(&home)),
        ),
        (
            "scanStage.crashReports",
            Box::new(move |home| scan_crash_reports(&home)),
        ),
        ("scanStage.trash", Box::new(move |home| scan_trash(&home))),
    ];
    let mut items = run_stages(stages, progress).await;
    items.retain(|i| i.size_bytes > 0);
    items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    let total_bytes = items.iter().map(|i| i.size_bytes).sum();
    let scanned_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    CacheScanResult {
        items,
        total_bytes,
        scanned_at_ms,
    }
}

// ========== 各类扫描器 ==========

fn scan_npm(home: &Path) -> Vec<CacheItem> {
    if find_tool("npm").is_none() {
        return vec![];
    }
    // 安全闸门：如果 npm/npx 正在跑，绝对不动
    if is_any_tool_busy(&["npm", "npx"]).is_some() {
        return vec![];
    }
    let cache_dir = home.join(".npm");
    let size = dir_size(&cache_dir);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "npm-cache".into(),
        category: CacheCategory::Npm,
        label_key: "cache.item.npmCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.npmCache".into(),
        description_params: Vec::new(),
        path: Some(cache_dir.display().to_string()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Npm,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 npm install 会自动重新下载，不影响已安装的包".into(),
    }]
}

fn scan_pnpm(home: &Path) -> Vec<CacheItem> {
    if find_tool("pnpm").is_none() {
        return vec![];
    }
    if is_any_tool_busy(&["pnpm"]).is_some() {
        return vec![];
    }
    let store_dir = home.join("Library/pnpm/store");
    let size = dir_size(&store_dir);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "pnpm-store".into(),
        category: CacheCategory::Pnpm,
        label_key: "cache.item.pnpmStore".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.pnpmStore".into(),
        description_params: Vec::new(),
        path: Some(store_dir.display().to_string()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Pnpm,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 pnpm install 会重新下载需要的包".into(),
    }]
}

fn scan_yarn(home: &Path) -> Vec<CacheItem> {
    if find_tool("yarn").is_none() {
        return vec![];
    }
    if is_any_tool_busy(&["yarn"]).is_some() {
        return vec![];
    }
    // Yarn v1 默认缓存路径
    let cache_dir = home.join("Library/Caches/Yarn");
    let size = dir_size(&cache_dir);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "yarn-cache".into(),
        category: CacheCategory::Yarn,
        label_key: "cache.item.yarnCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.yarnCache".into(),
        description_params: Vec::new(),
        path: Some(cache_dir.display().to_string()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Yarn,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 yarn install 会自动重新下载".into(),
    }]
}

fn scan_docker() -> Vec<CacheItem> {
    if find_tool("docker").is_none() {
        return vec![];
    }
    // 检查 daemon 是否运行
    let running = tool_command("docker")
        .and_then(|mut c| {
            c.args(["info", "--format", "{{.ServerVersion}}"])
                .output()
                .ok()
        })
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !running {
        return vec![];
    }

    let mut out = Vec::new();
    let df = docker_system_df();

    // 镜像可回收空间（包含悬空 + 未使用的）
    if let Some(&sz) = df.get("images") {
        if sz > 0 {
            out.push(CacheItem {
                id: "docker-images-reclaimable".into(),
                category: CacheCategory::Docker,
                label_key: "cache.item.dockerReclaimableImages".into(),
                label_params: Vec::new(),
                description_key: "cache.desc.dockerReclaimableImages".into(),
                description_params: Vec::new(),
                path: None,
                size_bytes: sz,
                safety: Safety::Low,
                default_select: false,
                action: CacheAction::Docker,
                stale_owner_uid: None,
                stale_canonical_path: None,
                recover_hint: "需要时用 docker pull 重新拉取".into(),
            });
        }
    }

    // 构建缓存
    if let Some(&sz) = df.get("build cache") {
        if sz > 0 {
            out.push(CacheItem {
                id: "docker-builder-cache".into(),
                category: CacheCategory::Docker,
                label_key: "cache.item.dockerBuildCache".into(),
                label_params: Vec::new(),
                description_key: "cache.desc.dockerBuildCache".into(),
                description_params: Vec::new(),
                path: None,
                size_bytes: sz,
                safety: Safety::Safe,
                default_select: true,
                action: CacheAction::Docker,
                stale_owner_uid: None,
                stale_canonical_path: None,
                recover_hint: "下次 docker build 会重新构建，首次较慢".into(),
            });
        }
    }

    // 容器可回收空间
    if let Some(&sz) = df.get("containers") {
        if sz > 0 {
            let stopped = tool_command("docker")
                .and_then(|mut c| {
                    c.args(["container", "ls", "-aq", "--filter", "status=exited"])
                        .output()
                        .ok()
                })
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.lines().filter(|l| !l.is_empty()).count())
                .unwrap_or(0);
            if stopped > 0 {
                out.push(CacheItem {
                    id: "docker-stopped-containers".into(),
                    category: CacheCategory::Docker,
                    label_key: "cache.item.dockerStoppedContainers".into(),
                    label_params: vec![("count".to_owned(), stopped.to_string())],
                    description_key: "cache.desc.dockerStoppedContainers".into(),
                    description_params: Vec::new(),
                    path: None,
                    size_bytes: sz,
                    safety: Safety::Safe,
                    default_select: true,
                    action: CacheAction::Docker,
                    stale_owner_uid: None,
                    stale_canonical_path: None,
                    recover_hint: "容器一旦删除无法恢复，但停止的容器通常已无价值".into(),
                });
            }
        }
    }

    // 未引用卷
    if let Some(&sz) = df.get("local volumes") {
        if sz > 0 {
            out.push(CacheItem {
                id: "docker-dangling-volumes".into(),
                category: CacheCategory::Docker,
                label_key: "cache.item.dockerUnreferencedVolumes".into(),
                label_params: Vec::new(),
                description_key: "cache.desc.dockerUnreferencedVolumes".into(),
                description_params: Vec::new(),
                path: None,
                size_bytes: sz,
                safety: Safety::Low,
                default_select: false,
                action: CacheAction::Docker,
                stale_owner_uid: None,
                stale_canonical_path: None,
                recover_hint: "卷中数据将永久丢失，请确认无重要数据".into(),
            });
        }
    }

    out
}

/// 3 个月（90 天）未使用的 Docker 镜像（非悬空）。
/// 这些镜像是用户拉下来用过、但近期没有容器引用过的 —— 大概率可安全删除。
/// 默认不选中（低风险），让用户自己勾。
fn scan_docker_stale_images() -> Vec<CacheItem> {
    if find_tool("docker").is_none() {
        return vec![];
    }
    let running = tool_command("docker")
        .and_then(|mut c| {
            c.args(["info", "--format", "{{.ServerVersion}}"])
                .output()
                .ok()
        })
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !running {
        return vec![];
    }

    // docker images --format: id|repo|tag|created_at|size
    let out = match tool_command("docker") {
        Some(mut c) => c
            .args([
                "images",
                "--format",
                "{{.ID}}|{{.Repository}}|{{.Tag}}|{{.CreatedSince}}|{{.Size}}",
            ])
            .output(),
        None => return vec![],
    };
    let Ok(out) = out else {
        return vec![];
    };
    let Ok(text) = String::from_utf8(out.stdout) else {
        return vec![];
    };

    // 用 `docker ps -a --format {{.Image}}` 拿被容器引用的 image 集合
    let in_use_raw = tool_command("docker")
        .and_then(|mut c| c.args(["ps", "-a", "--format", "{{.Image}}"]).output().ok())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let in_use_set: std::collections::HashSet<String> =
        in_use_raw.lines().map(|s| s.trim().to_string()).collect();

    let mut stale_size: u64 = 0;
    let mut stale_count: u32 = 0;
    for line in text.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 5 {
            continue;
        }
        let id = parts[0].trim();
        let repo = parts[1].trim();
        let tag = parts[2].trim();
        let created_since = parts[3].trim().to_lowercase(); // e.g. "4 months ago"
        let size_str = parts[4].trim();

        // 跳过悬空镜像（交给 scan_docker 处理）
        if repo == "<none>" && tag == "<none>" {
            continue;
        }
        // 被引用就跳过
        if in_use_set.contains(&format!("{}:{}", repo, tag)) || in_use_set.contains(id) {
            continue;
        }

        // 时间筛选：「X months ago」或「X years ago」且 X 对应天数 > 90
        let days = parse_docker_age(&created_since);
        if days < 90 {
            continue;
        }

        stale_size += parse_human_size(size_str).unwrap_or(0);
        stale_count += 1;
    }

    if stale_count == 0 {
        return vec![];
    }

    vec![CacheItem {
        id: "docker-stale-images".into(),
        category: CacheCategory::Docker,
        label_key: "cache.item.dockerStaleImages".into(),
        label_params: vec![("count".to_owned(), stale_count.to_string())],
        description_key: "cache.desc.dockerStaleImages".into(),
        description_params: Vec::new(),
        path: None,
        size_bytes: stale_size,
        safety: Safety::Low,
        default_select: false,
        action: CacheAction::Docker,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "如需再使用，用 docker pull 重新拉取".into(),
    }]
}

/// 解析 Docker 的 CreatedSince 文本为天数
fn parse_docker_age(s: &str) -> u64 {
    // 格式：`N seconds/minutes/hours/days/weeks/months/years ago`
    let lower = s.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();
    if parts.len() < 2 {
        return 0;
    }
    let n: u64 = parts[0].parse().unwrap_or(0);
    match parts[1] {
        "second" | "seconds" => 0,
        "minute" | "minutes" => 0,
        "hour" | "hours" => 0,
        "day" | "days" => n,
        "week" | "weeks" => n * 7,
        "month" | "months" => n * 30,
        "year" | "years" => n * 365,
        _ => 0,
    }
}

fn scan_stale_node_modules(home: &Path) -> Vec<CacheItem> {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(180 * 24 * 3600))
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut items = Vec::new();
    for root in STALE_PROJECT_ROOTS {
        let root_path = home.join(root);
        if !root_path.exists() {
            continue;
        }
        let walker = WalkDir::new(&root_path)
            .max_depth(5)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                !name.starts_with('.') && name != "node_modules" || entry.depth() == 0
            });
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_dir() || entry.file_name() != "node_modules" {
                continue;
            }
            let Ok(canonical_path) = entry.path().canonicalize() else {
                continue;
            };
            let Ok(metadata) = std::fs::metadata(&canonical_path) else {
                continue;
            };
            if !metadata.is_dir()
                || canonical_path.file_name().and_then(|name| name.to_str()) != Some("node_modules")
            {
                continue;
            }
            let accessed = metadata.accessed().or_else(|_| metadata.modified()).ok();
            if accessed.map(|time| time >= cutoff).unwrap_or(true) {
                continue;
            }
            let size = dir_size(&canonical_path);
            if size < 20 * 1024 * 1024 {
                continue;
            }
            let owner_uid = {
                use std::os::unix::fs::MetadataExt;
                metadata.uid()
            };
            items.push(stale_cache_item(
                canonical_path,
                size,
                owner_uid,
                items.len(),
            ));
        }
    }
    items
}

fn stale_cache_item(path: PathBuf, size: u64, owner_uid: u32, index: usize) -> CacheItem {
    CacheItem {
        id: format!("stale-node-modules-{index}"),
        category: CacheCategory::Npm,
        label_key: "cache.item.staleNodeModules".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.staleNodeModules".into(),
        // 路径走插值参数：key 里绝不拼用户路径
        description_params: vec![("path".to_owned(), path.display().to_string())],
        path: Some(path.display().to_string()),
        size_bytes: size,
        safety: Safety::Low,
        default_select: false,
        action: CacheAction::StaleNodeModules,
        stale_owner_uid: Some(owner_uid),
        stale_canonical_path: Some(path.clone()),
        recover_hint: "删除后需要在对应项目里重新 npm install / pnpm install".into(),
    }
}

fn scan_homebrew() -> Vec<CacheItem> {
    let brew = find_tool("brew");
    if brew.is_none() {
        return vec![];
    }
    if is_any_tool_busy(&["brew"]).is_some() {
        return vec![];
    }
    // 两个可能的缓存位置
    let paths = [
        dirs::home_dir().map(|h| h.join("Library/Caches/Homebrew")),
        Some(PathBuf::from("/opt/homebrew/Library/Homebrew/cache")),
    ];
    let total: u64 = paths
        .iter()
        .filter_map(|p| p.as_ref())
        .map(|p| dir_size(p))
        .sum();
    if total == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "homebrew-cleanup".into(),
        category: CacheCategory::Homebrew,
        label_key: "cache.item.homebrewCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.homebrewCache".into(),
        description_params: Vec::new(),
        path: Some("~/Library/Caches/Homebrew".into()),
        size_bytes: total,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Homebrew,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "已安装的工具完全不受影响，下次 brew install 会重新下载".into(),
    }]
}

fn scan_xcode(home: &Path) -> Vec<CacheItem> {
    // 安全闸门：Xcode 正在跑或 xcodebuild 正在执行 → 完全跳过
    if is_any_tool_busy(&["Xcode", "xcodebuild", "xcrun", "swift-frontend", "clang"]).is_some() {
        return vec![];
    }
    let derived = home.join("Library/Developer/Xcode/DerivedData");
    let archives = home.join("Library/Developer/Xcode/Archives");
    let simulator = home.join("Library/Developer/CoreSimulator/Caches");
    let ios_device = home.join("Library/Developer/Xcode/iOS DeviceSupport");

    let mut out = Vec::new();

    let dsz = dir_size(&derived);
    if dsz > 0 {
        out.push(CacheItem {
            id: "xcode-derived-data".into(),
            category: CacheCategory::Xcode,
            label_key: "cache.item.xcodeDerivedData".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.xcodeDerivedData".into(),
            description_params: Vec::new(),
            path: Some(derived.display().to_string()),
            size_bytes: dsz,
            safety: Safety::Safe,
            default_select: true,
            action: CacheAction::Xcode,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: "Xcode 下次构建会重新生成".into(),
        });
    }

    let ssz = dir_size(&simulator);
    if ssz > 0 {
        out.push(CacheItem {
            id: "xcode-simulator-caches".into(),
            category: CacheCategory::Xcode,
            label_key: "cache.item.xcodeSimulatorCaches".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.xcodeSimulatorCaches".into(),
            description_params: Vec::new(),
            path: Some(simulator.display().to_string()),
            size_bytes: ssz,
            safety: Safety::Safe,
            default_select: true,
            action: CacheAction::Xcode,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: "模拟器重启后自动重建".into(),
        });
    }

    let isz = dir_size(&ios_device);
    if isz > 0 {
        out.push(CacheItem {
            id: "xcode-ios-devicesupport".into(),
            category: CacheCategory::Xcode,
            label_key: "cache.item.xcodeIosDeviceSupport".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.xcodeIosDeviceSupport".into(),
            description_params: Vec::new(),
            path: Some(ios_device.display().to_string()),
            size_bytes: isz,
            safety: Safety::Low,
            default_select: false,
            action: CacheAction::Xcode,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: "下次连真机调试时 Xcode 会自动重新生成".into(),
        });
    }

    let asz = dir_size(&archives);
    if asz > 1024 * 1024 * 1024 {
        // Archives > 1GB 才列出，通常包含发布历史，用户应谨慎
        out.push(CacheItem {
            id: "xcode-archives".into(),
            category: CacheCategory::Xcode,
            label_key: "cache.item.xcodeArchives".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.xcodeArchives".into(),
            description_params: Vec::new(),
            path: Some(archives.display().to_string()),
            size_bytes: asz,
            safety: Safety::Medium,
            default_select: false,
            action: CacheAction::Xcode,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: "无法恢复，清理前请确认不再需要这些存档".into(),
        });
    }

    out
}

fn scan_cocoapods(home: &Path) -> Vec<CacheItem> {
    if is_any_tool_busy(&["pod"]).is_some() {
        return vec![];
    }
    let cache = home.join("Library/Caches/CocoaPods");
    let size = dir_size(&cache);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "cocoapods-cache".into(),
        category: CacheCategory::Cocoapods,
        label_key: "cache.item.cocoapodsCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.cocoapodsCache".into(),
        description_params: Vec::new(),
        path: Some(cache.display().to_string()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Cocoapods,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 pod install 会自动重建".into(),
    }]
}

fn scan_cargo(home: &Path) -> Vec<CacheItem> {
    // 安全闸门：cargo / rustc 正在跑 → 跳过
    if is_any_tool_busy(&["cargo", "rustc", "rustup"]).is_some() {
        return vec![];
    }
    // 只清 registry/cache（.crate 压缩包）—— 这是最安全的
    // 不动 registry/src（解压后的源码，cargo 偶尔会直接读）
    // 不动 git（git 检出的依赖，重新拉取很慢且可能失败）
    let registry_cache = home.join(".cargo/registry/cache");
    let size = dir_size(&registry_cache);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "cargo-registry-cache".into(),
        category: CacheCategory::Cargo,
        label_key: "cache.item.cargoRegistryCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.cargoRegistryCache".into(),
        description_params: Vec::new(),
        path: Some("~/.cargo/registry/cache".into()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Cargo,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 cargo build 会自动重新下载。不影响已解压的源码，项目仍可离线构建"
            .into(),
    }]
}

fn scan_pip(home: &Path) -> Vec<CacheItem> {
    if is_any_tool_busy(&["pip", "pip3"]).is_some() {
        return vec![];
    }
    let cache = home.join("Library/Caches/pip");
    let size = dir_size(&cache);
    if size == 0 {
        return vec![];
    }
    vec![CacheItem {
        id: "pip-cache".into(),
        category: CacheCategory::Pip,
        label_key: "cache.item.pipCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.pipCache".into(),
        description_params: Vec::new(),
        path: Some(cache.display().to_string()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Pip,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 pip install 会自动重新下载".into(),
    }]
}

fn scan_go(home: &Path) -> Vec<CacheItem> {
    if find_tool("go").is_none() {
        return vec![];
    }
    // 安全闸门：go build / go install / go test 正在跑 → 跳过
    if is_any_tool_busy(&["go", "gopls"]).is_some() {
        return vec![];
    }
    let build = home.join("Library/Caches/go-build");
    let size = dir_size(&build);
    if size == 0 {
        return vec![];
    }
    // 只清编译缓存（-cache），不动 modcache
    // modcache 清除后需要重新下载所有模块，对弱网用户风险大
    vec![CacheItem {
        id: "go-build-cache".into(),
        category: CacheCategory::Go,
        label_key: "cache.item.goBuildCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.goBuildCache".into(),
        description_params: Vec::new(),
        path: Some("~/Library/Caches/go-build".into()),
        size_bytes: size,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Go,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "下次 go build 会重新编译，首次慢一些，已下载的 modules 不受影响".into(),
    }]
}

// ========== 普通用户系统垃圾扫描 ==========

/// `~/Library/Caches` 下的开发工具缓存目录。
/// 这些目录不是「某个应用的缓存」，通用文案会误导用户，必须单独说明。
/// 用精确相等匹配而非前缀匹配：前缀会把形似但语义不同的目录（如 `pnpm-store`）
/// 误归类，那是启发式猜测。语义相同的目录逐个显式登记。
///
/// 第二列是 i18n key，不是文案。
const DEV_TOOL_CACHE_DIRS: &[(&str, &str)] = &[
    ("pnpm", "cache.desc.devToolPnpm"),
    ("ms-playwright", "cache.desc.devToolPlaywright"),
    ("ms-playwright-mcp", "cache.desc.devToolPlaywright"),
    ("Yarn", "cache.desc.yarnCache"),
    ("go-build", "cache.desc.devToolGoBuild"),
    ("Homebrew", "cache.desc.devToolHomebrew"),
];

fn dev_tool_cache_description(dir_name: &str) -> Option<&'static str> {
    DEV_TOOL_CACHE_DIRS
        .iter()
        .find(|(name, _)| *name == dir_name)
        .map(|(_, key)| *key)
}

/// 扫描 ~/Library/Caches 下的应用缓存（按应用拆分，排除开发者工具和正在运行的应用）
fn scan_app_caches(home: &Path) -> Vec<CacheItem> {
    let caches_dir = home.join("Library/Caches");
    if !caches_dir.exists() {
        return vec![];
    }
    // 排除已被其他扫描器覆盖的目录
    let skip_contains = [
        "Homebrew",
        "pip",
        "go-build",
        "CocoaPods",
        "Yarn",
        "com.apple.DeveloperTools",
        "org.swift.swiftpm",
    ];
    // 排除 macOS 系统核心缓存（删了可能导致系统异常）
    let skip_prefix = [
        "com.apple.nsurlsessiond",
        "com.apple.Safari", // Safari 缓存删了会丢已打开的标签页状态
        "com.apple.kernel",
        "com.apple.iconservices",
    ];

    // 获取正在运行的应用 bundle id 列表，避免清理正在使用的缓存
    let running_bundles = running_app_bundles();

    let entries = std::fs::read_dir(&caches_dir).ok();
    let Some(entries) = entries else {
        return vec![];
    };

    // 先收集候选目录，再并行算大小：单线程逐个递归遍历 100 个目录是扫描慢的主因
    let candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            !skip_contains.iter().any(|s| name.contains(s))
                && !skip_prefix.iter().any(|s| name.starts_with(s))
        })
        .map(|entry| entry.path())
        .collect();
    let sizes: Vec<u64> = candidates.par_iter().map(|path| dir_size(path)).collect();

    let mut items: Vec<CacheItem> = Vec::new();

    for (entry_path, sz) in candidates.iter().zip(sizes) {
        let name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if sz < 5 * 1024 * 1024 {
            continue; // < 5MB 不值得列出
        }

        // 正在运行的应用 → 标记为低风险且不默认选中
        let is_running = running_bundles.iter().any(|b| name.contains(b));
        let (safety, default_sel) = if is_running {
            (Safety::Low, false)
        } else {
            (Safety::Safe, true)
        };

        let app = friendly_app_name(&name);

        items.push(CacheItem {
            id: format!("app-cache-{}", name),
            category: CacheCategory::System,
            // 「正在运行」用独立 key 而不是插值片段：括号写法与语序都随语言变
            label_key: if is_running {
                "cache.item.appCacheRunning"
            } else {
                "cache.item.appCache"
            }
            .into(),
            label_params: vec![("app".to_owned(), app)],
            description_key: dev_tool_cache_description(&name)
                .unwrap_or("cache.desc.appCache")
                .to_owned(),
            description_params: Vec::new(),
            path: Some(entry_path.display().to_string()),
            size_bytes: sz,
            safety,
            default_select: default_sel,
            action: CacheAction::System,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: "应用下次打开会自动重建缓存".into(),
        });
    }

    // 按大小降序，只保留前 20 个（避免列表太长）
    items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    items.truncate(20);
    items
}

/// 获取正在运行的应用的 bundle identifier 列表
fn running_app_bundles() -> Vec<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut bundles = Vec::new();
    for proc in sys.processes().values() {
        let exe = proc
            .exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        // 从 /Applications/XXX.app/Contents/MacOS/xxx 提取 bundle id
        if exe.contains(".app/Contents/") {
            let name = proc.name().to_string_lossy().to_lowercase();
            bundles.push(name);
        }
    }
    bundles
}

/// 把 bundle id 风格的目录名转成友好的应用名
fn friendly_app_name(cache_dir_name: &str) -> String {
    // "com.google.Chrome" → "Chrome"
    // "com.tencent.xinWeChat" → "xinWeChat"
    // "org.mozilla.firefox" → "firefox"
    if let Some(last) = cache_dir_name.rsplit('.').next() {
        if last.len() > 1 {
            return last.to_string();
        }
    }
    cache_dir_name.to_string()
}

/// 扫描 ~/Library/Logs 下的应用日志
fn scan_app_logs(home: &Path) -> Vec<CacheItem> {
    let logs_dir = home.join("Library/Logs");
    if !logs_dir.exists() {
        return vec![];
    }
    let total = dir_size(&logs_dir);
    if total < 5 * 1024 * 1024 {
        return vec![]; // < 5MB 不值得
    }
    vec![CacheItem {
        id: "system-app-logs".into(),
        category: CacheCategory::System,
        label_key: "cache.item.appLogs".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.appLogs".into(),
        description_params: Vec::new(),
        path: Some("~/Library/Logs".into()),
        size_bytes: total,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::System,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "日志会在应用运行时自动重新生成".into(),
    }]
}

/// 扫描崩溃报告
fn scan_crash_reports(home: &Path) -> Vec<CacheItem> {
    let dirs = [
        home.join("Library/Logs/DiagnosticReports"),
        home.join("Library/Application Support/CrashReporter"),
    ];
    let total: u64 = dirs.iter().map(|d| dir_size(d)).sum();
    if total < 1024 * 1024 {
        return vec![]; // < 1MB 不值得
    }
    vec![CacheItem {
        id: "system-crash-reports".into(),
        category: CacheCategory::System,
        label_key: "cache.item.crashReports".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.crashReports".into(),
        description_params: Vec::new(),
        path: Some("~/Library/Logs/DiagnosticReports".into()),
        size_bytes: total,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::System,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "崩溃报告删除后不影响任何功能".into(),
    }]
}

/// 扫描废纸篓大小
fn scan_trash(home: &Path) -> Vec<CacheItem> {
    let trash = home.join(".Trash");
    if !trash.exists() {
        return vec![];
    }
    let total = dir_size(&trash);
    if total < 10 * 1024 * 1024 {
        return vec![]; // < 10MB 不值得
    }
    vec![CacheItem {
        id: "system-trash".into(),
        category: CacheCategory::System,
        label_key: "cache.item.trash".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.trash".into(),
        description_params: Vec::new(),
        path: Some("~/.Trash".into()),
        size_bytes: total,
        safety: Safety::Safe,
        default_select: false, // 废纸篓默认不选，用户可能还想恢复
        action: CacheAction::System,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: "清空后无法恢复，请确认废纸篓中没有需要的文件".into(),
    }]
}

// ========== 辅助函数 ==========

/// 递归计算目录大小。失败或不存在返回 0。
fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// 通过 `docker system df --format` 获取各类资源的可回收空间
fn docker_system_df() -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    let out = match tool_command("docker") {
        Some(mut c) => c
            .args(["system", "df", "--format", "{{.Type}}\t{{.Reclaimable}}"])
            .output(),
        None => return map,
    };
    let Ok(out) = out else { return map };
    let Ok(text) = String::from_utf8(out.stdout) else {
        return map;
    };
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let kind = parts[0].trim().to_lowercase();
        // Reclaimable 格式: "9.073GB (77%)" 或 "0B (0%)"
        let size_part = parts[1].split('(').next().unwrap_or("").trim();
        if let Some(sz) = parse_human_size(size_part) {
            map.insert(kind, sz);
        }
    }
    map
}

fn parse_human_size(s: &str) -> Option<u64> {
    // 寻找 "123.45 GB" / "234MB" 模式
    let re_chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < re_chars.len() {
        if re_chars[i].is_ascii_digit() {
            // 找数字起点
            let start = i;
            while i < re_chars.len() && (re_chars[i].is_ascii_digit() || re_chars[i] == '.') {
                i += 1;
            }
            let num_str: String = re_chars[start..i].iter().collect();
            // 跳空格
            while i < re_chars.len() && re_chars[i] == ' ' {
                i += 1;
            }
            // 单位
            let unit_start = i;
            while i < re_chars.len() && re_chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let unit: String = re_chars[unit_start..i].iter().collect();
            let mult: u64 = match unit.to_uppercase().as_str() {
                "B" => 1,
                "KB" | "K" => 1024,
                "MB" | "M" => 1024 * 1024,
                "GB" | "G" => 1024 * 1024 * 1024,
                "TB" | "T" => 1024u64.pow(4),
                _ => continue,
            };
            if let Ok(n) = num_str.parse::<f64>() {
                return Some((n * mult as f64) as u64);
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    type CacheScanTestFn = Box<dyn FnOnce(Arc<PathBuf>) -> Vec<CacheItem> + Send>;

    /// 16 个阶段 → 扫描器函数的权威清单（顺序即 `scan()` 阶段表的顺序）。
    ///
    /// 「用户看到的『正在扫描：X』」与「实际执行的扫描器」必须由同一行决定。
    /// 一旦两者错位，用户看到「正在扫描：废纸篓」而机器其实在扫 Xcode 缓存 ——
    /// 这正是本功能唯一的存在理由，所以每个 (名字, 函数) 配对都要被逐对断言。
    ///
    /// 第一列现在是 **i18n key**（`scanStage.*`）而不是中文阶段名：后端只发 key，
    /// 译文在前端词典里。`StageUpdate` 的字段数没变（仍是 stage / state /
    /// item_count / found_bytes），变的只是 `stage` 这个 String 装的内容。
    const EXPECTED_STAGES: &[(&str, &str)] = &[
        ("scanStage.npmCache", "scan_npm"),
        ("scanStage.pnpmCache", "scan_pnpm"),
        ("scanStage.yarnCache", "scan_yarn"),
        ("scanStage.dockerImagesAndContainers", "scan_docker"),
        ("scanStage.dockerUnusedImages", "scan_docker_stale_images"),
        ("scanStage.staleNodeModules", "scan_stale_node_modules"),
        ("scanStage.homebrewCache", "scan_homebrew"),
        ("scanStage.xcodeCache", "scan_xcode"),
        ("scanStage.cocoapodsCache", "scan_cocoapods"),
        ("scanStage.cargoCache", "scan_cargo"),
        ("scanStage.pipCache", "scan_pip"),
        ("scanStage.goModuleCache", "scan_go"),
        ("scanStage.appCache", "scan_app_caches"),
        ("scanStage.appLogs", "scan_app_logs"),
        ("scanStage.crashReports", "scan_crash_reports"),
        ("scanStage.trash", "scan_trash"),
    ];

    /// 16 个阶段名必须全是 `scanStage.*` 的纯 ASCII key。
    ///
    /// 这道门禁挡的是「有人顺手把阶段名改回中文」——那种改动编译得过、
    /// 单元测试（上面那条逐条比对）也会跟着一起改成中文，于是英文用户在进度条上
    /// 又看到中文，而没有任何一道门禁变红。
    #[test]
    fn every_stage_name_is_a_scan_stage_i18n_key() {
        for (name, scanner) in EXPECTED_STAGES {
            assert!(
                name.starts_with("scanStage."),
                "{scanner} 的阶段名必须是 scanStage.* 的 key，实际是 {name:?}",
            );
            assert!(
                name.is_ascii(),
                "{scanner} 的阶段名含非 ASCII 字符（疑似把文案塞进了 stage）：{name:?}",
            );
        }
    }

    /// 截出 `scan()` 里 `let stages: ... = vec![ ... ]` 这段阶段表源码。
    ///
    /// 阶段名 ↔ 扫描器函数是 `Vec<(&str, Box<dyn FnOnce(..)>)>` 这种「闭包 + 名字」
    /// 的形态，运行时拿不到「这个闭包到底包的是哪个函数」。想要一个**任何机器上
    /// 都会触发**的绑定门禁，只能回到源码上核对。事件流侧的行为断言做不到这件事：
    /// 阶段名集合在错位前后完全一样（名字还是那 16 个，只是换了函数），而且一半
    /// 扫描器共用 `CacheCategory::{Docker, Npm, System}`，无法按类别反查。
    fn stage_table_source() -> String {
        let source = include_str!("cache_scanner.rs");
        let start = source
            .find("let stages: Vec<(&'static str, ScanFn)> = vec![")
            .expect("scan() 里的阶段表声明不见了：这条门禁依赖阶段表的书写形式，请同步更新本测试");
        let rest = &source[start..];
        let end = rest
            .find("\n    ];")
            .expect("阶段表没有闭合的 `];`，源码被改坏了");
        rest[..end].to_owned()
    }

    #[test]
    fn stage_table_declares_exactly_the_sixteen_expected_stages() {
        let table = stage_table_source();
        let expected_names: Vec<&str> = EXPECTED_STAGES.iter().map(|(n, _)| *n).collect();

        // 阶段表里出现的所有字符串字面量都是阶段名（闭包里没有别的字面量），
        // 按出现顺序取出来，必须与权威清单逐个一致：改名 / 拼错 / 漏一项 /
        // 多一项 / 顺序乱序，全部都会在这里暴露。
        let parts: Vec<&str> = table.split('"').collect();
        let literals: Vec<&str> = parts.iter().skip(1).step_by(2).copied().collect();
        assert_eq!(
            literals, expected_names,
            "阶段表里的阶段名必须与权威清单逐个一致（含顺序）",
        );
        assert_eq!(
            table.matches("Box::new(").count(),
            EXPECTED_STAGES.len(),
            "阶段表必须恰好有 16 个扫描器",
        );
    }

    #[test]
    fn every_stage_name_is_bound_to_its_own_scanner() {
        let table = stage_table_source();
        for (name, scanner) in EXPECTED_STAGES {
            let needle = format!("\"{name}\"");
            let hits: Vec<usize> = table.match_indices(&needle).map(|(i, _)| i).collect();
            assert_eq!(
                hits.len(),
                1,
                "阶段名 {name:?} 必须在阶段表里恰好出现一次，实际出现 {} 次",
                hits.len(),
            );
            let after = &table[hits[0] + needle.len()..];
            // 窗口截止到下一个字符串字面量，保证不会跨到下一条阶段去匹配
            let window = match after.find('"') {
                Some(next) => &after[..next],
                None => after,
            };
            assert!(
                window.contains(&format!("{scanner}(")),
                "阶段 {name:?} 必须绑定扫描器 {scanner}，阶段表里这一行实际写的是：{window}",
            );
        }
    }

    #[test]
    fn cache_item_json_omits_command() {
        let item = CacheItem {
            id: "npm-cache".into(),
            category: CacheCategory::Npm,
            label_key: "cache.item.npmCache".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.npmCache".into(),
            description_params: Vec::new(),
            path: Some("~/.npm".into()),
            size_bytes: 1,
            safety: Safety::Safe,
            default_select: true,
            action: CacheAction::Npm,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: String::new(),
        };
        let value = serde_json::to_value(&item).unwrap();
        assert!(value.get("command").is_none());
        assert!(value.get("action").is_none());
        assert_eq!(
            value.get("path").and_then(|path| path.as_str()),
            Some("~/.npm")
        );
    }

    #[test]
    fn client_cache_json_with_command_is_rejected() {
        let item = CacheItem {
            id: "npm-cache".into(),
            category: CacheCategory::Npm,
            label_key: "cache.item.npmCache".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.npmCache".into(),
            description_params: Vec::new(),
            path: Some("~/.npm".into()),
            size_bytes: 1,
            safety: Safety::Safe,
            default_select: true,
            action: CacheAction::Npm,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: String::new(),
        };
        let mut value = serde_json::to_value(item).unwrap();
        value["command"] = serde_json::json!("rm -rf /");

        assert!(serde_json::from_value::<CacheItem>(value).is_err());
    }

    #[test]
    fn stale_cache_item_preserves_individual_target() {
        let path = PathBuf::from("/Users/example/Projects/app/node_modules");
        let item = stale_cache_item(path.clone(), 42, 501, 3);

        assert_eq!(item.id, "stale-node-modules-3");
        assert_eq!(item.path.as_deref(), Some(path.to_str().unwrap()));
        assert_eq!(item.stale_canonical_path.as_deref(), Some(path.as_path()));
        assert_eq!(item.action, CacheAction::StaleNodeModules);
        assert!(!item.default_select);
    }

    #[test]
    fn parse_gb() {
        let v = parse_human_size("9.073GB").unwrap();
        // 浮点精度：允许 0.1% 误差
        assert!(v > 9_700_000_000 && v < 9_800_000_000, "got {}", v);
    }

    #[test]
    fn parse_gb_with_space() {
        let v = parse_human_size("9.073 GB").unwrap();
        assert!(v > 9_700_000_000 && v < 9_800_000_000, "got {}", v);
    }

    #[test]
    fn parse_mb() {
        let v = parse_human_size("43.64MB").unwrap();
        assert!(v > 45_000_000 && v < 46_000_000, "got {}", v);
    }

    #[test]
    fn parse_zero_b() {
        assert_eq!(parse_human_size("0B"), Some(0));
    }

    #[test]
    fn parse_kb() {
        assert_eq!(parse_human_size("512KB"), Some(524_288));
    }

    #[test]
    fn parse_with_paren_suffix() {
        let trimmed = "9.073GB (77%)".split('(').next().unwrap().trim();
        let v = parse_human_size(trimmed).unwrap();
        assert!(v > 9_700_000_000 && v < 9_800_000_000, "got {}", v);
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(parse_human_size(""), None);
    }

    #[test]
    fn parse_no_unit_returns_none() {
        assert_eq!(parse_human_size("hello"), None);
    }

    // ── docker_system_df 集成测试（需要 Docker 运行） ──

    #[test]
    fn docker_system_df_returns_data_when_running() {
        // 跳过：Docker 没装或没运行
        let running = std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !running {
            eprintln!("跳过：Docker 未运行");
            return;
        }

        let df = docker_system_df();
        // docker system df 至少应该返回 images / containers / build cache / local volumes
        eprintln!("docker_system_df 结果: {:?}", df);
        assert!(
            !df.is_empty(),
            "Docker 正在运行但 docker_system_df 返回空 HashMap"
        );
        // 至少应该有 images 这个 key
        assert!(
            df.contains_key("images"),
            "缺少 images key，实际 keys: {:?}",
            df.keys().collect::<Vec<_>>()
        );
    }

    // ── docker system df --format 原始输出调试 ──

    #[test]
    fn docker_system_df_raw_output() {
        let running = std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !running {
            eprintln!("跳过：Docker 未运行");
            return;
        }

        let out = std::process::Command::new("docker")
            .args(["system", "df", "--format", "{{.Type}}\t{{.Reclaimable}}"])
            .output()
            .expect("docker system df 执行失败");

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("=== stdout ===\n{}", stdout);
        eprintln!("=== stderr ===\n{}", stderr);
        eprintln!("=== exit code: {:?} ===", out.status.code());

        assert!(
            out.status.success(),
            "docker system df 命令失败: {}",
            stderr
        );
        assert!(
            !stdout.is_empty(),
            "docker system df stdout 为空，stderr: {}",
            stderr
        );
    }

    // ── scan_docker 集成测试 ──

    #[test]
    fn scan_docker_finds_items_when_running() {
        let running = std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !running {
            eprintln!("跳过：Docker 未运行");
            return;
        }

        let items = scan_docker();
        eprintln!("scan_docker 返回 {} 项:", items.len());
        for item in &items {
            eprintln!(
                "  - {} | {} | {} bytes",
                item.id, item.label_key, item.size_bytes
            );
        }
        // Docker 正在运行时，至少应该有构建缓存或镜像可回收
        // （你的机器上有 9GB 镜像 + 3.2GB 构建缓存）
        assert!(
            !items.is_empty(),
            "Docker 正在运行且有缓存，但 scan_docker 返回空"
        );
    }

    // ── scan_docker_stale_images 集成测试 ──

    #[test]
    fn scan_docker_stale_images_does_not_panic() {
        // 不要求有结果，只要求不 panic
        let items = scan_docker_stale_images();
        eprintln!("scan_docker_stale_images 返回 {} 项", items.len());
        for item in &items {
            eprintln!("  - {} | {} bytes", item.label_key, item.size_bytes);
        }
    }

    // ── 完整 scan() 集成测试 ──

    #[test]
    fn full_cache_scan_returns_results() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(scan(None));
        eprintln!(
            "完整缓存扫描: {} 项, 总计 {} bytes",
            result.items.len(),
            result.total_bytes
        );
        for item in &result.items {
            eprintln!(
                "  [{:?}] {} — {} bytes (safety={:?}, default={})",
                item.category, item.label_key, item.size_bytes, item.safety, item.default_select
            );
        }
        // 你的机器上至少有 Cargo 缓存
        assert!(!result.items.is_empty(), "缓存扫描结果为空");
    }

    // ── scan_app_caches 的开发工具目录文案 ──

    #[test]
    fn dev_tool_cache_directories_get_an_accurate_description() {
        assert_eq!(
            dev_tool_cache_description("pnpm"),
            Some("cache.desc.devToolPnpm"),
        );
        assert_eq!(
            dev_tool_cache_description("ms-playwright"),
            Some("cache.desc.devToolPlaywright"),
        );
        // MCP 版 Playwright 同样是浏览器二进制，目录名不同但语义一致，必须显式登记
        assert_eq!(
            dev_tool_cache_description("ms-playwright-mcp"),
            Some("cache.desc.devToolPlaywright"),
        );
        // 词表里不得残留任何可读文案：这一列只能是 key
        for (dir, key) in DEV_TOOL_CACHE_DIRS {
            assert!(
                key.starts_with("cache.desc."),
                "{dir} 的说明必须是 cache.desc.* 的 key，实际是 {key:?}",
            );
        }
    }

    // ── i18n key 门禁：后端只发 key，不发文案 ──

    /// 截出 `#[cfg(test)] mod tests` 之前的源码。
    ///
    /// 测试自身的断言里必然出现中文（权威清单、fixture 名），把它们算进
    /// 「后端不许发中文」的门禁会让门禁永远红。
    fn production_source() -> String {
        let source = include_str!("cache_scanner.rs");
        let cut = source
            .find("#[cfg(test)]")
            .expect("找不到 #[cfg(test)] 模块边界");
        source[..cut].to_owned()
    }

    /// 「label/description 位上只放 key」的门禁。
    ///
    /// 改字段名（`label` → `label_key`）本身就由编译器守住了「不得再塞文案」；
    /// 这道门禁补的是编译器管不到的一半：
    /// 1. key 落在正确命名空间（label 用 `cache.item.*`、description 用 `cache.desc.*`）
    /// 2. key 是纯 ASCII 字面量 —— 想塞中文进去，赋值处当场编译不过
    ///
    /// 逐个 `CacheItem { .. }` 字面量核对，因此在任何机器、任何时刻结果都确定。
    #[test]
    fn cache_items_declare_keys_in_the_right_namespace() {
        let source = production_source();
        let label_keys = field_literals(&source, "label_key");
        let description_keys = field_literals(&source, "description_key");

        assert!(
            label_keys.len() >= 20 && description_keys.len() >= 20,
            "扫描器数量对不上：label_key {} 处、description_key {} 处",
            label_keys.len(),
            description_keys.len(),
        );
        for key in &label_keys {
            assert!(
                is_plain_key(key, "cache.item."),
                "label_key 必须是 cache.item.* 的纯 ASCII key，实际是 {key:?}",
            );
        }
        for key in &description_keys {
            assert!(
                is_plain_key(key, "cache.desc."),
                "description_key 必须是 cache.desc.* 的纯 ASCII key，实际是 {key:?}",
            );
        }
        assert!(
            label_keys.iter().any(|key| key == "cache.item.trash"),
            "废纸篓扫描器必须发出 cache.item.trash，实际发出：{label_keys:?}",
        );
        assert!(
            description_keys.iter().any(|key| key == "cache.desc.trash"),
            "废纸篓扫描器必须发出 cache.desc.trash，实际发出：{description_keys:?}",
        );
    }

    /// 抽出 `字段:` 后面初始化表达式里的全部字符串字面量内容（不含引号）。
    ///
    /// 允许两种写法：裸字面量（绝大多数扫描器）与 `if/else` 选 key
    /// （应用缓存的「正在运行」变体）。字段声明 `pub label_key: String,`
    /// 是唯一被跳过的非字面量出现 —— 它不是赋值。
    fn field_literals(source: &str, field: &str) -> Vec<String> {
        let needle = format!("{field}:");
        let mut out = Vec::new();
        let mut cursor = 0;
        while let Some(offset) = source[cursor..].find(&needle) {
            let start = cursor + offset + needle.len();
            let expr = initializer_expression(source, start).trim().to_owned();
            cursor = start;
            if expr == "String" {
                continue;
            }
            let literals = string_literals(&expr);
            assert!(
                !literals.is_empty(),
                "{field} 的初始化表达式里没有任何字符串字面量：{expr:?} —— 这里塞的不是 key",
            );
            out.extend(literals);
        }
        out
    }

    /// 从 `字段:` 之后取到该字段初始化表达式的结尾（同深度逗号为止）。
    fn initializer_expression(source: &str, from: usize) -> &str {
        let rest = &source[from..];
        let mut depth = 0_usize;
        for (i, c) in rest.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    if depth == 0 {
                        return &rest[..i];
                    }
                    depth -= 1;
                }
                ',' if depth == 0 => return &rest[..i],
                _ => {}
            }
        }
        rest
    }

    /// 依序取出 `expr` 里的字符串字面量内容。引号不成对时停止（源码坏了，交给上层报错）。
    fn string_literals(expr: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = expr;
        while let Some(open) = rest.find('"') {
            let body = &rest[open + 1..];
            let Some(close) = body.find('"') else {
                break;
            };
            out.push(body[..close].to_owned());
            rest = &body[close + 1..];
        }
        out
    }

    /// 判断 `key` 是不是 `prefix + ASCII 段`。
    fn is_plain_key(key: &str, prefix: &str) -> bool {
        let Some(suffix) = key.strip_prefix(prefix) else {
            return false;
        };
        !suffix.is_empty()
            && suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.')
    }

    #[test]
    fn stale_node_modules_label_is_a_key_and_the_target_travels_as_a_param() {
        let path = PathBuf::from("/Users/example/Projects/app/node_modules");
        let item = stale_cache_item(path.clone(), 42, 501, 0);

        assert_eq!(item.label_key, "cache.item.staleNodeModules");
        assert_eq!(item.description_key, "cache.desc.staleNodeModules");
        assert_eq!(
            item.description_params,
            vec![("path".to_owned(), path.display().to_string())]
        );
        assert!(item.label_params.is_empty());
        // 描述里只允许出现参数，不允许出现路径本身
        assert!(
            !item.description_key.contains(&path.display().to_string()),
            "路径必须走插值参数，不能拼进 key",
        );
    }

    #[test]
    fn scanned_items_expose_ascii_keys_end_to_end() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(scan(None));

        for item in &result.items {
            assert!(
                item.label_key.starts_with("cache.item."),
                "{} 的 label_key 非法：{:?}",
                item.id,
                item.label_key,
            );
            assert!(
                item.description_key.starts_with("cache.desc."),
                "{} 的 description_key 非法：{:?}",
                item.id,
                item.description_key,
            );
            for (field, value) in [
                ("label_key", &item.label_key),
                ("description_key", &item.description_key),
            ] {
                assert!(
                    value.is_ascii(),
                    "{} 的 {field} 含非 ASCII 字符（疑似把文案塞进了 key）：{value:?}",
                    item.id,
                );
            }
            for (name, value) in item
                .label_params
                .iter()
                .chain(item.description_params.iter())
            {
                assert!(
                    name.is_ascii(),
                    "{} 的插值参数名必须是 ASCII：{name:?}",
                    item.id,
                );
                let _ = value;
            }
        }
    }

    #[test]
    fn ordinary_app_cache_directories_have_no_special_description() {
        assert_eq!(dev_tool_cache_description("com.google.Chrome"), None);
        assert_eq!(dev_tool_cache_description("Slack"), None);
        // 词表用精确相等匹配：形似但语义不同的目录不得被前缀误伤
        assert_eq!(dev_tool_cache_description("ms-playwright-other"), None);
        assert_eq!(dev_tool_cache_description("pnpm-store"), None);
    }

    // ── 扫描进度上报 ──

    #[test]
    fn cache_scan_reports_every_stage_when_a_sink_is_attached() {
        use std::collections::HashSet;

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_for_sink = seen.clone();
        let sink: crate::scan_progress::ProgressSink = std::sync::Arc::new(move |u| {
            seen_for_sink.lock().unwrap().push(u);
        });

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(scan(Some(sink)));

        let seen = seen.lock().unwrap();
        let running: Vec<&str> = seen
            .iter()
            .filter(|u| u.state == "running")
            .map(|u| u.stage.as_str())
            .collect();
        let done: Vec<&str> = seen
            .iter()
            .filter(|u| u.state == "done")
            .map(|u| u.stage.as_str())
            .collect();

        assert_eq!(running.len(), 16, "16 个扫描器各上报一次开始");
        assert_eq!(done.len(), 16, "16 个扫描器各上报一次完成");

        // 完整 16 名集合，而不是抽样几个名字。
        // 注意：这里只能比「集合」不能比「顺序」——`running` 事件是在
        // `spawn_blocking` 的工作线程里发出的，顺序取决于 OS 线程调度，
        // tokio 并不保证 spawn_blocking 的开始顺序，实测 15 次里有 14 次
        // 与阶段表顺序不一致。阶段名 ↔ 函数的错位由
        // `every_stage_name_is_bound_to_its_own_scanner` 在源码层守住。
        let mut running_sorted = running.clone();
        running_sorted.sort_unstable();
        let mut done_sorted = done.clone();
        done_sorted.sort_unstable();
        let mut expected_names: Vec<&str> = EXPECTED_STAGES.iter().map(|(n, _)| *n).collect();
        expected_names.sort_unstable();

        assert_eq!(
            running_sorted, expected_names,
            "running 上报的阶段名必须恰好是权威清单里的 16 个（不多、不少、不少字）",
        );
        assert_eq!(
            done_sorted, expected_names,
            "done 上报的阶段名必须恰好是权威清单里的 16 个（不多、不少、不少字）",
        );
        // 集合相等 + 各自长度 16 已经能推出「无重复」，但显式钉住：
        // 阶段名重复会让前端进度条把同一个阶段画两遍。
        assert_eq!(
            HashSet::<&str>::from_iter(running.iter().copied()).len(),
            16,
            "running 的阶段名不允许重复：{running:?}",
        );
        assert_eq!(
            HashSet::<&str>::from_iter(done.iter().copied()).len(),
            16,
            "done 的阶段名不允许重复：{done:?}",
        );

        // item_count 与 found_bytes 都必须被验证：前端会把 item_count 渲染成
        // 「已发现 N 项」，错数是用户可见的。
        let sum_found_bytes: u64 = seen
            .iter()
            .filter(|u| u.state == "done")
            .map(|u| u.found_bytes)
            .sum();
        assert_eq!(
            sum_found_bytes, result.total_bytes,
            "各阶段 found_bytes 之和必须等于最终总量",
        );

        // Σ item_count = run_stages 返回的条目数（**过滤前**）。
        // `scan()` 在 run_stages 之后还有一道 `retain(|i| i.size_bytes > 0)`
        // （cache_scanner.rs 的 scan() 内），所以过滤前的数量只可能 >= 最终数量。
        // 两者今天相等，是因为各扫描器自己就先卡了体积阈值
        // （scan_npm/scan_pnpm/… 在 size == 0 时 return vec![]，
        // scan_app_caches 另有 < 5MB 阈值），不会产出零尺寸项，retain 是空操作。
        // 下面把两个关系都钉死：一旦哪天出现零尺寸项，assert_eq 会失败并指出
        // 期望值该改成什么，而不是悄悄放过。
        let sum_item_count: usize = seen
            .iter()
            .filter(|u| u.state == "done")
            .map(|u| u.item_count)
            .sum();
        assert!(
            sum_item_count >= result.items.len(),
            "过滤前的条目数不可能少于过滤后的：Σ item_count={sum_item_count}, items={}",
            result.items.len(),
        );
        assert_eq!(
            sum_item_count,
            result.items.len(),
            "各阶段 item_count 之和必须等于最终条目数（当前 retain 是空操作，见上方注释）",
        );
    }

    #[test]
    fn stage_completion_is_reported_in_real_completion_order() {
        use std::sync::{Arc, Barrier};
        // seen 记录 sink 上报的 done 顺序；executed 记录阶段体真实执行顺序。
        // 两者必须分开：写进同一个 vec 会让「阶段体跑过一次」被误记成两次上报，
        // 断言就失去了区分「真的跑过」与「只是多报了一次」的能力。
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let executed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let gate = Arc::new(Barrier::new(2));

        let slow_gate = gate.clone();
        let slow_seen = executed.clone();
        let fast_seen = executed.clone();
        let stages: Vec<(&'static str, CacheScanTestFn)> = vec![
            (
                "慢阶段",
                Box::new(move |_h| {
                    slow_gate.wait();
                    slow_seen.lock().unwrap().push("慢阶段".into());
                    vec![]
                }),
            ),
            (
                "快阶段",
                Box::new(move |_h| {
                    fast_seen.lock().unwrap().push("快阶段".into());
                    vec![]
                }),
            ),
        ];

        let seen_by_sink = seen.clone();
        let sink: crate::scan_progress::ProgressSink = Arc::new(move |u| {
            if u.state == "done" {
                seen_by_sink.lock().unwrap().push(u.stage);
            }
        });
        let sink_for_task = sink.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            // 必须真正驱动这个 future：交给多线程 runtime 调度，
            // 若只用 spawn_blocking 包一层闭包而不 await，future 永远不会被 poll，
            // 慢阶段的 barrier 就会和主流程互相死等。
            let task = tokio::spawn(async move { run_stages(stages, Some(sink_for_task)).await });
            // 快阶段必须在慢阶段放行之前就完成并上报
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            gate.wait();
            task.await.unwrap();
        });

        let order = seen.lock().unwrap().clone();
        assert_eq!(
            order,
            vec!["快阶段".to_string(), "慢阶段".to_string()],
            "完成顺序必须是真实完成顺序，而不是注册顺序",
        );
        let real_order = executed.lock().unwrap().clone();
        assert_eq!(
            real_order,
            vec!["快阶段".to_string(), "慢阶段".to_string()],
            "快阶段必须在 barrier 放行前就真的执行完，否则上面的顺序断言毫无意义",
        );
    }

    /// 构造一个体积已知、字段齐全的 CacheItem，供受控阶段使用。
    fn fixture_item(id: &str, size: u64) -> CacheItem {
        CacheItem {
            id: id.to_owned(),
            category: CacheCategory::System,
            label_key: "cache.item.appCache".into(),
            label_params: vec![("app".to_owned(), id.to_owned())],
            description_key: "cache.desc.appCache".into(),
            description_params: Vec::new(),
            path: None,
            size_bytes: size,
            safety: Safety::Safe,
            default_select: true,
            action: CacheAction::System,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: String::new(),
        }
    }

    /// 「挂上 sink 不得改变扫描产出」——硬约束 6 的真正守门测试。
    ///
    /// 阶段表是受控的固定闭包，产出只取决于闭包本身，与挂没挂 sink 无关，
    /// 因此这个比较在任何机器、任何时刻都是确定性的。
    ///
    /// 之所以在 `run_stages` 这一层比、而不是拿两次真实 `scan()` 的结果互相比：
    /// 正在运行的 Chrome 会持续往自己的缓存目录里写，两次真实扫描之间
    /// `app-cache-Google` 的体积就会差几十字节，真实扫描之间的比较天然有竞态
    /// （实测两次相隔数秒的 scan 就会命中，`app-cache-Google` 50368963 → 50369011）。
    /// 随机门禁比没有门禁更糟：它会训练团队忽略红色。
    #[test]
    fn attaching_a_sink_does_not_change_what_the_stages_produce() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let make_stages = || -> Vec<(&'static str, CacheScanTestFn)> {
            vec![
                (
                    "阶段甲",
                    Box::new(|_h| vec![fixture_item("a1", 100), fixture_item("a2", 200)]),
                ),
                ("阶段乙", Box::new(|_h| vec![fixture_item("b1", 300)])),
                ("阶段丙", Box::new(|_h| vec![])),
            ]
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let without_sink = rt.block_on(run_stages(make_stages(), None));

        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let calls_for_sink = calls.clone();
        let sink: crate::scan_progress::ProgressSink = std::sync::Arc::new(move |_update| {
            calls_for_sink.fetch_add(1, Ordering::SeqCst);
        });
        let with_sink = rt.block_on(run_stages(make_stages(), Some(sink)));

        assert_eq!(
            calls.load(Ordering::SeqCst),
            6,
            "3 个阶段各上报一次 running + 一次 done",
        );

        let fingerprint = |items: &Vec<CacheItem>| -> Vec<(String, u64)> {
            items
                .iter()
                .map(|item| (item.id.clone(), item.size_bytes))
                .collect()
        };
        assert_eq!(
            fingerprint(&without_sink),
            fingerprint(&with_sink),
            "挂上 sink 不得改变 run_stages 产出的条目（含顺序）",
        );

        // 顺带钉住受控阶段表的期望值，防止上面的比较退化成「两边都错、一样所以相等」
        assert_eq!(without_sink.len(), 3, "3 个阶段共产出 3 个条目");
        assert_eq!(
            without_sink.iter().map(|item| item.size_bytes).sum::<u64>(),
            600,
        );
    }

    /// `scan(None)` 这条路径自身的可证伪不变量。
    ///
    /// 旧写法 `assert!(!items.is_empty() || total_bytes == 0)` 是恒真的：
    /// items 非空时左侧成立，items 为空时 total_bytes（空迭代器求和）必然是 0
    /// 所以右侧成立 —— 任何输入都通过，什么都没守住。
    #[test]
    fn cache_scan_without_a_sink_reports_a_self_consistent_result() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(scan(None));

        assert!(
            result.items.iter().all(|item| item.size_bytes > 0),
            "scan() 里的 retain(|i| i.size_bytes > 0) 之后不应残留零尺寸项",
        );
        assert!(
            result
                .items
                .windows(2)
                .all(|pair| pair[0].size_bytes >= pair[1].size_bytes),
            "结果必须按体积降序排列（sort_by_key(Reverse(size_bytes)) 被改坏了？）",
        );
        assert_eq!(
            result.items.iter().map(|item| item.size_bytes).sum::<u64>(),
            result.total_bytes,
            "total_bytes 必须等于全部条目体积之和",
        );
        let mut ids: Vec<&str> = result.items.iter().map(|item| item.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "条目 id 不允许重复，重复会破坏快照注册");
        assert_ne!(
            result.scanned_at_ms, 0,
            "scanned_at_ms 必须是墙上时钟时间戳",
        );
    }
}

#[cfg(test)]
#[path = "cache_flavor_filter_tests.rs"]
mod cache_flavor_filter_tests;
