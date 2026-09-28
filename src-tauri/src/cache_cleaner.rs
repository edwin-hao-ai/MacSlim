use crate::cache_scanner::{is_any_tool_busy, CacheItem, STALE_PROJECT_ROOTS};
use crate::operations::CacheAction;
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

fn allowed_cleanup_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".npm"));
        out.push(home.join(".cargo/registry/cache"));
        out.push(home.join(".Trash"));
        out.push(home.join("Library/pnpm/store"));
        out.push(home.join("Library/Caches"));
        out.push(home.join("Library/Logs"));
        out.push(home.join("Library/Developer/Xcode/DerivedData"));
        out.push(home.join("Library/Developer/Xcode/Archives"));
        out.push(home.join("Library/Developer/Xcode/iOS DeviceSupport"));
        out.push(home.join("Library/Developer/CoreSimulator/Caches"));
        out.push(home.join("Library/Application Support/CrashReporter"));
    }
    out.push(PathBuf::from("/opt/homebrew/Library/Homebrew/cache"));
    out
}
fn is_cleanup_path_allowed(path: &Path) -> bool {
    let canon = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let dangerous_literal = [
        "/",
        "/usr",
        "/etc",
        "/var",
        "/bin",
        "/sbin",
        "/System",
        "/Library",
        "/Applications",
        "/private",
    ];
    if let Some(s) = canon.to_str() {
        if dangerous_literal.contains(&s) {
            return false;
        }
        if dirs::home_dir().is_some_and(|home| Path::new(s) == home) {
            return false;
        }
    }
    allowed_cleanup_roots().into_iter().any(|root| {
        root.canonicalize()
            .map(|root_canon| canon.starts_with(root_canon))
            .unwrap_or_else(|_| canon.starts_with(root))
    })
}
fn busy_check_for(item: &CacheItem) -> Option<String> {
    let tools: &[&str] = match item.action {
        CacheAction::Npm | CacheAction::StaleNodeModules => &["npm", "npx"],
        CacheAction::Pnpm => &["pnpm"],
        CacheAction::Yarn => &["yarn"],
        CacheAction::Docker => &[],
        CacheAction::Homebrew => &["brew"],
        CacheAction::Xcode => &["Xcode", "xcodebuild", "xcrun", "swift-frontend", "clang"],
        CacheAction::Cocoapods => &["pod"],
        CacheAction::Cargo => &["cargo", "rustc", "rustup"],
        CacheAction::Pip => &["pip", "pip3"],
        CacheAction::Go => &["go", "gopls"],
        CacheAction::System => &[],
    };
    (!tools.is_empty())
        .then(|| is_any_tool_busy(tools))
        .flatten()
}
#[derive(Serialize, Clone, Debug)]
pub struct CleanReport {
    pub id: String,
    /// i18n key + 插值参数：与 `CacheItem` 同一套约定，前端负责翻译。
    pub label_key: String,
    pub label_params: Vec<(String, String)>,
    pub success: bool,
    pub freed_bytes: u64,
    pub duration_ms: u64,
    /// 失败原因：带 `code` 的结构化错误（`error` 命名空间），不是裸中文串。
    pub error: Option<UserError>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CleanSummary {
    pub reports: Vec<CleanReport>,
    pub total_freed_bytes: u64,
    pub success_count: usize,
    pub fail_count: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheCommand {
    PnpmStorePrune,
    YarnCacheClean,
    DockerImagePrune,
    DockerBuilderPrune,
    DockerContainerPrune,
    DockerVolumePrune,
    DockerStaleImagePrune,
    GoCleanCache,
}

impl CacheCommand {
    fn shell(self) -> &'static str {
        match self {
            Self::PnpmStorePrune => "pnpm store prune",
            Self::YarnCacheClean => "yarn cache clean",
            Self::DockerImagePrune => "docker image prune -a -f",
            Self::DockerBuilderPrune => "docker builder prune -f",
            Self::DockerContainerPrune => "docker container prune -f",
            Self::DockerVolumePrune => "docker volume prune -f",
            Self::DockerStaleImagePrune => "docker image prune -a --force --filter \"until=2160h\"",
            Self::GoCleanCache => "go clean -cache",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheRequest {
    Remove { path: PathBuf, sudo: bool },
    Command(CacheCommand),
    Pip(CacheItem),
}

pub(crate) trait CacheRuntime: Send + Sync {
    fn is_busy(&self, item: &CacheItem) -> Option<String>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, UserError>;
    fn is_path_allowed(&self, path: &Path) -> bool;
    fn is_root_owned(&self, path: &Path) -> bool;
    fn owner_uid(&self, path: &Path) -> Result<u32, UserError>;
    fn is_symlink(&self, path: &Path) -> Result<bool, UserError>;
    fn is_directory(&self, path: &Path) -> Result<bool, UserError>;
    fn execute<'a>(
        &'a self,
        request: CacheRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserError>> + Send + 'a>>;
}

struct DomainRuntime;

impl CacheRuntime for DomainRuntime {
    fn is_busy(&self, item: &CacheItem) -> Option<String> {
        busy_check_for(item)
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, UserError> {
        path.canonicalize().map_err(|error| {
            UserError::one(
                ErrorCode::CACHE_ITEM_MISSING_PATH,
                format!("路径无法访问: {error}"),
                "reason",
                error,
            )
        })
    }

    fn is_path_allowed(&self, path: &Path) -> bool {
        is_cleanup_path_allowed(path)
    }

    fn is_root_owned(&self, path: &Path) -> bool {
        self.owner_uid(path).map(|uid| uid == 0).unwrap_or(false)
    }

    fn owner_uid(&self, path: &Path) -> Result<u32, UserError> {
        path.metadata()
            .map(|metadata| {
                use std::os::unix::fs::MetadataExt;
                metadata.uid()
            })
            .map_err(|error| {
                UserError::one(
                    ErrorCode::CACHE_ANCESTOR_CHECK_FAILED,
                    format!("无法读取路径所有权: {error}"),
                    "reason",
                    error,
                )
            })
    }

    fn is_symlink(&self, path: &Path) -> Result<bool, UserError> {
        std::fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .map_err(|error| {
                UserError::one(
                    ErrorCode::REFUSE_SYMLINK,
                    format!("无法检查路径类型: {error}"),
                    "reason",
                    error,
                )
            })
    }

    fn is_directory(&self, path: &Path) -> Result<bool, UserError> {
        std::fs::metadata(path)
            .map(|metadata| metadata.is_dir())
            .map_err(|error| {
                UserError::one(
                    ErrorCode::CACHE_ANCESTOR_CHECK_FAILED,
                    format!("无法检查路径目录: {error}"),
                    "reason",
                    error,
                )
            })
    }

    fn execute<'a>(
        &'a self,
        request: CacheRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserError>> + Send + 'a>> {
        Box::pin(async move {
            match request {
                CacheRequest::Remove { path, sudo } if sudo => remove_directory_sudo(&path).await,
                CacheRequest::Remove { path, .. } => remove_directory(&path).await,
                CacheRequest::Command(command) => run_shell_command(command.shell()).await,
                CacheRequest::Pip(_) => clean_pip_cache().await,
            }
        })
    }
}

pub(crate) async fn clean_snapshot_items(items: Vec<CacheItem>) -> CleanSummary {
    let runtime = DomainRuntime;
    clean_with_runtime(items, &runtime).await
}

pub(crate) async fn clean_with_runtime<R: CacheRuntime + ?Sized>(
    items: Vec<CacheItem>,
    runtime: &R,
) -> CleanSummary {
    let mut reports = Vec::with_capacity(items.len());
    for item in items {
        let start = Instant::now();
        let result = clean_item(&item, runtime).await;
        let (success, error) = match result {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error)),
        };
        reports.push(CleanReport {
            id: item.id.clone(),
            label_key: item.label_key.clone(),
            label_params: item.label_params.clone(),
            success,
            freed_bytes: if success { item.size_bytes } else { 0 },
            duration_ms: start.elapsed().as_millis() as u64,
            error,
        });
    }
    let total_freed_bytes = reports.iter().map(|report| report.freed_bytes).sum();
    let success_count = reports.iter().filter(|report| report.success).count();
    CleanSummary {
        fail_count: reports.len() - success_count,
        reports,
        total_freed_bytes,
        success_count,
    }
}

/// 三条反复出现的清理拒绝理由，抽成函数让 `ErrorCode` 只有一处权威定义。
fn cache_item_missing_path() -> UserError {
    UserError::new(ErrorCode::CACHE_ITEM_MISSING_PATH, "缓存项没有清理路径")
}

fn path_not_whitelisted(path: &Path) -> UserError {
    UserError::one(
        ErrorCode::PATH_NOT_WHITELISTED,
        format!("路径不在白名单内，拒绝删除: {}", path.display()),
        "path",
        path.display(),
    )
}

fn pre_delete_recheck_failed(path: &Path) -> UserError {
    UserError::one(
        ErrorCode::PRE_DELETE_RECHECK_FAILED,
        format!("执行前二次校验失败，拒绝删除: {}", path.display()),
        "path",
        path.display(),
    )
}

async fn clean_item<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(), UserError> {
    if let Some(busy) = runtime.is_busy(item) {
        return Err(UserError::one(
            ErrorCode::CACHE_BUSY_APP_SKIPPED,
            format!("检测到 {busy} 正在运行，已跳过清理以防止损坏"),
            "app",
            busy,
        ));
    }
    match item.action {
        CacheAction::Npm
        | CacheAction::Homebrew
        | CacheAction::Xcode
        | CacheAction::Cocoapods
        | CacheAction::Cargo
        | CacheAction::System => direct_cleanup(item, runtime).await,
        CacheAction::Pnpm => {
            command_cleanup(item, runtime, CacheCommand::PnpmStorePrune, true).await
        }
        CacheAction::Yarn => {
            command_cleanup(item, runtime, CacheCommand::YarnCacheClean, true).await
        }
        CacheAction::Go => command_cleanup(item, runtime, CacheCommand::GoCleanCache, false).await,
        CacheAction::Docker => docker_cleanup(item, runtime).await,
        CacheAction::Pip => pip_cleanup(item, runtime).await,
        CacheAction::StaleNodeModules => stale_cleanup(item, runtime).await,
    }
}

async fn direct_cleanup<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(), UserError> {
    let path = validated_path(item, runtime)?;
    let sudo = runtime.is_root_owned(&path);
    runtime.execute(CacheRequest::Remove { path, sudo }).await
}

async fn command_cleanup<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
    command: CacheCommand,
    fallback: bool,
) -> Result<(), UserError> {
    let result = runtime.execute(CacheRequest::Command(command)).await;
    if result.is_ok() || !fallback || item.path.is_none() {
        return result;
    }
    direct_cleanup(item, runtime).await
}

async fn docker_cleanup<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(), UserError> {
    let command = match item.id.as_str() {
        "docker-images-reclaimable" => CacheCommand::DockerImagePrune,
        "docker-builder-cache" => CacheCommand::DockerBuilderPrune,
        "docker-stopped-containers" => CacheCommand::DockerContainerPrune,
        "docker-dangling-volumes" => CacheCommand::DockerVolumePrune,
        "docker-stale-images" => CacheCommand::DockerStaleImagePrune,
        _ => {
            return Err(UserError::new(
                ErrorCode::CACHE_DOCKER_ACTION_MISMATCH,
                "Docker 缓存操作类型不匹配",
            ))
        }
    };
    runtime.execute(CacheRequest::Command(command)).await
}

async fn pip_cleanup<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(), UserError> {
    let result = runtime.execute(CacheRequest::Pip(item.clone())).await;
    if result.is_ok() || item.path.is_none() {
        return result;
    }
    direct_cleanup(item, runtime).await
}

async fn stale_cleanup<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(), UserError> {
    let (path, owner_uid) = validated_stale_path(item, runtime)?;
    runtime
        .execute(CacheRequest::Remove {
            path,
            sudo: owner_uid == 0,
        })
        .await
}

fn validated_stale_path<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<(PathBuf, u32), UserError> {
    let raw = item.path.as_deref().ok_or_else(cache_item_missing_path)?;
    let expanded = expand_tilde(raw);
    if has_symlink_ancestor(&expanded)? {
        return Err(UserError::new(
            ErrorCode::REFUSE_SYMLINK_ANCESTOR,
            "拒绝清理包含符号链接祖先的路径",
        ));
    }
    if runtime.is_symlink(&expanded)? {
        return Err(UserError::new(
            ErrorCode::REFUSE_SYMLINK,
            "拒绝清理符号链接",
        ));
    }
    if !runtime.is_directory(&expanded)? {
        return Err(UserError::new(
            ErrorCode::STALE_PATH_NOT_DIR,
            "stale node_modules 路径不是目录",
        ));
    }
    let snapshot_path = item.stale_canonical_path.as_ref().ok_or_else(|| {
        UserError::new(
            ErrorCode::STALE_MISSING_CANONICAL,
            "stale 缓存项缺少 canonical path 快照",
        )
    })?;
    let path = runtime.canonicalize(&expanded)?;
    if path != *snapshot_path {
        return Err(UserError::new(
            ErrorCode::STALE_PATH_CHANGED,
            "stale 路径已变化",
        ));
    }
    if runtime.is_symlink(&path)? || !runtime.is_directory(&path)? {
        return Err(UserError::new(
            ErrorCode::STALE_PATH_KIND_INVALID,
            "stale node_modules 路径类型无效",
        ));
    }
    if path.file_name().and_then(|name| name.to_str()) != Some("node_modules") {
        return Err(UserError::new(
            ErrorCode::STALE_PATH_NOT_NODE_MODULES,
            "stale 路径必须指向 node_modules",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        UserError::new(ErrorCode::STALE_PATH_MISSING_PARENT, "stale 路径缺少父目录")
    })?;
    if !is_stale_project_path(parent) {
        return Err(UserError::new(
            ErrorCode::STALE_PATH_OUTSIDE_PROJECTS,
            "stale 路径不在允许的项目目录内",
        ));
    }
    let expected = item.stale_owner_uid.ok_or_else(|| {
        UserError::new(
            ErrorCode::STALE_MISSING_OWNERSHIP,
            "stale 缓存项缺少所有权快照",
        )
    })?;
    let current = runtime.owner_uid(&path)?;
    if current != expected {
        return Err(UserError::new(
            ErrorCode::CACHE_OWNERSHIP_CHANGED,
            "缓存项所有权已变化",
        ));
    }
    Ok((path, current))
}

fn is_stale_project_path(path: &Path) -> bool {
    stale_project_root(path).is_some()
}

fn stale_project_root(path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    STALE_PROJECT_ROOTS
        .iter()
        .map(|root| home.join(root))
        .find(|root| path_starts_with_ignore_case(path, root))
}

fn has_symlink_ancestor(path: &Path) -> Result<bool, UserError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(UserError::one(
                    ErrorCode::CACHE_ANCESTOR_CHECK_FAILED,
                    format!("无法检查路径祖先: {error}"),
                    "reason",
                    error,
                ))
            }
        }
    }
    Ok(false)
}

fn path_starts_with_ignore_case(path: &Path, root: &Path) -> bool {
    let path_parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let root_parts: Vec<String> = root
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    path_parts.len() >= root_parts.len()
        && path_parts
            .iter()
            .zip(root_parts.iter())
            .all(|(path_part, root_part)| path_part == root_part)
}

fn validated_path<R: CacheRuntime + ?Sized>(
    item: &CacheItem,
    runtime: &R,
) -> Result<PathBuf, UserError> {
    let raw = item.path.as_deref().ok_or_else(cache_item_missing_path)?;
    let path = runtime.canonicalize(&expand_tilde(raw))?;
    if !runtime.is_path_allowed(&path) {
        return Err(path_not_whitelisted(&path));
    }
    Ok(path)
}

pub(crate) async fn run_shell_command(cmd: &str) -> Result<(), UserError> {
    let shell = detect_user_shell();
    let augmented = augmented_path_for_spawn();

    let output = tokio::process::Command::new(&shell)
        .args(["-l", "-c", cmd])
        .env("PATH", &augmented)
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env("TERM", "xterm-256color")
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::CACHE_COMMAND_FAILED,
                format!("启动失败 ({shell}): {e}"),
                "reason",
                e,
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("命令退出码 {:?}", output.status.code())
        };
        return Err(UserError::one(
            ErrorCode::CACHE_COMMAND_EXIT_NONZERO,
            msg.clone(),
            "output",
            &msg,
        ));
    }
    Ok(())
}

async fn clean_pip_cache() -> Result<(), UserError> {
    let attempts = [
        "python3 -m pip cache purge",
        "python -m pip cache purge",
        "pip3 cache purge",
        "pip cache purge",
    ];
    let mut last_err = String::new();
    for cmd in attempts {
        match run_shell_command(cmd).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = format!("`{cmd}` 失败: {e}");
            }
        }
    }

    Err(UserError::one(
        ErrorCode::CACHE_PIP_BROKEN,
        format!(
            "Pip 环境损坏（shebang 指向已卸载的 Python）：{last_err}。\n请手动执行 `python3 -m pip cache purge`，或重新安装 pip。"
        ),
        "reason",
        last_err,
    ))
}

fn detect_user_shell() -> String {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() && std::path::Path::new(&s).exists() {
            return s;
        }
    }
    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    "/bin/sh".to_string()
}

fn augmented_path_for_spawn() -> String {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let candidates = [
        format!("{}/.cargo/bin", home),
        format!("{}/.npm-global/bin", home),
        format!("{}/.yarn/bin", home),
        format!("{}/.bun/bin", home),
        format!("{}/Library/pnpm", home),
        format!("{}/Library/Python/3.9/bin", home),
        format!("{}/Library/Python/3.10/bin", home),
        format!("{}/Library/Python/3.11/bin", home),
        format!("{}/Library/Python/3.12/bin", home),
        format!("{}/Library/Python/3.13/bin", home),
        format!("{}/anaconda3/bin", home),
        format!("{}/miniconda3/bin", home),
        format!("{}/.local/bin", home),
        format!("{}/go/bin", home),
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/local/sbin".to_string(),
        "/Library/Frameworks/Python.framework/Versions/3.13/bin".to_string(),
        "/Library/Frameworks/Python.framework/Versions/3.12/bin".to_string(),
        "/Library/Frameworks/Python.framework/Versions/3.11/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
        "/usr/sbin".to_string(),
        "/sbin".to_string(),
    ];

    let existing = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = candidates.into_iter().collect();
    if !existing.is_empty() {
        parts.push(existing);
    }
    parts.join(":")
}

async fn remove_directory(path: &Path) -> Result<(), UserError> {
    if !path.exists() {
        return Ok(());
    }
    if !is_cleanup_path_allowed(path) {
        return Err(pre_delete_recheck_failed(path));
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let trash = std::env::temp_dir().join(format!(
        "macslim-trash-{}-{}",
        timestamp,
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    ));

    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || match std::fs::rename(&path, &trash) {
        Ok(_) => {
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(&trash);
            });
            Ok(())
        }
        Err(_) => std::fs::remove_dir_all(&path).map_err(|e| {
            UserError::one(
                ErrorCode::DELETE_FAILED,
                format!("删除失败: {e}"),
                "reason",
                e,
            )
        }),
    })
    .await
    .map_err(|e| {
        UserError::one(
            ErrorCode::DELETE_FAILED,
            format!("任务失败: {e}"),
            "reason",
            e,
        )
    })?
}

async fn remove_directory_sudo(path: &Path) -> Result<(), UserError> {
    if !path.exists() {
        return Ok(());
    }
    if !is_cleanup_path_allowed(path) {
        return Err(path_not_whitelisted(path));
    }

    let path_str = path.to_string_lossy().to_string();
    let script = format!(
        r#"do shell script "rm -rf '{}'" with administrator privileges"#,
        path_str.replace('\'', "'\\''")
    );

    let output = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::ADMIN_PRIVILEGES_REQUIRED,
                format!("osascript 启动失败: {e}"),
                "reason",
                e,
            )
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("(-128)") {
            Err(UserError::new(
                ErrorCode::AUTHORIZATION_CANCELLED,
                "用户取消了授权",
            ))
        } else {
            Err(UserError::one(
                ErrorCode::ADMIN_PRIVILEGES_REQUIRED,
                format!("需要管理员权限才能删除此目录: {}", stderr.trim()),
                "reason",
                stderr.trim(),
            ))
        }
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(stripped) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(s));
    }
    PathBuf::from(s)
}

#[cfg(test)]
#[path = "cache_cleaner_tests.rs"]
mod tests;
