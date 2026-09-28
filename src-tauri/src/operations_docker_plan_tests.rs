use super::*;
use crate::operations::DockerResourceKind;

fn docker_target(kind: DockerResourceKind, id: &str, name: &str, referenced: bool) -> DockerTarget {
    DockerTarget {
        resource_type: kind,
        id: id.to_owned(),
        name: name.to_owned(),
        size_bytes: 512,
        referenced,
        reclaimable: false,
    }
}

#[test]
fn docker_snapshot_keys_are_scoped_to_resource_type() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![
            docker_target(DockerResourceKind::Image, "sha256:a", "nginx:latest", false),
            docker_target(DockerResourceKind::Container, "c1", "web", true),
            docker_target(DockerResourceKind::Volume, "vol", "vol", false),
        ])
        .unwrap();

    assert_eq!(registration.selection_keys.len(), 3);
    assert!(registration
        .selection_keys
        .iter()
        .all(|key| { key.len() == 64 && key != "sha256:a" && key != "web" && key != "vol" }));

    let error = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveContainer,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap_err();
    assert_eq!(error, "Docker 选择项类型不匹配");

    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveContainer,
            vec![&registration.selection_keys[1]],
            "main",
        )
        .unwrap();
    let OperationPlan::Docker {
        action, targets, ..
    } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected docker plan")
    };
    assert_eq!(action, DockerAction::RemoveContainer);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].resource_type, DockerResourceKind::Container);
    assert_eq!(targets[0].id, "c1");
    assert_eq!(targets[0].name, "web");
    assert!(targets[0].referenced);
}

#[test]
fn docker_plan_keeps_client_supplied_ids_out() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![docker_target(
            DockerResourceKind::Image,
            "sha256:a",
            "nginx:latest",
            false,
        )])
        .unwrap();

    assert!(store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec!["sha256:a"],
            "main",
        )
        .is_err());
    assert!(store.operations.is_empty());
}

#[test]
fn docker_prune_accepts_only_empty_target_keys() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![docker_target(
            DockerResourceKind::Image,
            "sha256:a",
            "nginx:latest",
            false,
        )])
        .unwrap();

    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap();
    let OperationPlan::Docker {
        action, targets, ..
    } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected docker plan")
    };
    assert_eq!(action, DockerAction::Prune);
    assert!(targets.is_empty());

    assert!(store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .is_err());
}

#[test]
fn docker_removal_requires_at_least_one_target() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![docker_target(
            DockerResourceKind::Volume,
            "vol",
            "vol",
            false,
        )])
        .unwrap();

    let error = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveVolume,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "选择不能为空");
}

#[test]
fn docker_snapshot_must_use_the_dedicated_typed_registration() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![docker_target(
            DockerResourceKind::Image,
            "sha256:a",
            "nginx:latest",
            false,
        )])
        .unwrap();

    assert!(store
        .register_snapshot(
            SnapshotKind::Docker,
            SnapshotPayload::Docker(vec![docker_target(
                DockerResourceKind::Image,
                "sha256:b",
                "other:latest",
                false,
            )]),
        )
        .is_err());
    assert_eq!(
        store
            .active_snapshots
            .get(&SnapshotKind::Docker)
            .map(|snapshot| snapshot.id.clone())
            .unwrap(),
        registration.snapshot_id
    );
}

#[test]
fn docker_prune_plan_records_the_full_inventory_fingerprint() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![
            DockerTarget {
                reclaimable: true,
                ..docker_target(
                    DockerResourceKind::Image,
                    "sha256:aaa",
                    "nginx:latest",
                    false,
                )
            },
            docker_target(DockerResourceKind::Container, "c1", "web", false),
            DockerTarget {
                reclaimable: true,
                ..docker_target(DockerResourceKind::Volume, "cache", "cache", false)
            },
        ])
        .unwrap();

    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap();

    assert_eq!(
        prepared.item_count, 2,
        "运行中的容器不是可回收资源，不能计入数量"
    );
    assert_eq!(prepared.estimated_bytes, 1024);
    assert_eq!(
        super::describe_summary(&prepared),
        "opSummary.dockerPrune(count=2, size=1.0 KB)"
    );
    let OperationPlan::Docker {
        inventory, targets, ..
    } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected docker plan")
    };
    assert!(targets.is_empty());
    assert_eq!(inventory.image_count, 1);
    assert_eq!(inventory.container_count, 1);
    assert_eq!(inventory.volume_count, 1);
    assert_eq!(inventory.reclaimable_bytes, 1024);
    assert_eq!(
        inventory
            .resources
            .iter()
            .map(|target| (target.resource_type, target.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (DockerResourceKind::Image, "sha256:aaa"),
            (DockerResourceKind::Container, "c1"),
            (DockerResourceKind::Volume, "cache"),
        ]
    );
}

#[test]
fn docker_removal_plan_estimates_selected_target_sizes() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![
            docker_target(
                DockerResourceKind::Image,
                "sha256:aaa",
                "nginx:latest",
                false,
            ),
            docker_target(
                DockerResourceKind::Image,
                "sha256:bbb",
                "redis:latest",
                false,
            ),
        ])
        .unwrap();

    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();

    assert_eq!(prepared.item_count, 1);
    assert_eq!(prepared.estimated_bytes, 512);
    assert_eq!(
        super::describe_summary(&prepared),
        "opSummary.dockerRemoveImage(count=1, size=512 B)"
    );
}

#[test]
fn docker_non_prune_plan_keeps_the_selected_targets_for_revalidation() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![
            docker_target(
                DockerResourceKind::Image,
                "sha256:aaa",
                "nginx:latest",
                false,
            ),
            docker_target(
                DockerResourceKind::Image,
                "sha256:bbb",
                "redis:latest",
                false,
            ),
        ])
        .unwrap();

    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();

    let OperationPlan::Docker {
        targets, inventory, ..
    } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected docker plan")
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "sha256:aaa");
    assert_eq!(inventory.resources.len(), 2, "fingerprint 覆盖整份清单");
}

#[test]
fn prepared_summaries_are_domain_chinese_for_every_kind() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![crate::operations::InstalledAppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            app_name: "alpha".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap();
    let residues = store
        .register_residues(vec![ResidueIdentity {
            app_key: apps.selection_keys[0].clone(),
            path: dirs::home_dir()
                .expect("测试环境必须有用户主目录")
                .join("Library/Caches/com.example.alpha")
                .to_string_lossy()
                .to_string(),
            category: "Caches".to_owned(),
            size_bytes: 128,
        }])
        .unwrap();

    let uninstall = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&uninstall),
        "opSummary.uninstall(apps=1, residues=1, size=128 B)"
    );

    let process = store
        .register_process_snapshot(vec![crate::operations::ProcessTarget {
            identity: crate::operations::ProcessIdentity {
                pid: 42,
                name: "alpha".to_owned(),
                exe: "/usr/local/bin/alpha".to_owned(),
                start_time: 1,
            },
            protected: false,
            whitelisted: false,
        }])
        .unwrap();
    let forced = store
        .prepare_process(
            &process.snapshot_id,
            vec![&process.selection_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&forced),
        "opSummary.processForce(count=1)"
    );

    let running = store
        .register_application_snapshot(vec![crate::operations::AppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            app_name: "alpha".to_owned(),
            processes: vec![crate::operations::ProcessTarget {
                identity: crate::operations::ProcessIdentity {
                    pid: 43,
                    name: "alpha".to_owned(),
                    exe: "/Applications/alpha.app/Contents/MacOS/alpha".to_owned(),
                    start_time: 1,
                },
                protected: false,
                whitelisted: false,
            }],
        }])
        .unwrap();
    let terminated = store
        .prepare_app_termination(
            &running.snapshot_id,
            vec![&running.app_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&terminated),
        "opSummary.appTerminate(apps=1, processes=1)"
    );

    let cache = store
        .register_cache_snapshot(vec![crate::cache_scanner::CacheItem {
            id: "npm-cache".into(),
            category: crate::cache_scanner::CacheCategory::Npm,
            label_key: "cache.item.npmCache".into(),
            label_params: Vec::new(),
            description_key: "cache.desc.npmCache".into(),
            description_params: Vec::new(),
            path: None,
            size_bytes: 2048,
            safety: crate::cache_scanner::Safety::Safe,
            default_select: true,
            action: crate::operations::CacheAction::Npm,
            stale_owner_uid: None,
            stale_canonical_path: None,
            recover_hint: String::new(),
        }])
        .unwrap();
    let cleaned = store
        .prepare_cache(&cache.snapshot_id, vec![&cache.selection_keys[0]], "main")
        .unwrap();
    assert_eq!(
        super::describe_summary(&cleaned),
        "opSummary.cache(count=1, size=2.0 KB)"
    );
}
