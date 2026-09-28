/// macOS 系统核心进程白名单 —— 永远不扫描、不终止。
/// 这个列表保守地覆盖 macOS 13-15 Sequoia 的核心服务。
/// 参见 CLAUDE.md §4.4。
pub const SYSTEM_CORE_NAMES: &[&str] = &[
    // 内核与 launchd
    "kernel_task",
    "launchd",
    "logd",
    "syslogd",
    "UserEventAgent",
    "mds",
    "mds_stores",
    "mdworker",
    "mdworker_shared",
    // 窗口 / 图形 / 输入
    "WindowServer",
    "Dock",
    "Finder",
    "SystemUIServer",
    "loginwindow",
    "ControlCenter",
    "NotificationCenter",
    "Spotlight",
    // 安全与沙箱
    "securityd",
    "trustd",
    "sandboxd",
    "tccd",
    "opendirectoryd",
    "cfprefsd",
    "containermanagerd",
    "runningboardd",
    // 网络
    "mDNSResponder",
    "configd",
    "networkd",
    "symptomsd",
    "apsd",
    // 电源 / 硬件
    "powerd",
    "bluetoothd",
    "coreaudiod",
    "hidd",
    "usbd",
    // Apple 服务
    "nsurlsessiond",
    "cloudd",
    "bird",
    "akd",
    "amfid",
    "airportd",
    // Claude Code 自己 / Terminal / IDE 主进程（避免自杀）
    "Terminal",
    "iTerm2",
    "Claude",
    "ClaudeCode",
    // Tauri / MacSlim 自身
    "macslim",
    "MacSlim",
];

/// 用户自定义白名单（从 SQLite 读）—— 现在 stub 返回空，后续接入持久化。
pub fn user_whitelist() -> Vec<String> {
    Vec::new()
}

pub fn is_system_core(name: &str) -> bool {
    SYSTEM_CORE_NAMES
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
}

pub fn is_whitelisted(name: &str) -> bool {
    if is_system_core(name) {
        return true;
    }
    user_whitelist()
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
}
