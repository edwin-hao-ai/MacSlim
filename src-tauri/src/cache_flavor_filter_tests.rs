//! 按构建形态过滤「清不掉」的缓存条目。
//!
//! 这条不变式的意义：MAS 版**不能**把 `pnpm` / `yarn` / `docker` / `go` 这几类
//! 摆在列表里。沙箱里 exec 不出去，用户勾上点清理只会在不可逆流程的中途拿到一句
//! 「启动失败」。列表是用户对产品能力的判断依据，清不了的选项不如不给。
//!
//! 反过来，**不能**误伤：默认形态下必须一条都不少（这是主产品），pip 也必须留着
//! （`pip_cleanup` 有删目录兜底，沙箱里照样能清）。

use super::{drop_uncleanable, CacheAction};
use crate::cache_scanner::CacheItem;

fn item(action: CacheAction) -> CacheItem {
    let (label_key, description_key) = match action {
        CacheAction::Npm => ("cache.item.npmCache", "cache.desc.npmCache"),
        CacheAction::Pnpm => ("cache.item.pnpmCache", "cache.desc.pnpmCache"),
        CacheAction::Yarn => ("cache.item.yarnCache", "cache.desc.yarnCache"),
        CacheAction::Docker => ("cache.item.docker", "cache.desc.docker"),
        CacheAction::Homebrew => ("cache.item.homebrew", "cache.desc.homebrew"),
        CacheAction::Xcode => ("cache.item.xcode", "cache.desc.xcode"),
        CacheAction::Cocoapods => ("cache.item.cocoapods", "cache.desc.cocoapods"),
        CacheAction::Cargo => ("cache.item.cargo", "cache.desc.cargo"),
        CacheAction::Pip => ("cache.item.pipCache", "cache.desc.pipCache"),
        CacheAction::Go => ("cache.item.goBuildCache", "cache.desc.goBuildCache"),
        CacheAction::System => ("cache.item.system", "cache.desc.system"),
        CacheAction::StaleNodeModules => ("cache.item.staleNodeModules", "cache.desc.stale"),
    };
    CacheItem {
        id: format!("{action:?}"),
        category: crate::cache_scanner::CacheCategory::Npm,
        label_key: label_key.to_string(),
        label_params: Vec::new(),
        description_key: description_key.to_string(),
        description_params: Vec::new(),
        path: Some("/tmp/x".into()),
        size_bytes: 1,
        safety: crate::cache_scanner::Safety::Safe,
        default_select: false,
        action,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    }
}

const ALL: [CacheAction; 12] = [
    CacheAction::Npm,
    CacheAction::Pnpm,
    CacheAction::Yarn,
    CacheAction::Docker,
    CacheAction::Homebrew,
    CacheAction::Xcode,
    CacheAction::Cocoapods,
    CacheAction::Cargo,
    CacheAction::Pip,
    CacheAction::Go,
    CacheAction::System,
    CacheAction::StaleNodeModules,
];

#[test]
fn exactly_four_actions_require_an_external_cli() {
    // 这四个是 MAS 版被砍掉的部分。数量写死：将来新增一个走 CLI 的清理方式时，
    // 这条会先炸，迫使决定它该不该在 MAS 下被过滤掉。
    let external: Vec<CacheAction> = ALL.into_iter().filter(|a| a.needs_external_cli()).collect();
    assert_eq!(
        external,
        [
            CacheAction::Pnpm,
            CacheAction::Yarn,
            CacheAction::Docker,
            CacheAction::Go
        ]
    );
}

#[test]
fn pip_is_not_treated_as_cli_only() {
    // pip 必须留在 MAS 可清理清单里：pip_cleanup 在 CLI 失败时会 fall through 到
    // direct_cleanup（直接删目录）。把它误判成「只能靠 pip CLI」会让 MAS 版
    // 平白少一个能用的清理项。
    assert!(!CacheAction::Pip.needs_external_cli());
}

#[cfg(not(feature = "mas"))]
#[test]
fn developer_id_keeps_every_single_item() {
    // 默认形态是主产品，一条都不能少。少一条就是功能回退。
    let items: Vec<CacheItem> = ALL.into_iter().map(item).collect();
    let kept = drop_uncleanable(items);
    assert_eq!(kept.len(), 12, "全功能版必须保留全部 12 类");
    for action in ALL {
        assert!(action.is_cleanable());
    }
}

#[cfg(feature = "mas")]
#[test]
fn mas_drops_exactly_the_four_cli_only_actions() {
    let items: Vec<CacheItem> = ALL.into_iter().map(item).collect();
    let kept = drop_uncleanable(items);
    let labels: Vec<String> = kept.into_iter().map(|i| i.label_key).collect();

    assert_eq!(
        labels,
        [
            "cache.item.npmCache",
            "cache.item.homebrew",
            "cache.item.xcode",
            "cache.item.cocoapods",
            "cache.item.cargo",
            "cache.item.pipCache",
            "cache.item.system",
            "cache.item.staleNodeModules",
        ],
        "MAS 版应恰好保留 8 类（12 减 4 个依赖 CLI 的）"
    );
    // 精确比对被砍掉的 4 个 key。用子串匹配会误判：Go 的 key 是
    // `cache.item.goBuildCache`，而 `cache.item.cargo` 之类既不含 "go" 也不该
    // 被 "docker" 之类的模式误伤 —— 断言必须打在完整 key 上。
    for dropped in [
        "cache.item.pnpmCache",
        "cache.item.yarnCache",
        "cache.item.docker",
        "cache.item.goBuildCache",
    ] {
        assert!(
            !labels.iter().any(|l| l == dropped),
            "{dropped} 不该出现在 MAS 版列表里"
        );
    }
}

#[cfg(feature = "mas")]
#[test]
fn mas_keeps_pip_and_the_path_based_families() {
    let items: Vec<CacheItem> = ALL.into_iter().map(item).collect();
    let labels: Vec<String> = drop_uncleanable(items)
        .into_iter()
        .map(|i| i.label_key)
        .collect();
    for kept in [
        "cache.item.npmCache",
        "cache.item.pipCache",
        "cache.item.system",
        "cache.item.staleNodeModules",
    ] {
        assert!(labels.iter().any(|l| l == kept), "{kept} 应保留");
    }
}
