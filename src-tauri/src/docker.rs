//! Docker 深度视图 —— 列举镜像 / 容器 / 卷 / 构建缓存，支持单项删除。
//!
//! 使用 `docker` CLI（与 CacheView 的批量清理互补）。
//! 所有命令走 tokio::process 并显式设 PATH，避免 GUI 启动没继承 shell PATH 的问题。

use crate::operation_executor::{DockerDomain, DomainFuture};
use crate::operations::{
    DockerAction, DockerInventoryFingerprint, DockerResourceKind, DockerTarget, LiveDockerResource,
    OperationStore, SnapshotRegistration,
};
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct DockerImage {
    pub id: String,         // short id
    pub repository: String, // nginx
    pub tag: String,        // latest / <none>
    pub size_bytes: u64,
    pub created: String, // 2026-01-15 10:20:30 +0800 CST
    pub dangling: bool,  // repository/tag 为 <none>
    pub in_use: bool,    // 是否被某个容器引用
    pub selection_key: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String, // running / exited (0) 2 days ago
    pub running: bool,
    pub size_bytes: u64, // RW 层大小
    pub created: String,
    pub selection_key: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DockerVolume {
    pub name: String,
    pub driver: String,
    pub size_bytes: u64,
    pub in_use: bool,
    pub selection_key: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct DockerBuilderCache {
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct DockerInventory {
    pub daemon_running: bool,
    pub images: Vec<DockerImage>,
    pub containers: Vec<DockerContainer>,
    pub volumes: Vec<DockerVolume>,
    pub builder: DockerBuilderCache,
    /// 总可回收大小
    pub reclaimable_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct DockerExecutionReport {
    pub action: String,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub output: String,
}

impl DockerExecutionReport {
    pub(crate) fn new(action: DockerAction) -> Self {
        Self {
            action: action.label().to_owned(),
            succeeded: Vec::new(),
            failed: Vec::new(),
            output: String::new(),
        }
    }
}

pub(crate) struct SystemDocker;

impl DockerDomain for SystemDocker {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>> {
        Box::pin(inventory())
    }

    fn remove(
        &self,
        action: DockerAction,
        target: &DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>> {
        let typed = target.clone();
        Box::pin(async move { remove_target(action, &typed).await })
    }

    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>> {
        Box::pin(prune_all())
    }
}

pub(crate) fn docker_targets(inventory: &DockerInventory) -> Vec<DockerTarget> {
    let mut targets = Vec::with_capacity(
        inventory.images.len() + inventory.containers.len() + inventory.volumes.len(),
    );
    targets.extend(inventory.images.iter().map(|image| DockerTarget {
        resource_type: DockerResourceKind::Image,
        id: image.id.clone(),
        name: format!("{}:{}", image.repository, image.tag),
        size_bytes: image.size_bytes,
        referenced: image.in_use,
        reclaimable: image.dangling,
    }));
    targets.extend(inventory.containers.iter().map(|container| DockerTarget {
        resource_type: DockerResourceKind::Container,
        id: container.id.clone(),
        name: container.name.clone(),
        size_bytes: container.size_bytes,
        referenced: container.running,
        reclaimable: !container.running,
    }));
    targets.extend(inventory.volumes.iter().map(|volume| DockerTarget {
        resource_type: DockerResourceKind::Volume,
        id: volume.name.clone(),
        name: volume.name.clone(),
        size_bytes: volume.size_bytes,
        referenced: volume.in_use,
        reclaimable: !volume.in_use,
    }));
    targets
}

pub(crate) fn inventory_fingerprint(inventory: &DockerInventory) -> DockerInventoryFingerprint {
    let mut resources = docker_targets(inventory);
    resources.sort_by(|left, right| {
        (left.resource_type, &left.id, &left.name).cmp(&(
            right.resource_type,
            &right.id,
            &right.name,
        ))
    });
    DockerInventoryFingerprint::from_resources(resources)
}

pub fn register_docker_inventory(
    store: &mut OperationStore,
    inventory: &mut DockerInventory,
) -> Result<SnapshotRegistration, UserError> {
    let registration = store.register_docker_resources(docker_targets(inventory))?;
    let mut keys = registration.selection_keys.iter();
    for image in &mut inventory.images {
        image.selection_key = next_key(&mut keys, "Docker 镜像")?;
    }
    for container in &mut inventory.containers {
        container.selection_key = next_key(&mut keys, "Docker 容器")?;
    }
    for volume in &mut inventory.volumes {
        volume.selection_key = next_key(&mut keys, "Docker 卷")?;
    }
    if keys.next().is_some() {
        return Err(docker_selection_count_mismatch());
    }
    Ok(registration)
}

/// 三个 `next_key` 调用点共用一个 `code`。
fn docker_selection_count_mismatch() -> UserError {
    UserError::new(
        ErrorCode::DOCKER_SELECTION_COUNT_MISMATCH,
        "Docker 选择 key 数量与快照不一致",
    )
}

fn next_key<'a>(
    keys: &mut impl Iterator<Item = &'a String>,
    label: &str,
) -> Result<String, UserError> {
    keys.next().cloned().ok_or_else(|| {
        UserError::one(
            ErrorCode::DOCKER_SELECTION_COUNT_MISMATCH,
            format!("{label} 选择 key 数量与快照不一致"),
            "label",
            label,
        )
    })
}

pub(crate) fn find_live_resource(
    inventory: &DockerInventory,
    kind: DockerResourceKind,
    id: &str,
) -> Option<LiveDockerResource> {
    match kind {
        DockerResourceKind::Image => {
            inventory
                .images
                .iter()
                .find(|image| image.id == id)
                .map(|image| LiveDockerResource {
                    id: image.id.clone(),
                    name: format!("{}:{}", image.repository, image.tag),
                    referenced: image.in_use,
                })
        }
        DockerResourceKind::Container => inventory
            .containers
            .iter()
            .find(|container| container.id == id)
            .map(|container| LiveDockerResource {
                id: container.id.clone(),
                name: container.name.clone(),
                referenced: container.running,
            }),
        DockerResourceKind::Volume => inventory
            .volumes
            .iter()
            .find(|volume| volume.name == id)
            .map(|volume| LiveDockerResource {
                id: volume.name.clone(),
                name: volume.name.clone(),
                referenced: volume.in_use,
            }),
    }
}

/// Docker CLI 是否在 PATH + daemon 是否在运行
pub async fn is_available() -> bool {
    if which::which("docker").is_err() {
        return false;
    }
    run_docker(&["info", "--format", "{{.ServerVersion}}"])
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

pub async fn inventory() -> Result<DockerInventory, UserError> {
    if !is_available().await {
        return Ok(build_inventory(
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            DockerBuilderCache {
                total_bytes: 0,
                reclaimable_bytes: 0,
            },
        ));
    }

    let images = list_images().await.unwrap_or_default();
    let containers = list_containers().await.unwrap_or_default();
    let volumes = list_volumes().await.unwrap_or_default();
    let builder = builder_cache().await.unwrap_or(DockerBuilderCache {
        total_bytes: 0,
        reclaimable_bytes: 0,
    });
    Ok(build_inventory(true, images, containers, volumes, builder))
}

pub(crate) fn build_inventory(
    daemon_running: bool,
    images: Vec<DockerImage>,
    containers: Vec<DockerContainer>,
    volumes: Vec<DockerVolume>,
    builder: DockerBuilderCache,
) -> DockerInventory {
    let mut reclaimable = builder.reclaimable_bytes;
    // 悬空镜像 100% 可回收
    reclaimable += images
        .iter()
        .filter(|i| i.dangling)
        .map(|i| i.size_bytes)
        .sum::<u64>();
    // 已停止容器
    reclaimable += containers
        .iter()
        .filter(|c| !c.running)
        .map(|c| c.size_bytes)
        .sum::<u64>();
    // 未被容器引用的卷
    reclaimable += volumes
        .iter()
        .filter(|v| !v.in_use)
        .map(|v| v.size_bytes)
        .sum::<u64>();

    DockerInventory {
        daemon_running,
        images,
        containers,
        volumes,
        builder,
        reclaimable_bytes: reclaimable,
    }
}

async fn list_images() -> Result<Vec<DockerImage>, UserError> {
    // 用 Go template 拿结构化数据：id|repo|tag|size|created|dangling
    let out = run_docker(&[
        "images",
        "-a",
        "--no-trunc",
        "--format",
        "{{.ID}}|{{.Repository}}|{{.Tag}}|{{.Size}}|{{.CreatedAt}}|{{.Digest}}",
    ])
    .await?;

    // 收集被容器引用的镜像 ID（完整 digest）
    let in_use_ids = run_docker(&["ps", "-a", "--format", "{{.Image}}"])
        .await
        .unwrap_or_default();
    Ok(parse_image_lines(&out, &in_use_ids))
}

pub(crate) fn parse_image_lines(out: &str, in_use_ids: &str) -> Vec<DockerImage> {
    let in_use_set: std::collections::HashSet<String> = in_use_ids
        .lines()
        .map(|line| line.trim().to_string())
        .collect();
    let mut images = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 5 {
            continue;
        }
        let id_full = parts[0].trim().trim_start_matches("sha256:");
        let id_short = id_full.chars().take(12).collect::<String>();
        let repo = parts[1].trim().to_string();
        let tag = parts[2].trim().to_string();
        let dangling = repo == "<none>" && tag == "<none>";
        let in_use = is_image_referenced(&in_use_set, &repo, &tag, id_full, &id_short);
        images.push(DockerImage {
            id: id_short,
            dangling,
            in_use,
            repository: repo,
            tag,
            size_bytes: parse_human_size(parts[3].trim()),
            created: parts[4].trim().to_string(),
            selection_key: String::new(),
        });
    }
    images
}

fn is_image_referenced(
    in_use_set: &std::collections::HashSet<String>,
    repository: &str,
    tag: &str,
    id_full: &str,
    id_short: &str,
) -> bool {
    in_use_set.contains(&format!("{repository}:{tag}"))
        || in_use_set.contains(id_full)
        || in_use_set.contains(id_short)
}

async fn list_containers() -> Result<Vec<DockerContainer>, UserError> {
    let out = run_docker(&[
        "ps",
        "-a",
        "--size",
        "--format",
        "{{.ID}}|{{.Names}}|{{.Image}}|{{.Status}}|{{.State}}|{{.Size}}|{{.CreatedAt}}",
    ])
    .await?;
    Ok(parse_container_lines(&out))
}

pub(crate) fn parse_container_lines(out: &str) -> Vec<DockerContainer> {
    let mut containers = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 7 {
            continue;
        }
        containers.push(DockerContainer {
            id: parts[0].trim().chars().take(12).collect::<String>(),
            name: parts[1].trim().to_string(),
            image: parts[2].trim().to_string(),
            status: parts[3].trim().to_string(),
            running: parts[4].trim() == "running",
            // Size 形如 "1.2MB (virtual 150MB)"，取第一部分
            size_bytes: parse_human_size(parts[5].trim().split('(').next().unwrap_or("0").trim()),
            created: parts[6].trim().to_string(),
            selection_key: String::new(),
        });
    }
    containers
}

async fn list_volumes() -> Result<Vec<DockerVolume>, UserError> {
    let out = run_docker(&["volume", "ls", "--format", "{{.Name}}|{{.Driver}}"]).await?;
    let in_use_raw = run_docker(&["ps", "-a", "--format", "{{.Mounts}}"])
        .await
        .unwrap_or_default();
    Ok(parse_volume_lines(&out, &in_use_raw))
}

pub(crate) fn parse_volume_lines(out: &str, mounts: &str) -> Vec<DockerVolume> {
    let in_use_set = mount_tokens(mounts);
    let mut volumes = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].trim().to_string();
        volumes.push(DockerVolume {
            in_use: in_use_set.iter().any(|token| mount_covers(token, &name)),
            // size via inspect + du -sh（太慢），这里给 0，上游用 docker system df 估算
            size_bytes: 0,
            name: name.clone(),
            driver: parts[1].trim().to_string(),
            selection_key: String::new(),
        });
    }
    volumes
}

fn mount_tokens(mounts: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for line in mounts.lines() {
        for token in line.split(',') {
            let token = token.trim();
            if !token.is_empty() {
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

fn mount_covers(token: &str, volume_name: &str) -> bool {
    token == volume_name || token.ends_with(&format!("/{volume_name}"))
}

async fn builder_cache() -> Result<DockerBuilderCache, UserError> {
    // docker system df --format table 不够好，用 docker builder du
    let out = run_docker(&["builder", "du"]).await?;
    // 简单解析：最后一行 "Reclaimable: 1.5GB"
    let mut total = 0u64;
    let mut reclaimable = 0u64;
    for line in out.lines() {
        let line = line.trim().to_lowercase();
        if line.starts_with("reclaimable space:") || line.starts_with("reclaimable:") {
            reclaimable = parse_human_size(line.split(':').nth(1).unwrap_or("0").trim());
        } else if line.starts_with("shared space:") || line.starts_with("total:") {
            total = parse_human_size(line.split(':').nth(1).unwrap_or("0").trim());
        }
    }
    Ok(DockerBuilderCache {
        total_bytes: total,
        reclaimable_bytes: reclaimable,
    })
}

pub(crate) async fn remove_target(
    action: DockerAction,
    target: &DockerTarget,
) -> Result<(), UserError> {
    match action {
        DockerAction::RemoveImage => remove_image(&target.id).await,
        DockerAction::RemoveContainer => remove_container(&target.id).await,
        DockerAction::RemoveVolume => remove_volume(&target.id).await,
        DockerAction::Prune => Err(UserError::new(
            ErrorCode::DOCKER_PRUNE_REJECTS_TARGET,
            "Docker prune 不接受资源目标",
        )),
    }
}

pub(crate) async fn remove_image(id: &str) -> Result<(), UserError> {
    // -f 强制（镜像可能被停止容器引用）
    run_docker(&["image", "rm", "-f", id]).await.map(|_| ())
}

pub(crate) async fn remove_container(id: &str) -> Result<(), UserError> {
    run_docker(&["rm", "-f", id]).await.map(|_| ())
}

pub(crate) async fn remove_volume(name: &str) -> Result<(), UserError> {
    run_docker(&["volume", "rm", "-f", name]).await.map(|_| ())
}

/// `docker system prune -f --volumes` —— 一键删除悬空镜像 + 停止容器 + 构建缓存 + 未引用卷
pub(crate) async fn prune_all() -> Result<String, UserError> {
    run_docker(&["system", "prune", "-f", "--volumes"]).await
}

// ---- 辅助 ----

pub(crate) async fn run_docker(args: &[&str]) -> Result<String, UserError> {
    let home = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();
    let path = format!(
        "/opt/homebrew/bin:/usr/local/bin:{}/.docker/bin:/Applications/Docker.app/Contents/Resources/bin:{}",
        home,
        std::env::var("PATH").unwrap_or_default()
    );

    let output = tokio::process::Command::new("docker")
        .args(args)
        .env("PATH", &path)
        .output()
        .await
        .map_err(|e| {
            UserError::one(
                ErrorCode::DOCKER_CLI_FAILED,
                format!("启动 docker 失败: {e}"),
                "reason",
                e,
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(UserError::one(
            ErrorCode::DOCKER_CLI_FAILED,
            stderr.trim().to_string(),
            "reason",
            stderr.trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_human_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() || s == "0" || s == "0B" {
        return 0;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        i += 1;
    }
    let num: f64 = chars[..i].iter().collect::<String>().parse().unwrap_or(0.0);
    let unit: String = chars[i..]
        .iter()
        .collect::<String>()
        .trim()
        .to_uppercase()
        .replace('B', "");
    let mult: u64 = match unit.as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024u64.pow(3),
        "T" | "TB" | "TIB" => 1024u64.pow(4),
        _ => 1,
    };
    (num * mult as f64) as u64
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
