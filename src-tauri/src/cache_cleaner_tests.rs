use super::*;
use crate::cache_scanner::{CacheCategory, Safety};
use crate::operations::CacheAction;
use crate::user_error::UserError;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct RecordingRuntime {
    requests: Arc<Mutex<Vec<CacheRequest>>>,
    busy: Option<String>,
    allowed: bool,
    root_owned: bool,
}

impl Default for RecordingRuntime {
    fn default() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            busy: None,
            allowed: true,
            root_owned: false,
        }
    }
}

impl CacheRuntime for RecordingRuntime {
    fn is_busy(&self, _item: &CacheItem) -> Option<String> {
        self.busy.clone()
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, UserError> {
        Ok(expand_tilde(&path.to_string_lossy()))
    }

    fn is_path_allowed(&self, _path: &Path) -> bool {
        self.allowed
    }

    fn is_root_owned(&self, _path: &Path) -> bool {
        self.root_owned
    }

    fn owner_uid(&self, _path: &Path) -> Result<u32, UserError> {
        Ok(0)
    }

    fn is_symlink(&self, _path: &Path) -> Result<bool, UserError> {
        Ok(false)
    }

    fn is_directory(&self, _path: &Path) -> Result<bool, UserError> {
        Ok(true)
    }

    fn execute<'a>(
        &'a self,
        request: CacheRequest,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserError>> + Send + 'a>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(()) })
    }
}

fn test_item(action: CacheAction, id: &str, path: &str) -> CacheItem {
    CacheItem {
        id: id.into(),
        category: CacheCategory::System,
        label_key: "cache.item.appCache".into(),
        label_params: vec![("app".to_owned(), id.to_owned())],
        description_key: "cache.desc.appCache".into(),
        description_params: Vec::new(),
        path: Some(path.into()),
        size_bytes: 1,
        safety: Safety::Safe,
        default_select: true,
        action,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    }
}

#[tokio::test]
async fn typed_action_dispatch_uses_fixed_request() {
    let runtime = RecordingRuntime::default();
    let item = test_item(CacheAction::Pnpm, "pnpm-store", "~/.cache/pnpm");
    let summary = clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(summary.success_count, 1);
    assert_eq!(
        *runtime.requests.lock().unwrap(),
        vec![CacheRequest::Command(CacheCommand::PnpmStorePrune)]
    );
}

#[tokio::test]
async fn docker_dispatch_rejects_unknown_internal_id() {
    let runtime = RecordingRuntime::default();
    let item = test_item(CacheAction::Docker, "client-forged", "");
    let summary = clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn docker_dispatch_uses_typed_command() {
    let runtime = RecordingRuntime::default();
    let item = test_item(CacheAction::Docker, "docker-builder-cache", "");
    clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(
        *runtime.requests.lock().unwrap(),
        vec![CacheRequest::Command(CacheCommand::DockerBuilderPrune)]
    );
}

#[tokio::test]
async fn rejected_path_is_not_executed() {
    let runtime = RecordingRuntime {
        allowed: false,
        ..RecordingRuntime::default()
    };
    let item = test_item(CacheAction::System, "system", "/tmp/not-allowed");
    let summary = clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn busy_tool_is_not_executed() {
    let runtime = RecordingRuntime {
        busy: Some("npm".into()),
        ..RecordingRuntime::default()
    };
    let item = test_item(CacheAction::Npm, "npm-cache", "~/.npm");
    let summary = clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(summary.fail_count, 1);
    assert!(runtime.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn root_owned_path_requests_sudo() {
    let runtime = RecordingRuntime {
        root_owned: true,
        ..RecordingRuntime::default()
    };
    let item = test_item(CacheAction::Npm, "npm-cache", "~/.npm");
    clean_with_runtime(vec![item], &runtime).await;

    assert_eq!(
        *runtime.requests.lock().unwrap(),
        vec![CacheRequest::Remove {
            path: expand_tilde("~/.npm"),
            sudo: true,
        }]
    );
}

#[test]
fn reject_root() {
    assert!(!is_cleanup_path_allowed(Path::new("/")));
}

#[test]
fn reject_system_dirs() {
    for p in ["/usr", "/etc", "/var", "/bin", "/System", "/Applications"] {
        assert!(!is_cleanup_path_allowed(Path::new(p)), "必须拒绝 {}", p);
    }
}

#[test]
fn reject_home() {
    if let Some(home) = dirs::home_dir() {
        assert!(!is_cleanup_path_allowed(&home), "绝不能允许删 home");
    }
}

#[test]
fn reject_documents_or_downloads() {
    if let Some(home) = dirs::home_dir() {
        assert!(!is_cleanup_path_allowed(&home.join("Documents")));
        assert!(!is_cleanup_path_allowed(&home.join("Downloads")));
        assert!(!is_cleanup_path_allowed(&home.join("Desktop")));
    }
}

#[test]
fn reject_nonexistent_random_path() {
    assert!(!is_cleanup_path_allowed(Path::new(
        "/this/does/not/exist/anywhere"
    )));
}

#[test]
fn accept_npm_cache_when_exists() {
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".npm");
        if p.exists() {
            assert!(is_cleanup_path_allowed(&p), "~/.npm 应在白名单内");
        }
    }
}

#[test]
fn expand_tilde_works() {
    let home = dirs::home_dir().unwrap();
    assert_eq!(expand_tilde("~/.npm"), home.join(".npm"));
    assert_eq!(
        expand_tilde("/absolute/path"),
        PathBuf::from("/absolute/path")
    );
}
