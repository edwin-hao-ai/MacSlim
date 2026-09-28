use super::*;

// ===== snapshot wrapper =====

#[test]
fn process_scan_snapshot_returns_id_and_backfills_keys() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));

    let snapshot = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("进程扫描快照必须注册成功");

    assert!(!snapshot.snapshot_id.is_empty());
    assert!(snapshot.expires_at_ms > 0);
    assert_eq!(snapshot.value.processes.len(), 2);
    let keys: Vec<String> = snapshot
        .value
        .processes
        .iter()
        .map(|info| info.selection_key.clone())
        .collect();
    assert!(keys.iter().all(|key| !key.is_empty()));
    assert_ne!(keys[0], keys[1]);
    let prepared = prepare(
        &mut harness,
        process_request(&snapshot.snapshot_id, keys, ProcessMode::Graceful),
    )
    .expect("扫描返回的 key 必须能直接用于 prepare");
    assert_eq!(prepared.kind, "process");
    assert_eq!(prepared.item_count, 2);
}

#[test]
fn process_scan_snapshot_hides_policy_whitelisted_entries() {
    let harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&["beta"])));

    let snapshot = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("进程扫描快照必须注册成功");

    let names: Vec<String> = snapshot
        .value
        .processes
        .iter()
        .map(|info| info.name.clone())
        .collect();
    assert_eq!(names, vec!["alpha".to_owned()]);
}

#[test]
fn process_row_snapshot_flags_match_the_execution_policy() {
    let mut harness = Harness::new();
    let source = Arc::new(FakeWhitelist::new(&["beta"]));
    let policy = process_whitelist_policy(source.clone());

    let snapshot = snapshot_process_rows(
        &mut harness.store.lock().unwrap(),
        vec![
            process_row(11, "alpha"),
            // 白名单里存的是**原始名** "beta"，展示名是 "Beta"。
            // 判定若误用展示名，beta 会漏标成非白名单并被放行。
            process_row_with_display_name(12, "Beta", "beta"),
        ],
        &policy,
    )
    .expect("进程列表快照必须注册成功");

    let beta = snapshot
        .value
        .iter()
        .find(|row| row.full_name == "beta")
        .expect("beta 必须仍在快照中");
    assert_eq!(beta.name, "Beta", "前提：展示名与原始名不同");
    assert!(policy(&beta.full_name), "前提：原始名命中白名单");
    assert!(!policy(&beta.name), "前提：展示名不命中白名单");
    assert!(
        beta.whitelisted,
        "白名单判定用了展示名 {} 而不是原始名 {}",
        beta.name, beta.full_name
    );
    assert!(beta.protected);
    assert!(policy("beta"));
    let alpha = snapshot
        .value
        .iter()
        .find(|row| row.name == "alpha")
        .expect("alpha 必须仍在快照中");
    assert!(!alpha.whitelisted);
    assert!(!alpha.protected);
    assert!(!policy("alpha"));
    let prepared = prepare(
        &mut harness,
        process_request(
            &snapshot.snapshot_id,
            vec![key_of(&alpha.selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .expect("非白名单进程可以 prepare");
    assert_eq!(prepared.item_count, 1);
    assert!(prepare(
        &mut harness,
        process_request(
            &snapshot.snapshot_id,
            vec![key_of(&beta.selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .is_err());
}

/// 白名单判定只能看**原始名**，因为执行期拿到的是现场 `proc.name()`。
///
/// 扫描期 `snapshot_process_rows` 与执行期 `SystemProcessObserver` 共用同一个
/// policy，但前者的输入是 `ProcessRow`，后者是原始 `proc.name()`。两边一旦
/// 看到不同的名字，`revalidate_targets` 会判定「进程保护状态已变化」——
/// 用户白名单里的进程在进程管理页一键终止必定报错（graceful 与 force 都一样）。
#[test]
fn process_row_snapshot_whitelists_by_the_raw_name_not_the_display_name() {
    let mut harness = Harness::new();
    // 用户持久化白名单里存的是原始名（`add_whitelist` 写入 SQLite 的形态）
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[
        "com.apple.SafariPlatformSupport.Helper",
    ])));

    let snapshot = snapshot_process_rows(
        &mut harness.store.lock().unwrap(),
        vec![
            process_row_with_display_name(11, "Safari", "com.apple.SafariPlatformSupport.Helper"),
            process_row(12, "alpha"),
        ],
        &policy,
    )
    .expect("进程列表快照必须注册成功");

    let helper = snapshot
        .value
        .iter()
        .find(|row| row.pid == 11)
        .expect("helper 行必须仍在快照中");

    // —— 前提：两个名字的判定结论必须不同，否则本测试对目标 bug 不可见 ——
    assert_ne!(helper.name, helper.full_name, "前提：展示名与原始名不同");
    assert!(policy(&helper.full_name), "前提：原始名命中用户白名单");
    assert!(!policy(&helper.name), "前提：展示名不命中用户白名单");

    assert!(
        helper.whitelisted,
        "白名单判定用了展示名 {} 而不是原始名 {} —— 用户持久化白名单静默失效",
        helper.name, helper.full_name
    );
    assert!(
        helper.protected,
        "命中白名单的行必须同时受保护，否则终止按钮不会被禁用"
    );
    assert_eq!(
        helper.protected_reason_key.as_deref(),
        Some("process.protect.whitelisted"),
    );

    // 白名单行不得被 prepare 放行（否则执行期 revalidate_targets 会硬失败）
    assert!(
        prepare(
            &mut harness,
            process_request(
                &snapshot.snapshot_id,
                vec![key_of(&helper.selection_key)],
                ProcessMode::Graceful,
            ),
        )
        .is_err(),
        "白名单行被 prepare 放行了"
    );
    assert!(
        prepare(
            &mut harness,
            process_request(
                &snapshot.snapshot_id,
                vec![key_of(&helper.selection_key)],
                ProcessMode::Force,
            ),
        )
        .is_err(),
        "白名单行被 prepare 放行了（force）"
    );
}

#[test]
fn application_snapshot_backfills_app_and_child_keys() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));

    let snapshot = snapshot_applications(
        &mut harness.store.lock().unwrap(),
        vec![app_info("Alpha"), app_info("Beta")],
        &policy,
    )
    .expect("应用快照必须注册成功");

    assert!(!snapshot.snapshot_id.is_empty());
    let apps = &snapshot.value;
    assert!(apps.iter().all(|app| !app.selection_key.is_empty()));
    assert!(apps.iter().all(|app| app
        .children
        .iter()
        .all(|child| !child.selection_key.is_empty())));
    assert_ne!(apps[0].selection_key, apps[1].selection_key);
    let graceful = prepare(
        &mut harness,
        graceful_quit_request(&snapshot.snapshot_id, vec![key_of(&apps[0].selection_key)]),
    )
    .expect("应用 key 必须能 prepare 优雅退出计划");
    assert_eq!(graceful.kind, "app_graceful_quit");
    let forced = prepare(
        &mut harness,
        app_request(
            &snapshot.snapshot_id,
            vec![key_of(&apps[0].selection_key)],
            ProcessMode::Force,
        ),
    )
    .expect("应用 key 必须能 prepare 强制终止计划");
    assert_eq!(forced.kind, "app_terminate");
}

#[test]
fn cache_snapshot_pairs_every_item_with_an_opaque_key() {
    let mut harness = Harness::new();
    let scan = CacheScanResult {
        items: vec![cache_item("npm"), cache_item("pnpm")],
        total_bytes: 256,
        scanned_at_ms: 1_700_000_000_000,
    };

    let snapshot =
        snapshot_cache(&mut harness.store.lock().unwrap(), scan).expect("缓存扫描快照必须注册成功");

    assert_eq!(snapshot.value.items.len(), 2);
    assert_eq!(snapshot.value.total_bytes, 256);
    let keys: Vec<String> = snapshot
        .value
        .items
        .iter()
        .map(|item| item.selection_key.clone())
        .collect();
    assert!(keys.iter().all(|key| !key.is_empty()));
    assert_ne!(keys[0], keys[1]);
    let json = serde_json::to_string(&snapshot.value.items[0]).expect("缓存视图必须可序列化");
    assert!(!json.contains("command"));
    let prepared = prepare(&mut harness, cache_request(&snapshot.snapshot_id, keys))
        .expect("缓存 key 必须能 prepare 清理计划");
    assert_eq!(prepared.kind, "cache");
    assert_eq!(prepared.item_count, 2);
    assert_eq!(prepared.estimated_bytes, 256);
}

#[test]
fn docker_snapshot_backfills_keys_for_every_resource_type() {
    let mut harness = Harness::new();

    let snapshot = snapshot_docker_inventory(
        &mut harness.store.lock().unwrap(),
        docker_inventory_fixture(),
    )
    .expect("Docker 清单快照必须注册成功");

    let inventory = &snapshot.value;
    assert_eq!(inventory.images[0].selection_key.len(), 64);
    assert_eq!(inventory.containers[0].selection_key.len(), 64);
    assert_eq!(inventory.volumes[0].selection_key.len(), 64);
    let prepared = prepare(
        &mut harness,
        docker_request(
            &snapshot.snapshot_id,
            DockerAction::RemoveImage,
            vec![key_of(&inventory.images[0].selection_key)],
        ),
    )
    .expect("镜像 key 必须能 prepare");
    assert_eq!(prepared.kind, "docker");
    assert!(prepare(
        &mut harness,
        docker_request(
            &snapshot.snapshot_id,
            DockerAction::RemoveImage,
            vec![key_of(&inventory.volumes[0].selection_key)],
        ),
    )
    .is_err());
}

#[test]
fn installed_app_snapshot_backfills_keys() {
    let mut harness = Harness::new();
    let snapshot = alpha_fixture(&mut harness);

    let app = &snapshot.value[0];
    assert!(!app.selection_key.is_empty());
    let residue = scan_residue_batch(
        &mut harness,
        &snapshot.snapshot_id,
        std::slice::from_ref(&app.selection_key),
        residue_for,
    )
    .expect("残留批量注册必须成功");
    assert_eq!(residue.value.len(), 1);
}

#[test]
fn residue_batch_registers_a_single_snapshot_for_multiple_apps() {
    let mut harness = Harness::new();
    let apps = two_app_fixture(&mut harness);
    let selected = app_keys(&apps);
    let residues = scan_residue_batch(&mut harness, &apps.snapshot_id, &selected, residue_for)
        .expect("批量残留扫描必须注册成功");

    assert!(!residues.snapshot_id.is_empty());
    assert_eq!(residues.value.len(), 2);
    assert_eq!(residues.value[0].app_key, selected[0]);
    assert_eq!(residues.value[1].app_key, selected[1]);
    let residue_keys = residue_keys(&residues);
    assert_eq!(residue_keys.len(), 2);
    let prepared = prepare(
        &mut harness,
        uninstall_request(
            &apps.snapshot_id,
            &residues.snapshot_id,
            selected.clone(),
            residue_keys,
        ),
    )
    .expect("多应用残留必须聚合成同一个卸载计划");
    assert_eq!(prepared.kind, "uninstall");
    assert_eq!(prepared.item_count, 4);
    assert!(harness
        .store
        .lock()
        .unwrap()
        .snapshot_expiry_ms(SnapshotKind::Residue)
        .is_ok());
}

#[test]
fn residue_batch_keeps_one_snapshot_when_selected_apps_have_no_residue() {
    let mut harness = Harness::new();
    let apps = two_app_fixture(&mut harness);
    let selected = app_keys(&apps);
    let residues = scan_residue_batch(&mut harness, &apps.snapshot_id, &selected, |_identity| {
        app_residue("com.example.alpha", "Alpha", &[])
    })
    .expect("无残留也必须注册快照");

    assert_eq!(residues.value.len(), 2);
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
    assert_eq!(prepared.item_count, 2);
}

#[test]
fn residue_batch_rejects_unknown_app_key() {
    let mut harness = Harness::new();
    let apps = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha")],
    )
    .expect("已安装应用快照必须注册成功");

    let error = scan_residue_batch(
        &mut harness,
        &apps.snapshot_id,
        &["forged-app-key".to_owned()],
        |_identity| app_residue("com.example.Alpha", "Alpha", &[]),
    )
    .expect_err("未知应用 key 必须被拒绝");

    assert!(!error.is_empty());
}

#[test]
fn residue_batch_rejects_empty_and_duplicate_app_selections() {
    let mut harness = Harness::new();
    let apps = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha")],
    )
    .expect("已安装应用快照必须注册成功");
    let key = apps.value[0].selection_key.clone();

    assert!(
        scan_residue_batch(&mut harness, &apps.snapshot_id, &[], |_identity| {
            app_residue("com.example.Alpha", "Alpha", &[])
        })
        .is_err()
    );
    assert!(scan_residue_batch(
        &mut harness,
        &apps.snapshot_id,
        &[key.clone(), key],
        |_identity| app_residue("com.example.Alpha", "Alpha", &[]),
    )
    .is_err());
}

#[test]
fn residue_batch_rejects_replaced_app_snapshot() {
    let mut harness = Harness::new();
    let first = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha")],
    )
    .expect("已安装应用快照必须注册成功");
    let stale_key = first.value[0].selection_key.clone();
    let second = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Beta")],
    )
    .expect("新的已安装应用快照必须注册成功");

    assert!(scan_residue_batch(
        &mut harness,
        &first.snapshot_id,
        &[stale_key],
        |_identity| app_residue("com.example.Alpha", "Alpha", &[])
    )
    .is_err());
    assert!(scan_residue_batch(
        &mut harness,
        &second.snapshot_id,
        &[second.value[0].selection_key.clone()],
        |identity| app_residue(&identity.bundle_id, &identity.app_name, &[]),
    )
    .is_ok());
}

#[test]
fn overview_and_managed_process_snapshots_survive_interleaved_registration() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let snapshots = interleaved_process_snapshots(&mut harness, &policy);

    let managed_prepared = prepare(
        &mut harness,
        process_request(
            &snapshots.managed.snapshot_id,
            vec![key_of(&snapshots.managed.value[0].selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .expect("进程管理快照不能被主扫描顶掉");
    let overview_prepared = prepare(
        &mut harness,
        process_request(
            &snapshots.second_overview.snapshot_id,
            vec![key_of(
                &snapshots.second_overview.value.processes[0].selection_key,
            )],
            ProcessMode::Graceful,
        ),
    )
    .expect("最新主扫描快照必须可用");
    let stale_key = key_of(&snapshots.first_overview.value.processes[0].selection_key);
    assert!(prepare(
        &mut harness,
        process_request(
            &snapshots.first_overview.snapshot_id,
            vec![stale_key],
            ProcessMode::Graceful,
        ),
    )
    .is_err());

    let mut store = harness.store.lock().unwrap();
    assert!(store
        .consume(&managed_prepared.operation_id, "main")
        .is_ok());
    assert!(store
        .consume(&overview_prepared.operation_id, "main")
        .is_ok());
}

#[test]
fn process_keys_are_scoped_to_their_own_snapshot_slot() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let overview = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("主扫描快照必须注册成功");
    let managed = snapshot_process_rows(
        &mut harness.store.lock().unwrap(),
        vec![process_row(21, "gamma")],
        &policy,
    )
    .expect("进程管理快照必须注册成功");
    let overview_key = key_of(&overview.value.processes[0].selection_key);
    let managed_key = key_of(&managed.value[0].selection_key);

    assert!(prepare(
        &mut harness,
        process_request(
            &overview.snapshot_id,
            vec![managed_key.clone()],
            ProcessMode::Graceful
        )
    )
    .is_err());
    assert!(prepare(
        &mut harness,
        process_request(
            &managed.snapshot_id,
            vec![overview_key],
            ProcessMode::Graceful
        )
    )
    .is_err());
    assert!(prepare(
        &mut harness,
        process_request(
            &managed.snapshot_id,
            vec![key_of(&managed.value[0].selection_key)],
            ProcessMode::Graceful
        ),
    )
    .is_ok());
}

#[test]
fn process_snapshot_expiry_is_reported_per_slot() {
    let harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let overview = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("主扫描快照必须注册成功");
    let managed = snapshot_process_rows(
        &mut harness.store.lock().unwrap(),
        vec![process_row(21, "gamma")],
        &policy,
    )
    .expect("进程管理快照必须注册成功");
    let store = harness.store.lock().unwrap();

    assert_eq!(
        store.snapshot_expiry_ms(SnapshotKind::OverviewProcess),
        Ok(overview.expires_at_ms)
    );
    assert_eq!(
        store.snapshot_expiry_ms(SnapshotKind::Process),
        Ok(managed.expires_at_ms)
    );
    drop(store);
    let refreshed = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("再次扫描必须注册成功");
    assert_eq!(
        harness
            .store
            .lock()
            .unwrap()
            .snapshot_expiry_ms(SnapshotKind::Process),
        Ok(managed.expires_at_ms)
    );
    assert_ne!(refreshed.snapshot_id, overview.snapshot_id);
}

#[test]
fn cache_keyed_view_shape_is_pinned_for_the_frontend() {
    let harness = Harness::new();
    let snapshot = snapshot_cache(
        &mut harness.store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item("npm")],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照必须注册成功");

    let json = serde_json::to_value(&snapshot.value.items[0]).expect("缓存视图必须可序列化");
    let mut keys: Vec<String> = json
        .as_object()
        .expect("缓存视图必须是对象")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    let mut expected = vec![
        "default_select",
        "description_key",
        "description_params",
        "id",
        "label_key",
        "label_params",
        "path",
        "recover_hint",
        "safety",
        "selection_key",
        "size_bytes",
        "category",
    ];
    expected.sort();
    assert_eq!(keys, expected);
    assert_eq!(
        snapshot.value.items[0].selection_key.len(),
        64,
        "selection key 必须是 32 字节 hex"
    );
}
