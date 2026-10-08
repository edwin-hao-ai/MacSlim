use super::*;

// ===== execute =====

#[tokio::test]
async fn execute_operation_routes_cache_plan_to_cache_domain() {
    let mut harness = Harness::new();
    let prepared = cache_plan_fixture(&mut harness);

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
    .expect("缓存执行必须成功");

    assert!(matches!(result, OperationResult::Cache(_)));
    assert_eq!(harness.cache.calls.lock().unwrap().len(), 1);
    assert!(harness.uninstall.calls.lock().unwrap().is_empty());
    assert!(harness.docker.removed.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "cache");
    assert_eq!(entries[0].target, "1 项缓存");
    assert_eq!(entries[0].freed_bytes, 128);
    assert_eq!(entries[0].deleted_bytes, 128);
    assert_eq!(entries[0].trashed_bytes, 0);
    assert_eq!(entries[0].reclaimed_bytes, None);
    assert!(entries[0].success);
    assert!(!entries[0].detail.is_empty());
    assert!(!entries[0].detail.contains(&prepared.operation_id));
    assert!(!entries[0].target.contains(&prepared.operation_id));
}

#[tokio::test]
async fn execute_operation_routes_uninstall_plan_to_uninstall_domain() {
    let mut harness = Harness::new();
    let prepared = uninstall_plan_fixture(&mut harness);

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
    .expect("卸载执行必须成功");

    match result {
        OperationResult::Uninstall(reports) => {
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].app_name, "Alpha");
        }
        other => panic!("期望卸载结果，得到 {other:?}"),
    }
    assert_eq!(harness.uninstall.calls.lock().unwrap().len(), 1);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "uninstall");
    assert_eq!(entries[0].target, "1 个应用");
    // 卸载主口径走 trashed_bytes；本 Task 尚未实测 reclaimed，两者暂同源。
    assert_eq!(entries[0].deleted_bytes, 0);
    assert_eq!(entries[0].trashed_bytes, entries[0].freed_bytes);
    assert_eq!(entries[0].reclaimed_bytes, None);
    assert!(!entries[0].detail.contains(&prepared.operation_id));
}

#[tokio::test]
async fn execute_operation_routes_docker_plan_to_docker_domain() {
    let mut harness = Harness::new();
    let inventory = snapshot_docker_inventory(
        &mut harness.store.lock().unwrap(),
        docker_inventory_fixture(),
    )
    .expect("Docker 快照");
    let prepared = prepare(
        &mut harness,
        docker_request(&inventory.snapshot_id, DockerAction::Prune, Vec::new()),
    )
    .expect("Docker prune 计划");

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
    .expect("Docker prune 执行必须成功");

    assert!(matches!(result, OperationResult::Docker(_)));
    assert_eq!(*harness.docker.pruned.lock().unwrap(), 1);
    assert!(harness.docker.removed.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "docker");
    assert!(entries[0].target.contains("Docker"));
}

#[tokio::test]
async fn execute_operation_routes_termination_plan_to_process_services() {
    let mut harness = Harness::new();
    let (prepared, _snapshot) = process_plan_fixture(&mut harness);
    let mut probe = TerminationProbe::new(vec![live_process(11, "alpha")]);
    let observed = Arc::clone(&probe.observed);
    let calls = Arc::clone(&probe.calls);

    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_termination_plan(plan, &mut probe.services()) },
    )
    .await
    .expect("进程执行必须成功");

    assert!(matches!(result, OperationResult::Process(_)));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[(11, ProcessMode::Graceful)]
    );
    assert_eq!(observed.lock().unwrap().as_slice(), &[vec![11]]);
    assert!(harness.cache.calls.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "process");
    assert_eq!(entries[0].target, "1 个进程");
}

#[tokio::test]
async fn execute_operation_routes_app_termination_plan_to_process_services() {
    let mut harness = Harness::new();
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let apps = snapshot_applications(
        &mut harness.store.lock().unwrap(),
        vec![app_info("Alpha")],
        &policy,
    )
    .expect("应用快照");
    let prepared = prepare(
        &mut harness,
        app_request(
            &apps.snapshot_id,
            vec![key_of(&apps.value[0].selection_key)],
            ProcessMode::Force,
        ),
    )
    .expect("应用终止计划");
    let mut probe = TerminationProbe::new(vec![live_process_with(
        42,
        "Alpha",
        "/Applications/Alpha.app/Contents/MacOS/Alpha",
    )]);
    let calls = Arc::clone(&probe.calls);

    let history = harness.history.clone();
    let result = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_termination_plan(plan, &mut probe.services()) },
    )
    .await
    .expect("应用终止执行必须成功");

    assert!(matches!(result, OperationResult::AppTerminate(_)));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[(42, ProcessMode::Force)]
    );
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries[0].operation, "app_terminate");
}

#[tokio::test]
async fn execute_operation_rejects_missing_operation_id() {
    let harness = Harness::new();
    let domains = harness.domains();
    let history = harness.history.clone();

    for operation_id in ["", "   ", "forged-operation-id"] {
        let error =
            execute_operation_with(&harness.store, &history, "main", operation_id, |plan| {
                let domains = &domains;
                async move { execute_domain_plan(plan, domains).await }
            })
            .await
            .expect_err("未知 operation 必须失败");
        assert!(!error.is_empty());
    }
    assert!(harness.cache.calls.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn execute_operation_rejects_wrong_owner_without_side_effect() {
    let mut harness = Harness::new();
    let prepared = cache_plan_fixture(&mut harness);
    let domains = harness.domains();
    let history = harness.history.clone();

    let error = execute_operation_with(
        &harness.store,
        &history,
        "other-window",
        &prepared.operation_id,
        |plan| {
            let domains = &domains;
            async move { execute_domain_plan(plan, domains).await }
        },
    )
    .await
    .expect_err("其他窗口不能执行");
    assert!(error.contains("所有者"));
    assert!(harness.cache.calls.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn execute_operation_is_single_use() {
    let mut harness = Harness::new();
    let snapshot = snapshot_cache(
        &mut harness.store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item("npm")],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照");
    let prepared = prepare(
        &mut harness,
        cache_request(
            &snapshot.snapshot_id,
            vec![key_of(&snapshot.value.items[0].selection_key)],
        ),
    )
    .expect("缓存计划");

    let history = harness.history.clone();
    for expected_calls in 1..=2 {
        let domains = harness.domains();
        let outcome = execute_operation_with(
            &harness.store,
            &history,
            "main",
            &prepared.operation_id,
            |plan| {
                let domains = &domains;
                async move { execute_domain_plan(plan, domains).await }
            },
        )
        .await;
        if expected_calls == 1 {
            assert!(outcome.is_ok());
        } else {
            assert!(outcome.is_err(), "operation 不能重放");
        }
        assert_eq!(harness.cache.calls.lock().unwrap().len(), 1);
        let _ = expected_calls;
    }
    assert_eq!(history.entries.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn execute_operation_rejects_expired_operation() {
    let now = Arc::new(Mutex::new(1_000_u64));
    let store = Mutex::new(OperationStore::with_clock({
        let now = Arc::clone(&now);
        move || *now.lock().unwrap()
    }));
    let cache = RecordingCache::default();
    let uninstall = RecordingUninstall {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let docker = RecordingDocker::new(docker_inventory_fixture());
    let app_quit = RecordingAppQuitter::with_live(Vec::new());
    let history = RecordingHistory::default();
    let snapshot = snapshot_cache(
        &mut store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item("npm")],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照");
    let prepared = prepare_operation_with(
        &mut store.lock().unwrap(),
        "main",
        cache_request(
            &snapshot.snapshot_id,
            vec![key_of(&snapshot.value.items[0].selection_key)],
        ),
    )
    .expect("缓存计划");

    assert!(prepared.expires_at_ms <= 1_000 + SNAPSHOT_TTL_MS);
    *now.lock().unwrap() += SNAPSHOT_TTL_MS + 1;
    let domains = DomainServices {
        cache: &cache,
        uninstall: &uninstall,
        app_quit: &app_quit,
        docker: &docker,
    };

    let error = execute_operation_with(&store, &history, "main", &prepared.operation_id, |plan| {
        let domains = &domains;
        async move { execute_domain_plan(plan, domains).await }
    })
    .await
    .expect_err("过期 operation 必须失败");

    assert!(!error.is_empty());
    assert!(cache.calls.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn execute_operation_rejects_plan_whose_snapshot_was_replaced() {
    let mut harness = Harness::new();
    let first = snapshot_cache(
        &mut harness.store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item("npm")],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照");
    let prepared = prepare(
        &mut harness,
        cache_request(
            &first.snapshot_id,
            vec![key_of(&first.value.items[0].selection_key)],
        ),
    )
    .expect("缓存计划");
    snapshot_cache(
        &mut harness.store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item("pnpm")],
            total_bytes: 256,
            scanned_at_ms: 2,
        },
    )
    .expect("重新扫描");
    let domains = harness.domains();
    let history = harness.history.clone();

    let error = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| {
            let domains = &domains;
            async move { execute_domain_plan(plan, domains).await }
        },
    )
    .await
    .expect_err("快照被替换后必须失败");

    assert!(error.contains("已失效"), "{error}");
    assert!(harness.cache.calls.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn execute_operation_fails_when_domain_revalidation_rejects() {
    let mut harness = Harness::new();
    let (prepared, _snapshot) = process_plan_fixture(&mut harness);
    let mut probe = TerminationProbe::new(vec![LiveProcess {
        identity: crate::operations::ProcessIdentity {
            pid: 11,
            name: "alpha".to_owned(),
            exe: "/usr/local/bin/alpha".to_owned(),
            start_time: 1_000,
        },
        protected: false,
        whitelisted: true,
    }]);
    let calls = Arc::clone(&probe.calls);
    let history = harness.history.clone();

    let error = execute_operation_with(
        &harness.store,
        &history,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_termination_plan(plan, &mut probe.services()) },
    )
    .await
    .expect_err("白名单状态变化必须失败");

    assert!(error.contains("保护状态"), "{error}");
    assert!(calls.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "process");
    assert!(!entries[0].success);
    assert!(entries[0].detail.contains("复核"));
    assert!(!entries[0].detail.contains(&prepared.operation_id));
}

#[tokio::test]
async fn history_write_failure_after_execution_is_reported() {
    let mut harness = Harness::new();
    let prepared = cache_plan_fixture(&mut harness);
    let failing = FailingHistory::new();
    let domains = harness.domains();

    let error = execute_operation_with(
        &harness.store,
        &failing,
        "main",
        &prepared.operation_id,
        |plan| {
            let domains = &domains;
            async move { execute_domain_plan(plan, domains).await }
        },
    )
    .await
    .expect_err("历史写入失败不得被吞掉");

    assert!(error.contains("操作已执行"), "{error}");
    assert!(error.contains("写入历史记录失败"), "{error}");
    assert!(error.contains("历史数据库不可写"), "{error}");
    assert_eq!(
        harness.cache.calls.lock().unwrap().len(),
        1,
        "领域动作只允许执行一次"
    );
    assert!(harness.history.entries.lock().unwrap().is_empty());
    let attempts = failing.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].operation, "cache");
    assert!(!attempts[0].detail.contains(&prepared.operation_id));
}

#[tokio::test]
async fn history_write_failure_on_rejection_is_reported() {
    let mut harness = Harness::new();
    let (prepared, _snapshot) = process_plan_fixture(&mut harness);
    let failing = FailingHistory::new();
    let mut probe = TerminationProbe::new(vec![LiveProcess {
        identity: crate::operations::ProcessIdentity {
            pid: 11,
            name: "alpha".to_owned(),
            exe: "/usr/local/bin/alpha".to_owned(),
            start_time: 1_000,
        },
        protected: false,
        whitelisted: true,
    }]);
    let calls = Arc::clone(&probe.calls);

    let error = execute_operation_with(
        &harness.store,
        &failing,
        "main",
        &prepared.operation_id,
        |plan| async move { execute_termination_plan(plan, &mut probe.services()) },
    )
    .await
    .expect_err("复核失败且历史写入失败必须显式报错");

    assert!(error.contains("保护状态"), "{error}");
    assert!(error.contains("操作未执行"), "{error}");
    assert!(error.contains("写入历史记录失败"), "{error}");
    assert!(calls.lock().unwrap().is_empty());
    let attempts = failing.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].operation, "process");
    assert!(!attempts[0].success);
    assert!(!attempts[0].detail.contains(&prepared.operation_id));
}
