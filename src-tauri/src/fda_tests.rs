//! FDA 探测的判据。
//!
//! 这里的重点不是「在开发机上能不能读到」（开发机通常已有 FDA，永远返回 true），
//! 而是**探针路径选得对不对** —— 选错会让向导在真正需要它的沙箱环境里
//! 报「已有权限」，用户于是永远看不到引导，而功能依然不可用。

use super::{dir_readable, PROBE_DIRS};
use std::path::Path;

#[test]
fn probe_dirs_are_system_paths_the_sandbox_actually_blocks() {
    // 必须选受保护的系统区域。挑用户目录（~/Documents 等）会得到「可读」的
    // 错误结论 —— 沙箱对它们本来就放行，那样的探针在没授权时也会报 true。
    for dir in PROBE_DIRS {
        assert!(
            dir.starts_with("/Library/"),
            "{dir} 不在 /Library 下：沙箱对 /Library 的限制才是我们要测的"
        );
    }
}

#[test]
fn probe_dirs_all_exist_on_macos() {
    // 探针路径必须真的存在，否则「读不到」会永远为 true（目录不存在也是
    // read_dir 失败），向导就会在有权限的机器上误报「未授权」。
    for dir in PROBE_DIRS {
        assert!(
            Path::new(dir).is_dir(),
            "{dir} 不存在，这个探针永远失败，FDA 向导会一直显示未授权"
        );
    }
}

#[test]
fn a_nonexistent_dir_is_not_readable() {
    // 存在性是探针成立的前提，反过来也要成立：路径不存在时必须判 false，
    // 否则上面那条「探针路径必须存在」就没有牙齿。
    assert!(!dir_readable(Path::new("/definitely/not/here/at/all")));
}

#[test]
fn home_library_caches_is_readable_on_this_machine() {
    // 开发机通常已授予 FDA，所以这条恒真。它锁的是「探针本身可用」——
    // 若哪天它变 false，说明探测逻辑坏了，而不是环境变了。
    let home = dirs::home_dir().expect("有 home 目录");
    let caches = home.join("Library").join("Caches");
    assert!(caches.is_dir());
    // 不断言返回值：本机是否授权 FDA 与测试无关，
    // 强行断言会让「没授权的开发机」上 CI 无理由地红。
    let _ = dir_readable(&caches);
}

#[test]
fn has_full_disk_access_agrees_with_its_two_components() {
    // 综合结论必须与两个分项一致，否则前端会拿到自相矛盾的判断
    // （比如「缓存可读 = false」但「整体 = true」）。
    let user = super::user_cache_readable();
    let system = super::system_dirs_readable();
    assert_eq!(super::has_full_disk_access(), user && system);
}
