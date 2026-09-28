//! `flavor` 的形状与能力表。
//!
//! 这里测的是**契约**，不是当前构建的取值 —— 因为默认构建永远是
//! `DeveloperId`，测「`CURRENT == DeveloperId`」在 `--features mas` 下
//! 必然失败。所以断言写成两种形态都必须成立的性质，外加一条「默认构建
//! 必须是 DeveloperId」的显式提醒（MAS 才是那个需要单独跑 CI 的例外）。

use super::{Flavor, CURRENT};

#[test]
fn wire_names_match_the_frontend_union() {
    // 前端 `src/lib/flavor.ts` 的 `Flavor` 联合是
    // `"developer_id" | "mas"`。这里逐个钉住，改名会立刻炸。
    assert_eq!(Flavor::DeveloperId.as_str(), "developer_id");
    assert_eq!(Flavor::Mas.as_str(), "mas");
}

#[test]
fn wire_names_are_pure_ascii_snake_case() {
    for flavor in [Flavor::DeveloperId, Flavor::Mas] {
        let wire = flavor.as_str();
        assert!(
            wire.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{wire} 必须是纯 ASCII snake_case，它会直接进 JSON 交给前端"
        );
    }
}

#[test]
fn only_developer_id_can_terminate_processes() {
    // 这是 MAS 版最大的能力缺口，写成反向断言以免将来被「顺手优化」掉。
    assert!(Flavor::DeveloperId.can_terminate_processes());
    assert!(!Flavor::Mas.can_terminate_processes());
}

#[test]
fn only_developer_id_can_exec_external_tools() {
    // 沙箱只能 exec 自带二进制。`plutil` / `sips` / `osascript` / `docker` /
    // `lsof` 全部不可用，这是 MAS 侧剩余改造项存在的唯一原因。
    assert!(Flavor::DeveloperId.can_exec_external_tools());
    assert!(!Flavor::Mas.can_exec_external_tools());
}

#[test]
fn current_matches_the_compiled_feature() {
    let expected = if cfg!(feature = "mas") {
        Flavor::Mas
    } else {
        Flavor::DeveloperId
    };
    assert_eq!(CURRENT, expected);
    // 默认构建（不带 --features）必须是全功能版。带 mas 跑测试时这条依然成立。
    #[cfg(not(feature = "mas"))]
    assert_eq!(CURRENT, Flavor::DeveloperId);
}
