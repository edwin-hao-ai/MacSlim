use super::*;
use crate::user_error::UserError;

// ===== scan → prepare → execute → history 跨层集成 =====

fn identity_for(bundle_path: &str) -> InstalledAppIdentity {
    let app_name = bundle_path
        .rsplit('/')
        .next()
        .unwrap_or("Unknown")
        .trim_end_matches(".app")
        .to_owned();
    InstalledAppIdentity {
        bundle_path: bundle_path.to_owned(),
        bundle_id: format!("com.example.{app_name}"),
        app_name,
        is_system: false,
        bundle_size_bytes: 2_048,
    }
}

struct IdentityUninstall {
    calls: Arc<Mutex<Vec<String>>>,
}

impl IdentityUninstall {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl UninstallDomain for IdentityUninstall {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        let observed = bundle_paths.iter().map(|path| identity_for(path)).collect();
        Box::pin(async move { Ok(observed) })
    }

    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        let observed = paths
            .iter()
            .map(|path| LiveResidue {
                path: path.clone(),
                exists: true,
            })
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, _app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        Box::pin(async { Ok(()) })
    }

    fn remove(
        &self,
        app: &InstalledAppIdentity,
        _residues: &[crate::operations::ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        self.calls.lock().unwrap().push(app.bundle_path.clone());
        let app_name = app.app_name.clone();
        Box::pin(async move { uninstall_report(&app_name) })
    }
}

fn live_apps(names: &[&str]) -> Vec<InstalledAppIdentity> {
    names
        .iter()
        .map(|name| identity_for(&format!("/Applications/{name}.app")))
        .collect()
}

fn application_snapshot(harness: &mut Harness, names: &[&str]) -> SnapshotResult<Vec<AppInfo>> {
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    snapshot_applications(
        &mut harness.store.lock().unwrap(),
        names.iter().copied().map(app_info).collect(),
        &policy,
    )
    .expect("应用快照必须注册成功")
}

fn assert_audit_is_operation_id_free(entries: &[OperationHistoryEntry], operation_id: &str) {
    for entry in entries {
        assert!(
            !entry.detail.contains(operation_id),
            "detail 泄露 operation id"
        );
        assert!(
            !entry.target.contains(operation_id),
            "target 泄露 operation id"
        );
    }
}

#[tokio::test]
async fn graceful_quit_walks_scan_prepare_execute_and_history() {
    let mut harness = Harness::new();
    harness.app_quit = RecordingAppQuitter::with_live(live_apps(&["Alpha"]));
    let apps = application_snapshot(&mut harness, &["Alpha"]);
    let prepared = prepare(
        &mut harness,
        graceful_quit_request(
            &apps.snapshot_id,
            vec![key_of(&apps.value[0].selection_key)],
        ),
    )
    .expect("优雅退出计划");
    assert_eq!(prepared.kind, "app_graceful_quit");
    assert_eq!(prepared.item_count, 1);
    assert!(!prepared.summary_key.contains(&prepared.operation_id));

    let domains = harness.domains();
    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect("优雅退出执行必须成功");

    match result {
        OperationResult::AppGracefulQuit(reports) => {
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].app_name, "Alpha");
            assert_eq!(reports[0].bundle_id, "com.example.Alpha");
            assert!(reports[0].quit_error.is_none());
        }
        other => panic!("期望优雅退出结果，得到 {other:?}"),
    }
    assert_eq!(
        harness.app_quit.quit_names.lock().unwrap().as_slice(),
        &["Alpha".to_owned()]
    );
    assert!(harness.cache.calls.lock().unwrap().is_empty());
    assert!(harness.uninstall.calls.lock().unwrap().is_empty());
    assert!(harness.docker.removed.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "app_graceful_quit");
    assert_eq!(entries[0].target, "1 个应用");
    assert!(entries[0].success);
    assert_audit_is_operation_id_free(&entries, &prepared.operation_id);
}

#[tokio::test]
async fn graceful_quit_is_rejected_after_a_rescan_without_quitting_anything() {
    let mut harness = Harness::new();
    harness.app_quit = RecordingAppQuitter::with_live(live_apps(&["Alpha"]));
    let apps = application_snapshot(&mut harness, &["Alpha"]);
    let prepared = prepare(
        &mut harness,
        graceful_quit_request(
            &apps.snapshot_id,
            vec![key_of(&apps.value[0].selection_key)],
        ),
    )
    .expect("优雅退出计划");
    let _rescanned = application_snapshot(&mut harness, &["Alpha"]);

    let domains = harness.domains();
    let history = harness.history.clone();
    let error = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect_err("重扫后旧 operation 必须失效");

    assert!(
        error.contains("已失效") || error.contains("快照"),
        "{error}"
    );
    assert!(harness.app_quit.quit_names.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert!(
        entries.is_empty(),
        "计划在 store 层已被作废，没有可执行动作，不应产生审计记录"
    );
}

#[tokio::test]
async fn graceful_quit_and_forced_termination_share_one_application_snapshot() {
    let mut harness = Harness::new();
    harness.app_quit = RecordingAppQuitter::with_live(live_apps(&["Alpha"]));
    let apps = application_snapshot(&mut harness, &["Alpha"]);
    let app_key = vec![key_of(&apps.value[0].selection_key)];
    let graceful = prepare(
        &mut harness,
        graceful_quit_request(&apps.snapshot_id, app_key.clone()),
    )
    .expect("优雅退出计划");
    let forced = prepare(
        &mut harness,
        app_request(&apps.snapshot_id, app_key, ProcessMode::Force),
    )
    .expect("强制终止计划");
    assert_ne!(graceful.operation_id, forced.operation_id);
    assert_eq!(graceful.kind, "app_graceful_quit");
    assert_eq!(forced.kind, "app_terminate");

    let rejection = prepare(
        &mut harness,
        app_request(
            &apps.snapshot_id,
            vec![key_of(&apps.value[0].selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .expect_err("优雅模式必须被路由到 app_graceful_quit");
    assert!(rejection.contains("app_graceful_quit"), "{rejection}");

    let history = harness.history.clone();
    let domains = harness.domains();
    execute_operation_with(
        &harness.store,
        &history,
        "main",
        &graceful.operation_id,
        |plan| {
            let domains = &domains;
            async move { execute_domain_plan(plan, domains).await }
        },
    )
    .await
    .expect("优雅退出必须成功");
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "app_graceful_quit");
}

#[tokio::test]
async fn batch_residue_scan_feeds_one_uninstall_operation_for_every_app() {
    let mut harness = Harness::new();
    let apps = two_app_fixture(&mut harness);
    let selected = app_keys(&apps);
    let residues = scan_residue_batch(&mut harness, &apps.snapshot_id, &selected, residue_for)
        .expect("批量残留快照");
    assert_eq!(residues.value.len(), 2);
    let prepared = prepare(
        &mut harness,
        uninstall_request(
            &apps.snapshot_id,
            &residues.snapshot_id,
            selected,
            residue_keys(&residues),
        ),
    )
    .expect("批量卸载计划");
    assert_eq!(prepared.item_count, 4, "2 个应用本体 + 2 条残留");

    let uninstall = IdentityUninstall::new();
    let domains = DomainServices {
        cache: &harness.cache,
        uninstall: &uninstall,
        app_quit: &harness.app_quit,
        docker: &harness.docker,
    };
    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect("批量卸载执行必须成功");

    match result {
        OperationResult::Uninstall(reports) => assert_eq!(reports.len(), 2),
        other => panic!("期望卸载结果，得到 {other:?}"),
    }
    assert_eq!(uninstall.calls.lock().unwrap().len(), 2);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "uninstall");
    assert_eq!(entries[0].target, "2 个应用");
    assert_audit_is_operation_id_free(&entries, &prepared.operation_id);
}

#[tokio::test]
async fn empty_residue_selection_still_uninstalls_the_bundle() {
    let mut harness = Harness::new();
    let apps = two_app_fixture(&mut harness);
    let selected = app_keys(&apps);
    let residues = scan_residue_batch(&mut harness, &apps.snapshot_id, &selected, |identity| {
        app_residue(&identity.bundle_id, &identity.app_name, &[])
    })
    .expect("无残留也必须注册快照");
    assert!(residue_keys(&residues).is_empty());
    let prepared = prepare(
        &mut harness,
        uninstall_request(
            &apps.snapshot_id,
            &residues.snapshot_id,
            selected,
            Vec::new(),
        ),
    )
    .expect("没有残留时仍可卸载应用本体");

    let uninstall = IdentityUninstall::new();
    let domains = DomainServices {
        cache: &harness.cache,
        uninstall: &uninstall,
        app_quit: &harness.app_quit,
        docker: &harness.docker,
    };
    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect("空残留卸载必须成功");

    match result {
        OperationResult::Uninstall(reports) => assert_eq!(reports.len(), 2),
        other => panic!("期望卸载结果，得到 {other:?}"),
    }
    assert_eq!(uninstall.calls.lock().unwrap().len(), 2);
    let entries = history.entries.lock().unwrap().clone();
    assert!(entries[0].success);
    assert_eq!(entries[0].target, "2 个应用");
}

fn prune_plan(harness: &mut Harness) -> crate::operations::PreparedOperation {
    let inventory = snapshot_docker_inventory(
        &mut harness.store.lock().unwrap(),
        docker_inventory_fixture(),
    )
    .expect("Docker 快照");
    prepare(
        harness,
        docker_request(&inventory.snapshot_id, DockerAction::Prune, Vec::new()),
    )
    .expect("Docker prune 计划")
}

#[tokio::test]
async fn docker_prune_executes_only_while_the_inventory_fingerprint_holds() {
    let mut harness = Harness::new();
    let prepared = prune_plan(&mut harness);

    let domains = harness.domains();
    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect("清单未变化时 prune 必须成功");

    assert!(matches!(result, OperationResult::Docker(_)));
    assert_eq!(*harness.docker.pruned.lock().unwrap(), 1);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "docker");
    assert!(entries[0].success);
    assert_audit_is_operation_id_free(&entries, &prepared.operation_id);
}

#[tokio::test]
async fn docker_prune_is_rejected_once_the_inventory_fingerprint_changes() {
    let mut harness = Harness::new();
    let prepared = prune_plan(&mut harness);
    let mut changed = harness.docker.build_inventory();
    changed.images.push(DockerImage {
        id: "sha256-new".to_owned(),
        repository: "redis".to_owned(),
        tag: "latest".to_owned(),
        size_bytes: 700,
        created: "2026-02-01".to_owned(),
        dangling: false,
        in_use: false,
        selection_key: String::new(),
    });
    changed.reclaimable_bytes += 700;
    harness.docker.inventory = changed;

    let domains = harness.domains();
    let history = harness.history.clone();
    let error = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_domain_plan(plan, &domains).await },
    )
    .await
    .expect_err("清单变化后 prune 必须被拒绝");

    assert!(
        error.contains("清单已变化") || error.contains("重新扫描"),
        "{error}"
    );
    assert_eq!(*harness.docker.pruned.lock().unwrap(), 0);
    assert!(harness.docker.removed.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "docker");
    assert!(!entries[0].success);
    assert_audit_is_operation_id_free(&entries, &prepared.operation_id);
}

#[tokio::test]
async fn a_single_owner_can_only_consume_each_operation_once_across_kinds() {
    let mut harness = Harness::new();
    let prepared = cache_plan_fixture(&mut harness);
    let history = harness.history.clone();
    let mut outcomes = Vec::new();
    for owner in ["main", "cli", "main"] {
        let domains = harness.domains();
        outcomes.push(
            execute_operation_with(
                &harness.store,
                &history,
                owner,
                &prepared.operation_id,
                |plan| {
                    let domains = &domains;
                    async move { execute_domain_plan(plan, domains).await }
                },
            )
            .await
            .is_ok(),
        );
    }

    assert_eq!(outcomes, vec![true, false, false]);
    assert_eq!(harness.cache.calls.lock().unwrap().len(), 1);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "cache");
}
