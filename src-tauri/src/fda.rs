//! 完全磁盘访问权限（Full Disk Access, FDA）探测。
//!
//! ## 为什么必须有它
//!
//! App Sandbox 下即使带了 `com.apple.security.files.all`，**用户没在系统设置里
//! 实际授权时，敏感路径一律读不到**。实测（2026-09-29，MAS 包真机）：
//! 缓存页显示「没有发现可清理的缓存」，而同一台机器的完整版扫出 13.99 GB。
//! 进程列表同样为 0。
//!
//! 参考：CleanMyMac 官方文档说明它的 App Store 版要读硬盘 SMART 也必须
//! 用户授予 FDA —— 这是同一类限制，不是我们独有。
//!
//! ## 为什么不能用「读一个已知文件」来判断
//!
//! 沙箱对**很多**路径仍然可读（用户目录、文档、下载…），随便挑一个路径探测
//! 只会得到「有权限」的错误结论。而 `~/Library/Developer`（Xcode）、`/Library/`
//! 这类系统区域才是真正会被拦的地方。所以探针必须挑**沙箱确实会拦的路径**。

use std::path::Path;

/// 探针路径：受保护的 macOS 目录，非 FDA 下沙箱读不到。
///
/// 选 `/Library/Application Support/com.apple.TCC/TCC.db` 附近的目录而不是
/// 某个具体文件 —— 具体文件可能不存在（判断不了），而**目录能否列举**是
/// 稳定的权限信号。`Library/Preferences` 与 `Library/Application Support`
/// 在未授权 FDA 时都不可列举。
const PROBE_DIRS: [&str; 3] = [
    "/Library/Application Support",
    "/Library/Preferences",
    "/Library/Caches",
];

/// 用户级探针：`~/Library/Caches` 的父目录是否可列举。
///
/// 这一条与前三条不同：它决定**缓存清理**能不能扫到东西。MAS 版实测卡在
/// 这里，所以单独列出来，前端可以按「哪一类能力被挡」给不同的引导文案。
#[must_use]
pub fn user_cache_readable() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    dir_readable(&home.join("Library").join("Caches"))
}

/// 系统级探针：没有它则进程枚举、SMART 等一律不可用。
#[must_use]
pub fn system_dirs_readable() -> bool {
    PROBE_DIRS.iter().all(|dir| dir_readable(Path::new(dir)))
}

/// 能不能列举某个目录的内容。
///
/// 用 `read_dir` 而不是 `metadata`：沙箱允许你看到目录存在、却拒绝列举
/// 里面的条目，这正是被拦时的表现。
fn dir_readable(path: &Path) -> bool {
    // 只要 `read_dir` 本身成功就说明能列举 —— 不去看首个子项，因为受保护目录
    // 里读到第一个条目会失败，那反映的是**条目**的权限，不是目录的列举权限，
    // 两者混在一起会让判断在「目录可列举但首项不可读」时误报为未授权。
    std::fs::read_dir(path).is_ok()
}

/// 综合结论。只给前端一个布尔值，具体是哪一类被挡由前端按需再查。
#[must_use]
pub fn has_full_disk_access() -> bool {
    user_cache_readable() && system_dirs_readable()
}

#[cfg(test)]
#[path = "fda_tests.rs"]
mod tests;
