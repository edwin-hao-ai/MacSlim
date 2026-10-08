use super::*;
use crate::cache_cleaner::{clean_with_runtime, CacheRequest, CacheRuntime};
use crate::cache_scanner::{CacheCategory, Safety};
use crate::operations::{
    CacheAction, ConsumedPlan, DockerAction, DockerResourceKind, DockerTarget, OperationStore,
};
use crate::user_error::UserError;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Default)]
struct FakeCleaner {
    items: Arc<Mutex<Vec<CacheItem>>>,
}

impl CacheCleaner for FakeCleaner {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        self.items.lock().unwrap().extend(items);
        Box::pin(async {
            CleanSummary {
                reports: Vec::new(),
                deleted_bytes: 0,
                reclaimed_bytes: None,
                success_count: 0,
                fail_count: 0,
            }
        })
    }
}

struct RuntimeProbe {
    requests: Arc<Mutex<Vec<CacheRequest>>>,
    owner_uid: Arc<Mutex<Option<u32>>>,
    allowed: bool,
    canonicalize_error: bool,
    canonical_path: Option<PathBuf>,
    symlink: bool,
    directory: bool,
}

impl RuntimeProbe {
    fn new(owner_uid: Option<u32>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            owner_uid: Arc::new(Mutex::new(owner_uid)),
            allowed: true,
            canonicalize_error: false,
            canonical_path: None,
            symlink: false,
            directory: true,
        }
    }

    fn set_owner_uid(&self, owner_uid: Option<u32>) {
        *self.owner_uid.lock().unwrap() = owner_uid;
    }

    fn owner_uid(&self) -> Option<u32> {
        *self.owner_uid.lock().unwrap()
    }
}

impl CacheRuntime for RuntimeProbe {
    fn is_busy(&self, _item: &CacheItem) -> Option<String> {
        None
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, UserError> {
        if self.canonicalize_error {
            return Err("canonicalize 失败".into());
        }
        Ok(self
            .canonical_path
            .clone()
            .unwrap_or_else(|| path.to_path_buf()))
    }

    fn is_path_allowed(&self, _path: &Path) -> bool {
        self.allowed
    }

    fn is_root_owned(&self, _path: &Path) -> bool {
        self.owner_uid().map(|uid| uid == 0).unwrap_or(false)
    }

    fn owner_uid(&self, _path: &Path) -> Result<u32, UserError> {
        self.owner_uid()
            .ok_or_else(|| UserError::from("没有 ownership 状态"))
    }

    fn is_symlink(&self, _path: &Path) -> Result<bool, UserError> {
        Ok(self.symlink)
    }

    fn is_directory(&self, _path: &Path) -> Result<bool, UserError> {
        Ok(self.directory)
    }

    fn execute<'a>(
        &'a self,
        request: CacheRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserError>> + Send + 'a>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(()) })
    }
}

fn stale_item(path: PathBuf, owner_uid: u32) -> CacheItem {
    CacheItem {
        id: "stale-node-modules-0".into(),
        category: CacheCategory::Npm,
        label_key: "cache.item.staleNodeModules".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.staleNodeModules".into(),
        description_params: vec![("path".to_owned(), path.to_string_lossy().into_owned())],
        path: Some(path.to_string_lossy().into_owned()),
        size_bytes: 1,
        safety: Safety::Low,
        default_select: false,
        action: CacheAction::StaleNodeModules,
        stale_owner_uid: Some(owner_uid),
        stale_canonical_path: Some(path),
        recover_hint: String::new(),
    }
}

fn cache_item() -> CacheItem {
    CacheItem {
        id: "cache".into(),
        category: CacheCategory::System,
        label_key: "cache.item.appLogs".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.appLogs".into(),
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
}

fn consumed_cache_plan(store: &mut OperationStore, item: CacheItem) -> ConsumedPlan {
    let registration = store.register_cache_snapshot(vec![item]).unwrap();
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn consumed_docker_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_docker_resources(vec![DockerTarget {
            resource_type: DockerResourceKind::Image,
            id: "image-id".into(),
            name: "image".into(),
            size_bytes: 0,
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
async fn executor_requires_store_consumed_wrapper() {
    let mut store = OperationStore::new();
    let registration = store.register_cache_snapshot(vec![cache_item()]).unwrap();
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    let consumed: ConsumedPlan = store.consume(&prepared.operation_id, "main").unwrap();
    let cleaner = FakeCleaner::default();

    let result = execute_cache_plan_with(consumed, &cleaner).await;

    assert!(result.is_ok());
    assert_eq!(cleaner.items.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn executor_public_boundary_accepts_consumed_cache_plan_without_side_effects() {
    let mut store = OperationStore::new();
    let consumed = consumed_cache_plan(&mut store, cache_item());

    let result = execute_cache_plan(consumed).await.unwrap();

    assert_eq!(result.success_count, 0);
    assert_eq!(result.fail_count, 1);
}

#[tokio::test]
async fn executor_passes_only_typed_cache_items_to_fake_cleaner() {
    let cleaner = FakeCleaner::default();
    let mut store = OperationStore::new();
    let consumed = consumed_cache_plan(&mut store, cache_item());

    let result = execute_cache_plan_with(consumed, &cleaner).await;

    assert!(result.is_ok());
    let received = cleaner.items.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].action, CacheAction::System);
}

#[tokio::test]
async fn executor_rejects_non_cache_plan_after_consume() {
    let cleaner = FakeCleaner::default();
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(&mut store);

    let result = execute_cache_plan_with(consumed, &cleaner).await;

    assert_eq!(result.unwrap_err(), "操作计划不是缓存计划");
    assert!(cleaner.items.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_cleanup_uses_snapshot_path_without_rescanning() {
    let runtime = RuntimeProbe::new(Some(0));
    let path = dirs::home_dir()
        .unwrap()
        .join("Projects/example/node_modules");

    let summary = clean_with_runtime(vec![stale_item(path.clone(), 0)], &runtime).await;

    assert_eq!(summary.success_count, 1);
    assert_eq!(
        *runtime.requests.lock().unwrap(),
        vec![CacheRequest::Remove { path, sudo: true }]
    );
}

#[tokio::test]
async fn stale_cleanup_rejects_canonical_path_change() {
    let mut runtime = RuntimeProbe::new(Some(0));
    let path = dirs::home_dir()
        .unwrap()
        .join("Projects/example/node_modules");
    runtime.canonical_path = Some(
        dirs::home_dir()
            .unwrap()
            .join("Projects/other/node_modules"),
    );

    let summary = clean_with_runtime(vec![stale_item(path, 0)], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_cleanup_rejects_ownership_change() {
    let runtime = RuntimeProbe::new(Some(501));
    let path = dirs::home_dir()
        .unwrap()
        .join("Projects/example/node_modules");

    let summary = clean_with_runtime(vec![stale_item(path, 0)], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_cleanup_rejects_canonicalize_failure() {
    let mut runtime = RuntimeProbe::new(Some(0));
    runtime.canonicalize_error = true;
    let path = dirs::home_dir()
        .unwrap()
        .join("Projects/example/node_modules");

    let summary = clean_with_runtime(vec![stale_item(path, 0)], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_cleanup_rejects_paths_outside_project_roots() {
    let runtime = RuntimeProbe::new(Some(0));
    let path = dirs::home_dir().unwrap().join("Other/example/node_modules");

    let summary = clean_with_runtime(vec![stale_item(path, 0)], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

struct TempTree {
    base: PathBuf,
    link: PathBuf,
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.link);
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[tokio::test]
async fn stale_cleanup_rejects_ancestor_symlink_change() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = dirs::home_dir().unwrap().join("Projects").join(format!(
        ".macslim-stale-test-{}-{stamp}",
        std::process::id()
    ));
    let original = base.join("project");
    let replacement = base.join("replacement");
    let original_target = original.join("node_modules");
    let replacement_target = replacement.join("node_modules");
    fs::create_dir_all(&original_target).unwrap();
    fs::create_dir_all(&replacement_target).unwrap();
    let _cleanup = TempTree {
        base: base.clone(),
        link: original.clone(),
    };
    let item = stale_item(original_target.clone(), 0);
    fs::remove_dir_all(&original).unwrap();
    symlink(&replacement, &original).unwrap();
    let mut runtime = RuntimeProbe::new(Some(0));
    runtime.canonical_path = Some(original_target);

    let summary = clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_cleanup_rejects_symlink_and_non_directory_targets() {
    let mut runtime = RuntimeProbe::new(Some(0));
    runtime.symlink = true;
    let path = dirs::home_dir()
        .unwrap()
        .join("Projects/example/node_modules");
    let symlink_summary = clean_with_runtime(vec![stale_item(path.clone(), 0)], &runtime).await;
    assert_eq!(symlink_summary.fail_count, 1);

    runtime.symlink = false;
    runtime.directory = false;
    let non_directory_summary = clean_with_runtime(vec![stale_item(path, 0)], &runtime).await;
    assert_eq!(non_directory_summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}
