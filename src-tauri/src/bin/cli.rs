//! MacSlim CLI —— 与桌面端共享核心逻辑
//!
//! 当前补齐的命令：
//!   macslim-cli --scan
//!   macslim-cli --cache
//!   macslim-cli --disk
//!   macslim-cli --npm
//!   macslim-cli --xcode
//!   macslim-cli --process [--scan]
//!   macslim-cli --list
//!   macslim-cli --docker [--scan]
//!   macslim-cli --history
//!   macslim-cli --whitelist list
//!   macslim-cli --whitelist add <kind> <value> [note]
//!   macslim-cli --whitelist remove <id>

use macslim_lib::cache_cleaner::CleanSummary;
use macslim_lib::cache_scanner::CacheCategory;
use macslim_lib::cli_operations::{
    CacheCleanOutcome, CacheScope, ProcessCleanOutcome, ProcessScanView,
};
use macslim_lib::operations::ProcessKillReport;
use macslim_lib::scanner::Risk;
use macslim_lib::storage::{HistoryEntry, Storage};
use macslim_lib::{run_tauri, scanner_read_health};
use std::env;

enum Command {
    Help,
    Version,
    Scan,
    Cache,
    Disk { scan_only: bool },
    Npm { scan_only: bool },
    Xcode { scan_only: bool },
    Process { scan_only: bool },
    List,
    Docker { scan_only: bool },
    History,
    Whitelist(WhitelistCommand),
}

enum WhitelistCommand {
    List,
    Add {
        kind: String,
        value: String,
        note: String,
    },
    Remove {
        id: i64,
    },
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        run_tauri();
        return;
    }

    let command = match parse_args(&args) {
        Ok(cmd) => cmd,
        Err(err) => {
            eprintln!("{}\n", err);
            print_help();
            std::process::exit(1);
        }
    };

    match command {
        Command::Help => print_help(),
        Command::Version => println!("macslim-cli {}", env!("CARGO_PKG_VERSION")),
        Command::Scan => run_cache_scan(CacheFilter::All, false),
        Command::Cache => run_cache_scan(CacheFilter::All, true),
        Command::Disk { scan_only } => run_cache_scan(CacheFilter::All, !scan_only),
        Command::Npm { scan_only } => run_cache_scan(CacheFilter::NodeFamily, !scan_only),
        Command::Xcode { scan_only } => run_cache_scan(CacheFilter::Xcode, !scan_only),
        Command::Process { scan_only } => run_process(scan_only),
        Command::List => run_process(true),
        Command::Docker { scan_only } => {
            run_cache_scan(CacheFilter::Single(CacheCategory::Docker), !scan_only)
        }
        Command::History => run_history(),
        Command::Whitelist(cmd) => run_whitelist(cmd),
    }
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    let has_scan = args.iter().any(|arg| arg == "--scan");
    match args[0].as_str() {
        "--help" | "-h" => Ok(Command::Help),
        "--version" | "-V" => Ok(Command::Version),
        "--scan" => Ok(Command::Scan),
        "--cache" => Ok(Command::Cache),
        "--disk" => Ok(Command::Disk {
            scan_only: has_scan,
        }),
        "--npm" => Ok(Command::Npm {
            scan_only: has_scan,
        }),
        "--xcode" => Ok(Command::Xcode {
            scan_only: has_scan,
        }),
        "--process" => Ok(Command::Process {
            scan_only: has_scan,
        }),
        "--list" => Ok(Command::List),
        "--docker" => Ok(Command::Docker {
            scan_only: has_scan,
        }),
        "--history" => Ok(Command::History),
        "--whitelist" => parse_whitelist_args(&args[1..]),
        other => Err(format!("未知参数: {other}")),
    }
}

fn parse_whitelist_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::Whitelist(WhitelistCommand::List));
    }

    match args[0].as_str() {
        "list" => Ok(Command::Whitelist(WhitelistCommand::List)),
        "add" => {
            if args.len() < 3 {
                return Err("用法：--whitelist add <kind> <value> [note]".into());
            }
            let kind = args[1].clone();
            let value = args[2].clone();
            let note = if args.len() > 3 {
                args[3..].join(" ")
            } else {
                String::new()
            };
            Ok(Command::Whitelist(WhitelistCommand::Add {
                kind,
                value,
                note,
            }))
        }
        "remove" => {
            if args.len() < 2 {
                return Err("用法：--whitelist remove <id>".into());
            }
            let id = args[1]
                .parse::<i64>()
                .map_err(|_| format!("白名单 id 无效: {}", args[1]))?;
            Ok(Command::Whitelist(WhitelistCommand::Remove { id }))
        }
        other => Err(format!("未知的 whitelist 子命令: {other}")),
    }
}

fn print_help() {
    println!(
        r#"MacSlim CLI {}

用法:
  macslim-cli              打开桌面应用（无参数）
  macslim-cli --scan       扫描全部缓存项，不执行清理
  macslim-cli --cache      清理默认选中的缓存项
  macslim-cli --disk       清理默认选中的硬盘类缓存项
  macslim-cli --disk --scan
                           仅扫描硬盘类缓存项
  macslim-cli --npm        仅扫描/清理 Node 生态缓存（NPM/PNPM/Yarn/node_modules）
  macslim-cli --npm --scan
                           仅扫描 Node 生态缓存
  macslim-cli --xcode      仅扫描/清理 Xcode 缓存
  macslim-cli --xcode --scan
                           仅扫描 Xcode 缓存
  macslim-cli --process    扫描并清理默认选中的进程项
  macslim-cli --process --scan
                           仅扫描进程项
  macslim-cli --list       列出所有可优化进程（等价于 --process --scan）
  macslim-cli --docker     扫描并清理默认选中的 Docker 项
  macslim-cli --docker --scan
                           仅扫描 Docker 项
  macslim-cli --history    查看最近清理历史
  macslim-cli --whitelist list
                           查看白名单
  macslim-cli --whitelist add <kind> <value> [note]
                           添加白名单（kind 通常为 process）
  macslim-cli --whitelist remove <id>
                           删除白名单
  macslim-cli --version    显示版本
  macslim-cli --help       显示帮助"#,
        env!("CARGO_PKG_VERSION")
    );
}

#[derive(Clone)]
enum CacheFilter {
    All,
    NodeFamily,
    Xcode,
    Single(CacheCategory),
}

impl CacheFilter {
    fn scope(&self) -> CacheScope {
        match self {
            Self::All => CacheScope::All,
            Self::NodeFamily => CacheScope::NodeFamily,
            Self::Xcode => CacheScope::Xcode,
            Self::Single(category) => CacheScope::Category(category.clone()),
        }
    }
}

fn cache_outcome(filter: &CacheFilter, clean: bool) -> CacheCleanOutcome {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = if clean {
        rt.block_on(macslim_lib::cli_operations::run_default_cache_clean(
            filter.scope(),
        ))
    } else {
        rt.block_on(macslim_lib::cli_operations::scan_cache(filter.scope()))
            .map(|scan| CacheCleanOutcome {
                summary: CleanSummary {
                    reports: Vec::new(),
                    deleted_bytes: 0,
                    reclaimed_bytes: None,
                    success_count: 0,
                    fail_count: 0,
                },
                default_safe_count: 0,
                scan,
            })
    };
    match result {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("缓存操作失败: {}", err);
            std::process::exit(1);
        }
    }
}

fn run_cache_scan(filter: CacheFilter, clean: bool) {
    print_header();
    print_health();

    let outcome = cache_outcome(&filter, clean);
    if outcome.scan.items.is_empty() {
        println!("没有匹配的可清理项。");
        return;
    }

    print_cache_items(&outcome.scan.items);

    if !clean {
        println!("当前为仅扫描模式。");
        return;
    }

    if outcome.default_safe_count == 0 {
        println!("没有默认选中的安全项可清理。");
        return;
    }

    println!("开始清理 {} 项默认安全项...", outcome.default_safe_count);
    let deleted = format_bytes(outcome.summary.deleted_bytes);
    let reclaimed = match outcome.summary.reclaimed_bytes {
        Some(bytes) => format_bytes(bytes),
        None => "无法测量".to_owned(),
    };
    println!(
        "完成：成功 {} 项，失败 {} 项，已删除 {}，实测释放 {}",
        outcome.summary.success_count, outcome.summary.fail_count, deleted, reclaimed
    );
    for report in outcome.summary.reports {
        let status = if report.success { "OK" } else { "FAIL" };
        // CLI 还没有 locale 层，先原样打印 i18n key（桌面端由前端词典翻译）。
        println!(
            "  [{}] {} · {} ms",
            status, report.label_key, report.duration_ms
        );
        if let Some(error) = report.error {
            println!("       {}", error);
        }
    }
}

fn print_cache_items(items: &[macslim_lib::cache_scanner::CacheItem]) {
    let total_gb =
        items.iter().map(|item| item.size_bytes).sum::<u64>() as f64 / 1024.0 / 1024.0 / 1024.0;
    println!("发现 {} 项，共计 {:.2} GB", items.len(), total_gb);
    println!("------------------------------------------");
    for item in items {
        let size_mb = item.size_bytes as f64 / 1024.0 / 1024.0;
        let mark = if item.default_select { "[*]" } else { "[ ]" };
        // 同上：CLI 直出 i18n key 与插值参数，不自己拼文案
        println!(
            "{} {:>8.0} MB  {} {:?}",
            mark, size_mb, item.label_key, item.label_params
        );
        println!(
            "       风险: {} · {} {:?}",
            safety_label(&item.safety),
            item.description_key,
            item.description_params
        );
    }
    println!();
}

fn process_outcome(scan_only: bool) -> ProcessCleanOutcome {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = if scan_only {
        rt.block_on(macslim_lib::cli_operations::scan_processes())
            .map(|scan| ProcessCleanOutcome {
                default_safe_count: 0,
                report: ProcessKillReport::default(),
                scan,
            })
    } else {
        rt.block_on(macslim_lib::cli_operations::run_default_process_clean())
    };
    match result {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("进程操作失败: {}", err);
            std::process::exit(1);
        }
    }
}

fn run_process(scan_only: bool) {
    print_header();
    print_health();

    let outcome = process_outcome(scan_only);
    if outcome.scan.items.is_empty() {
        println!("没有匹配的进程优化项。");
        return;
    }

    print_process_items(&outcome.scan);

    if scan_only {
        println!("当前为仅扫描模式。");
        return;
    }

    if outcome.default_safe_count == 0 {
        println!("没有默认选中的安全进程可终止。");
        return;
    }

    println!("开始处理 {} 个默认安全进程...", outcome.default_safe_count);
    let mut success = 0usize;
    let mut failed = 0usize;
    for detail in outcome.report.details.iter() {
        if detail.success {
            success += 1;
        } else {
            failed += 1;
        }
        println!(
            "  [{}] {} (PID {}) · {}",
            if detail.success { "OK" } else { "FAIL" },
            detail.name,
            detail.pid,
            detail.message
        );
    }

    println!("完成：成功 {} 个，失败 {} 个。", success, failed);
}

fn print_process_items(scan: &ProcessScanView) {
    println!("发现 {} 个进程优化项", scan.items.len());
    println!("------------------------------------------");
    for process in &scan.items {
        let mark = if process.default_select { "[*]" } else { "[ ]" };
        println!(
            "{} PID {:>6}  {:>6.0} MB  {:>5.1}%  {}",
            mark, process.pid, process.memory_mb, process.cpu_percent, process.name
        );
        println!(
            "       风险: {} · {}",
            risk_label(&process.risk),
            // CLI 保持中文输出（AGENTS.md §0），但分类理由的译文只存在于前端
            // 词典里，后端不再持有中文 —— 这里打印 key + 参数以便排查。
            describe_text(&process.reason_key, &process.reason_params)
        );
    }
    println!();
}

fn run_whitelist(command: WhitelistCommand) {
    let storage = match Storage::open() {
        Ok(storage) => storage,
        Err(err) => {
            eprintln!("打开存储失败: {}", err);
            std::process::exit(1);
        }
    };

    match command {
        WhitelistCommand::List => match storage.list_whitelist() {
            Ok(entries) if entries.is_empty() => println!("白名单为空。"),
            Ok(entries) => {
                println!("当前白名单：");
                for entry in entries {
                    println!(
                        "  [{}] {} = {}{}",
                        entry.id,
                        entry.kind,
                        entry.value,
                        if entry.note.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", entry.note)
                        }
                    );
                }
            }
            Err(err) => {
                eprintln!("读取白名单失败: {}", err);
                std::process::exit(1);
            }
        },
        WhitelistCommand::Add { kind, value, note } => {
            if let Err(err) = storage.add_whitelist(&kind, &value, &note) {
                eprintln!("添加白名单失败: {}", err);
                std::process::exit(1);
            }
            println!("已添加白名单：{} = {}", kind, value);
        }
        WhitelistCommand::Remove { id } => {
            if let Err(err) = storage.remove_whitelist(id) {
                eprintln!("删除白名单失败: {}", err);
                std::process::exit(1);
            }
            println!("已删除白名单项 {}", id);
        }
    }
}

fn run_history() {
    let storage = match Storage::open() {
        Ok(storage) => storage,
        Err(err) => {
            eprintln!("打开存储失败: {}", err);
            std::process::exit(1);
        }
    };

    match storage.recent_history(30) {
        Ok(entries) if entries.is_empty() => println!("还没有历史记录。"),
        Ok(entries) => print_history_entries(&entries),
        Err(err) => {
            eprintln!("读取历史失败: {}", err);
            std::process::exit(1);
        }
    }
}

fn print_header() {
    println!("MacSlim CLI v{}", env!("CARGO_PKG_VERSION"));
    println!("==========================================");
}

fn print_health() {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    let h = scanner_read_health(&mut sys);
    println!(
        "系统状态: CPU {:>4.1}%   内存 {:>4.1}%   磁盘 {:>4.1}%",
        h.cpu_percent, h.memory_percent, h.disk_percent
    );
    println!();
}

fn print_history_entries(entries: &[HistoryEntry]) {
    print_header();
    println!("最近 {} 条历史：", entries.len());
    println!("------------------------------------------");
    for entry in entries {
        println!(
            "[{}] {} · {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            history_operation_label(&entry.operation),
            entry.target
        );
        println!(
            "       {}{}",
            if entry.success { "成功" } else { "失败" },
            if entry.freed_bytes > 0 {
                format!(" · 释放 {}", format_bytes(entry.freed_bytes))
            } else {
                String::new()
            }
        );
        if !entry.detail.is_empty() {
            println!("       {}", entry.detail);
        }
    }
}

fn safety_label(safety: &macslim_lib::cache_scanner::Safety) -> &'static str {
    match safety {
        macslim_lib::cache_scanner::Safety::Safe => "安全",
        macslim_lib::cache_scanner::Safety::Low => "低风险",
        macslim_lib::cache_scanner::Safety::Medium => "谨慎",
    }
}

fn history_operation_label(op: &str) -> &'static str {
    match op {
        "process" | "process_kill" => "进程清理",
        "cache" | "cache_clean" => "缓存清理",
        "uninstall" | "app_uninstall" => "应用卸载",
        "app_terminate" | "app_quit" | "app_force_quit" => "强制退出应用",
        "app_graceful_quit" => "应用优雅退出",
        "docker" => "Docker 清理",
        _ => "未知操作",
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// 打印「i18n key + 插值参数」。
///
/// 后端只持有 key（译文在 GUI 的 `src/i18n/*.ts` 里），CLI 想要中文就得自己维护
/// 一份译文表 —— 那正是本项目禁止的「Rust 侧另抄一份译文」。所以 CLI 打印
/// `key(param=value, …)` 形式：可读、可 grep、且永远不会与 GUI 界面对不上。
fn describe_text(key: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return key.to_owned();
    }
    let rendered: Vec<String> = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{key}({})", rendered.join(", "))
}

fn risk_label(risk: &Risk) -> &'static str {
    match risk {
        Risk::Safe => "安全",
        Risk::Low => "低风险",
        Risk::Dev => "开发者确认",
        Risk::Hidden => "隐藏",
    }
}

#[cfg(test)]
mod cli_args_tests {
    use super::{parse_args, CacheFilter, Command, WhitelistCommand};
    use macslim_lib::cache_scanner::CacheCategory;
    use macslim_lib::cli_operations::CacheScope;

    fn arg(value: &str) -> Vec<String> {
        vec![value.to_owned()]
    }

    #[test]
    fn rejects_every_raw_target_parameter() {
        for flag in [
            "--pid",
            "--pids",
            "--path",
            "--paths",
            "--target",
            "--targets",
            "--bundle",
            "--bundle-path",
            "--app",
            "--item",
            "--items",
            "--kill",
            "--force",
            "--key",
        ] {
            assert!(parse_args(&arg(flag)).is_err(), "{flag} 必须被拒绝");
        }
    }

    #[test]
    fn keeps_the_existing_scan_only_semantics() {
        assert!(matches!(
            parse_args(&arg("--scan")).expect("--scan 可解析"),
            Command::Scan
        ));
        assert!(matches!(
            parse_args(&arg("--list")).expect("--list 可解析"),
            Command::List
        ));
        for flag in ["--disk", "--npm", "--xcode", "--process", "--docker"] {
            let mut args = arg(flag);
            assert!(parse_args(&args).is_ok(), "{flag} 必须可解析");
            args.push("--scan".to_owned());
            let scan_only = match parse_args(&args).expect("子命令可解析") {
                Command::Disk { scan_only }
                | Command::Npm { scan_only }
                | Command::Xcode { scan_only }
                | Command::Process { scan_only }
                | Command::Docker { scan_only } => scan_only,
                _ => panic!("{flag} 落到了非预期分支"),
            };
            assert!(scan_only, "{flag} --scan 必须是仅扫描");
        }
    }

    #[test]
    fn maps_each_flag_onto_a_broker_scope() {
        assert!(matches!(CacheFilter::All.scope(), CacheScope::All));
        assert!(matches!(
            CacheFilter::NodeFamily.scope(),
            CacheScope::NodeFamily
        ));
        assert!(matches!(CacheFilter::Xcode.scope(), CacheScope::Xcode));
        assert!(matches!(
            CacheFilter::Single(CacheCategory::Docker).scope(),
            CacheScope::Category(CacheCategory::Docker)
        ));
    }

    #[test]
    fn whitelist_remove_only_accepts_a_numeric_id() {
        assert!(parse_args(&[
            "--whitelist".to_owned(),
            "remove".to_owned(),
            "not-a-number".to_owned(),
        ])
        .is_err());
        assert!(matches!(
            parse_args(&[
                "--whitelist".to_owned(),
                "remove".to_owned(),
                "7".to_owned(),
            ])
            .expect("数字 id 可解析"),
            Command::Whitelist(WhitelistCommand::Remove { id: 7 })
        ));
    }
}
