use crate::cache_scanner::CacheItem;
use crate::user_error::UserError;
use serde::{Deserialize, Serialize};

pub type SnapshotId = String;
pub type OperationId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheAction {
    Npm,
    Pnpm,
    Yarn,
    Docker,
    Homebrew,
    Xcode,
    Cocoapods,
    Cargo,
    Pip,
    Go,
    System,
    StaleNodeModules,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapshotKind {
    Cache,
    Process,
    OverviewProcess,
    Application,
    InstalledApps,
    Residue,
    Docker,
}

pub(crate) const PROCESS_SNAPSHOT_KINDS: [SnapshotKind; 2] =
    [SnapshotKind::Process, SnapshotKind::OverviewProcess];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMode {
    Graceful,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerAction {
    RemoveImage,
    RemoveContainer,
    RemoveVolume,
    Prune,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Cache,
    Process,
    AppTerminate,
    AppGracefulQuit,
    Uninstall,
    Docker,
}

impl OperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cache => "cache",
            Self::Process => "process",
            Self::AppTerminate => "app_terminate",
            Self::AppGracefulQuit => "app_graceful_quit",
            Self::Uninstall => "uninstall",
            Self::Docker => "docker",
        }
    }

    pub(crate) fn of(plan: &OperationPlan) -> Self {
        match plan {
            OperationPlan::Cache { .. } => Self::Cache,
            OperationPlan::Process { .. } => Self::Process,
            OperationPlan::AppTerminate { .. } => Self::AppTerminate,
            OperationPlan::AppGracefulQuit { .. } => Self::AppGracefulQuit,
            OperationPlan::Uninstall { .. } => Self::Uninstall,
            OperationPlan::Docker { .. } => Self::Docker,
        }
    }

    pub fn is_termination(self) -> bool {
        matches!(self, Self::Process | Self::AppTerminate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockerResourceKind {
    Image,
    Container,
    Volume,
}

impl DockerAction {
    pub fn resource_type(self) -> Option<DockerResourceKind> {
        match self {
            Self::RemoveImage => Some(DockerResourceKind::Image),
            Self::RemoveContainer => Some(DockerResourceKind::Container),
            Self::RemoveVolume => Some(DockerResourceKind::Volume),
            Self::Prune => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RemoveImage => "删除镜像",
            Self::RemoveContainer => "删除容器",
            Self::RemoveVolume => "删除卷",
            Self::Prune => "清理 Docker",
        }
    }
}

impl DockerResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "镜像",
            Self::Container => "容器",
            Self::Volume => "卷",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub start_time: u64,
}

/// 可执行的进程目标。它同时携带 identity 与保护状态，只有 crate 内部的
/// OperationStore / executor 能构造它；library 外的 crate（包括 `bin/cli.rs`）
/// 无法伪造一个 ProcessTarget 去驱动信号通道。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessTarget {
    pub(crate) identity: ProcessIdentity,
    pub(crate) protected: bool,
    pub(crate) whitelisted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppIdentity {
    pub(crate) bundle_path: String,
    pub(crate) bundle_id: String,
    pub(crate) app_name: String,
    pub(crate) processes: Vec<ProcessTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledAppIdentity {
    pub bundle_path: String,
    pub app_name: String,
    pub bundle_id: String,
    pub is_system: bool,
    pub bundle_size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidueIdentity {
    pub app_key: String,
    pub path: String,
    pub category: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveResidue {
    pub path: String,
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerTarget {
    pub resource_type: DockerResourceKind,
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub referenced: bool,
    pub reclaimable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerInventoryFingerprint {
    pub resources: Vec<DockerTarget>,
    pub reclaimable_bytes: u64,
    pub image_count: usize,
    pub container_count: usize,
    pub volume_count: usize,
}

impl DockerInventoryFingerprint {
    pub fn from_resources(resources: Vec<DockerTarget>) -> Self {
        let count_of = |kind: DockerResourceKind| {
            resources
                .iter()
                .filter(|target| target.resource_type == kind)
                .count()
        };
        Self {
            reclaimable_bytes: resources
                .iter()
                .filter(|target| target.reclaimable)
                .fold(0_u64, |total, target| {
                    total.saturating_add(target.size_bytes)
                }),
            image_count: count_of(DockerResourceKind::Image),
            container_count: count_of(DockerResourceKind::Container),
            volume_count: count_of(DockerResourceKind::Volume),
            resources,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveDockerResource {
    pub id: String,
    pub name: String,
    pub referenced: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum SnapshotPayload {
    Cache(Vec<CacheItem>),
    Process(Vec<ProcessTarget>),
    InstalledApps(Vec<InstalledAppIdentity>),
    Residue(Vec<ResidueIdentity>),
    Docker(Vec<DockerTarget>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRegistration {
    pub snapshot_id: String,
    pub selection_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationChildBinding {
    pub app_key: String,
    pub child_key: String,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationSnapshotRegistration {
    pub snapshot_id: String,
    pub app_keys: Vec<String>,
    pub child_bindings: Vec<ApplicationChildBinding>,
}

#[derive(Clone, Debug)]
pub struct UninstallPlanTarget {
    pub app_key: String,
    pub app: InstalledAppIdentity,
    pub residues: Vec<ResidueIdentity>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum OperationPlan {
    Cache {
        snapshot_id: SnapshotId,
        items: Vec<CacheItem>,
    },
    Process {
        mode: ProcessMode,
        targets: Vec<ProcessTarget>,
    },
    AppTerminate {
        mode: ProcessMode,
        targets: Vec<AppIdentity>,
    },
    AppGracefulQuit {
        targets: Vec<AppIdentity>,
    },
    Uninstall {
        targets: Vec<UninstallPlanTarget>,
        quit_running: bool,
    },
    Docker {
        action: DockerAction,
        targets: Vec<DockerTarget>,
        inventory: DockerInventoryFingerprint,
    },
}

#[derive(Debug)]
pub(crate) struct ConsumedPlan {
    operation_id: String,
    plan: OperationPlan,
}

impl ConsumedPlan {
    pub(super) fn from_plan(operation_id: String, plan: OperationPlan) -> Self {
        Self { operation_id, plan }
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(crate) fn kind(&self) -> OperationKind {
        OperationKind::of(&self.plan)
    }

    pub(crate) fn into_plan(self) -> OperationPlan {
        self.plan
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProcessKillDetail {
    pub pid: u32,
    pub name: String,
    pub success: bool,
    /// 结果行文案：带 `code` 的结构化消息（`error` 命名空间），不是裸中文串。
    pub message: UserError,
}

/// 进程终止的公开 DTO。它只是执行结果，不含任何可执行能力，
/// 所以 `bin/cli.rs` 只需要它来做结果打印。
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessKillReport {
    pub killed: Vec<u32>,
    pub failed: Vec<u32>,
    pub details: Vec<ProcessKillDetail>,
}

impl ProcessKillReport {
    pub fn record(&mut self, detail: ProcessKillDetail) {
        if detail.success {
            self.killed.push(detail.pid);
        } else {
            self.failed.push(detail.pid);
        }
        self.details.push(detail);
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PreparedOperation {
    pub operation_id: String,
    pub kind: String,
    pub expires_at_ms: u64,
    pub item_count: usize,
    pub estimated_bytes: u64,
    /// 操作摘要的 i18n key + 插值参数（`opSummary.*`）。译文在前端词典里。
    ///
    /// 纯展示字段：`operation_id` 的 owner / 过期 / 单次消费校验与它无关。
    pub summary_key: String,
    pub summary_params: crate::i18n_text::I18nParams,
}
