use super::*;
use crate::operations::{
    ConsumedPlan, DockerAction, DockerResourceKind, DockerTarget, InstalledAppIdentity,
    OperationStore, ResidueIdentity,
};
use crate::uninstaller::{MoveResult, UninstallReport};
use crate::user_error::UserError;
use std::sync::{Arc, Mutex};

fn installed_identity(label: &str) -> InstalledAppIdentity {
    InstalledAppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        app_name: label.to_owned(),
        bundle_id: format!("com.example.{label}"),
        is_system: false,
        bundle_size_bytes: 0,
    }
}

fn residue_identity(app_key: &str, label: &str, size: u64) -> ResidueIdentity {
    ResidueIdentity {
        app_key: app_key.to_owned(),
        path: library_path(label),
        category: "Caches".to_owned(),
        size_bytes: size,
    }
}

fn library_path(label: &str) -> String {
    dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(format!("com.example.{label}"))
        .to_string_lossy()
        .to_string()
}

fn live_residue(path: &str) -> LiveResidue {
    LiveResidue {
        path: path.to_owned(),
        exists: true,
    }
}

type RemoveCall = (InstalledAppIdentity, Vec<String>);

#[derive(Clone, Default)]
struct FakeUninstallDomain {
    live_apps: Arc<Mutex<Vec<InstalledAppIdentity>>>,
    live_residues: Arc<Mutex<Vec<LiveResidue>>>,
    observe_error: Arc<Mutex<Option<UserError>>>,
    quit_error: Arc<Mutex<Option<UserError>>>,
    observed_apps: Arc<Mutex<Vec<Vec<String>>>>,
    observed_residues: Arc<Mutex<Vec<Vec<String>>>>,
    quit_calls: Arc<Mutex<Vec<String>>>,
    remove_calls: Arc<Mutex<Vec<RemoveCall>>>,
}

impl FakeUninstallDomain {
    fn new(apps: Vec<InstalledAppIdentity>, residues: Vec<LiveResidue>) -> Self {
        Self {
            live_apps: Arc::new(Mutex::new(apps)),
            live_residues: Arc::new(Mutex::new(residues)),
            ..Self::default()
        }
    }

    fn with_observe_error(message: &str) -> Self {
        let domain = Self::default();
        *domain.observe_error.lock().unwrap() = Some(UserError::from(message));
        domain
    }

    fn with_quit_error(message: &str) -> Self {
        let domain = Self::default();
        *domain.quit_error.lock().unwrap() = Some(UserError::from(message));
        domain
    }

    fn removed_paths(&self) -> Vec<String> {
        self.remove_calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, paths)| paths.join(","))
            .collect()
    }
}

impl UninstallDomain for FakeUninstallDomain {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        let requested = bundle_paths.to_vec();
        self.observed_apps.lock().unwrap().push(requested.clone());
        if let Some(error) = self.observe_error.lock().unwrap().clone() {
            return Box::pin(async move { Err(error) });
        }
        let live = self.live_apps.lock().unwrap().clone();
        Box::pin(async move {
            Ok(live
                .into_iter()
                .filter(|app| requested.contains(&app.bundle_path))
                .collect())
        })
    }

    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        self.observed_residues.lock().unwrap().push(paths.to_vec());
        let answers = self.live_residues.lock().unwrap().clone();
        Box::pin(async move { Ok(answers) })
    }

    fn quit(&self, app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        self.quit_calls.lock().unwrap().push(app_name.to_owned());
        let error = self.quit_error.lock().unwrap().clone();
        Box::pin(async move { error.map_or(Ok(()), Err) })
    }

    fn remove(
        &self,
        app: &InstalledAppIdentity,
        residues: &[ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        self.remove_calls.lock().unwrap().push((
            app.clone(),
            residues
                .iter()
                .map(|residue| residue.path.clone())
                .collect(),
        ));
        let report = UninstallReport {
            app_name: app.app_name.clone(),
            bundle_id: app.bundle_id.clone(),
            total_freed_bytes: residues.iter().map(|residue| residue.size_bytes).sum(),
            moved_count: residues.len(),
            failed_count: 0,
            details: residues
                .iter()
                .map(|residue| MoveResult {
                    path: residue.path.clone(),
                    success: true,
                    error: None,
                    size_bytes: residue.size_bytes,
                })
                .collect(),
            quit_error: None,
        };
        Box::pin(async move { report })
    }
}

fn consumed_uninstall_plan(
    store: &mut OperationStore,
    apps: Vec<(InstalledAppIdentity, Vec<ResidueIdentity>)>,
    quit_running: bool,
) -> ConsumedPlan {
    let installed = store
        .register_installed_apps(apps.iter().map(|(app, _)| app.clone()).collect())
        .unwrap();
    let residues = store
        .register_residues(
            apps.iter()
                .zip(&installed.selection_keys)
                .flat_map(|((_, residues), app_key)| {
                    residues.iter().map(move |residue| ResidueIdentity {
                        app_key: app_key.clone(),
                        ..residue.clone()
                    })
                })
                .collect(),
        )
        .unwrap();
    let app_keys: Vec<&str> = installed
        .selection_keys
        .iter()
        .map(String::as_str)
        .collect();
    let residue_keys: Vec<&str> = residues.selection_keys.iter().map(String::as_str).collect();
    let prepared = store
        .prepare_uninstall(
            &installed.snapshot_id,
            &residues.snapshot_id,
            app_keys,
            residue_keys,
            quit_running,
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn single_app_plan(store: &mut OperationStore, quit_running: bool) -> ConsumedPlan {
    consumed_uninstall_plan(
        store,
        vec![(
            installed_identity("alpha"),
            vec![residue_identity("placeholder", "alpha-cache", 64)],
        )],
        quit_running,
    )
}

fn alpha_domain() -> FakeUninstallDomain {
    FakeUninstallDomain::new(
        vec![installed_identity("alpha")],
        vec![live_residue(&library_path("alpha-cache"))],
    )
}

fn consumed_docker_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_docker_resources(vec![DockerTarget {
            resource_type: DockerResourceKind::Image,
            id: "sha256:a".into(),
            name: "nginx:latest".into(),
            size_bytes: 1024,
            referenced: false,
            reclaimable: false,
        }])
        .unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

#[tokio::test]
async fn uninstall_plan_revalidates_then_dispatches_typed_remove() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain = alpha_domain();

    let reports = execute_uninstall_plan(consumed, &domain).await.unwrap();

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].app_name, "alpha");
    assert_eq!(reports[0].bundle_id, "com.example.alpha");
    assert_eq!(reports[0].total_freed_bytes, 64);
    assert_eq!(domain.removed_paths(), vec![library_path("alpha-cache")]);
    assert!(domain.quit_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_observation_is_scoped_to_backend_paths() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain = alpha_domain();

    execute_uninstall_plan(consumed, &domain).await.unwrap();

    assert_eq!(
        *domain.observed_apps.lock().unwrap(),
        vec![vec!["/Applications/alpha.app".to_owned()]]
    );
    assert_eq!(
        *domain.observed_residues.lock().unwrap(),
        vec![vec![library_path("alpha-cache")]]
    );
}

#[tokio::test]
async fn uninstall_plan_rejects_vanished_bundle() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain =
        FakeUninstallDomain::new(Vec::new(), vec![live_residue(&library_path("alpha-cache"))]);

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已不存在"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
    assert!(domain.quit_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_rejects_moved_bundle_path() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let mut moved = installed_identity("alpha");
    moved.bundle_path = "/Applications/Moved.app".to_owned();
    let domain = FakeUninstallDomain::new(
        vec![moved],
        vec![live_residue(&library_path("alpha-cache"))],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已不存在"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
    assert_eq!(
        *domain.observed_apps.lock().unwrap(),
        vec![vec!["/Applications/alpha.app".to_owned()]]
    );
}

#[tokio::test]
async fn uninstall_plan_rejects_bundle_id_change() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let mut replaced = installed_identity("alpha");
    replaced.bundle_id = "com.evil.alpha".to_owned();
    let domain = FakeUninstallDomain::new(
        vec![replaced],
        vec![live_residue(&library_path("alpha-cache"))],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("bundle ID"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_rejects_app_name_change() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let mut renamed = installed_identity("alpha");
    renamed.app_name = "Alpha Trojan".to_owned();
    let domain = FakeUninstallDomain::new(
        vec![renamed],
        vec![live_residue(&library_path("alpha-cache"))],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("名称已变化"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_rejects_vanished_residue() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain = FakeUninstallDomain::new(
        vec![installed_identity("alpha")],
        vec![LiveResidue {
            path: library_path("alpha-cache"),
            exists: false,
        }],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("残留"), "{}", error);
    assert!(error.contains("已不存在"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_rejects_residue_root_swapped_to_another_path() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain = FakeUninstallDomain::new(
        vec![installed_identity("alpha")],
        vec![live_residue(&library_path("evil-cache"))],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("路径已变化"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_signals_nothing_when_one_app_mismatches() {
    let mut store = OperationStore::new();
    let consumed = consumed_uninstall_plan(
        &mut store,
        vec![
            (
                installed_identity("alpha"),
                vec![residue_identity("p", "alpha-cache", 8)],
            ),
            (
                installed_identity("beta"),
                vec![residue_identity("p", "beta-cache", 8)],
            ),
        ],
        false,
    );
    let domain = FakeUninstallDomain::new(
        vec![installed_identity("alpha")],
        vec![
            live_residue(&library_path("alpha-cache")),
            live_residue(&library_path("beta-cache")),
        ],
    );

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已不存在"), "{}", error);
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_quits_backend_app_name_only_when_requested() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, true);
    let domain = alpha_domain();

    execute_uninstall_plan(consumed, &domain).await.unwrap();

    assert_eq!(*domain.quit_calls.lock().unwrap(), vec!["alpha"]);
    assert_eq!(domain.remove_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn uninstall_plan_keeps_uninstalling_when_quit_reports_failure() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, true);
    let domain = alpha_domain();
    *domain.quit_error.lock().unwrap() = Some(UserError::from("用户取消授权"));

    let reports = execute_uninstall_plan(consumed, &domain).await.unwrap();

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].moved_count, 1);
    assert_eq!(
        reports[0].quit_error.as_deref(),
        Some("用户取消授权"),
        "退出失败必须如实写进逐项报告"
    );
    assert_eq!(domain.remove_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn uninstall_plan_propagates_observation_failure() {
    let mut store = OperationStore::new();
    let consumed = single_app_plan(&mut store, false);
    let domain = FakeUninstallDomain::with_observe_error("无法读取应用目录");

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert_eq!(error, "无法读取应用目录");
    assert!(domain.remove_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_plan_rejects_non_uninstall_plan_after_consume() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(&mut store);
    let domain = alpha_domain();

    let error = execute_uninstall_plan(consumed, &domain).await.unwrap_err();

    assert_eq!(error, "操作计划不是卸载计划");
    assert!(domain.remove_calls.lock().unwrap().is_empty());
    assert!(domain.observed_apps.lock().unwrap().is_empty());
}

#[test]
fn uninstall_revalidation_rule_rejects_residue_bound_to_another_app() {
    let app = InstalledAppIdentity {
        bundle_path: "/Applications/alpha.app".to_owned(),
        app_name: "alpha".to_owned(),
        bundle_id: "com.example.alpha".to_owned(),
        is_system: false,
        bundle_size_bytes: 0,
    };
    let target = UninstallPlanTarget {
        app_key: "app-key-a".to_owned(),
        app: app.clone(),
        residues: vec![residue_identity("app-key-b", "alpha-cache", 8)],
    };

    let error = revalidate_uninstall_targets(
        std::slice::from_ref(&target),
        &[app],
        &[live_residue(&library_path("alpha-cache"))],
    )
    .unwrap_err();

    assert!(error.contains("不属于该应用"), "{}", error);
}

#[test]
fn uninstall_revalidation_rule_rejects_duplicate_residue_paths() {
    let app = installed_identity("alpha");
    let residue = residue_identity("app-key-a", "alpha-cache", 8);
    let target = UninstallPlanTarget {
        app_key: "app-key-a".to_owned(),
        app: app.clone(),
        residues: vec![residue.clone(), residue],
    };

    let error = revalidate_uninstall_targets(
        std::slice::from_ref(&target),
        std::slice::from_ref(&app),
        &[
            live_residue(&library_path("alpha-cache")),
            live_residue(&library_path("alpha-cache")),
        ],
    )
    .unwrap_err();

    assert!(error.contains("重复"), "{}", error);
}

#[tokio::test]
async fn uninstall_plan_accepts_residue_inside_allowed_roots() {
    let mut store = OperationStore::new();
    let app = installed_identity("alpha");
    let registration = store.register_installed_apps(vec![app.clone()]).unwrap();
    let app_key = registration.selection_keys[0].clone();
    let identities = store
        .register_residues_for_apps(vec![(
            app_key.clone(),
            vec![ResidueIdentity {
                app_key: app_key.clone(),
                path: library_path("alpha-cache"),
                category: "Caches".to_owned(),
                size_bytes: 8,
            }],
        )])
        .unwrap();
    assert_eq!(identities.selection_keys.len(), 1);
    let prepared = store
        .prepare_uninstall(
            &registration.snapshot_id,
            &identities.snapshot_id,
            vec![&app_key],
            vec![&identities.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();
    let consumed = store.consume(&prepared.operation_id, "main").unwrap();
    let domain = alpha_domain();

    let reports = execute_uninstall_plan(consumed, &domain).await.unwrap();

    assert_eq!(reports.len(), 1);
    assert_eq!(domain.removed_paths(), vec![library_path("alpha-cache")]);
}

#[test]
fn uninstall_revalidation_rejects_planned_path_outside_allowed_roots() {
    let app = installed_identity("alpha");
    let target = UninstallPlanTarget {
        app_key: "app-key-a".to_owned(),
        app,
        residues: vec![ResidueIdentity {
            app_key: "app-key-a".to_owned(),
            path: "/etc/alpha".to_owned(),
            category: "Caches".to_owned(),
            size_bytes: 8,
        }],
    };

    let error = revalidate_uninstall_targets(
        std::slice::from_ref(&target),
        std::slice::from_ref(&target.app),
        &[live_residue("/etc/alpha")],
    )
    .unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
}

#[test]
fn uninstall_revalidation_rejects_live_path_before_root_check() {
    let app = installed_identity("alpha");
    let planned = library_path("alpha-cache");
    let target = UninstallPlanTarget {
        app_key: "app-key-a".to_owned(),
        app: app.clone(),
        residues: vec![ResidueIdentity {
            app_key: "app-key-a".to_owned(),
            path: planned.clone(),
            category: "Caches".to_owned(),
            size_bytes: 8,
        }],
    };

    let error = revalidate_uninstall_targets(
        std::slice::from_ref(&target),
        &[app],
        &[live_residue("/tmp/macslim-escaped-cache")],
    )
    .unwrap_err();

    assert_eq!(error, format!("残留 {planned} 路径已变化，请重新扫描"));
}

#[test]
fn uninstall_revalidation_allows_dev_tool_root_for_its_own_bundle() {
    let app = InstalledAppIdentity {
        bundle_path: "/Applications/Code.app".to_owned(),
        app_name: "Code".to_owned(),
        bundle_id: "com.microsoft.VSCode".to_owned(),
        is_system: false,
        bundle_size_bytes: 0,
    };
    let declared = dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join(".vscode/extensions")
        .to_string_lossy()
        .to_string();
    let target = UninstallPlanTarget {
        app_key: "app-key-a".to_owned(),
        app: app.clone(),
        residues: vec![ResidueIdentity {
            app_key: "app-key-a".to_owned(),
            path: declared.clone(),
            category: "VS Code 专属数据".to_owned(),
            size_bytes: 8,
        }],
    };

    assert!(revalidate_uninstall_targets(
        std::slice::from_ref(&target),
        &[app],
        &[live_residue(&declared)]
    )
    .is_ok());
}

#[tokio::test]
async fn uninstall_plan_rejects_roots_escape_introduced_after_prepare() {
    let mut store = OperationStore::new();
    let registration = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let app_key = registration.selection_keys[0].clone();
    let identities = store
        .register_residues_for_apps(vec![(
            app_key.clone(),
            vec![ResidueIdentity {
                app_key: app_key.clone(),
                path: library_path("alpha-cache"),
                category: "Caches".to_owned(),
                size_bytes: 8,
            }],
        )])
        .unwrap();
    let prepared = store
        .prepare_uninstall(
            &registration.snapshot_id,
            &identities.snapshot_id,
            vec![&app_key],
            vec![&identities.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();
    let escaped = FakeUninstallDomain::new(
        vec![installed_identity("alpha")],
        vec![live_residue("/tmp/macslim-post-prepare-escape")],
    );

    let consumed = store.consume(&prepared.operation_id, "main").unwrap();
    let error = execute_uninstall_plan(consumed, &escaped)
        .await
        .unwrap_err();

    assert_eq!(
        error,
        format!(
            "残留 {} 路径已变化，请重新扫描",
            library_path("alpha-cache")
        )
    );
    assert!(escaped.remove_calls.lock().unwrap().is_empty());
    assert!(escaped.quit_calls.lock().unwrap().is_empty());
}
