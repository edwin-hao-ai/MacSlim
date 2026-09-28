use super::*;

// ===== prepare =====

struct DomainFixtures {
    cache: SnapshotResult<CacheSnapshotView>,
    processes: SnapshotResult<scanner::ScanResult>,
    apps: SnapshotResult<Vec<applications::AppInfo>>,
    installed: SnapshotResult<Vec<InstalledApp>>,
    residues: SnapshotResult<Vec<ResidueAppGroup>>,
    inventory: SnapshotResult<docker::DockerInventory>,
}

fn domain_fixtures(harness: &mut Harness) -> DomainFixtures {
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let mut guard = harness.store.lock().unwrap();
    let store = &mut *guard;
    let cache = snapshot_cache(
        store,
        CacheScanResult {
            items: vec![cache_item("npm")],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照");
    let processes = snapshot_process_scan(store, scan_result_fixture(), &policy).expect("进程快照");
    let apps = snapshot_applications(store, vec![app_info("Alpha")], &policy).expect("应用快照");
    let installed =
        snapshot_installed_apps(store, vec![installed_app("Alpha")]).expect("已安装应用快照");
    let inventory =
        snapshot_docker_inventory(store, docker_inventory_fixture()).expect("Docker 快照");
    drop(guard);
    let residues = scan_residue_batch(
        harness,
        &installed.snapshot_id,
        std::slice::from_ref(&installed.value[0].selection_key),
        residue_for,
    )
    .expect("残留快照");
    DomainFixtures {
        cache,
        processes,
        apps,
        installed,
        residues,
        inventory,
    }
}

fn prepare_cases(fixtures: &DomainFixtures) -> Vec<(PrepareOperationRequest, &'static str, usize)> {
    let mut cases = scan_prepare_cases(fixtures);
    cases.extend(vec![
        (
            uninstall_request(
                &fixtures.installed.snapshot_id,
                &fixtures.residues.snapshot_id,
                vec![key_of(&fixtures.installed.value[0].selection_key)],
                vec![key_of(
                    &fixtures.residues.value[0].residue.items[0].selection_key,
                )],
            ),
            "uninstall",
            2,
        ),
        (
            docker_request(
                &fixtures.inventory.snapshot_id,
                DockerAction::Prune,
                Vec::new(),
            ),
            "docker",
            1,
        ),
    ]);
    cases
}

fn scan_prepare_cases(
    fixtures: &DomainFixtures,
) -> Vec<(PrepareOperationRequest, &'static str, usize)> {
    vec![
        (
            cache_request(
                &fixtures.cache.snapshot_id,
                vec![key_of(&fixtures.cache.value.items[0].selection_key)],
            ),
            "cache",
            1,
        ),
        (
            process_request(
                &fixtures.processes.snapshot_id,
                vec![key_of(&fixtures.processes.value.processes[0].selection_key)],
                ProcessMode::Graceful,
            ),
            "process",
            1,
        ),
        (
            app_request(
                &fixtures.apps.snapshot_id,
                vec![key_of(&fixtures.apps.value[0].selection_key)],
                ProcessMode::Force,
            ),
            "app_terminate",
            1,
        ),
        (
            graceful_quit_request(
                &fixtures.apps.snapshot_id,
                vec![key_of(&fixtures.apps.value[0].selection_key)],
            ),
            "app_graceful_quit",
            1,
        ),
    ]
}

#[test]
fn prepare_operation_returns_metadata_for_every_request_kind() {
    let mut harness = Harness::new();
    let fixtures = domain_fixtures(&mut harness);

    for (request, kind, item_count) in prepare_cases(&fixtures) {
        let prepared = prepare(&mut harness, request)
            .unwrap_or_else(|error| panic!("{kind} 准备失败: {error}"));
        assert_eq!(prepared.kind, kind);
        assert_eq!(prepared.item_count, item_count);
        assert!(!prepared.operation_id.is_empty());
        assert!(prepared.expires_at_ms > 0);
        assert!(!prepared.summary_key.is_empty());
        assert!(
            !prepared.summary_key.contains("Cache") && !prepared.summary_key.contains("Process")
        );
    }
}

#[test]
fn prepare_operation_rejects_raw_target_fields() {
    let forbidden = [
        r#"{"type":"process","snapshot_id":"s","process_keys":["k"],"pid":42}"#,
        r#"{"type":"cache","snapshot_id":"s","item_keys":["k"],"path":"/etc/passwd"}"#,
        r#"{"type":"cache","snapshot_id":"s","item_keys":["k"],"command":"rm -rf /"}"#,
        r#"{"type":"uninstall","app_snapshot_id":"a","residue_snapshot_id":"r","app_keys":["k"],"residue_keys":[],"quit_running":false,"bundle_path":"/Applications/Alpha.app"}"#,
        r#"{"type":"docker","snapshot_id":"s","action":"remove_image","target_keys":["k"],"args":["rm","-f"]}"#,
        r#"{"type":"app_terminate","snapshot_id":"s","app_keys":["k"],"mode":"graceful","name":"Alpha"}"#,
        r#"{"type":"app_graceful_quit","snapshot_id":"s","app_keys":["k"],"app_name":"Alpha"}"#,
        r#"{"type":"app_graceful_quit","snapshot_id":"s","app_keys":["k"],"mode":"force"}"#,
        r#"{"type":"app_graceful_quit","snapshot_id":"s","app_keys":["k"],"bundle_path":"/Applications/Alpha.app"}"#,
    ];

    for payload in forbidden {
        assert!(
            serde_json::from_str::<PrepareOperationRequest>(payload).is_err(),
            "必须拒绝携带 raw 字段的请求: {payload}"
        );
    }
}

#[test]
fn prepare_operation_rejects_unknown_or_empty_selections() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let processes = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("进程快照");

    assert!(prepare(
        &mut harness,
        process_request(&processes.snapshot_id, Vec::new(), ProcessMode::Graceful)
    )
    .is_err());
    assert!(prepare(
        &mut harness,
        process_request(
            &processes.snapshot_id,
            vec!["forged".to_owned()],
            ProcessMode::Graceful
        )
    )
    .is_err());
    assert!(prepare(
        &mut harness,
        process_request(
            "forged-snapshot",
            vec!["k".to_owned()],
            ProcessMode::Graceful
        )
    )
    .is_err());
}

#[test]
fn prepare_operation_does_not_execute_anything() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let processes = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("进程快照");
    let signaller = RecordingSignaller::default();
    let observer = RecordingObserver {
        observed: Arc::new(Mutex::new(Vec::new())),
        live: vec![live_process(11, "alpha")],
    };

    let prepared = prepare(
        &mut harness,
        process_request(
            &processes.snapshot_id,
            vec![key_of(&processes.value.processes[0].selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .expect("prepare 必须成功");

    assert!(harness.cache.calls.lock().unwrap().is_empty());
    assert!(harness.uninstall.calls.lock().unwrap().is_empty());
    assert!(harness.docker.removed.lock().unwrap().is_empty());
    assert!(signaller.calls.lock().unwrap().is_empty());
    assert!(observer.observed.lock().unwrap().is_empty());
    assert!(harness
        .store
        .lock()
        .unwrap()
        .consume(&prepared.operation_id, "main")
        .is_ok());
}
