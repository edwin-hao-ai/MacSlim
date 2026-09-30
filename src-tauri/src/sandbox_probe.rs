//! 沙箱能力探针：**实测**当前构建到底能碰到哪些路径，不靠推断。
//!
//! ## 为什么要专门写这个模块
//!
//! 交接文档里那一整套「MAS 版缓存清理 0 B → 让用户去开 FDA」的推理链，
//! 建立在一个**没验证过的前提**上：认为 `~/Library/Caches` 读不到是被沙箱
//! 拦了。实际上 App Sandbox 做的第一件事是**把进程的 `$HOME` 换成应用自己
//! 的 container**：
//!
//! ```text
//! $HOME = ~/Library/Containers/com.vgoapp.macslim/Data
//! ```
//!
//! 而全代码库用 `dirs::home_dir()` 拼用户路径（`cache_scanner.rs:175`、
//! `cache_cleaner.rs:12`、`fda.rs:39`、`residue_policy.rs:25` …）。于是
//! `~/Library/Caches` 在 MAS 版里解析到**应用自己那个空 container 下的
//! Caches** —— 目录可读、条目 0，扫出 0 B。
//!
//! 也就是说：**这不是权限问题，是路径解析问题。给多少 FDA 都不会变。**
//! 而 FDA 引导卡片会因此把用户领到系统设置里做一个完全无效的操作。
//!
//! 这个模块就是用来把上面这些从「推断」变成「实测」的：跑一次
//! `--probe-sandbox`，就能拿到一份可读的路径可达性报告，MAS 版与完整版
//! 各跑一次，差异一目了然。
//!
//! ## 用法
//!
//! ```bash
//! # 无 GUI、无 Tauri，直接打印 JSON 后退出
//! MacSlim.app/Contents/MacOS/macslim --probe-sandbox
//! ```
//!
//! 沙箱由内核按签名里的 entitlement 施加，跟从 Finder 启动还是从终端启动
//! 无关，所以这样跑出来的结论与 GUI 里一致。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 真实 home 目录（passwd 数据库里的那条），**不是** `$HOME`。
///
/// 沙箱会把 `$HOME` 指向自己的 container，所以 `dirs::home_dir()` 在 MAS
/// 版里给的是 container 路径。凡是要拼「用户的家目录」的地方都必须用这个。
#[must_use]
pub fn real_home() -> Option<PathBuf> {
    // `nix` 的 user feature 已在 Cargo.toml 里启用（process_ops 用得到），
    // 这里复用同一个依赖，不新增。
    let uid = nix::unistd::getuid();
    nix::unistd::User::from_uid(uid)
        .ok()
        .flatten()
        .map(|user| user.dir)
}

/// `$HOME` 是否被沙箱重定向进了应用自己的 container。
///
/// 判定拆成两半，缺一不可：
///
/// 1. `home_env` 必须在 `real_home` 之下 —— 否则 `/Users/别人的
///    Library/Containers/...` 会被误判成本进程的 container；
/// 2. 路径里必须出现 `Library/Containers/<bundleid>` 这一段**路径分量**。
///    按分量比而不是字符串前缀，`Containers-backup` 这种同前缀的兄弟
///    目录才判得对；只看前缀是这里最容易犯的错。
#[must_use]
pub fn home_points_at_container(home_env: &Path, real_home: &Path) -> bool {
    if home_env == real_home || !home_env.starts_with(real_home) {
        return false;
    }
    use std::path::Component;
    home_env
        .components()
        .collect::<Vec<_>>()
        .windows(3)
        .any(|window| {
            window[0] == Component::Normal(std::ffi::OsStr::new("Library"))
                && window[1] == Component::Normal(std::ffi::OsStr::new("Containers"))
                // 第三段必须是 bundle id，不能是又一层 Containers
                && window[2] != Component::Normal(std::ffi::OsStr::new("Containers"))
        })
}

/// 一条路径探针的定义。
#[derive(Debug, Clone, Copy)]
pub struct ProbeSpec {
    /// 稳定标识，报告与测试都按它索引。
    pub key: &'static str,
    /// 中文说明，给人看。
    pub label: &'static str,
    /// 相对真实 home 的路径；`None` 表示绝对路径。
    pub relative: Option<&'static str>,
    /// 绝对路径；给了就忽略 `relative`。
    pub absolute: Option<&'static str>,
    /// 这个路径守护哪项产品能力。**不许为空** —— 不为任何能力服务的
    /// 探针是噪音，会让人误判那个路径重要。
    pub gates: &'static [&'static str],
}

/// 探针清单。覆盖产品真正依赖的路径，不要凭「看起来重要」往里加。
#[must_use]
pub fn probe_catalog() -> Vec<ProbeSpec> {
    vec![
        ProbeSpec {
            key: "own_container",
            label: "应用自己的沙箱 container",
            relative: Some("Library/Containers/com.vgoapp.macslim/Data"),
            absolute: None,
            gates: &["沙箱内可写（探针自身的对照组）"],
        },
        ProbeSpec {
            key: "applications",
            label: "/Applications 应用列表",
            relative: None,
            absolute: Some("/Applications"),
            gates: &["应用列表", "应用卸载", "应用体积分析"],
        },
        ProbeSpec {
            key: "user_applications",
            label: "~/Applications",
            relative: Some("Applications"),
            absolute: None,
            gates: &["应用列表"],
        },
        ProbeSpec {
            key: "user_caches",
            label: "用户缓存 ~/Library/Caches",
            relative: Some("Library/Caches"),
            absolute: None,
            gates: &["缓存清理", "可释放空间"],
        },
        ProbeSpec {
            key: "user_logs",
            label: "用户日志 ~/Library/Logs",
            relative: Some("Library/Logs"),
            absolute: None,
            gates: &["日志清理"],
        },
        ProbeSpec {
            key: "xcode_derived",
            label: "Xcode 编译缓存",
            relative: Some("Library/Developer/Xcode"),
            absolute: None,
            gates: &["开发者缓存清理"],
        },
        ProbeSpec {
            key: "npm_cache",
            label: "npm 缓存 ~/.npm",
            relative: Some(".npm"),
            absolute: None,
            gates: &["npm 缓存清理"],
        },
        ProbeSpec {
            key: "cargo_registry",
            label: "Rust 工具链 ~/.cargo",
            relative: Some(".cargo"),
            absolute: None,
            gates: &["Rust 缓存清理"],
        },
        ProbeSpec {
            key: "trash",
            label: "废纸篓 ~/.Trash",
            relative: Some(".Trash"),
            absolute: None,
            gates: &["清空废纸篓"],
        },
        ProbeSpec {
            key: "downloads",
            label: "下载目录 ~/Downloads",
            relative: Some("Downloads"),
            absolute: None,
            gates: &["过期下载清理"],
        },
        ProbeSpec {
            key: "documents",
            label: "文档 ~/Documents",
            relative: Some("Documents"),
            absolute: None,
            gates: &["大文件扫描"],
        },
        ProbeSpec {
            key: "other_containers",
            label: "其他应用的 container ~/Library/Containers",
            relative: Some("Library/Containers"),
            absolute: None,
            gates: &["应用残留清理"],
        },
        ProbeSpec {
            key: "homebrew",
            label: "Homebrew /usr/local",
            relative: None,
            absolute: Some("/usr/local"),
            gates: &["brew 缓存清理"],
        },
        ProbeSpec {
            key: "system_caches",
            label: "系统缓存 /Library/Caches",
            relative: None,
            absolute: Some("/Library/Caches"),
            gates: &["系统缓存清理"],
        },
        ProbeSpec {
            key: "system_app_support",
            label: "系统配置 /Library/Application Support",
            relative: None,
            absolute: Some("/Library/Application Support"),
            gates: &["系统配置清理"],
        },
    ]
}

/// 把探针定义渲染成**真实 home** 下的绝对路径。
///
/// 传 `real_home` 而不是 `$HOME` 是这个模块的关键：沙箱版里 `$HOME` 是
/// container，用它拼出来的路径会全部指向那个空目录，得到的结论恰好
/// 与事实相反。
#[must_use]
pub fn render_probe_path(spec: &ProbeSpec, real_home: &Path) -> PathBuf {
    match (spec.absolute, spec.relative) {
        (Some(abs), _) => PathBuf::from(abs),
        (None, Some(rel)) => real_home.join(rel),
        (None, None) => real_home.to_path_buf(),
    }
}

/// 一次路径探测的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathProbe {
    pub key: String,
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub listable: bool,
    pub entries: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    /// 路径不存在。这台机器没装对应的东西，**不是**被拦。
    Absent,
    /// 存在但列举不了 —— 沙箱或 TCC 拦住了。
    Blocked,
    /// 能列举。
    Readable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityFinding {
    pub key: String,
    pub label: String,
    pub path: String,
    pub status: ProbeStatus,
    pub entries: Option<usize>,
    pub gates: Vec<String>,
}

/// 把原始探测结果解读成结论。
///
/// 「可读但 0 条目」必须判为 `Readable` 而不是 `Blocked` —— 沙箱重定向
/// 的典型形态就是这个样子，把它当成被拦会得出「必须申请 FDA」的错误结论。
#[must_use]
pub fn interpret(probe: PathProbe) -> CapabilityFinding {
    let status = if !probe.exists {
        ProbeStatus::Absent
    } else if !probe.listable {
        ProbeStatus::Blocked
    } else {
        ProbeStatus::Readable
    };
    let gates = probe_catalog()
        .into_iter()
        .find(|spec| spec.key == probe.key)
        .map(|spec| spec.gates.iter().map(|g| (*g).to_string()).collect())
        .unwrap_or_default();
    CapabilityFinding {
        key: probe.key,
        label: probe.label,
        path: probe.path,
        status,
        entries: probe.entries,
        gates,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReport {
    pub flavor: String,
    pub home_env: Option<String>,
    pub real_home: Option<String>,
    pub home_is_container: bool,
    pub findings: Vec<CapabilityFinding>,
    pub processes: ProcessVisibility,
    pub health_readable: bool,
}

pub struct VerdictInput {
    pub home_is_container: bool,
    pub blocked: Vec<String>,
    pub readable_non_empty: Vec<String>,
}

/// 一句话结论。**必须区分「沙箱把 ~ 换成 container」与「真缺权限」** ——
/// 这两者解法完全相反：前者要改代码拼真实路径，后者去申请授权。
#[must_use]
pub fn verdict(input: VerdictInput) -> String {
    let blocked = input.blocked.join("、");
    if input.home_is_container {
        return format!(
            "$HOME 被沙箱重定向到应用自己的 container —— 用户数据路径解析到了空目录，\
             0 条目是路径问题，不是权限问题，授权也解决不了。受影响：{blocked}。\
             正确解法是用 security-scoped bookmark 拿到真实 home 下的路径。"
        );
    }
    if input.blocked.is_empty() {
        return format!(
            "路径可达性正常，可用：{}",
            input.readable_non_empty.join("、")
        );
    }
    format!("以下路径不可达，需要用户授权：{blocked}")
}

/// 是否要求跑无头探针。
///
/// 单独一个函数是为了能测：argv 分发本身很薄，但它是「实测 vs 推断」这条
/// 链路的入口，拼错一个字符就会静默退回 GUI 流程，让人以为探针没测出东西
/// 其实是压根没跑。
#[must_use]
pub fn probe_requested(args: Vec<String>) -> bool {
    args.iter().any(|arg| arg == "--probe-sandbox")
}

/// 无头探针：打印 JSON 报告后退出。不建 Tauri 应用、不开窗口。
pub fn print_report() {
    let report = run();
    let blocked: Vec<String> = report
        .findings
        .iter()
        .filter(|f| f.status == ProbeStatus::Blocked)
        .map(|f| f.key.clone())
        .collect();
    let readable_non_empty: Vec<String> = report
        .findings
        .iter()
        .filter(|f| f.status == ProbeStatus::Readable && f.entries.unwrap_or(0) > 0)
        .map(|f| f.key.clone())
        .collect();
    let summary = verdict(VerdictInput {
        home_is_container: report.home_is_container,
        blocked,
        readable_non_empty,
    });
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        println!("{json}");
    }
    println!("\n结论：{summary}");
    println!(
        "进程可见性：sysinfo={} / sysctl={:?}（{}）/ 自建快照={} / 能给自己发信号={}",
        report.processes.sysinfo_count,
        report.processes.sysctl_count,
        report
            .processes
            .sysctl_error
            .as_deref()
            .unwrap_or("读取成功"),
        report.processes.snapshot_count,
        report.processes.can_signal_self
    );
    if !report.processes.top_by_memory.is_empty() {
        println!(
            "内存占用最高的三个：{}",
            report
                .processes
                .top_by_memory
                .iter()
                .map(|(name, bytes)| format!("{name}={:.0}MB", *bytes as f64 / 1048576.0))
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    println!(
        "可执行路径：{}/{} 个进程拿得到，样例 {}",
        report.processes.with_exe_path,
        report.processes.snapshot_count,
        report
            .processes
            .exe_samples
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("、")
    );
    println!(
        "文件夹授权能力：{}",
        if report.processes.bookmark_supported {
            format!(
                "security-scoped bookmark 往返成功（{}）—— 「授权目录」这条路可用",
                report.processes.bookmark_path.as_deref().unwrap_or("?")
            )
        } else {
            format!(
                "不可用：{}",
                report
                    .processes
                    .bookmark_error
                    .as_deref()
                    .unwrap_or("未知原因")
            )
        }
    );
    println!("系统健康读数可信：{}", report.health_readable);
}

/// 真跑一遍探测。这是唯一能替代推断的东西。
#[must_use]
pub fn run() -> ProbeReport {
    let home_env = std::env::var_os("HOME").map(PathBuf::from);
    let real_home = real_home();
    let home_is_container = match (&home_env, &real_home) {
        (Some(env_home), Some(real)) => home_points_at_container(env_home, real),
        _ => false,
    };

    let mut findings = Vec::new();
    for spec in probe_catalog() {
        let path = match &real_home {
            Some(real) => render_probe_path(&spec, real),
            None => match spec.absolute {
                Some(abs) => PathBuf::from(abs),
                None => continue,
            },
        };
        findings.push(interpret(probe_one(&spec, &path)));
    }

    ProbeReport {
        flavor: crate::flavor::CURRENT.as_str().to_string(),
        home_env: home_env.map(|p| p.display().to_string()),
        real_home: real_home.map(|p| p.display().to_string()),
        home_is_container,
        findings,
        processes: {
            let mut visibility = process_visibility();
            match bookmark_roundtrip() {
                Ok(path) => {
                    visibility.bookmark_supported = true;
                    visibility.bookmark_path = Some(path.display().to_string());
                }
                Err(reason) => {
                    visibility.bookmark_supported = false;
                    visibility.bookmark_error = Some(reason);
                }
            }
            visibility
        },
        health_readable: health_is_plausible(),
    }
}

fn probe_one(spec: &ProbeSpec, path: &Path) -> PathProbe {
    let exists = path.exists();
    // 只看 read_dir 本身成功与否。受保护目录里读到第一个条目就失败，
    // 那是**条目**的权限而不是目录的列举权限，混在一起会误判。
    let listing = std::fs::read_dir(path);
    let (listable, entries) = match listing {
        Ok(iter) => {
            let count = iter.count();
            (true, Some(count))
        }
        Err(_) => (false, None),
    };
    PathProbe {
        key: spec.key.to_string(),
        label: spec.label.to_string(),
        path: path.display().to_string(),
        exists,
        listable,
        entries,
    }
}

/// 系统健康读数是否可信。
///
/// `read_health` 永远返回一个结构体、不返回 `Result`，所以「能不能读」
/// 只能看数值是否退化。全 0 意味着沙箱把 `sysinfo` / `vm_stat` 也挡了 ——
/// 交接文档记录它返回真实值（CPU 42.3% / 内存 5.2GB），这里把那条结论
/// 变成每次构建都能自动复核的断言。
fn health_is_plausible() -> bool {
    let mut sys = sysinfo::System::new();
    let health = crate::scanner::read_health(&mut sys);
    health.memory_total_mb > 0.0 && health.disk_total_gb > 0.0
}

/// 进程可见性：两条枚举路径各说各话，好过只给一个 0。
///
/// 之所以要两条，是因为交接文档里「进程管理 0 项」只记了现象、没查原因。
/// 现在知道 `sysinfo` 在 MAS 构建下走 `apple-sandbox` 分支，而那个分支是
/// **返回全 0 的桩**（sysinfo 0.33.1 `src/unix/apple/app_store/process.rs`），
/// 所以 0 有可能压根不是沙箱拦的、只是依赖自己交白卷。第二条路
/// `sysctl(KERN_PROC_ALL)` 直接问内核，不经过 sysinfo。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessVisibility {
    pub sysinfo_count: usize,
    pub sysctl_count: Option<usize>,
    pub sysctl_error: Option<String>,
    pub can_signal_self: bool,
    /// 走 `sysctl` + `proc_pidinfo` 这条自己拼的路，能不能拿到明细。
    pub snapshot_count: usize,
    /// 明细里最重的三个进程，按常驻内存降序。只为人工核对用。
    pub top_by_memory: Vec<(String, u64)>,
    /// 快照里有多少个进程拿得到可执行路径。「应用程序」页按 .app bundle
    /// 聚合靠的就是它 —— 拿不到就聚合不出来。
    pub with_exe_path: usize,
    /// 拿到的可执行路径样例，人工核对用。
    pub exe_samples: Vec<String>,
    /// security-scoped bookmark 在沙箱里能不能创建并解析。
    ///
    /// 这是「文件夹访问授权」这条路的**全部技术前提**：entitlement
    /// （`bookmarks.app-scope`）生效、`startAccessingSecurityScopedResource`
    /// 能拿到访问权。在开发机上这一项必然为 true（不在沙箱里），只有 MAS
    /// 包的输出才有意义。
    pub bookmark_supported: bool,
    pub bookmark_path: Option<String>,
    pub bookmark_error: Option<String>,
}

/// 用 `sysctl(KERN_PROC_ALL)` 枚举进程。`None` 表示系统调用本身被拒。
#[must_use]
pub fn sysctl_process_count() -> Option<usize> {
    // KERN_PROC_ALL = 1。先问需要多大缓冲区，再真读。沙箱若拦这个调用，
    // 第二次 sysctl 会以 EPERM 失败（返回 None），而不是「返回 0 条」——
    // 这个区别很重要：0 条说明枚举得到但是空的，None 说明压根不让问。
    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    let needed = sysctl_needed(&mut mib)?;
    if needed == 0 {
        return Some(0);
    }
    let mut buffer = vec![0u8; needed];
    let filled = sysctl_read(&mut mib, &mut buffer)?;
    Some(filled / kinfo_proc_size())
}

/// `struct kinfo_proc` 的大小（字节）。
///
/// libc 没有导出这个类型，只能写死。
///
/// **648 字节**，x86_64 与 arm64 一致（别按「arm64 结构体更小」去猜 152 ——
/// 那是老版本/iOS 的数，拿它算会把进程数放大 4.29 倍）。
///
/// 写错的后果不是编译错误，而是**进程数离谱**，所以这个常量由
/// `kinfo_proc_size_is_not_merely_a_guess` 那条测试守着：它拿 sysctl 的
/// 结果跟 sysinfo 的独立计数对账，偏差超过三成就失败。
fn kinfo_proc_size() -> usize {
    648
}

/// 能不能给自己发一个无害信号。
///
/// 这是**对照项**，恒为真。它恒为真恰恰说明「能给自己发信号」推不出
/// 「能枚举/能终止别的进程」—— 别在别处拿它当权限证据。
#[must_use]
pub fn can_signal_self() -> bool {
    nix::sys::signal::kill(nix::unistd::getpid(), None).is_ok()
}

/// 两条路径的进程可见性。
#[must_use]
pub fn process_visibility() -> ProcessVisibility {
    let sysinfo_count = count_processes();
    let sysctl_count = sysctl_process_count();
    let mut snapshot = crate::process_snapshot::process_snapshot();
    snapshot.sort_by_key(|sample| std::cmp::Reverse(sample.resident_bytes));
    let top_by_memory = snapshot
        .iter()
        .take(3)
        .map(|p| (p.name.clone(), p.resident_bytes))
        .collect();
    ProcessVisibility {
        sysinfo_count,
        sysctl_error: if sysctl_count.is_none() {
            Some("sysctl(KERN_PROC_ALL) 被拒".to_string())
        } else {
            None
        },
        sysctl_count,
        can_signal_self: can_signal_self(),
        snapshot_count: snapshot.len(),
        top_by_memory,
        bookmark_supported: false,
        bookmark_path: None,
        bookmark_error: None,
        with_exe_path: snapshot.iter().filter(|p| p.exe_path.is_some()).count(),
        exe_samples: snapshot
            .iter()
            .filter_map(|p| p.exe_path.clone())
            .take(3)
            .collect(),
    }
}

/// 先问缓冲区要多大。`oldp` 传 null 是「只问大小」的惯用法。
fn sysctl_needed(mib: &mut [libc::c_int]) -> Option<usize> {
    let mut needed: libc::size_t = 0;
    // SAFETY: mib 非空且长度合法；oldp 为 null 时内核只写 newlen，不解引用。
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut needed,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(needed)
}

fn sysctl_read(mib: &mut [libc::c_int], buffer: &mut [u8]) -> Option<usize> {
    let mut len = buffer.len();
    // SAFETY: mib 合法；oldp 指向长度为 len 的已初始化缓冲区，
    // newlen 描述的正是这块缓冲区，newp 为 null 表示不需要写回内核。
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buffer.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(len)
}

/// 在应用**自己 container 里**跑完整 bookmark 往返。
///
/// 选 container 而不是 `/Applications`：container 内的路径本来就带隐式授权，
/// 所以这里测的纯粹是「FFI 声明 + entitlement + 解析 + startAccessing」
/// 这条链路本身。拿 `/Applications` 测会被一个合法现象骗到 ——
/// 那个目录虽然沙箱放行，但**从来没有用户通过文件选择框授权过它**，
/// 于是书签背后没有用户同意，`startAccessing` 拿不到访问权。
/// 那是正确行为，不是 bug。
fn bookmark_roundtrip() -> Result<PathBuf, String> {
    use crate::folder_access::ffi;
    let target = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "拿不到 $HOME".to_string())?
        .join("Documents");
    std::fs::create_dir_all(&target).map_err(|e| format!("准备探针目录失败：{e}"))?;

    let bookmark = ffi::create_bookmark(&target).ok_or_else(|| "创建书签被拒".to_string())?;
    // 二分诊断：同一份书签**不带** security-scope 选项再解析一次。
    // 能解析 → 创建时的掩码写错了，产出的根本不是 scoped 书签；
    // 不能解析 → 问题在沙箱层面（entitlement 未生效 / 缺用户授权）。
    // 注意这里是**排除**而不是**判定**：不带 scope 也能解析，只能说明
    // 「创建时的掩码没写错」（否则带 scope 的那份本来就解析不了）。
    // 它不能推出「所以掩码是对的」—— 缺 startAccessing 的授权也会这样。
    let plain = if ffi::resolve_plain(&bookmark).is_some() {
        "已排除「创建掩码写错」这一原因"
    } else {
        "连不带 scope 的同一书签都解析不了 → 问题比掩码更靠前（书签损坏或 entitlement 未生效）"
    };
    let scope = ffi::resolve_detailed(&bookmark).map_err(|error| {
        let reason = match error {
            ffi::BookmarkError::ResolveFailed => {
                "URLByResolvingBookmarkData 返回 nil：书签损坏、路径已不存在，或 entitlement 未生效"
            }
            ffi::BookmarkError::AccessNotGranted => {
                "解析成功但 startAccessing 返回 false：书签背后没有用户授权"
            }
            ffi::BookmarkError::DecodeFailed => "base64 解码失败",
            _ => "未知",
        };
        format!("卡在 {} —— {reason}（{plain}）", error.i18n_key())
    })?;
    std::fs::read_dir(scope.path())
        .map_err(|_| "拿到作用域但仍读不了目录 —— startAccessing 没生效".to_string())?;
    Ok(PathBuf::from(scope.path()))
}

fn count_processes() -> usize {
    sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_processes(sysinfo::ProcessRefreshKind::nothing()),
    )
    .processes()
    .len()
}

#[cfg(test)]
#[path = "sandbox_probe_tests.rs"]
mod tests;
