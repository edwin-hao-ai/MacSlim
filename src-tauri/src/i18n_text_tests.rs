//! `i18n_text` 的门禁：后端只发 key，不发文案。

use super::*;

#[test]
fn namespaces_cover_every_backend_slice() {
    // 名单不是装饰：三个片各自的 key 前缀都必须在这里。
    for prefix in [
        "cache.item.",
        "cache.desc.",
        "process.reason.",
        "process.protect.",
        "process.status.",
        "process.name.",
        "opSummary.",
        "scanStage.",
    ] {
        assert!(
            NAMESPACES.contains(&prefix),
            "命名空间清单漏了 {prefix:?}，该片的 key 会逃过 is_plain_key 门禁",
        );
    }
    assert_eq!(
        NAMESPACES.len(),
        8,
        "命名空间清单与上面的权威集合不一致：{:?}",
        NAMESPACES
    );
}

#[test]
fn plain_keys_are_accepted_and_cjk_keys_are_rejected() {
    for good in [
        "process.reason.zombie",
        "process.protect.whitelisted",
        "opSummary.dockerRemoveImage",
        "scanStage.npmCache",
    ] {
        assert!(is_plain_key(good), "{good} 应该是合法 key");
    }
    for bad in [
        "",
        "process.reason.",
        "unknownNamespace.foo",
        "process.reason.僵尸进程",
        "process.reason",
    ] {
        assert!(!is_plain_key(bad), "{bad:?} 不该被当成合法 key");
    }
}

#[test]
fn split_optional_maps_none_to_an_absent_key() {
    let (key, params) = super::split_optional(&None);
    assert!(key.is_none(), "None 必须落成 key = None，不能是空串");
    assert!(params.is_empty());

    let (key, params) = super::split_optional(&Some(super::I18nText::with(
        "process.reason.idle",
        vec![("uptime_min".to_owned(), "42".to_owned())],
    )));
    assert_eq!(key.as_deref(), Some("process.reason.idle"));
    assert_eq!(params, vec![("uptime_min".to_owned(), "42".to_owned())]);
}

#[test]
fn constructors_never_allow_cjk_into_a_key() {
    // `plain` / `with` 只是搬运，不做校验 —— 校验由各模块的源码层门禁 + 前端
    // 词典门禁负责。这里钉住的是「构造出来的 key 一定通过命名空间检查」这一
    // 组合在真实使用中的样子，避免有人误以为构造器会兜底。
    let text = super::I18nText::one("process.reason.idle", "uptime_min", 42_u64);
    assert_ascii_key(&text.key);
    assert_eq!(
        text.params,
        vec![("uptime_min".to_owned(), "42".to_owned())]
    );

    assert_eq!(
        super::I18nText::plain("process.protect.parentOfOthers")
            .params
            .len(),
        0
    );
}
