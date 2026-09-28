use super::*;
use crate::cache_scanner::{CacheCategory, CacheItem, Safety};
use crate::operations::CacheAction;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);

impl TestClock {
    fn new(now: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now)))
    }

    fn advance(&self, milliseconds: u64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }

    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

fn test_store() -> (OperationStore, TestClock) {
    let clock = TestClock::new(1_000);
    let value = clock.clone();
    let store = OperationStore::with_clock(move || value.0.load(Ordering::SeqCst));
    (store, clock)
}

fn is_lower_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn cache_payload<S: AsRef<str>>(labels: impl IntoIterator<Item = S>) -> SnapshotPayload {
    SnapshotPayload::Cache(
        labels
            .into_iter()
            .map(|label| {
                let label = label.as_ref();
                CacheItem {
                    id: label.to_owned(),
                    category: CacheCategory::System,
                    label_key: "cache.item.appCache".into(),
                    label_params: vec![("app".to_owned(), label.to_owned())],
                    description_key: "cache.desc.appCache".into(),
                    description_params: Vec::new(),
                    path: None,
                    size_bytes: 1,
                    safety: Safety::Safe,
                    default_select: true,
                    action: CacheAction::System,
                    stale_owner_uid: None,
                    stale_canonical_path: None,
                    recover_hint: String::new(),
                }
            })
            .collect(),
    )
}

fn process_payload(label: &str) -> SnapshotPayload {
    SnapshotPayload::Process(vec![process_target(1, label, 1)])
}

fn process_target(pid: u32, label: &str, start_time: u64) -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid,
            name: label.to_owned(),
            exe: format!("/usr/local/bin/{label}"),
            start_time,
        },
        protected: false,
        whitelisted: false,
    }
}

fn app_identity(label: &str, children: &[(u32, &str, u64)]) -> AppIdentity {
    AppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        bundle_id: format!("com.example.{label}"),
        app_name: label.to_owned(),
        processes: children
            .iter()
            .map(|(pid, name, start_time)| ProcessTarget {
                identity: ProcessIdentity {
                    pid: *pid,
                    name: (*name).to_owned(),
                    exe: format!("/Applications/{label}.app/Contents/MacOS/{name}"),
                    start_time: *start_time,
                },
                protected: false,
                whitelisted: false,
            })
            .collect(),
    }
}

fn installed_payload<S: AsRef<str>>(labels: impl IntoIterator<Item = S>) -> SnapshotPayload {
    SnapshotPayload::InstalledApps(
        labels
            .into_iter()
            .map(|label| {
                let label = label.as_ref();
                InstalledAppIdentity {
                    bundle_path: format!("/Applications/{label}.app"),
                    app_name: label.to_owned(),
                    bundle_id: format!("com.example.{label}"),
                    is_system: false,
                    bundle_size_bytes: 0,
                }
            })
            .collect(),
    )
}

fn residue_rows(entries: &[(&str, usize)]) -> Vec<ResidueIdentity> {
    let library = dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .to_string_lossy()
        .to_string();
    entries
        .iter()
        .map(|(label, _)| ResidueIdentity {
            app_key: String::new(),
            path: format!("{library}/com.example.{label}"),
            category: "cache".to_owned(),
            size_bytes: 2,
        })
        .collect()
}

fn many_cache(count: usize) -> SnapshotPayload {
    cache_payload((0..count).map(|index| format!("item-{index}")))
}

fn setup_uninstall(store: &mut OperationStore) -> (SnapshotRegistration, SnapshotRegistration) {
    let apps = register_installed(store, &["app-a", "app-b"]);
    let residues = register_residue_snapshot(store, &apps, &[("residue-a", 0), ("residue-b", 1)]);
    (apps, residues)
}

fn register_installed(store: &mut OperationStore, labels: &[&str]) -> SnapshotRegistration {
    let SnapshotPayload::InstalledApps(apps) = installed_payload(labels.iter().copied()) else {
        unreachable!("installed_payload 只会构造 InstalledApps")
    };
    store.register_installed_apps(apps).unwrap()
}

fn register_residue_snapshot(
    store: &mut OperationStore,
    apps: &SnapshotRegistration,
    entries: &[(&str, usize)],
) -> SnapshotRegistration {
    let residues = residue_rows(entries)
        .into_iter()
        .zip(entries.iter())
        .map(|(mut residue, (_, app_index))| {
            residue.app_key = apps.selection_keys[*app_index].clone();
            residue
        })
        .collect();
    store.register_residues(residues).unwrap()
}

fn prepare_uninstall(
    store: &mut OperationStore,
    apps: &SnapshotRegistration,
    residues: &SnapshotRegistration,
) -> PreparedOperation {
    store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap()
}

#[test]
fn uninstall_allows_empty_residue_keys() {
    let (mut store, _) = test_store();
    let (apps, residues) = setup_uninstall(&mut store);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap();
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };

    assert_eq!(targets.len(), 1);
    assert!(targets[0].residues.is_empty());
}

#[test]
fn uninstall_rejects_empty_app_keys() {
    let (mut store, _) = test_store();
    let (apps, residues) = setup_uninstall(&mut store);
    assert!(store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            Vec::<&str>::new(),
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .is_err());
}

#[test]
fn registration_generates_unique_opaque_keys_in_input_order() {
    let mut store = OperationStore::new();
    let registration = store
        .register_snapshot(
            SnapshotKind::Cache,
            cache_payload(["alpha", "beta", "gamma"]),
        )
        .unwrap();

    assert!(is_lower_hex(&registration.snapshot_id));
    assert_eq!(registration.selection_keys.len(), 3);
    assert!(registration
        .selection_keys
        .iter()
        .all(|key| is_lower_hex(key)));
    assert_eq!(
        registration
            .selection_keys
            .iter()
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    assert!(registration
        .selection_keys
        .iter()
        .all(|key| key != "alpha" && key != "beta" && key != "gamma"));

    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![
                &registration.selection_keys[2],
                &registration.selection_keys[0],
            ],
            "main",
        )
        .unwrap();
    let OperationPlan::Cache { items, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected cache plan")
    };
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["gamma", "alpha"]
    );
}

#[test]
fn caller_identifiers_and_cross_snapshot_keys_are_rejected() {
    let mut store = OperationStore::new();
    let cache = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["cache-item"]))
        .unwrap();
    let process = store
        .register_snapshot(SnapshotKind::Process, process_payload("process-item"))
        .unwrap();

    assert_ne!(cache.selection_keys[0], process.selection_keys[0]);
    assert!(store
        .prepare_cache(&cache.snapshot_id, vec!["cache-item"], "main")
        .is_err());
    assert!(store
        .prepare_process(
            &process.snapshot_id,
            vec![&cache.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .is_err());
    assert!(store
        .prepare_process(
            &process.snapshot_id,
            vec!["1", "process-item"],
            ProcessMode::Graceful,
            "main",
        )
        .is_err());
}

#[test]
fn cache_snapshot_registration_uses_typed_items_and_generated_keys() {
    let mut store = OperationStore::new();
    let item = CacheItem {
        id: "client-id".into(),
        category: CacheCategory::System,
        label_key: "cache.item.appCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.appCache".into(),
        description_params: Vec::new(),
        path: Some("~/.cache".into()),
        size_bytes: 1,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::System,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    };
    let registration = store.register_cache_snapshot(vec![item]).unwrap();
    assert_eq!(registration.selection_keys.len(), 1);
    assert_ne!(registration.selection_keys[0], "client-id");
    assert!(store
        .prepare_cache(&registration.snapshot_id, vec!["client-id"], "main",)
        .is_err());
    assert!(store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .is_ok());
}

#[test]
fn operation_is_single_use_and_owner_bound() {
    let mut store = OperationStore::new();
    let registration = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["item"]))
        .unwrap();
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();

    assert!(store.consume(&prepared.operation_id, "other").is_err());
    assert!(store.consume(&prepared.operation_id, "main").is_ok());
    assert!(store.consume(&prepared.operation_id, "main").is_err());
}

#[test]
fn new_snapshot_invalidates_old_operation() {
    let mut store = OperationStore::new();
    let old = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["item"]))
        .unwrap();
    let operation = store
        .prepare_cache(&old.snapshot_id, vec![&old.selection_keys[0]], "main")
        .unwrap();
    store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["new-item"]))
        .unwrap();

    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn ttl_uses_controlled_clock() {
    let (mut store, clock) = test_store();
    let registration = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["item"]))
        .unwrap();
    let first = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    clock.advance(OPERATION_TTL_MS - 1);
    assert!(store.consume(&first.operation_id, "main").is_ok());

    let second = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    clock.advance(1);
    assert!(store.consume(&second.operation_id, "main").is_err());
    assert!(store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .is_err());
}

#[test]
fn concurrent_consume_has_one_winner() {
    let (mut store, _) = test_store();
    let registration = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["item"]))
        .unwrap();
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    let store = Arc::new(Mutex::new(store));
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            let id = prepared.operation_id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.lock().unwrap().consume(&id, "main").is_ok()
            })
        })
        .collect();

    barrier.wait();
    let results: Vec<bool> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|success| **success).count(), 1);
}

#[test]
fn uninstall_binds_app_and_residue_snapshots() {
    let (mut store, _) = test_store();
    let (apps, residues) = setup_uninstall(&mut store);
    let operation = prepare_uninstall(&mut store, &apps, &residues);

    register_residue_snapshot(&mut store, &apps, &[("replacement", 0)]);
    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn uninstall_app_snapshot_replacement_invalidates_operation() {
    let (mut store, _) = test_store();
    let (apps, residues) = setup_uninstall(&mut store);
    let operation = prepare_uninstall(&mut store, &apps, &residues);

    register_installed(&mut store, &["replacement"]);
    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn uninstall_expired_app_snapshot_invalidates_operation() {
    let (mut store, clock) = test_store();
    let apps = register_installed(&mut store, &["app"]);
    clock.advance(1);
    let residues = register_residue_snapshot(&mut store, &apps, &[("residue", 0)]);
    let operation = prepare_uninstall(&mut store, &apps, &residues);

    clock.advance(OPERATION_TTL_MS - 1);
    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn uninstall_expired_residue_snapshot_invalidates_operation() {
    let (mut store, clock) = test_store();
    clock.set(2_000);
    let apps = register_installed(&mut store, &["app"]);
    clock.set(1_000);
    let residues = register_residue_snapshot(&mut store, &apps, &[("residue", 0)]);
    clock.set(2_000);
    let operation = prepare_uninstall(&mut store, &apps, &residues);

    clock.advance(OPERATION_TTL_MS - 1_000);
    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn uninstall_and_docker_keys_are_scoped() {
    let (mut store, _) = test_store();
    let (apps, residues) = setup_uninstall(&mut store);

    assert!(store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[1]],
            false,
            "main",
        )
        .is_err());
    let docker = store
        .register_docker_resources(vec![DockerTarget {
            resource_type: DockerResourceKind::Image,
            id: "id-image".to_owned(),
            name: "name-image".to_owned(),
            size_bytes: 0,
            referenced: false,
            reclaimable: false,
        }])
        .unwrap();
    assert!(store
        .prepare_docker(
            &docker.snapshot_id,
            DockerAction::RemoveContainer,
            vec![&docker.selection_keys[0]],
            "main",
        )
        .is_err());
}

#[test]
fn operation_limit_is_enforced() {
    let (mut store, _) = test_store();
    let registration = store
        .register_snapshot(SnapshotKind::Cache, many_cache(MAX_OPERATIONS + 1))
        .unwrap();
    let first = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    for key in registration
        .selection_keys
        .iter()
        .skip(1)
        .take(MAX_OPERATIONS)
    {
        store
            .prepare_cache(&registration.snapshot_id, vec![key], "main")
            .unwrap();
    }
    assert_eq!(store.operations.len(), MAX_OPERATIONS);
    assert!(store.consume(&first.operation_id, "main").is_err());
}

#[test]
fn snapshot_and_selection_limits_are_enforced() {
    let (mut store, _) = test_store();
    store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["cache"]))
        .unwrap();
    store
        .register_snapshot(SnapshotKind::Process, process_payload("process"))
        .unwrap();
    let installed = register_installed(&mut store, &["installed"]);
    register_residue_snapshot(&mut store, &installed, &[("residue", 0)]);
    store.register_docker_resources(Vec::new()).unwrap();
    store
        .register_application_snapshot(vec![app_identity("application", &[(2, "application", 1)])])
        .unwrap();
    assert!(store.active_snapshots.len() <= MAX_SNAPSHOTS);

    let mut empty = OperationStore::new();
    let registration = empty
        .register_snapshot(SnapshotKind::Cache, cache_payload(["item"]))
        .unwrap();
    assert!(empty
        .prepare_cache(&registration.snapshot_id, Vec::<&str>::new(), "main")
        .is_err());
    assert!(empty
        .prepare_cache(
            &registration.snapshot_id,
            vec![
                &registration.selection_keys[0],
                &registration.selection_keys[0],
            ],
            "main",
        )
        .is_err());
}

#[test]
fn prepared_operation_ids_are_random_and_unique() {
    let mut store = OperationStore::new();
    let first = store
        .register_snapshot(SnapshotKind::Cache, cache_payload(["first"]))
        .unwrap();
    let second = store
        .register_snapshot(SnapshotKind::Process, process_payload("second"))
        .unwrap();
    let first_operation = store
        .prepare_cache(&first.snapshot_id, vec![&first.selection_keys[0]], "main")
        .unwrap();
    let second_operation = store
        .prepare_process(
            &second.snapshot_id,
            vec![&second.selection_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();

    assert_ne!(first_operation.operation_id, second_operation.operation_id);
    assert!(is_lower_hex(&first_operation.operation_id));
    assert!(is_lower_hex(&second_operation.operation_id));
}

// ── i18n key 门禁：操作摘要只发 key，不发文案 ──

/// 「`summary` 位上只放 key」的源码层门禁。
///
/// 摘要是执行前确认弹窗的标题，ASC 截图必然拍到。`PreparedOperation` 的字段名
/// 从 `summary: String` 变成 `summary_key` + `summary_params` 之后，编译器已经
/// 挡住了「把中文塞进 summary」；这道门禁补上的是：
///
/// 1. key 落在 `opSummary.*` 命名空间
/// 2. key 是纯 ASCII（想把整句中文当 key 用，当场红）
/// 3. 恰好 11 枚 —— 6 种操作 × (卸载的 2 个变体 / Docker 的 4 个变体)。
///    少一枚就有一个组合退化成查不到翻译的裸 key，多一枚说明有人加文案时
///    忘了走门禁。
#[test]
fn operation_summaries_ship_only_ascii_i18n_keys() {
    let source = include_str!("operation_registry.rs");
    // 本门禁把整份文件都算作生产代码（没有 `#[cfg(test)]` 可切）。哪天有人给
    // 这个文件加了内联测试模块，测试里的 key 字面量就会混进「后端在发」集合，
    // 让孤儿检测失效 —— 所以这里显式钉住前提。
    assert!(
        !source.contains("#[cfg(test)]"),
        "operation_registry.rs 出现了内联测试模块：key 抽取必须改成先切掉它",
    );
    let keys = crate::i18n_text::key_literals_before_tests(source, "opSummary.");

    assert_eq!(
        keys.len(),
        11,
        "opSummary.* 必须恰好 11 枚（cache / processForce / processGraceful / \
         appTerminate / appGracefulQuit / uninstall / uninstallQuitFirst / \
         dockerPrune / dockerRemoveImage / dockerRemoveContainer / dockerRemoveVolume），\
         实际 {} 枚：{keys:?}",
        keys.len(),
    );
    for key in &keys {
        crate::i18n_text::assert_ascii_key(key);
    }
    for expected in [
        "opSummary.cache",
        "opSummary.processForce",
        "opSummary.processGraceful",
        "opSummary.appTerminate",
        "opSummary.appGracefulQuit",
        "opSummary.uninstall",
        "opSummary.uninstallQuitFirst",
        "opSummary.dockerPrune",
        "opSummary.dockerRemoveImage",
        "opSummary.dockerRemoveContainer",
        "opSummary.dockerRemoveVolume",
    ] {
        assert!(
            keys.iter().any(|key| key == expected),
            "{expected} 必须存在，否则这类操作的确认弹窗会显示裸 key",
        );
    }
}
