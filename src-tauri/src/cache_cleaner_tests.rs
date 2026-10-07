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

/// `~` 必须按**真实 home** 展开，不能按沙箱里的 `$HOME`。
///
/// ## 后果不是「路径不对」，是这个功能整个不работа
///
/// App Store 版里 `$HOME` 指向应用自己的空 container（实测
/// `home_env = .../Containers/com.vgoapp.macslim/Data`，而 `real_home =
/// /Users/edwinhao`）。而扫描器走的是 `folder_access::scanner_home()`，也就是
/// passwd 里的真实 home —— 于是同一个目录，**扫描时看的是真实家目录，清理时
/// 看的是容器**。
///
/// 清理项的 path 是 `"~/Library/Logs"` 这种带波浪号的字符串，展开后落到
/// 容器里一个不存在的路径，`canonicalize` 直接失败。实测表现：
///
///     历史：成功 0 项，失败 1 项，释放 0
///
/// 也就是说：用户授权了目录、看到 10.5 GB 可释放、按下清理、确认，然后
/// **什么都没删**。好在它如实上报了失败（没有静默假成功），但功能等于没有。
///
/// ## 这条测试必须自己给 home，否则抓不到
///
/// 直接断言 `expand_tilde("~/x") == scanner_home().join("x")` 在**非沙箱**的
/// cargo test 里恒真 —— 那里 `$HOME` 就是真实 home。所以真正有鉴别力的是
/// 下面那条：拿一个假的 home 喂进去，看它会不会用错。
#[test]
fn expand_tilde_from_expands_against_the_home_it_is_given() {
    let fake = PathBuf::from("/Users/someone-else");

    assert_eq!(
        expand_tilde_from("~/Library/Logs", &fake),
        fake.join("Library/Logs"),
        "波浪号必须展开成**给定的** home"
    );
    assert_eq!(expand_tilde_from("~", &fake), fake);
    assert_eq!(
        expand_tilde_from("/absolute/path", &fake),
        PathBuf::from("/absolute/path"),
        "绝对路径不受 home 影响"
    );
    assert_eq!(
        expand_tilde_from("relative/path", &fake),
        PathBuf::from("relative/path"),
        "相对路径不该被拼到 home 上"
    );
}

/// 清理用的那个 `expand_tilde` 必须把真实 home 传进去。
///
/// 这条是「接口接对了」的断言：上面的纯函数测试证明 `expand_tilde_from` 行为
/// 正确，这条证明生产路径真的用了 `scanner_home()` 而不是 `$HOME`。
#[test]
fn the_cleanup_expander_uses_the_real_home() {
    let expected = crate::folder_access::scanner_home();
    assert_eq!(
        expand_tilde("~/Library/Logs"),
        expected.join("Library/Logs")
    );
}

/// 清理白名单必须和扫描器用**同一个 home**。
///
/// 这条抓的是上架版的致命组合：扫描器走 `scanner_home()`（真实 home），
/// 而白名单曾经走 `dirs::home_dir()`（沙箱里是应用自己的 container）。
/// 于是待删路径 `/Users/edwinhao/Library/Logs` 永远不以白名单根
/// `.../Containers/com.vgoapp.macslim/Data/Library/Logs` 开头，
/// `is_cleanup_path_allowed` 恒为 false —— 界面扫得出 10.5 GB、也确认了清理，
/// 结果是「成功 0 项，失败 1 项，释放 0」。
///
/// 在非沙箱的 `cargo test` 里两个 home 恰好相同，所以这条断言本身抓不到
/// 沙箱内的差异；它的价值是**把两者钉在一起**：谁再改回 `dirs::home_dir()`，
/// 在 MAS 构建里就会立刻不一致（`scanner_home()` 的 MAS 分支返回真实 home）。
#[test]
fn the_cleanup_whitelist_uses_the_real_home() {
    let home = crate::folder_access::scanner_home();
    let roots = allowed_cleanup_roots();
    for expected in [
        home.join("Library/Logs"),
        home.join("Library/Caches"),
        home.join(".npm"),
        home.join(".cargo/registry/cache"),
    ] {
        assert!(
            roots.contains(&expected),
            "白名单缺少 {}（说明它没跟着 scanner_home() 走）",
            expected.display()
        );
    }
}
