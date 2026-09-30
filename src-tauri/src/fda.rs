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
//! 沙箱对**很多**路径仍然可读（`/Applications`、`/Library/Application Support`…），
//! 随便挑一个路径探测只会得到「有权限」的错误结论。而真实 home 下的
//! `~/Library/Caches`、`~/Library/Developer`、`/Library/Caches` 这类才是真正
//! 会被拦的地方。所以探针必须挑**沙箱确实会拦的路径**。
//!
//! ## 第二版修正（2026-09-30）：探针必须拼真实 home
//!
//! 上一版用 `dirs::home_dir()`，而沙箱把 `$HOME` 指向应用自己的 container。
//! 于是探针读到的是 container 里那个空的 Caches：存在、可读、0 条目，
//! 于是报「已授权」—— 而真实缓存一个字节都读不到。实测 MAS 包里
//! `~/Library/Caches` 的 `read_dir` 返回 EPERM（被拦），不是空。
//! 现在探针走 `real_home_for_probe()`，并对「home 被重定向」单独判 false。

use std::path::{Path, PathBuf};

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

/// 探针用的**真实** home（passwd 数据库那条），不是 `$HOME`。
///
/// 沙箱会把 `$HOME` 指向应用自己的 container，而 `dirs::home_dir()` 读的
/// 就是 `$HOME`。拿它拼用户路径，探到的会是 container 里那个空的 Caches：
/// 存在、可读、0 条目 → 报「已授权」，而真实缓存一个字节都读不到。
#[must_use]
pub fn real_home_for_probe() -> Option<PathBuf> {
    crate::sandbox_probe::real_home()
}

/// `$HOME` 是否被沙箱重定向进了自己的 container。
#[must_use]
pub fn home_redirected(home: &Path, real_home: &Path) -> bool {
    crate::sandbox_probe::home_points_at_container(home, real_home)
}

/// 用户级探针：真实 home 下的 `~/Library/Caches` 能否列举。
#[must_use]
pub fn user_cache_readable() -> bool {
    let Some(real) = real_home_for_probe() else {
        return false;
    };
    dir_readable(&real.join("Library").join("Caches"))
}

/// 综合判定。**`home` 被重定向时直接返回 false** —— 哪怕 container 里的
/// 目录确实可列举。
///
/// 这是整条修复的落点：只判「能不能 read_dir」会把「读到了空 container」
/// 误报成「有权限」，于是卡片显示「全部能力可用」而缓存页是空的。
/// 用户被告知一切正常、功能却不能用，比明确报错更糟。
#[must_use]
pub fn full_disk_access_for(home: &Path, real_home: &Path) -> bool {
    if home_redirected(home, real_home) {
        return false;
    }
    dir_readable(&real_home.join("Library").join("Caches")) && system_dirs_readable()
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
    let Some(real) = real_home_for_probe() else {
        return false;
    };
    let home = dirs::home_dir().unwrap_or_else(|| real.clone());
    full_disk_access_for(&home, &real)
}

#[cfg(test)]
#[path = "fda_tests.rs"]
mod tests;
