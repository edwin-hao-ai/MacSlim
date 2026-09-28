//! 后端下发给前端的「待翻译文案」载体 —— 只装 key 与插值参数，**不装文案**。
//!
//! 约定（与 `CacheItem` 的 `label_key` / `label_params` 同一套）：
//!
//! 1. `key` 必须是**纯 ASCII**，形如 `process.reason.zombie`。
//!    译文只存在于 `src/i18n/*.ts`，后端任何一侧都不得再抄一份中文或英文。
//! 2. 插值参数名必须是纯 ASCII；值是**要显示的内容本身**（数字、路径、端口列表），
//!    不是文案。
//! 3. DTO 上暴露的字段名一律带 `_key` 后缀（`reason_key` / `protected_reason_key` …），
//!    让「这里应该是 key 而不是文案」在字段名上就显然。
//!
//! DTO 用**扁平的两个字段**（`key` + `params`）而不是嵌套 `I18nText`，
//! 是为了和 `CacheItem` 的 JSON 形状保持一致；`I18nText` 只在模块内部搬运时使用。
//!
//! 铁律：**`I18nText` 里的内容不参与任何安全判定。**
//! 白名单、风险等级、默认选中、selection key、TTL、单次消费、owner 绑定
//! 全部只看枚举 / 布尔 / 随机 hex，与文案零关系。
//! 唯一例外要特别小心：拼装句子时不要把它塞进 `ProcessIdentity.name` ——
//! `revalidate_targets` 会拿那个字段和现场 `proc.name()` 逐字比较。

use serde::Serialize;

/// 后端 i18n 命名空间清单。key 必须落在其中之一，否则前端词典里查不到。
///
/// 这些前缀是**字面量清单**：第 1 片（cache）、第 2 片（process）、第 3 片
/// （opSummary / scanStage）各往里加过一批。新增命名空间时**必须**同时在这里
/// 追加，否则那一批 key 会静默逃过 `is_plain_key` 门禁。
pub const NAMESPACES: &[&str] = &[
    "cache.item.",
    "cache.desc.",
    "process.reason.",
    "process.protect.",
    "process.status.",
    "process.name.",
    "opSummary.",
    "scanStage.",
];

/// 判断 `key` 是不是「某命名空间 + ASCII 段」。
pub fn is_plain_key(key: &str) -> bool {
    NAMESPACES.iter().any(|prefix| {
        key.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.')
        })
    })
}

/// key 必须是纯 ASCII 且落在已知命名空间里。
///
/// 「纯 ASCII」这一条挡的是最阴险的失败模式：有人把中文文案直接写进
/// `reason_key`，编译照过、序列化照发，只有英文用户能看到。
pub fn assert_ascii_key(key: &str) {
    assert!(
        key.is_ascii(),
        "i18n key 必须是纯 ASCII（疑似把文案塞进了 key 位）：{key:?}"
    );
    assert!(
        is_plain_key(key),
        "i18n key 必须落在已知命名空间 {NAMESPACES:?} 里：{key:?}"
    );
}

/// 后端插值参数的形状：`[参数名, 参数值]` 二元组数组。
pub type I18nParams = Vec<(String, String)>;

/// 内部搬运用的「key + 参数」。跨模块传递比两个平行字段更不容易错配。
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct I18nText {
    pub key: String,
    pub params: I18nParams,
}

impl I18nText {
    /// 无插值参数的文案。
    pub fn plain(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            params: Vec::new(),
        }
    }

    /// 带插值参数的文案。参数顺序即 DTO 里的顺序，便于逐字比对。
    pub fn with(key: &str, params: I18nParams) -> Self {
        Self {
            key: key.to_owned(),
            params,
        }
    }

    /// 单参数快捷构造。
    pub fn one(key: &str, name: &str, value: impl ToString) -> Self {
        Self::with(key, vec![(name.to_owned(), value.to_string())])
    }
}

/// `Option<I18nText>` → DTO 上的 `(key, params)` 两个扁平字段。
///
/// `None` 必须落成 `key = None` + `params = []`：这样「没有受保护原因」在前端是
/// 「没有 key」，而不是「key 是空串」——后者会让 `t()` 渲染出一个诡异的空文案。
pub fn split_optional(text: &Option<I18nText>) -> (Option<String>, I18nParams) {
    match text {
        None => (None, Vec::new()),
        Some(text) => (Some(text.key.clone()), text.params.clone()),
    }
}

/// 测试用：抽出一份源码里属于 `prefix` 命名空间的字符串字面量。
///
/// 只取双引号包起来的内容，所以文档注释里用反引号写的 `` `process.reason.x` ``
/// 不会被误当成「后端在发这枚 key」——这一点很关键：抽取结果要能当「后端发过的
/// key 集合」用，反向门禁（孤儿检测）才成立。
///
/// 为什么要先切掉 `#[cfg(test)]`：测试自身的断言里必然出现 key 字面量，
/// 把它们算进「后端在发」会让「某处改成硬编码中文后 key 消失」这件事被盖掉。
#[cfg(test)]
pub(crate) fn key_literals_before_tests(source: &str, prefix: &str) -> Vec<String> {
    // 找不到 `#[cfg(test)]` 说明这个文件根本没有内联测试模块（例如
    // `operation_registry.rs`），整份源码就是生产代码。
    let production = match source.find("#[cfg(test)]") {
        Some(cut) => &source[..cut],
        None => source,
    };

    let mut out = Vec::new();
    let mut rest = production;
    while let Some(open) = rest.find('"') {
        let body = &rest[open + 1..];
        let Some(close) = body.find('"') else {
            break;
        };
        let literal = &body[..close];
        if literal.starts_with(prefix) {
            out.push(literal.to_owned());
        }
        rest = &body[close + 1..];
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
#[path = "i18n_text_tests.rs"]
mod tests;
