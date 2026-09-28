use crate::dev_tool_rules::get_dev_tool_rules;
use crate::user_error::{ErrorCode, UserError};
use std::path::{Path, PathBuf};

pub(crate) const LIBRARY_SUBDIRS: &[&str] = &[
    "Application Support",
    "Caches",
    "Preferences",
    "Logs",
    "Containers",
    "Group Containers",
    "Saved Application State",
    "HTTPStorages",
    "WebKit",
];

#[derive(Default)]
pub(crate) struct ResidueRoots {
    pub(crate) containers: Vec<PathBuf>,
    pub(crate) exact: Vec<PathBuf>,
}

pub(crate) fn allowed_roots(bundle_id: &str) -> ResidueRoots {
    let mut roots = ResidueRoots::default();
    let Some(home) = dirs::home_dir() else {
        return roots;
    };
    for (root, is_container) in library_roots(&home)
        .into_iter()
        .map(|root| (root, true))
        .chain(
            dev_tool_roots(bundle_id, &home)
                .into_iter()
                .map(|root| (root, false)),
        )
    {
        if let Some(canonical) = resolve_canonical(&root) {
            if is_container {
                roots.containers.push(canonical);
            } else {
                roots.exact.push(canonical);
            }
        }
    }
    roots
}

fn library_roots(home: &Path) -> Vec<PathBuf> {
    LIBRARY_SUBDIRS
        .iter()
        .map(|subdir| home.join("Library").join(subdir))
        .collect()
}

fn dev_tool_roots(bundle_id: &str, home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for rule in get_dev_tool_rules(bundle_id) {
        for extra in &rule.extra_paths {
            let Some(rest) = extra.strip_prefix("~/") else {
                continue;
            };
            if has_dotdot(Path::new(rest)) {
                continue;
            }
            roots.push(home.join(rest));
        }
    }
    roots
}

pub(crate) fn ensure_within_roots(path: &Path, roots: &ResidueRoots) -> Result<(), UserError> {
    if !path.is_absolute() || has_dotdot(path) {
        return Err(out_of_scope(path));
    }
    let Some(canonical) = resolve_canonical(path) else {
        return Err(out_of_scope(path));
    };
    let inside_container = roots
        .containers
        .iter()
        .any(|root| is_strictly_under_root(&canonical, root));
    let is_declared_target = roots.exact.iter().any(|root| under_root(&canonical, root));
    if inside_container || is_declared_target {
        return Ok(());
    }
    Err(out_of_scope(path))
}

pub(crate) fn is_strictly_under_root(candidate: &Path, root: &Path) -> bool {
    under_root(candidate, root) && candidate != root
}

pub(crate) fn under_root(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

fn out_of_scope(path: &Path) -> UserError {
    UserError::one(
        ErrorCode::RESIDUE_PATH_OUT_OF_SCOPE,
        format!(
            "残留路径 {} 不在允许的清理范围内，已拒绝",
            path.to_string_lossy()
        ),
        "path",
        path.to_string_lossy(),
    )
}

pub(crate) fn has_dotdot(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

pub(crate) fn resolve_canonical(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = current.canonicalize() {
            let mut resolved = canonical;
            for part in tail.iter().rev() {
                resolved.push(part);
            }
            return Some(resolved);
        }
        tail.push(current.file_name()?.to_os_string());
        current = current.parent()?.to_path_buf();
    }
}

#[cfg(test)]
#[path = "residue_policy_tests.rs"]
mod tests;
