//! security-scoped bookmark 的 ObjC 桥接层。
//!
//! # 这一层为什么全用 `msg_send` 自己声明
//!
//! `objc2-foundation` 0.3.2 **没有**导出 NSURL 的书签方法
//! （`URLByResolvingBookmarkData:`、`bookmarkDataWithOptions:…`、
//! `startAccessingSecurityScopedResource`），而且它的 `src/generated/` 缺
//! `mod.rs`，作为直接依赖根本编译不过。所以这里只依赖 `objc2` 运行时，
//! 把 NSURL / NSString / NSData 全当不透明对象，方法签名逐个自己声明。
//!
//! 这么做还有一个好处：**需要声明的东西少到能被人读完**。这一层是唯一
//! 直接碰不安全内存的地方，而 Apple 文档里的四步流程原样对应四个函数，
//! 任何一步写错都能一眼看出来。
//!
//! # Apple 文档规定的流程（顺序不能变）
//!
//! 1. `bookmarkDataWithOptions:` 带 `NSURLBookmarkCreationWithSecurityScope`
//! 2. 存下来（base64）
//! 3. `URLByResolvingBookmarkData:` 带 `NSURLBookmarkResolutionWithSecurityScope`
//!    → 解析成 URL；`bookmarkDataIsStale` 为真时要重新存一遍
//! 4. **`startAccessingSecurityScopedResource`** → 才开始能读
//!    用完 `stopAccessingSecurityScopedResource`
//!
//! 第 4 步最容易被漏：**解析出 URL 不等于有了访问权**。少了这行调用，
//! 后面所有 `read_dir` 都会 EPERM，而代码看起来完全正常。

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

/// `NSURLBookmarkCreationWithSecurityScope`
const CREATION_WITH_SECURITY_SCOPE: usize = 1 << 11;
/// `NSURLBookmarkResolutionWithSecurityScope`
const RESOLUTION_WITH_SECURITY_SCOPE: usize = 1;

/// 把 `Option<Retained<..>>` 的「无」当成硬错误。
///
/// **只用于「不可能失败」的那几步**（构造 NSString、造 NSURL）。
/// 返回 nil 只可能是我们把方法签名声明错了 —— 那是编程错误，
/// 应该立刻炸出来，而不是被当成「用户没授权」静默吞掉。
///
/// 运行期真的会失败的调用（`bookmarkDataWithOptions:`：目录被删、无权限）
/// 一律走 `?` 返回 `None`。
fn expect_object(object: Option<Retained<AnyObject>>) -> Retained<AnyObject> {
    object.expect("Objective-C 工厂方法返回 nil —— 方法签名声明错了")
}

/// ObjC 字符串 → Rust `String`（走 `UTF8String`，调用方不持有所有权）。
///
/// # Safety
///
/// receiver 必须真的是 NSString。
unsafe fn nsstring_to_rust(object: &AnyObject) -> String {
    let pointer: *const std::ffi::c_char = msg_send![object, UTF8String];
    if pointer.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(pointer)
        .to_string_lossy()
        .into_owned()
}

/// Rust `&str` → NSString。
///
/// # Safety
///
/// 只在 ObjC 运行时已初始化后调用。
unsafe fn nsstring_from_str(value: &str) -> Retained<AnyObject> {
    let c_string =
        std::ffi::CString::new(value).expect("路径里不该有 NUL —— 那是文件系统不允许的字符");
    // 用便利构造器而不是 alloc + init：alloc 属于 init-family，
    // objc2 会要求返回类型与 receiver 类型绑定，对「不透明对象」很不友好。
    let string: Option<Retained<AnyObject>> = msg_send![
        class!(NSString),
        stringWithUTF8String: c_string.as_ptr(),
    ];
    expect_object(string)
}

/// `NSData` → `Vec<u8>`
///
/// # Safety
///
/// receiver 必须真的是 NSData。
unsafe fn nsdata_to_vec(data: &AnyObject) -> Vec<u8> {
    // 先问长度再取指针：指针和长度必须来自同一次调用，否则中间对象被
    // 回收就会读越界。
    let length: usize = msg_send![data, length];
    if length == 0 {
        return Vec::new();
    }
    let pointer: *const u8 = msg_send![data, bytes];
    if pointer.is_null() {
        return Vec::new();
    }
    std::slice::from_raw_parts(pointer, length).to_vec()
}

/// 为一个路径创建 app-scoped security bookmark，返回 base64。
///
/// 「app-scoped」用于「持久化用户授予的目录」；另一种 document-scope 是给
/// 「某个文档连同它的附属资源」用的 —— 我们不是后者。
/// 失败原因。分成几步是因为**每一步失败看起来都一样**（拿不到目录），
/// 不分开报就永远只能猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkError {
    /// 造 NSString / NSURL 失败 —— 只可能是签名声明错了。
    ConstructionFailed,
    /// `bookmarkDataWithOptions:` 返回 nil。目录不存在、或当前无访问权。
    CreationRejected,
    /// base64 或 `dataWithBytes:` 失败。
    DecodeFailed,
    /// `URLByResolvingBookmarkData:` 返回 nil —— 书签损坏、路径已不存在，
    /// 或**缺 `bookmarks.app-scope` entitlement**。
    ResolveFailed,
    /// 解析成功但 `startAccessingSecurityScopedResource` 返回 false ——
    /// 这是最容易踩的一步：少看返回值就会以为成功，然后 read_dir 全部 EPERM。
    AccessNotGranted,
}

impl BookmarkError {
    /// i18n key（纯 ASCII，不是文案）。
    #[must_use]
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::ConstructionFailed => "access.error.construction",
            Self::CreationRejected => "access.error.creationRejected",
            Self::DecodeFailed => "access.error.decode",
            Self::ResolveFailed => "access.error.resolve",
            Self::AccessNotGranted => "access.error.accessNotGranted",
        }
    }
}

/// 带失败原因的版本。`Option` 版给「只需要知道成不成」的地方用。
pub fn create_bookmark_detailed(path: &std::path::Path) -> Result<String, BookmarkError> {
    let bookmark = create_bookmark(path);
    match bookmark {
        Some(value) => Ok(value),
        None if !path.exists() => Err(BookmarkError::CreationRejected),
        None if !std::fs::read_dir(path).is_ok() => Err(BookmarkError::AccessNotGranted),
        None => Err(BookmarkError::CreationRejected),
    }
}

/// 解析 + 取访问权，带**分步**失败原因。
///
/// 「解析不出来」和「解析出来了但拿不到访问权」是两回事，而且症状在
/// 上层完全一样（都表现为 read_dir EPERM）。合成一句「授权失败」会让人
/// 一直查错方向 —— 实际踩过：明明 `bookmarks.app-scope` 已经在签名里，
/// 却因为把这两步混在一起，误判成「profile 缺 entitlement」
/// （而 MAS 的 profile 本来就不该有沙箱 entitlement，那是个错误方向）。
pub fn resolve_detailed(bookmark_base64: &str) -> Result<SecurityScope, BookmarkError> {
    let mut stale: u8 = 0;
    // SAFETY: 逐个对应 Apple 文档的四步；错误原因按「停在哪一步」区分。
    let (url, stale) =
        unsafe { resolve_url(bookmark_base64, RESOLUTION_WITH_SECURITY_SCOPE, &mut stale)? };
    // SAFETY: 对 NSURL 声明方法。
    let started: bool = unsafe { msg_send![&*url, startAccessingSecurityScopedResource] };
    if !started {
        return Err(BookmarkError::AccessNotGranted);
    }
    let path = unsafe { url_path(&url) };
    Ok(SecurityScope {
        url,
        path,
        stale: stale != 0,
    })
}

/// 不带 security-scope 选项解析书签。**仅用于诊断**。
///
/// 存在的唯一理由是一次二分诊断：同一份书签带 scope 解析失败时，再用不带
/// scope 解析一次 —— 能解析说明创建时的掩码写错（产出的根本不是 scoped
/// 书签），不能解析说明问题在沙箱层面。这两种猜错的修法完全相反
/// （一个改常量，一个查 entitlement），没有它就只能二选一地赌。
#[must_use]
pub fn resolve_plain(bookmark_base64: &str) -> Option<String> {
    let mut stale: u8 = 0;
    // SAFETY: 同 resolve_detailed，只是 options 传 0，且不取访问权。
    let (url, _) = unsafe { resolve_url(bookmark_base64, 0, &mut stale) }.ok()?;
    Some(unsafe { url_path(&url) })
}

/// 解 base64 → 造 NSData → `URLByResolvingBookmarkData:`。
///
/// # Safety
///
/// 纯 FFI；`options` 由调用方决定。
#[allow(clippy::explicit_auto_deref)]
unsafe fn resolve_url(
    bookmark_base64: &str,
    options: usize,
    stale_out: &mut u8,
) -> Result<(Retained<AnyObject>, u8), BookmarkError> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(bookmark_base64)
        .map_err(|_| BookmarkError::DecodeFailed)?;
    let data: Option<Retained<AnyObject>> = msg_send![
        class!(NSData),
        dataWithBytes: raw.as_ptr(),
        length: raw.len(),
    ];
    // 纯内存拷贝，返回 nil 只能说明签名声明错了
    let data = expect_object(data);
    let url: Option<Retained<AnyObject>> = msg_send![
        class!(NSURL),
        URLByResolvingBookmarkData: &*data,
        options: options,
        relativeToURL: std::ptr::null::<AnyObject>(),
        bookmarkDataIsStale: &mut *stale_out,
        error: std::ptr::null_mut::<*mut AnyObject>(),
    ];
    let url = url.ok_or(BookmarkError::ResolveFailed)?;
    Ok((url, *stale_out))
}

/// NSURL 的 `path`。
///
/// # Safety
///
/// receiver 必须真的是 NSURL。
#[allow(clippy::explicit_auto_deref)] // Retained 没有 AsRef<AnyObject>，必须显式解引用
unsafe fn url_path(url: &AnyObject) -> String {
    let ns_path: Option<Retained<AnyObject>> = msg_send![url, path];
    match ns_path {
        Some(object) => nsstring_to_rust(&object),
        None => String::new(),
    }
}

#[must_use]
#[allow(clippy::explicit_auto_deref)] // Retained 没有 AsRef<AnyObject>，必须显式解引用
pub fn create_bookmark(path: &std::path::Path) -> Option<String> {
    use base64::Engine;
    let path_text = path.to_string_lossy().into_owned();
    // SAFETY: 逐个对应 Apple 文档的 NSURL / NSString 声明方法。
    // error: 一律传 NULL —— 这些方法允许不取 NSError。
    unsafe {
        let ns_path = nsstring_from_str(&path_text);
        let url = expect_object(msg_send![
            class!(NSURL),
            fileURLWithPath: &*ns_path,
        ]);
        let data: Option<Retained<AnyObject>> = msg_send![
            &*url,
            bookmarkDataWithOptions: CREATION_WITH_SECURITY_SCOPE,
            includingResourceValuesForKeys: std::ptr::null::<AnyObject>(),
            relativeToURL: std::ptr::null::<AnyObject>(),
            error: std::ptr::null_mut::<*mut AnyObject>(),
        ];
        // 目录被删、或当前没有访问权时这里是 nil —— 属运行期情况，返回 None。
        let data = data?;
        // 显式解引用不是笔误：函数要 &AnyObject，而 Retained 没有
        // 实现 AsRef<AnyObject>（clippy 的 explicit-auto-deref 在这里
        // 建议的写法编译不过）。
        let bytes = nsdata_to_vec(&*data);
        Some(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}

/// 解析书签并**开启访问作用域**。
///
/// 成功返回的对象持有那次 `startAccessingSecurityScopedResource`，析构时
/// 配对 `stopAccessingSecurityScopedResource`。少一次配对会泄漏沙箱授权
/// 计数，多一次会让后续访问莫名其妙地失效。
pub struct SecurityScope {
    url: Retained<AnyObject>,
    path: String,
    stale: bool,
}

impl SecurityScope {
    /// 拿到的真实路径。
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 书签是否已过期（目录被移动或重命名）。
    ///
    /// 为真时调用方应当重新创建并存一次书签 —— Apple 文档明确要求。
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.stale
    }
}

impl Drop for SecurityScope {
    fn drop(&mut self) {
        // SAFETY: 只有 start 成功才会构造出这个对象，所以这里配对 stop
        // 一定合法、且不多不少。
        unsafe {
            let _: () = msg_send![&*self.url, stopAccessingSecurityScopedResource];
        }
    }
}

/// 解析书签并开启访问作用域。
///
/// **解析成功但 `startAccessingSecurityScopedResource` 返回 false 时返回
/// `None`** —— 这正是「解析出来了却还是读什么都 EPERM」的陷阱，必须显式
/// 处理，不能返回一个看起来成功、实际没权限的对象。
#[allow(clippy::explicit_auto_deref)] // Retained 没有 AsRef<AnyObject>，必须显式解引用
#[must_use]
pub fn resolve_and_access(bookmark_base64: &str) -> Option<SecurityScope> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(bookmark_base64)
        .ok()?;
    // SAFETY: 逐个对应 Apple 文档的四步：resolve → (stale) → startAccessing。
    unsafe {
        let data: Option<Retained<AnyObject>> = msg_send![
            class!(NSData),
            dataWithBytes: raw.as_ptr(),
            length: raw.len(),
        ];
        // 纯内存拷贝，nil 只能说明签名声明错了
        let data = expect_object(data);
        let mut stale: u8 = 0;
        let url: Option<Retained<AnyObject>> = msg_send![
            class!(NSURL),
            URLByResolvingBookmarkData: &*data,
            options: RESOLUTION_WITH_SECURITY_SCOPE,
            relativeToURL: std::ptr::null::<AnyObject>(),
            bookmarkDataIsStale: &mut stale,
            error: std::ptr::null_mut::<*mut AnyObject>(),
        ];
        let url = url?;
        let started: bool = msg_send![&*url, startAccessingSecurityScopedResource];
        if !started {
            // 没拿到访问权就已经把 URL 解析出来了 —— 这里返回 None，
            // 绝不能返回一个「看起来解析成功但读什么都 EPERM」的对象。
            return None;
        }
        let ns_path: Option<Retained<AnyObject>> = msg_send![&*url, path];
        // 同上：必须显式解引用，Retained 没有 AsRef<AnyObject>
        let path = nsstring_to_rust(&*expect_object(ns_path));
        Some(SecurityScope {
            url,
            path,
            stale: stale != 0,
        })
    }
}

// ============================================================================
// NSOpenPanel —— 让用户亲手授权一个目录
// ============================================================================
//
// ## 为什么必须是系统文件选择框
//
// 沙箱里没有任何 entitlement 能让我们直接读用户目录（Apple 文档：即使拿到
// FDA 沙箱仍强制执行自己的文件限制）。唯一合规的入口是让用户在标准文件
// 选择框里**亲手选定**一个目录 —— 这与「绕过沙箱」有本质区别，也是
// App Store 认可的方式（先例：PureSpace）。
//
// ## 调用位置
//
// `NSOpenPanel` 的 `runModal` 必须在**主线程**、且应用已激活时调用。
// Tauri 的命令默认跑在 async runtime 上，所以这里由 `pick_folder` 只负责
// 组参数，真正弹窗由调用方通过 `run_on_main_thread` 调度。

/// 让用户在文件选择框里选一个目录。
///
/// 返回用户选中的路径；用户取消返回 `None`。**必须在主线程调用。**
///
/// # Safety
///
/// `runModal` 会阻塞到用户做出选择。Tauri 的 `run_on_main_thread` 正是为此
/// 提供的 —— 在别的线程调用会直接崩。
pub unsafe fn pick_folder(prompt: &str) -> Option<std::path::PathBuf> {
    let panel: Option<Retained<AnyObject>> = msg_send![class!(NSOpenPanel), openPanel];
    let panel = panel?;

    // 只选目录、单选。选了文件的话用户会以为授权了整个上级目录，
    // 而我们实际只拿到那一个文件 —— 期望与现实不一致。
    let _: () = msg_send![&*panel, setCanChooseDirectories: true];
    let _: () = msg_send![&*panel, setCanChooseFiles: false];
    let _: () = msg_send![&*panel, setAllowsMultipleSelection: false];
    // 允许直接定位到隐藏目录（~/.npm、~/.cargo 都是点号开头的，
    // 不放开的话用户在选择框里根本看不见）
    let _: () = msg_send![&*panel, setShowsHiddenFiles: true];

    let title = nsstring_from_str(prompt);
    let _: () = msg_send![&*panel, setMessage: &*title];

    // NSModalResponseOK == 1
    let response: isize = msg_send![&*panel, runModal];
    if response != 1 {
        return None;
    }

    let url: Option<Retained<AnyObject>> = msg_send![&*panel, URL];
    let url = url?;
    Some(std::path::PathBuf::from(url_path(&url)))
}
