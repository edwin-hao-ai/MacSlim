// 残留文件扫描器：根据 Bundle ID 和应用名在 ~/Library/ 中查找关联残留
use crate::app_scanner::dir_size;
use crate::dev_tool_rules::get_dev_tool_rules;
use crate::operations::{OperationStore, ResidueIdentity, SnapshotRegistration};
use crate::residue_policy::{self, LIBRARY_SUBDIRS};
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 残留文件条目
#[derive(Serialize, Clone, Debug)]
pub struct ResidueItem {
    pub path: String,
    pub category: String,
    pub size_bytes: u64,
    pub is_dev_tool: bool,
    pub selected: bool,
    pub selection_key: String,
}

/// 单个应用的残留扫描结果
#[derive(Serialize, Clone, Debug)]
pub struct AppResidue {
    pub bundle_id: String,
    pub app_name: String,
    pub items: Vec<ResidueItem>,
    pub total_bytes: u64,
    pub scan_complete: bool,
}

pub fn register_app_residues(
    store: &mut OperationStore,
    app_key: &str,
    result: &mut AppResidue,
) -> Result<SnapshotRegistration, UserError> {
    register_app_residue_batch(store, vec![(app_key.to_owned(), result)])
}

pub fn register_app_residue_batch(
    store: &mut OperationStore,
    batches: Vec<(String, &mut AppResidue)>,
) -> Result<SnapshotRegistration, UserError> {
    if batches.is_empty() {
        return Err(UserError::new(
            ErrorCode::RESIDUE_BATCH_EMPTY,
            "残留批次不能为空",
        ));
    }
    let payload: Vec<(String, Vec<ResidueIdentity>)> = batches
        .iter()
        .map(|(app_key, result)| (app_key.clone(), residue_identities(app_key, &result.items)))
        .collect();
    let registration = store.register_residues_for_apps(payload)?;
    let mut cursor = 0usize;
    for (_, result) in batches {
        cursor = bind_keys(&mut result.items, &registration.selection_keys, cursor)?;
    }
    if cursor != registration.selection_keys.len() {
        return Err(UserError::new(
            ErrorCode::RESIDUE_SELECTION_COUNT_MISMATCH,
            "残留选择 key 数量与快照不一致",
        ));
    }
    Ok(registration)
}

fn residue_identities(app_key: &str, items: &[ResidueItem]) -> Vec<ResidueIdentity> {
    items
        .iter()
        .map(|item| ResidueIdentity {
            app_key: app_key.to_owned(),
            path: item.path.clone(),
            category: item.category.clone(),
            size_bytes: item.size_bytes,
        })
        .collect()
}

fn bind_keys(
    items: &mut [ResidueItem],
    selection_keys: &[String],
    cursor: usize,
) -> Result<usize, UserError> {
    let end = cursor.saturating_add(items.len());
    let slice = selection_keys
        .get(cursor..end)
        .ok_or_else(|| "残留选择 key 数量与快照不一致".to_owned())?;
    for (item, key) in items.iter_mut().zip(slice) {
        item.selection_key = key.clone();
    }
    Ok(end)
}

/// ~/Library 下各子目录的顶层清单。
/// 批量扫描时只 read_dir 一次，所有 app 复用，避免 N 个 app 重复遍历同一批目录。
pub struct LibraryIndex {
    home: PathBuf,
    dirs: Vec<LibraryDir>,
}

struct LibraryDir {
    category: &'static str,
    entries: Vec<(String, PathBuf)>,
}

impl LibraryIndex {
    pub fn build() -> Option<Self> {
        let home = dirs::home_dir()?;
        let library = home.join("Library");
        if !library.exists() {
            return None;
        }
        let mut dirs = Vec::new();
        for subdir in LIBRARY_SUBDIRS {
            let dir = library.join(subdir);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let entries = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_string(),
                        entry.path(),
                    )
                })
                .collect();
            dirs.push(LibraryDir {
                category: subdir,
                entries,
            });
        }
        Some(Self { home, dirs })
    }
}

/// 扫描指定应用的残留文件（单 app 入口，自行构建目录索引）
pub fn scan_residues(bundle_id: &str, app_name: &str) -> AppResidue {
    let index = LibraryIndex::build();
    scan_residues_with_index(index.as_ref(), bundle_id, app_name)
}

/// 扫描指定应用的残留文件，复用调用方已经建好的 ~/Library 目录索引
pub fn scan_residues_with_index(
    index: Option<&LibraryIndex>,
    bundle_id: &str,
    app_name: &str,
) -> AppResidue {
    let Some(index) = index else {
        return empty_result(bundle_id, app_name, false);
    };
    let scan_complete = !bundle_id.is_empty();
    let name_lower = app_name.to_lowercase();
    let mut items = Vec::new();

    let roots = residue_policy::allowed_roots(bundle_id);

    for dir in &index.dirs {
        scan_directory(
            &dir.entries,
            dir.category,
            bundle_id,
            &name_lower,
            &roots,
            &mut items,
        );
    }

    // 集成开发者工具规则的额外路径
    if scan_complete {
        scan_dev_tool_paths(bundle_id, &index.home, &roots, &mut items);
    }

    let total_bytes = items.iter().map(|i| i.size_bytes).sum();
    AppResidue {
        bundle_id: bundle_id.to_string(),
        app_name: app_name.to_string(),
        items,
        total_bytes,
        scan_complete,
    }
}

/// 在已读好的目录清单里查找匹配的残留
fn scan_directory(
    entries: &[(String, PathBuf)],
    category: &str,
    bundle_id: &str,
    name_lower: &str,
    roots: &residue_policy::ResidueRoots,
    items: &mut Vec<ResidueItem>,
) {
    for (entry_name, path) in entries {
        if !matches_residue(entry_name, bundle_id, name_lower) {
            continue;
        }
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        if residue_policy::ensure_within_roots(&abs_path, roots).is_err() {
            continue;
        }
        let size = compute_entry_size(&abs_path);
        items.push(ResidueItem {
            path: abs_path.to_string_lossy().to_string(),
            category: category.to_string(),
            size_bytes: size,
            is_dev_tool: false,
            selected: true,
            selection_key: String::new(),
        });
    }
}

/// 扫描开发者工具规则定义的额外路径
fn scan_dev_tool_paths(
    bundle_id: &str,
    home: &Path,
    roots: &residue_policy::ResidueRoots,
    items: &mut Vec<ResidueItem>,
) {
    let rules = get_dev_tool_rules(bundle_id);
    for rule in rules {
        for extra in &rule.extra_paths {
            let expanded = expand_tilde(extra, home);
            if !expanded.exists() {
                continue;
            }
            let abs_path = expanded.canonicalize().unwrap_or(expanded);
            let path_str = abs_path.to_string_lossy().to_string();
            // 避免与已扫描的路径重复
            if items.iter().any(|i| i.path == path_str) {
                continue;
            }
            if residue_policy::ensure_within_roots(&abs_path, roots).is_err() {
                continue;
            }
            let size = compute_entry_size(&abs_path);
            items.push(ResidueItem {
                path: path_str,
                category: rule.label.clone(),
                size_bytes: size,
                is_dev_tool: true,
                selected: true,
                selection_key: String::new(),
            });
        }
    }
}

/// 判断条目名称是否匹配 Bundle ID 或应用名称
fn matches_residue(entry_name: &str, bundle_id: &str, name_lower: &str) -> bool {
    // Bundle ID 精确子串匹配
    if !bundle_id.is_empty() && entry_name.contains(bundle_id) {
        return true;
    }
    // 应用名称大小写不敏感匹配
    if !name_lower.is_empty() && entry_name.to_lowercase().contains(name_lower) {
        return true;
    }
    false
}

/// 计算文件或目录大小
fn compute_entry_size(path: &Path) -> u64 {
    if path.is_dir() {
        dir_size(path)
    } else {
        path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

/// 展开路径中的 ~ 为用户主目录
fn expand_tilde(path: &str, home: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path)
    }
}

/// 构造空的扫描结果
fn empty_result(bundle_id: &str, app_name: &str, scan_complete: bool) -> AppResidue {
    AppResidue {
        bundle_id: bundle_id.to_string(),
        app_name: app_name.to_string(),
        items: Vec::new(),
        total_bytes: 0,
        scan_complete,
    }
}

#[cfg(test)]
#[path = "residue_scanner_tests.rs"]
mod tests;
