//! Mac App Store 版的「文件夹访问授权」层。
//!
//! # 为什么需要它
//!
//! 实测（2026-09-30，MAS 包真机，详见 `docs/mas-capability-matrix.md`）：
//! App Sandbox 把 `$HOME` 重定向到应用自己的 container，真实 home 下的
//! `~/Library/Caches` 的 `read_dir` 直接返回 **EPERM** —— 是被拦，不是空的。
//! 于是缓存清理在 App Store 版扫出 0 B，而同一台机器的完整版扫出 13.99 GB。
//!
//! **给多少完全磁盘访问权限都没用。** Apple 文档明说 App Store 应用即使拿到
//! FDA，沙箱仍然强制执行自己的文件限制。
//!
//! # 走哪条路
//!
//! **security-scoped bookmark + `NSOpenPanel`**，这是 Apple 为「沙箱应用持久
//! 访问用户目录」准备的官方机制，也是竞品在用的：
//!
//! - PureSpace（App Store 版）首次用文件选择框授权 `~/Library`，设置里可撤销
//! - CleanMyMac 的 App Store 版至今仍教用户开 FDA，**却能**清 User Cache
//!   —— 说明它多半也是逐目录授权，不是靠 FDA
//!
//! 与「想办法绕过沙箱」的根本区别：前者用户知情、可撤销、审核认可；
//! 后者过不了审，而且 QA1773 明文禁止为访问别的 app 数据申请文件例外。
//!
//! # 三条硬约束
//!
//! 1. 路径一律拼**真实 home**（passwd 那条），不是 `$HOME`。
//! 2. 授权范围只增不减的唯一凭据是 bookmark；扫描根必须落在真实 home 之下，
//!    否则宁可不用（我们要在那条路径下执行删除）。
//! 3. 不做提权、不 exec 外部程序。Apple 明确：不能用 user-selected 授权去
//!    运行 bundle 之外的程序。

#[path = "folder_access_ffi.rs"]
pub mod ffi;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 一个可供用户授权的目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantTarget {
    /// 稳定标识，落盘时用它对上号。
    pub key: &'static str,
    /// 相对**真实 home** 的路径。
    pub relative: &'static str,
    /// 「授权之后能清什么」的 i18n key（纯 ASCII）。**不是文案**。
    pub reason_key: &'static str,
}

/// 可供授权的目录清单。
///
/// 顺序即展示顺序：从「最大最安全」排到「需要用户判断」。不要凭「看起来
/// 可能有用」往里加 —— 每多一项都是多一份要向用户解释的授权。
#[must_use]
pub fn offerable_targets() -> Vec<GrantTarget> {
    vec![
        GrantTarget {
            key: "user_caches",
            relative: "Library/Caches",
            reason_key: "access.target.userCaches",
        },
        GrantTarget {
            key: "user_logs",
            relative: "Library/Logs",
            reason_key: "access.target.userLogs",
        },
        GrantTarget {
            key: "xcode",
            relative: "Library/Developer",
            reason_key: "access.target.xcode",
        },
        GrantTarget {
            key: "npm",
            relative: ".npm",
            reason_key: "access.target.npm",
        },
        GrantTarget {
            key: "cargo",
            relative: ".cargo",
            reason_key: "access.target.cargo",
        },
        GrantTarget {
            key: "trash",
            relative: ".Trash",
            reason_key: "access.target.trash",
        },
    ]
}

/// 授权项 → 真实路径。
#[must_use]
pub fn target_path(target: &GrantTarget, real_home: &Path) -> PathBuf {
    real_home.join(target.relative)
}

/// 一条已授权的目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// 对应 `offerable_targets` 里的 key。
    pub target_key: String,
    /// 授权时的展示名（bundle 名或目录名），纯展示。
    pub display_name: String,
    /// 解析出来的真实路径。
    pub path: PathBuf,
    /// security-scoped bookmark 数据（base64）。跨启动存活靠它。
    pub bookmark: String,
}

/// 某条路径是否落在已授权范围内。
///
/// 用 `Path::starts_with` —— 它按**路径分量**比，所以
/// `~/Library/Caches` 授权**不会**顺带放行 `~/Library/CachesBackup`。
/// 这里放错一个目录，我们就会去删用户没授权过的数据。
#[must_use]
pub fn grant_covers(grants: &[Grant], path: &Path) -> bool {
    grants.iter().any(|grant| path.starts_with(&grant.path))
}

/// 可扫描的根，去重并剔除越界项。
#[must_use]
pub fn scan_roots(grants: &[Grant], real_home: &Path) -> Vec<PathBuf> {
    scan_roots_with_reason(grants, real_home).0
}

/// 扫描根 + 「为什么扫不到」的原因。
///
/// 两者一起返回，是因为「扫不出东西」有两种完全不同的解释：用户没授权，
/// 和这台机器确实干净。前者必须告诉他去授权，否则他会以为应用坏了。
#[must_use]
pub fn scan_roots_with_reason(grants: &[Grant], real_home: &Path) -> (Vec<PathBuf>, RootsReason) {
    let mut roots: Vec<PathBuf> = Vec::new();
    for grant in grants {
        // 书签解析出来的路径理论上不会跑出 home，但数据库被改坏时（用户
        // 手改、备份恢复）不能排除。我们要在那条路径下执行删除，所以
        // 这里必须挡住。
        if !grant.path.starts_with(real_home) {
            continue;
        }
        if !roots.contains(&grant.path) {
            roots.push(grant.path.clone());
        }
    }
    let reason = if roots.is_empty() {
        RootsReason::NoFoldersGranted
    } else {
        RootsReason::Ready
    };
    (roots, reason)
}

/// 为什么扫不到东西。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootsReason {
    /// 一个目录都没授权。
    NoFoldersGranted,
    /// 有授权，可以扫。
    Ready,
}

/// 书签数据看起来是不是像 base64。
///
/// 用来在**读取时**就把明显损坏的记录丢掉，而不是等到扫描时才炸。
/// 典型来源：用户手改配置、从旧版本恢复、剪贴板里粘了半行。
#[must_use]
pub fn bookmark_is_plausible(bookmark: &str) -> bool {
    !bookmark.is_empty()
        && bookmark.len() % 4 == 0
        && bookmark
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

#[cfg(test)]
#[path = "folder_access_tests.rs"]
mod tests;
