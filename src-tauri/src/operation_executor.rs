use crate::cache_cleaner::{clean_snapshot_items, CleanSummary};
use crate::cache_scanner::CacheItem;
use crate::docker::{
    find_live_resource, inventory_fingerprint, DockerExecutionReport, DockerInventory,
};
use crate::operations::ProcessKillReport;
use crate::operations::{
    ensure_targets_terminable, AppIdentity, ConsumedPlan, DockerAction, DockerInventoryFingerprint,
    DockerTarget, InstalledAppIdentity, LiveDockerResource, LiveResidue, OperationKind,
    OperationPlan, ProcessMode, ProcessTarget, ResidueIdentity, UninstallPlanTarget,
};
use crate::process_ops::{LiveProcess, ProcessObserver, ProcessSignaller};
use crate::residue_policy;
use crate::uninstaller::UninstallReport;
use crate::user_error::{ErrorCode, UserError};
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub(crate) type DomainFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) trait CacheCleaner: Send + Sync {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>>;
}

pub(crate) struct SystemCacheCleaner;

impl CacheCleaner for SystemCacheCleaner {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        Box::pin(clean_snapshot_items(items))
    }
}

pub(crate) struct DomainServices<'a, C: ?Sized, U: ?Sized, Q: ?Sized, D: ?Sized> {
    pub cache: &'a C,
    pub uninstall: &'a U,
    pub app_quit: &'a Q,
    pub docker: &'a D,
}

pub(crate) fn system_domain_services<'a, C, U>(
    cache: &'a C,
    uninstall: &'a U,
    app_quit: &'a crate::applications::SystemAppQuitter,
) -> DomainServices<'a, C, U, crate::applications::SystemAppQuitter, crate::docker::SystemDocker>
where
    C: CacheCleaner + ?Sized,
    U: UninstallDomain + ?Sized,
{
    DomainServices {
        cache,
        uninstall,
        app_quit,
        docker: &crate::docker::SystemDocker,
    }
}

pub(crate) struct ProcessServices<'a> {
    pub observer: &'a mut (dyn ProcessObserver + Send),
    pub signaller: &'a mut (dyn ProcessSignaller + Send),
}

/// 「操作计划类型对不上」的五处共用一个 `code`，只有计划种类不同。
///
/// `kind` 直接拼进中文兜底，所以五个调用点传的字面量必须**逐字**与改造前
/// 的消息一致 —— 既有测试用 `assert_eq!(error, "操作计划不是 Docker 计划")`
/// 逐字比对，正是这条约束的护栏。
///
/// `Docker` 是拉丁文，前后各带一个空格（`" Docker "`）才能与中文的
/// 「操作计划不是缓存计划」排版一致。
fn plan_kind_mismatch(kind: &str) -> UserError {
    UserError::with(
        ErrorCode::PLAN_KIND_MISMATCH,
        format!("操作计划不是{kind}计划"),
        vec![("kind".to_owned(), kind.to_owned())],
    )
}

#[derive(Debug)]
pub(crate) enum OperationOutcome {
    Cache(CleanSummary),
    Process(ProcessKillReport),
    AppTerminate(ProcessKillReport),
    AppGracefulQuit(Vec<crate::applications::AppGracefulQuitReport>),
    Uninstall(Vec<UninstallReport>),
    Docker(DockerExecutionReport),
}

impl OperationOutcome {
    pub(crate) fn kind(&self) -> OperationKind {
        match self {
            Self::Cache(_) => OperationKind::Cache,
            Self::Process(_) => OperationKind::Process,
            Self::AppTerminate(_) => OperationKind::AppTerminate,
            Self::AppGracefulQuit(_) => OperationKind::AppGracefulQuit,
            Self::Uninstall(_) => OperationKind::Uninstall,
            Self::Docker(_) => OperationKind::Docker,
        }
    }
}

pub(crate) async fn execute_domain_plan<C, U, Q, D>(
    plan: ConsumedPlan,
    domains: &DomainServices<'_, C, U, Q, D>,
) -> Result<OperationOutcome, UserError>
where
    C: CacheCleaner + ?Sized,
    U: UninstallDomain + ?Sized,
    Q: AppQuitter + ?Sized,
    D: DockerDomain + ?Sized,
{
    match plan.into_plan() {
        OperationPlan::AppGracefulQuit { targets } => Ok(OperationOutcome::AppGracefulQuit(
            run_app_graceful_quit(targets, domains.app_quit).await?,
        )),
        OperationPlan::Cache { items, .. } => {
            Ok(OperationOutcome::Cache(domains.cache.clean(items).await))
        }
        OperationPlan::Uninstall {
            targets,
            quit_running,
        } => Ok(OperationOutcome::Uninstall(
            run_uninstall(targets, quit_running, domains.uninstall).await?,
        )),
        OperationPlan::Docker {
            action,
            targets,
            inventory,
        } => Ok(OperationOutcome::Docker(
            run_docker(action, targets, &inventory, domains.docker).await?,
        )),
        OperationPlan::Process { .. } | OperationPlan::AppTerminate { .. } => Err(UserError::new(
            ErrorCode::PROCESS_PLAN_REQUIRES_BLOCKING,
            "进程操作必须通过阻塞执行通道",
        )),
    }
}

pub(crate) fn execute_termination_plan(
    plan: ConsumedPlan,
    processes: &mut ProcessServices<'_>,
) -> Result<OperationOutcome, UserError> {
    match plan.into_plan() {
        OperationPlan::Process { mode, targets } => Ok(OperationOutcome::Process(
            terminate_targets_now(&targets, mode, processes.observer, processes.signaller)?,
        )),
        OperationPlan::AppTerminate { mode, targets } => {
            Ok(OperationOutcome::AppTerminate(terminate_targets_now(
                &expand_app_targets(&targets)?,
                mode,
                processes.observer,
                processes.signaller,
            )?))
        }
        _ => Err(UserError::new(
            ErrorCode::BLOCKING_CHANNEL_PLAN_MISMATCH,
            "阻塞执行通道只接受进程与应用终止计划",
        )),
    }
}

pub(crate) async fn execute_cache_plan(plan: ConsumedPlan) -> Result<CleanSummary, UserError> {
    execute_cache_plan_with(plan, &SystemCacheCleaner).await
}

pub(crate) async fn execute_cache_plan_with<C: CacheCleaner + ?Sized>(
    plan: ConsumedPlan,
    cleaner: &C,
) -> Result<CleanSummary, UserError> {
    let OperationPlan::Cache { items, .. } = plan.into_plan() else {
        return Err(plan_kind_mismatch("缓存"));
    };
    Ok(cleaner.clean(items).await)
}

pub(crate) async fn execute_process_plan<O, S>(
    plan: ConsumedPlan,
    observer: &mut O,
    signaller: &mut S,
) -> Result<ProcessKillReport, UserError>
where
    O: ProcessObserver + ?Sized,
    S: ProcessSignaller + ?Sized,
{
    let OperationPlan::Process { mode, targets } = plan.into_plan() else {
        return Err(plan_kind_mismatch("进程"));
    };
    terminate_targets(targets, mode, observer, signaller).await
}

pub(crate) async fn execute_app_termination_plan<O, S>(
    plan: ConsumedPlan,
    observer: &mut O,
    signaller: &mut S,
) -> Result<ProcessKillReport, UserError>
where
    O: ProcessObserver + ?Sized,
    S: ProcessSignaller + ?Sized,
{
    let OperationPlan::AppTerminate { mode, targets } = plan.into_plan() else {
        return Err(plan_kind_mismatch("应用终止"));
    };
    let processes = expand_app_targets(&targets)?;
    terminate_targets(processes, mode, observer, signaller).await
}

async fn terminate_targets<O, S>(
    targets: Vec<ProcessTarget>,
    mode: ProcessMode,
    observer: &mut O,
    signaller: &mut S,
) -> Result<ProcessKillReport, UserError>
where
    O: ProcessObserver + ?Sized,
    S: ProcessSignaller + ?Sized,
{
    terminate_targets_now(&targets, mode, observer, signaller)
}

fn terminate_targets_now<O, S>(
    targets: &[ProcessTarget],
    mode: ProcessMode,
    observer: &mut O,
    signaller: &mut S,
) -> Result<ProcessKillReport, UserError>
where
    O: ProcessObserver + ?Sized,
    S: ProcessSignaller + ?Sized,
{
    if targets.is_empty() {
        return Err(UserError::new(
            ErrorCode::NO_TERMINABLE_PROCESS_TARGETS,
            "没有可终止的进程目标",
        ));
    }
    ensure_targets_terminable(targets, mode)?;
    let pids: Vec<u32> = targets.iter().map(|target| target.identity.pid).collect();
    let live = observer.observe(&pids)?;
    revalidate_targets(targets, &live)?;
    Ok(signaller.terminate_all(targets, mode))
}

fn revalidate_targets(targets: &[ProcessTarget], live: &[LiveProcess]) -> Result<(), UserError> {
    for target in targets {
        let pid = target.identity.pid;
        let current = live
            .iter()
            .find(|process| process.identity.pid == pid)
            .ok_or_else(|| {
                UserError::one(
                    ErrorCode::PROCESS_GONE,
                    format!("进程已不存在（PID {pid}），请重新扫描"),
                    "pid",
                    pid,
                )
            })?;
        if current.identity.start_time != target.identity.start_time {
            return Err(UserError::one(
                ErrorCode::PROCESS_PID_REUSED,
                format!("PID {pid} 已被其他进程复用，请重新扫描"),
                "pid",
                pid,
            ));
        }
        if current.identity.name != target.identity.name
            || current.identity.exe != target.identity.exe
        {
            return Err(UserError::one(
                ErrorCode::PROCESS_IDENTITY_CHANGED,
                format!("进程身份已变化（PID {pid}），请重新扫描"),
                "pid",
                pid,
            ));
        }
        if current.protected != target.protected || current.whitelisted != target.whitelisted {
            return Err(UserError::one(
                ErrorCode::PROCESS_PROTECTION_CHANGED,
                format!("进程保护状态已变化（PID {pid}），请重新扫描"),
                "pid",
                pid,
            ));
        }
    }
    Ok(())
}

fn expand_app_targets(apps: &[AppIdentity]) -> Result<Vec<ProcessTarget>, UserError> {
    let mut expanded: Vec<ProcessTarget> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for app in apps {
        if app.processes.is_empty() {
            return Err(UserError::one(
                ErrorCode::APP_NO_TERMINABLE_PROCESS,
                format!("应用 {} 没有可终止的进程，请重新扫描", app.app_name),
                "app",
                &app.app_name,
            ));
        }
        let bundle_prefix = format!("{}/", app.bundle_path);
        for process in &app.processes {
            if !process.identity.exe.starts_with(&bundle_prefix) {
                return Err(UserError::with(
                    ErrorCode::APP_CHILD_NOT_IN_APP,
                    format!(
                        "应用 {} 的子进程 {} 不属于该应用，请重新扫描",
                        app.app_name, process.identity.name
                    ),
                    vec![
                        ("app".to_owned(), app.app_name.clone()),
                        ("process".to_owned(), process.identity.name.clone()),
                    ],
                ));
            }
            if seen.insert(process.identity.pid) {
                expanded.push(process.clone());
            }
        }
    }
    Ok(expanded)
}

pub(crate) trait AppQuitter: Send + Sync {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>>;
    fn quit(&self, app: &InstalledAppIdentity) -> DomainFuture<'_, Result<(), UserError>>;
}

pub(crate) async fn run_app_graceful_quit<Q: AppQuitter + ?Sized>(
    targets: Vec<AppIdentity>,
    domain: &Q,
) -> Result<Vec<crate::applications::AppGracefulQuitReport>, UserError> {
    if targets.is_empty() {
        return Err(UserError::new(
            ErrorCode::NO_QUITTABLE_APP_TARGETS,
            "没有可退出的应用目标",
        ));
    }
    let live = domain
        .observe_apps(&bundle_paths_for_apps(&targets))
        .await?;
    let mut reports = Vec::with_capacity(targets.len());
    for app in &targets {
        let identity = installed_identity_of(app);
        revalidate_installed_app(&identity, &live)?;
        let quit_error = domain.quit(&identity).await.err();
        reports.push(crate::applications::AppGracefulQuitReport {
            app_name: app.app_name.clone(),
            bundle_id: app.bundle_id.clone(),
            quit_error,
        });
    }
    Ok(reports)
}

fn installed_identity_of(app: &AppIdentity) -> InstalledAppIdentity {
    InstalledAppIdentity {
        bundle_path: app.bundle_path.clone(),
        app_name: app.app_name.clone(),
        bundle_id: app.bundle_id.clone(),
        is_system: false,
        bundle_size_bytes: 0,
    }
}

fn bundle_paths_for_apps(targets: &[AppIdentity]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::with_capacity(targets.len());
    for target in targets {
        if !paths.contains(&target.bundle_path) {
            paths.push(target.bundle_path.clone());
        }
    }
    paths
}

pub(crate) trait UninstallDomain: Send + Sync {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>>;
    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>>;
    fn quit(&self, app_name: &str) -> DomainFuture<'_, Result<(), UserError>>;
    fn remove(
        &self,
        app: &InstalledAppIdentity,
        residues: &[ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport>;
}

pub(crate) trait DockerDomain: Send + Sync {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>>;
    fn remove(
        &self,
        action: DockerAction,
        target: &DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>>;
    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>>;
}

pub(crate) async fn execute_uninstall_plan<D: UninstallDomain + ?Sized>(
    plan: ConsumedPlan,
    domain: &D,
) -> Result<Vec<UninstallReport>, UserError> {
    let OperationPlan::Uninstall {
        targets,
        quit_running,
    } = plan.into_plan()
    else {
        return Err(plan_kind_mismatch("卸载"));
    };
    run_uninstall(targets, quit_running, domain).await
}

async fn run_uninstall<D: UninstallDomain + ?Sized>(
    targets: Vec<UninstallPlanTarget>,
    quit_running: bool,
    domain: &D,
) -> Result<Vec<UninstallReport>, UserError> {
    if targets.is_empty() {
        return Err(UserError::new(
            ErrorCode::NO_UNINSTALLABLE_APP_TARGETS,
            "没有可卸载的应用目标",
        ));
    }
    let live_apps = domain.observe_apps(&bundle_paths(&targets)).await?;
    let requested_residues = residue_paths(&targets);
    let live_residues = domain.observe_residues(&requested_residues).await?;
    if live_residues.len() != requested_residues.len() {
        return Err(UserError::new(
            ErrorCode::RESIDUE_RECHECK_COUNT_MISMATCH,
            "残留复核结果数量不一致，请重新扫描",
        ));
    }
    revalidate_uninstall_targets(&targets, &live_apps, &live_residues)?;
    let mut reports = Vec::with_capacity(targets.len());
    for target in targets {
        let quit_error = if quit_running {
            domain.quit(&target.app.app_name).await.err()
        } else {
            None
        };
        let mut report = domain.remove(&target.app, &target.residues).await;
        report.quit_error = quit_error;
        reports.push(report);
    }
    Ok(reports)
}

fn bundle_paths(targets: &[UninstallPlanTarget]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::with_capacity(targets.len());
    for target in targets {
        if !paths.contains(&target.app.bundle_path) {
            paths.push(target.app.bundle_path.clone());
        }
    }
    paths
}

fn residue_paths(targets: &[UninstallPlanTarget]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for target in targets {
        for residue in &target.residues {
            paths.push(residue.path.clone());
        }
    }
    paths
}

pub(crate) fn revalidate_uninstall_targets(
    targets: &[UninstallPlanTarget],
    live_apps: &[InstalledAppIdentity],
    live_residues: &[LiveResidue],
) -> Result<(), UserError> {
    let mut cursor = 0usize;
    for target in targets {
        revalidate_installed_app(&target.app, live_apps)?;
        revalidate_target_residues(target, live_residues, &mut cursor)?;
    }
    Ok(())
}

pub(crate) fn revalidate_installed_app(
    app: &InstalledAppIdentity,
    live_apps: &[InstalledAppIdentity],
) -> Result<(), UserError> {
    let current = live_apps
        .iter()
        .find(|candidate| candidate.bundle_path == app.bundle_path)
        .ok_or_else(|| {
            UserError::one(
                ErrorCode::APP_GONE,
                format!("应用 {} 已不存在，请重新扫描", app.app_name),
                "app",
                &app.app_name,
            )
        })?;
    if current.bundle_id != app.bundle_id {
        return Err(UserError::one(
            ErrorCode::APP_BUNDLE_ID_CHANGED,
            format!("应用 {} 的 bundle ID 已变化，请重新扫描", app.app_name),
            "app",
            &app.app_name,
        ));
    }
    if current.app_name != app.app_name {
        return Err(UserError::one(
            ErrorCode::APP_NAME_CHANGED,
            format!("应用 {} 的名称已变化，请重新扫描", app.app_name),
            "app",
            &app.app_name,
        ));
    }
    Ok(())
}

fn revalidate_target_residues(
    target: &UninstallPlanTarget,
    live_residues: &[LiveResidue],
    cursor: &mut usize,
) -> Result<(), UserError> {
    let roots = residue_policy::allowed_roots(&target.app.bundle_id);
    let mut seen: HashSet<&str> = HashSet::with_capacity(target.residues.len());
    for residue in &target.residues {
        if residue.app_key != target.app_key {
            // ⚠️ 必须与 `operations_prepare.rs` 的同名理由分成两枚 code：
            // 前者的旧文案是「残留选择项不属于所选应用」（**不含**「请重新
            // 扫描」，旧分类 = failed），本条旧文案含「请重新扫描」
            // （旧分类 = stale）。共用一枚 code 就必然有一个分类被改掉，
            // 违反「改造前后分类结果必须逐条一致」。
            return Err(residue_rejected(
                ErrorCode::RESIDUE_NOT_IN_APP,
                "不属于该应用",
                &residue.path,
            ));
        }
        if !seen.insert(residue.path.as_str()) {
            return Err(residue_rejected(
                ErrorCode::RESIDUE_PATH_DUPLICATED,
                "残留路径重复",
                &residue.path,
            ));
        }
        residue_policy::ensure_within_roots(Path::new(&residue.path), &roots)?;
        revalidate_one_residue(residue, live_residues, cursor)?;
    }
    Ok(())
}

/// 卸载侧的残留复核里五种「这一项不能动」的理由。
///
/// 全部是「残留 <path> <理由>，请重新扫描」这一句式，差别只有理由与 code；
/// 合成一个函数是为了让 5 处的**旧文案**逐字一致（既有测试逐字比对），也避免
/// 复制粘贴漏改一处。
fn residue_rejected(code: ErrorCode, reason: &str, path: &str) -> UserError {
    let message = if code == ErrorCode::RESIDUE_PATH_DUPLICATED {
        format!("残留路径重复：{path}")
    } else {
        format!("残留 {path} {reason}，请重新扫描")
    };
    UserError::one(code, message, "path", path)
}

/// 现场状态与快照必须逐项吻合（存在性 + 路径不变）。
fn revalidate_one_residue(
    residue: &ResidueIdentity,
    live_residues: &[LiveResidue],
    cursor: &mut usize,
) -> Result<(), UserError> {
    let live = live_residues
        .get(*cursor)
        .ok_or_else(|| residue_rejected(ErrorCode::RESIDUE_GONE, "已不存在", &residue.path))?;
    *cursor += 1;
    if !live.exists {
        return Err(residue_rejected(
            ErrorCode::RESIDUE_GONE,
            "已不存在",
            &residue.path,
        ));
    }
    if live.path != residue.path {
        return Err(residue_rejected(
            ErrorCode::RESIDUE_PATH_CHANGED,
            "路径已变化",
            &residue.path,
        ));
    }
    Ok(())
}

pub(crate) async fn execute_docker_plan<D: DockerDomain + ?Sized>(
    plan: ConsumedPlan,
    domain: &D,
) -> Result<DockerExecutionReport, UserError> {
    let OperationPlan::Docker {
        action,
        targets,
        inventory,
    } = plan.into_plan()
    else {
        return Err(plan_kind_mismatch(" Docker "));
    };
    run_docker(action, targets, &inventory, domain).await
}

async fn run_docker<D: DockerDomain + ?Sized>(
    action: DockerAction,
    targets: Vec<DockerTarget>,
    planned: &DockerInventoryFingerprint,
    domain: &D,
) -> Result<DockerExecutionReport, UserError> {
    let live = domain.inventory().await?;
    if !live.daemon_running {
        return Err(UserError::new(
            ErrorCode::DOCKER_NOT_RUNNING,
            "Docker 未运行，无法执行操作，请先启动 Docker",
        ));
    }
    if action == DockerAction::Prune {
        return execute_docker_prune(domain, planned, &live).await;
    }
    if targets.is_empty() {
        return Err(UserError::new(
            ErrorCode::NO_DOCKER_TARGETS,
            "没有可删除的 Docker 资源",
        ));
    }
    revalidate_docker_targets(&targets, &live)?;
    let mut report = DockerExecutionReport::new(action);
    for target in targets {
        match domain.remove(action, &target).await {
            Ok(()) => report.succeeded.push(target.name.clone()),
            Err(error) => report.failed.push((target.name.clone(), error.message)),
        }
    }
    Ok(report)
}

async fn execute_docker_prune<D: DockerDomain + ?Sized>(
    domain: &D,
    planned: &DockerInventoryFingerprint,
    live: &DockerInventory,
) -> Result<DockerExecutionReport, UserError> {
    let current = inventory_fingerprint(live);
    if current != *planned {
        return Err(UserError::new(
            ErrorCode::DOCKER_INVENTORY_CHANGED,
            "Docker 资源清单已变化，请重新扫描后再清理",
        ));
    }
    let output = domain.prune().await?;
    let mut report = DockerExecutionReport::new(DockerAction::Prune);
    report.output = output;
    Ok(report)
}

pub(crate) fn revalidate_docker_targets(
    targets: &[DockerTarget],
    inventory: &DockerInventory,
) -> Result<(), UserError> {
    for target in targets {
        let kind = target.resource_type.label();
        let live: LiveDockerResource =
            find_live_resource(inventory, target.resource_type, &target.id).ok_or_else(|| {
                UserError::with(
                    ErrorCode::DOCKER_RESOURCE_GONE,
                    format!("Docker {kind} {} 已不存在，请重新扫描", target.name),
                    vec![
                        ("kind".to_owned(), kind.to_owned()),
                        ("name".to_owned(), target.name.clone()),
                    ],
                )
            })?;
        if live.name != target.name {
            return Err(UserError::with(
                ErrorCode::DOCKER_ID_REUSED,
                format!(
                    "Docker {kind} ID {} 已被其他资源占用，请重新扫描",
                    target.id
                ),
                vec![
                    ("kind".to_owned(), kind.to_owned()),
                    ("id".to_owned(), target.id.clone()),
                ],
            ));
        }
        if live.referenced != target.referenced {
            return Err(UserError::with(
                ErrorCode::DOCKER_REFERENCED_CHANGED,
                format!("Docker {kind} {} 引用状态已变化，请重新扫描", target.name),
                vec![
                    ("kind".to_owned(), kind.to_owned()),
                    ("name".to_owned(), target.name.clone()),
                ],
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "operation_executor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "operation_executor_dispatch_tests.rs"]
mod dispatch_tests;

#[cfg(test)]
#[path = "operation_executor_app_quit_tests.rs"]
mod app_quit_tests;

#[cfg(test)]
#[path = "operation_executor_process_tests.rs"]
mod process_tests;

#[cfg(test)]
#[path = "operation_executor_uninstall_tests.rs"]
mod uninstall_tests;

#[cfg(test)]
#[path = "operation_executor_docker_tests.rs"]
mod docker_tests;
