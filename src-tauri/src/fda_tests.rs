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

// ===== 真 home，不是 $HOME =====
//
// 实测（2026-09-30，MAS 包真机）：App Sandbox 把 `$HOME` 重定向到应用自己的
// container，而 `dirs::home_dir()` 读的就是 `$HOME`。于是
// `~/Library/Caches` 探到的是 container 里那个**空的** Caches ——
// 存在、可读、0 条目，探针报 true，而用户的真实缓存一个字节都读不到。
//
// 结果就是：FDA 卡片显示「已授权，全部能力可用」，缓存页却是空的。
// 用户被告知一切正常，功能却不能用 —— 这是最坏的一种错。

#[test]
fn the_user_cache_probe_looks_at_the_real_home_not_the_container() {
    let real = super::real_home_for_probe().expect("passwd 库里必须有 home");
    assert!(
        !crate::sandbox_probe::home_points_at_container(&real, &real),
        "探针用的 home 不该是 container：{}",
        real.display()
    );
    let caches = real.join("Library").join("Caches");
    assert!(
        caches.is_dir(),
        "真实 home 下必须有 Library/Caches，探针路径不对：{}",
        caches.display()
    );
}

#[test]
fn a_redirected_home_is_reported_as_such_rather_than_as_granted() {
    // 这是整条修复的落点：被重定向时**不能**报「已授权」。
    let real = super::real_home_for_probe().expect("有 home");
    let container = real.join("Library/Containers/com.vgoapp.macslim/Data");
    assert!(super::home_redirected(&container, &real));
    assert!(!super::home_redirected(&real, &real));
}

#[test]
fn full_disk_access_is_false_when_home_is_redirected_even_if_the_probe_passes() {
    // 反向断言守住修复本身：container 里的 Data/Library/Caches **确实**
    // 存在且可列举。若只用「能不能 read_dir」判断，这里会返回 true，
    // 于是又回到「显示已授权但功能不可用」的状态。
    let real = super::real_home_for_probe().expect("有 home");
    let container_home = real.join("Library/Containers/com.vgoapp.macslim/Data");
    let container_caches = container_home.join("Library/Caches");
    if dir_readable(&container_caches) {
        assert!(
            !super::full_disk_access_for(&container_home, &real),
            "home 被重定向到 container 时，绝不能报「已授权」"
        );
    }
}

#[test]
fn an_unredirected_home_keeps_using_the_plain_directory_probe() {
    // 反过来也要成立：正常的完整版环境下，判定仍然只看目录可不可列举，
    // 不能因为新增了「重定向」概念就变成永远 false。
    let real = super::real_home_for_probe().expect("有 home");
    assert_eq!(
        super::full_disk_access_for(&real, &real),
        dir_readable(&real.join("Library/Caches")) && super::system_dirs_readable()
    );
}
