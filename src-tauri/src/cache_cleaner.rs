use crate::cache_scanner::{is_any_tool_busy, CacheItem, STALE_PROJECT_ROOTS};
use crate::operations::CacheAction;
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

fn allowed_cleanup_roots() -> Vec<PathBuf> {
    // 必须用 `scanner_home()` 而不是 `dirs::home_dir()`。
    //
    // App Store 版里 `$HOME` 指向应用自己的空 container，而扫描出来的路径在
    // 真实 home 下（扫描器走的就是 `scanner_home()`）。两者不一致的后果不是
    // 「白名单少一项」，而是**上架版的清理 100% 失败**：待删路径
    // `/Users/edwinhao/Library/Logs` 永远不以白名单根
    // `.../Containers/com.vgoapp.macslim/Data/Library/Logs` 开头，
    // `starts_with` 恒为 false → `PATH_NOT_WHITELISTED`。
    //
    // 用户在界面上看到的是：扫出 10.50 GB、授权、勾选、二次确认，
    // 然后「成功 0 项，失败 1 项，释放 0」。而 `cargo test` 里 `$HOME`
    // 恰好就是真实 home，所以这条路径在测试里永远是绿的。
    // 同一个坑 `expand_tilde_from` 的注释里已经写过一次，这里是漏改的那处。
    let home = crate::folder_access::scanner_home();
    vec![
        home.join(".npm"),
        home.join(".cargo/registry/cache"),
        home.join(".Trash"),
        home.join("Library/pnpm/store"),
        home.join("Library/Caches"),
        home.join("Library/Logs"),
        home.join("Library/Developer/Xcode/DerivedData"),
        home.join("Library/Developer/Xcode/Archives"),
        home.join("Library/Developer/Xcode/iOS DeviceSupport"),
        home.join("Library/Developer/CoreSimulator/Caches"),
        home.join("Library/Application Support/CrashReporter"),
        PathBuf::from("/opt/homebrew/Library/Homebrew/cache"),
    ]
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
        if Path::new(s) == crate::folder_access::scanner_home() {
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
    // 同 `allowed_cleanup_roots()`：沙箱里 `dirs::home_dir()` 是 container，
    // 会让「陈旧 node_modules」这类项目路径永远匹配不上（扫描器给的是真实
    // home 下的路径）。
    let home = crate::folder_access::scanner_home();
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
    // 同 `allowed_cleanup_roots()`：沙箱里 `dirs::home_dir()` 是 container，
    // 拼出来的 `~/.cargo/bin`、`~/Library/pnpm` 等全指向空目录，于是
    // npm / cargo / brew 这些**装在真实 home 下**的工具一个都找不到，
    // 需要调 CLI 的清理项（npm cache clean 之类）会静默失败。
    let home = crate::folder_access::scanner_home()
        .to_string_lossy()
        .to_string();

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
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let trash = std::env::temp_dir().join(format!(
        "macslim-trash-{}-{}",
        timestamp,
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    ));

    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // 进入 security scope 必须早于**任何**文件系统访问 —— 包括下面那句
        // `path.exists()`。
        //
        // 这是 2bcbbcf 漏掉的一半：那次只把 scope 加到了 rename 之前，而
        // `exists()` 与 `is_cleanup_path_allowed()`（内部 `canonicalize()`）
        // 仍在作用域之外。沙箱下它们被拒 → `canonicalize` 失败 →
        // `is_cleanup_path_allowed` 返回 false → 清理**必然**失败，报的还是
        // 「路径不在白名单内」这种把人引向错误方向的理由。实测日志：
        //
        //     Sandbox: macslim deny(1) file-read-data /Users/edwinhao/Library/Logs
        //
        // 用户表现仍是「授权了、选中了、确认了，然后什么都没删」。
        //
        // 为什么整段都在这个闭包里：SecurityScope 里是一个 ObjC NSURL，不是
        // Send，跨 `.await` 持有会让 future 失去 Send，而命令宏要求
        // Future + Send。在阻塞任务里进入、随闭包结束析构，start/stop 严格
        // 配对，又不碰 Send 边界。
        let _granted_scopes = crate::folder_access::enter_granted_scopes();

        if !path.exists() {
            return Ok(());
        }
        if !is_cleanup_path_allowed(&path) {
            let error = pre_delete_recheck_failed(&path);
            log_cleanup_failure(&path, &error);
            return Err(error);
        }

        match std::fs::rename(&path, &trash) {
            Ok(_) => {
                std::thread::spawn(move || {
                    let _ = std::fs::remove_dir_all(&trash);
                });
                Ok(())
            }
            // rename 失败通常只是跨设备或目标已存在，紧接着的 remove_dir_all
            // 才是真正的兜底；要报的是**它**的错误，所以这里不绑定 rename 的。
            Err(_) => clear_directory_contents(&path).map_err(|e| {
                let error = UserError::one(
                    ErrorCode::DELETE_FAILED,
                    format!("删除失败: {e}"),
                    "reason",
                    e,
                );
                log_cleanup_failure(&path, &error);
                error
            }),
        }
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

/// 清空目录内容，再尽力删掉目录本身。
///
/// ## 为什么不能直接 `remove_dir_all(path)`
///
/// security-scoped bookmark 授权的是**这个目录及其内容**，不含它的父目录。
/// 于是「删掉目录本身」这一步要修改父目录，被沙箱拒绝：
///
///     Sandbox: macslim deny(1) file-write-unlink /Users/edwinhao/Library
///     删除失败: Permission denied (os error 13)
///
/// 而此时内容其实**已经清空了** —— 旧代码把这当成整体失败，用户看到的是
/// 「释放 0」，实际空间已经释放。更糟的是它会让「清理」在授权根目录上
/// 永远报错，而 `~/Library/Logs`、`~/Library/Caches` 这些恰恰都是授权根。
///
/// 所以：内容清空即视为成功；目录本身删不掉就留着（这类目录本来也该存在，
/// 系统与应用都预期它在）。
fn clear_directory_contents(path: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(&child)?;
        } else {
            std::fs::remove_file(&child)?;
        }
    }
    // 尽力删掉空目录；删不掉不算失败（父目录不在授权范围内）。
    let _ = std::fs::remove_dir(path);
    Ok(())
}

/// 清理失败时把**路径与原因**写进 stderr。
///
/// 此前失败只落一条中文摘要进历史（「成功 0 项，失败 1 项，释放 0」），
/// 具体原因被丢掉：用户不知道该怎么办，排查的人也只能靠猜。日志里至少要有
/// 「哪个路径、为什么」这两件事。
fn log_cleanup_failure(path: &Path, error: &UserError) {
    eprintln!(
        "[macslim] 清理失败 path={} code={:?} message={}",
        path.display(),
        error.code,
        error.message
    );
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
    expand_tilde_from(s, &crate::folder_access::scanner_home())
}

/// 把 `~/` 展开成 `home` 下的路径。**home 由调用方给**，这样它才能被测。
///
/// ## 为什么不能直接用 `dirs::home_dir()`
///
/// App Store 版里 `$HOME` 指向应用自己的空 container（实测 `home_env =
/// .../Containers/com.vgoapp.macslim/Data`，而 `real_home = /Users/edwinhao`）。
/// 而扫描器走的是 `folder_access::scanner_home()` —— passwd 里的真实 home。
/// 两者不一致的后果不是「路径算错」，而是**这个功能整个不работа**：
///
/// - 扫描阶段：真实 home 下的 `~/Library/Logs`，量出 10.5 GB
/// - 清理阶段：`~` 展开到 container 里一个**不存在**的路径，`canonicalize`
///   直接失败
///
/// 实测表现：界面列出 10.5 GB 可释放、按清理、确认，然后
/// 「成功 0 项，失败 1 项，释放 0」—— 什么都没删。
///
/// 之前这里写的是 `dirs::home_dir()`，而在**非沙箱**的 `cargo test` 里
/// `$HOME` 恰好就是真实 home，所以测试一直是绿的 —— 这类「只在沙箱里才
/// 出错」的 bug 用直接断言是抓不到的，必须把 home 变成参数才能测。
fn expand_tilde_from(s: &str, home: &Path) -> PathBuf {
    if let Some(stripped) = s.strip_prefix("~/") {
        return home.join(stripped);
    }
    if s == "~" {
        return home.to_path_buf();
    }
    PathBuf::from(s)
}

#[cfg(test)]
#[path = "cache_cleaner_tests.rs"]
mod tests;
