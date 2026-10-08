// 卸载执行器：将应用和残留文件移至废纸篓
use crate::operation_executor::{DomainFuture, UninstallDomain};
use crate::operations::{InstalledAppIdentity, LiveResidue, ResidueIdentity};
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::path::Path;

/// 卸载目标（只能由已消费的 operation plan 构造，客户端无法提交）
#[derive(Clone, Debug)]
pub struct UninstallTarget {
    pub bundle_path: String,
    pub app_name: String,
    pub bundle_id: String,
    pub residue_paths: Vec<String>,
}

impl UninstallTarget {
    pub(crate) fn from_plan(app: &InstalledAppIdentity, residues: &[ResidueIdentity]) -> Self {
        Self {
            bundle_path: app.bundle_path.clone(),
            app_name: app.app_name.clone(),
            bundle_id: app.bundle_id.clone(),
            residue_paths: residues
                .iter()
                .map(|residue| residue.path.clone())
                .collect(),
        }
    }
}

/// 单个文件的移动结果
#[derive(Serialize, Clone, Debug)]
pub struct MoveResult {
    pub path: String,
    pub success: bool,
    /// 失败原因：带 `code` 的结构化错误（`error` 命名空间），不是裸中文串。
    pub error: Option<UserError>,
    pub size_bytes: u64,
}

/// 卸载报告
#[derive(Serialize, Clone, Debug)]
pub struct UninstallReport {
    pub app_name: String,
    pub bundle_id: String,
    /// 移入废纸篓的体积之和：文件在废纸篓里，**尚未真正释放**。
    pub trashed_bytes: u64,
    /// 卷可用空间的实测增量；`None` 表示未能测量（区别于 0）。
    pub reclaimed_bytes: Option<u64>,
    pub moved_count: usize,
    pub failed_count: usize,
    pub details: Vec<MoveResult>,
    /// 优雅退出失败原因：结构化错误，`None` 表示退出成功。
    pub quit_error: Option<UserError>,
}

pub(crate) struct SystemUninstaller;

impl UninstallDomain for SystemUninstaller {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        let observed: Vec<InstalledAppIdentity> = bundle_paths
            .iter()
            .filter_map(|path| read_installed_identity(Path::new(path)))
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        let observed: Vec<LiveResidue> = paths
            .iter()
            .map(|path| {
                let candidate = Path::new(path);
                let exists = candidate.symlink_metadata().is_ok();
                let current = candidate
                    .canonicalize()
                    .unwrap_or_else(|_| candidate.to_path_buf());
                LiveResidue {
                    path: current.to_string_lossy().to_string(),
                    exists,
                }
            })
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        let requested = app_name.to_owned();
        Box::pin(async move { quit_app(&requested).await })
    }

    fn remove(
        &self,
        app: &InstalledAppIdentity,
        residues: &[ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        let target = UninstallTarget::from_plan(app, residues);
        Box::pin(async move { uninstall_app(&target).await })
    }
}

pub(crate) fn read_installed_identity(bundle_path: &Path) -> Option<InstalledAppIdentity> {
    if !bundle_path.is_dir() || bundle_path.extension().map_or(true, |ext| ext != "app") {
        return None;
    }
    let plist_path = bundle_path.join("Contents/Info.plist");
    let (name, bundle_id) = crate::applications::read_plist_metadata(&plist_path);
    let bundle_id = bundle_id.unwrap_or_default();
    Some(InstalledAppIdentity {
        bundle_path: bundle_path.to_string_lossy().to_string(),
        app_name: bundle_display_name(bundle_path, name),
        is_system: crate::app_scanner::is_system_app(&bundle_id),
        bundle_id,
        bundle_size_bytes: 0,
    })
}

pub(crate) fn bundle_display_name(bundle_path: &Path, plist_name: Option<String>) -> String {
    plist_name.unwrap_or_else(|| {
        bundle_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "未知应用".to_owned())
    })
}

/// 执行卸载（移至废纸篓）
///
/// 流程：
/// 1. 先尝试 NSFileManager.trashItem（用户权限）
/// 2. 失败后尝试 rename 到 ~/.Trash/
/// 3. 如因权限不足失败（如 /Applications/ 下的 app），收集起来在最后用
///    `do shell script with administrator privileges` 一次性弹出系统授权框
///    批量移动，避免多次重复弹窗
pub(crate) async fn uninstall_app(target: &UninstallTarget) -> UninstallReport {
    let mut all_paths: Vec<String> = Vec::with_capacity(1 + target.residue_paths.len());
    all_paths.push(target.bundle_path.clone());
    all_paths.extend(target.residue_paths.iter().cloned());

    let before = crate::volume::VolumeCapacity::read();
    let (mut details, needs_admin) = trash_with_user_permission(&all_paths).await;
    if !needs_admin.is_empty() {
        details.extend(trash_with_admin(&needs_admin).await);
    }
    let after = crate::volume::VolumeCapacity::read();

    build_report(target, details, crate::volume::reclaimed(before, after))
}

async fn trash_with_user_permission(all_paths: &[String]) -> (Vec<MoveResult>, Vec<(String, u64)>) {
    let mut details: Vec<MoveResult> = Vec::new();
    let mut needs_admin: Vec<(String, u64)> = Vec::new();
    for path_str in all_paths {
        match try_trash_user(path_str).await {
            TrashOutcome::Done(result) => details.push(result),
            TrashOutcome::NeedsAdmin { size } => {
                needs_admin.push((path_str.clone(), size));
            }
        }
    }
    (details, needs_admin)
}

async fn trash_with_admin(needs_admin: &[(String, u64)]) -> Vec<MoveResult> {
    let paths: Vec<&str> = needs_admin.iter().map(|(p, _)| p.as_str()).collect();
    match trash_via_admin_batch(&paths).await {
        Ok(()) => needs_admin
            .iter()
            .map(|(path, size)| MoveResult {
                path: path.clone(),
                success: true,
                error: None,
                size_bytes: *size,
            })
            .collect(),
        Err(e) => {
            let error = if is_user_canceled(&e) {
                UserError::new(ErrorCode::AUTHORIZATION_CANCELLED, "用户取消授权")
            } else {
                UserError::one(
                    ErrorCode::MOVE_TO_TRASH_FAILED,
                    format!("授权移动失败: {e}"),
                    "reason",
                    &e,
                )
            };
            needs_admin
                .iter()
                .map(|(path, _)| MoveResult {
                    path: path.clone(),
                    success: false,
                    error: Some(error.clone()),
                    size_bytes: 0,
                })
                .collect()
        }
    }
}

enum TrashOutcome {
    Done(MoveResult),
    NeedsAdmin { size: u64 },
}

/// 用户权限尝试：osascript NSFileManager → rename。
/// 如全部因权限不足失败，则返回 NeedsAdmin 让上层批量授权处理。
async fn try_trash_user(path_str: &str) -> TrashOutcome {
    let path = Path::new(path_str);
    let size = compute_size(path);

    if !path.exists() {
        return TrashOutcome::Done(MoveResult {
            path: path_str.to_string(),
            success: false,
            error: Some(UserError::new(
                ErrorCode::MOVE_TO_TRASH_FAILED,
                "文件不存在",
            )),
            size_bytes: 0,
        });
    }

    // 1. NSFileManager.trashItem
    match trash_via_osascript(path_str).await {
        Ok(()) => {
            return TrashOutcome::Done(MoveResult {
                path: path_str.to_string(),
                success: true,
                error: None,
                size_bytes: size,
            });
        }
        Err(e) if is_permission_denied_msg(&e) => {
            // 权限问题，先不返回失败，继续尝试 rename
        }
        Err(_) => {
            // 非权限错误也继续 rename 试一下
        }
    }

    // 2. rename 到 ~/.Trash/
    match try_rename_to_trash(path_str) {
        Ok(()) => TrashOutcome::Done(MoveResult {
            path: path_str.to_string(),
            success: true,
            error: None,
            size_bytes: size,
        }),
        Err(e) if is_permission_denied_io(&e) => TrashOutcome::NeedsAdmin { size },
        Err(e) => TrashOutcome::Done(MoveResult {
            path: path_str.to_string(),
            success: false,
            error: Some(UserError::one(
                ErrorCode::MOVE_TO_TRASH_FAILED,
                format!("移动失败: {e}"),
                "reason",
                e,
            )),
            size_bytes: 0,
        }),
    }
}

/// 通过 osascript 调用 Finder 移至废纸篓
async fn trash_via_osascript(path_str: &str) -> Result<(), UserError> {
    let escaped = path_str.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"use framework "Foundation"
set fm to current application's NSFileManager's defaultManager()
set theURL to current application's NSURL's fileURLWithPath:"{}"
set {{result_, theError}} to fm's trashItemAtURL:theURL resultingItemURL:(missing value) |error|:(reference)
if result_ as boolean is false then
    error (theError's localizedDescription() as text)
end if"#,
        escaped
    );

    let output = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::MOVE_TO_TRASH_FAILED,
                format!("启动 osascript 失败: {e}"),
                "reason",
                e,
            )
        })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(UserError::one(
            ErrorCode::MOVE_TO_TRASH_FAILED,
            err.trim().to_string(),
            "reason",
            err.trim(),
        ));
    }
    Ok(())
}

/// 备选方案：直接 rename 到 ~/.Trash/
fn try_rename_to_trash(path_str: &str) -> Result<(), std::io::Error> {
    let path = Path::new(path_str);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let trash_dir = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "无法获取用户主目录"))?
        .join(".Trash");

    let dest = trash_dir.join(&file_name);
    std::fs::rename(path, &dest)
}

/// 用 `do shell script ... with administrator privileges` 弹出系统授权框，
/// 一次输入密码即可批量将多个路径移动到 ~/.Trash/。
async fn trash_via_admin_batch(paths: &[&str]) -> Result<(), UserError> {
    if paths.is_empty() {
        return Ok(());
    }
    let trash_dir = dirs::home_dir()
        .ok_or_else(|| UserError::new(ErrorCode::MOVE_TO_TRASH_FAILED, "无法获取用户主目录"))?
        .join(".Trash");

    // 转义为 AppleScript 字符串字面量
    let as_escaped = applescript_quote(&admin_move_script(paths, &trash_dir));
    let script = format!(
        r#"do shell script {} with administrator privileges"#,
        as_escaped
    );

    let output = tokio::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::MOVE_TO_TRASH_FAILED,
                format!("启动 osascript 失败: {e}"),
                "reason",
                e,
            )
        })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(UserError::one(
            ErrorCode::MOVE_TO_TRASH_FAILED,
            err.trim().to_string(),
            "reason",
            err.trim(),
        ));
    }
    Ok(())
}

pub(crate) fn admin_move_script(paths: &[&str], trash_dir: &Path) -> String {
    let trash_str = trash_dir.to_string_lossy().to_string();
    let mut shell_cmd = String::new();
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            shell_cmd.push_str(" ; ");
        }
        shell_cmd.push_str(&format!(
            "/bin/mv -f {} {}",
            shell_single_quote(path),
            shell_single_quote(&trash_str)
        ));
    }
    shell_cmd
}

/// 用单引号包裹 shell 参数，路径中若有 `'` 替换为 `'\''`
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 用双引号包裹 AppleScript 字符串字面量，转义 `\` 和 `"`
fn applescript_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

fn is_permission_denied_io(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(13)
}

fn is_permission_denied_msg(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("permission")
        || lower.contains("operation not permitted")
        || lower.contains("not authorized")
        || lower.contains("nscocoaerrordomain error 513")
        || lower.contains("nscocoaerrordomain error 257")
}

fn is_user_canceled(msg: &str) -> bool {
    // osascript 在用户取消授权时返回 errAEEventNotPermitted (-1743) 或 -128
    msg.contains("-128") || msg.contains("User canceled") || msg.contains("用户已取消")
}

/// 计算文件或目录大小
fn compute_size(path: &Path) -> u64 {
    if path.is_dir() {
        crate::app_scanner::dir_size(path)
    } else {
        path.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

/// 从移动结果列表构建卸载报告
fn build_report(
    target: &UninstallTarget,
    details: Vec<MoveResult>,
    reclaimed_bytes: Option<u64>,
) -> UninstallReport {
    let moved_count = details.iter().filter(|d| d.success).count();
    let failed_count = details.iter().filter(|d| !d.success).count();
    let trashed_bytes: u64 = details.iter().map(|d| d.size_bytes).sum();

    UninstallReport {
        app_name: target.app_name.clone(),
        bundle_id: target.bundle_id.clone(),
        trashed_bytes,
        reclaimed_bytes,
        moved_count,
        failed_count,
        details,
        quit_error: None,
    }
}

/// 检查应用是否正在运行（通过 bundle 路径匹配进程）
pub fn is_app_running(bundle_path: &str, sys: &mut sysinfo::System) -> bool {
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for proc in sys.processes().values() {
        let Some(exe) = proc.exe() else { continue };
        let exe_str = exe.to_string_lossy();
        if exe_str.starts_with(bundle_path) {
            return true;
        }
    }
    false
}

pub(crate) fn quit_script(app_name: &str) -> String {
    format!("tell application {} to quit", applescript_quote(app_name))
}

pub(crate) async fn quit_app(app_name: &str) -> Result<(), UserError> {
    let output = tokio::process::Command::new("osascript")
        .args(["-e", &quit_script(app_name)])
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::MOVE_TO_TRASH_FAILED,
                format!("启动 osascript 失败: {e}"),
                "reason",
                e,
            )
        })?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(UserError::one(
            ErrorCode::MOVE_TO_TRASH_FAILED,
            err.trim().to_string(),
            "reason",
            err.trim(),
        ));
    }

    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let check_script = format!(
            "tell application {} to (name of processes) contains {}",
            applescript_quote("System Events"),
            applescript_quote(app_name)
        );
        let checked = tokio::process::Command::new("osascript")
            .args(["-e", &check_script])
            .output()
            .await;
        if let Ok(out) = checked {
            if String::from_utf8_lossy(&out.stdout).trim() == "false" {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "uninstaller_tests.rs"]
mod tests;
