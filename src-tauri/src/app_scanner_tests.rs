use super::*;
use crate::operations::OperationStore;
use std::sync::atomic::{AtomicU64, Ordering};

fn installed_app(label: &str) -> InstalledApp {
    InstalledApp {
        bundle_path: format!("/Applications/{label}.app"),
        name: label.to_owned(),
        bundle_id: format!("com.example.{label}"),
        icon_base64: None,
        bundle_size_bytes: 1024,
        is_system: false,
        is_running: true,
        estimated_residue_bytes: 0,
        selection_key: String::new(),
    }
}

#[test]
fn uninstaller_app_identities_are_derived_from_scan_rows() {
    let apps = vec![installed_app("alpha"), installed_app("beta")];

    let identities = installed_app_identities(&apps);

    assert_eq!(identities.len(), 2);
    assert_eq!(identities[0].bundle_path, "/Applications/alpha.app");
    assert_eq!(identities[0].app_name, "alpha");
    assert_eq!(identities[0].bundle_id, "com.example.alpha");
    assert_eq!(identities[1].bundle_id, "com.example.beta");
}

#[test]
fn uninstaller_app_registration_binds_one_opaque_key_per_row() {
    let mut store = OperationStore::new();
    let mut apps = vec![installed_app("alpha"), installed_app("beta")];

    let registration = register_installed_apps(&mut store, &mut apps).unwrap();

    assert_eq!(registration.selection_keys.len(), 2);
    assert!(apps
        .iter()
        .all(|app| app.selection_key.len() == 64 && !app.selection_key.is_empty()));
    assert_eq!(apps[0].selection_key, registration.selection_keys[0]);
    assert_eq!(apps[1].selection_key, registration.selection_keys[1]);
    assert_ne!(apps[0].selection_key, "/Applications/alpha.app");
    assert_ne!(apps[0].selection_key, "com.example.alpha");
}

#[test]
fn uninstaller_app_registration_keys_drive_prepare_uninstall() {
    let mut store = OperationStore::new();
    let mut apps = vec![installed_app("alpha")];
    let registration = register_installed_apps(&mut store, &mut apps).unwrap();
    let app_key = apps[0].selection_key.clone();

    let prepared = store
        .prepare_uninstall(
            &registration.snapshot_id,
            &registration.snapshot_id,
            vec![&app_key],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(prepared, "快照不存在或已失效");
}

#[test]
fn uninstaller_system_app_classification_ignores_empty_bundle_id() {
    assert!(is_system_app("com.apple.finder"));
    assert!(!is_system_app("com.example.alpha"));
    assert!(!is_system_app(""));
}

// ========== 扫描并行化 + 磁盘派生缓存 ==========

/// 缓存是进程级全局状态，测试之间必须串行，否则会互相污染
fn cache_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: Mutex<()> = Mutex::new(());
    GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 临时目录 fixture，析构时清理
struct ScanFixture {
    root: PathBuf,
}

impl ScanFixture {
    fn new(label: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "macslim_app_scanner_fixture_{}_{}_{label}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建 fixture 根目录");
        Self { root }
    }

    /// 写一个最小可用的 .app bundle：`payload_bytes` 用来制造确定且互异的体积
    fn write_bundle(
        &self,
        bundle_name: &str,
        bundle_id: &str,
        payload_bytes: usize,
        with_icon: bool,
    ) -> PathBuf {
        let bundle = self.root.join(format!("{bundle_name}.app"));
        let resources = bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources).expect("创建 bundle 目录");

        let icon_key = if with_icon {
            // 真实 icns：手搓的 1x1 PNG 包成 ICNS，sips 能正常解
            std::fs::write(resources.join("AppIcon.icns"), mini_icns()).expect("写 icns");
            "\t<key>CFBundleIconFile</key>\n\t<string>AppIcon</string>\n"
        } else {
            ""
        };
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <plist version=\"1.0\">\n<dict>\n\
                 \t<key>CFBundleName</key>\n\t<string>{bundle_name}</string>\n\
                 \t<key>CFBundleIdentifier</key>\n\t<string>{bundle_id}</string>\n\
                 {icon_key}</dict>\n</plist>\n"
            ),
        )
        .expect("写 Info.plist");
        std::fs::write(resources.join("payload.bin"), vec![0u8; payload_bytes])
            .expect("写 payload");
        bundle
    }

    fn scan_dirs(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }
}

impl Drop for ScanFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 1x1 PNG，包成 ICNS（icns 头 + ic07 块），够 sips 转码用
fn mini_icns() -> Vec<u8> {
    const PNG: [u8; 69] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xC9, 0xFE, 0x92, 0xEF, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let chunk_len = (PNG.len() + 8) as u32;
    let total_len = (PNG.len() + 16) as u32;
    let mut icns = Vec::with_capacity(total_len as usize);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&total_len.to_be_bytes());
    icns.extend_from_slice(b"ic07");
    icns.extend_from_slice(&chunk_len.to_be_bytes());
    icns.extend_from_slice(&PNG);
    icns
}

/// 串行参照实现：只存在于测试里，作为并行版的正确性基准
fn serial_reference_scan(scan_dirs: &[PathBuf], running: &HashSet<String>) -> Vec<InstalledApp> {
    let mut apps: Vec<InstalledApp> = scan_dirs
        .iter()
        .flat_map(|dir| enumerate_apps(dir))
        .filter_map(|path| build_installed_app(&path))
        .collect();
    apply_running_state(&mut apps, running);
    apps.sort_by(compare_apps);
    apps
}

fn ids(apps: &[InstalledApp]) -> Vec<String> {
    apps.iter().map(|a| a.bundle_id.clone()).collect()
}

/// InstalledApp 没派生 PartialEq（不该为了测试去动生产类型的 derive），
/// 所以这里逐字段比对，顺带让失败信息指出具体是哪一行、哪个字段。
fn assert_rows_equal(actual: &[InstalledApp], expected: &[InstalledApp], context: &str) {
    assert_eq!(actual.len(), expected.len(), "{context}: 行数不一致");
    for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            got.bundle_path, want.bundle_path,
            "{context}: 第 {index} 行路径"
        );
        assert_eq!(got.name, want.name, "{context}: 第 {index} 行名称");
        assert_eq!(
            got.bundle_id, want.bundle_id,
            "{context}: 第 {index} 行 bundle_id"
        );
        assert_eq!(
            got.bundle_size_bytes, want.bundle_size_bytes,
            "{context}: 第 {index} 行体积"
        );
        assert_eq!(
            got.icon_base64.is_some(),
            want.icon_base64.is_some(),
            "{context}: 第 {index} 行图标存在性"
        );
        assert_eq!(
            got.is_running, want.is_running,
            "{context}: 第 {index} 行运行态"
        );
        assert_eq!(
            got.is_system, want.is_system,
            "{context}: 第 {index} 行系统应用标记"
        );
        assert_eq!(
            got.estimated_residue_bytes, want.estimated_residue_bytes,
            "{context}: 第 {index} 行残留体积"
        );
    }
}

/// 测试 1：并行结果与串行参照逐项等价（名字 / bundle_id / 体积排序 / 图标存在性）
#[test]
fn parallel_scan_matches_a_serial_reference_row_for_row() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("parallel_equivalence");
    // 体积互不相同，排序才有意义
    fixture.write_bundle("Alpha", "com.example.alpha", 4096, true);
    fixture.write_bundle("Beta", "com.example.beta", 1024, false);
    fixture.write_bundle("Gamma", "com.example.gamma", 8192, true);
    let scan_dirs = fixture.scan_dirs();

    let running: HashSet<String> = ["com.example.gamma".to_owned()].into_iter().collect();
    let expected = serial_reference_scan(&scan_dirs, &running);

    // 走生产路径（并行 + 缓存），逐次强制冷缓存
    let mut parallel_runs: Vec<Vec<InstalledApp>> = Vec::new();
    for _ in 0..3 {
        invalidate_app_scan_cache();
        parallel_runs.push(scan_installed_apps_in_dirs(&scan_dirs, &running).apps);
    }

    assert_eq!(expected.len(), 3, "fixture 应产出 3 个 app");
    for (round, actual) in parallel_runs.iter().enumerate() {
        assert_rows_equal(actual, &expected, &format!("第 {round} 轮并行扫描"));
    }

    // 体积降序：Gamma(8192) > Alpha(4096) > Beta(1024)
    assert_eq!(
        ids(&parallel_runs[0]),
        vec![
            "com.example.gamma".to_owned(),
            "com.example.alpha".to_owned(),
            "com.example.beta".to_owned()
        ],
        "必须按体积降序"
    );

    // 多次并行运行之间也必须完全一致（rayon 无序 collect 不得泄漏到结果里）
    assert_rows_equal(&parallel_runs[1], &parallel_runs[0], "第 2 轮 vs 第 1 轮");
    assert_rows_equal(&parallel_runs[2], &parallel_runs[0], "第 3 轮 vs 第 1 轮");

    // 图标分支确实被走到，而不是两边都是 None 的空断言。
    //
    // MAS 形态下**按设计**就没有图标（`icns_to_base64_png` 在 mas feature 下
    // 直接返回 None，理由见那里的注释），所以这里反过来断言「全为 None」——
    // 既保住了这条断言的意义，又不会让 mas 形态的 CI 无理由地红。
    #[cfg(feature = "mas")]
    {
        assert!(
            expected.iter().all(|a| a.icon_base64.is_none()),
            "MAS 形态不产出图标（沙箱里 sips 不可用），不应有 app 带 icon"
        );
    }
    #[cfg(not(feature = "mas"))]
    assert!(
        expected.iter().any(|a| a.icon_base64.is_some()),
        "fixture 必须至少有一个 app 带真实图标，否则图标断言是空转"
    );
    assert!(
        expected.iter().any(|a| a.icon_base64.is_none()),
        "fixture 必须至少有一个 app 无图标，才能锁住 None 分支"
    );
}

/// 同体积的 app 必须按路径稳定排序（否则并行/串行无法逐项等价）
#[test]
fn equal_sized_apps_fall_back_to_path_order_so_parallelism_stays_deterministic() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("tie_breaker");
    // 名字与 bundle_id 等长、payload 等长 → 总体积逐字节相同
    fixture.write_bundle("TwinA", "com.example.twina", 2048, false);
    fixture.write_bundle("TwinB", "com.example.twinb", 2048, false);
    let scan_dirs = fixture.scan_dirs();
    let running = HashSet::new();

    let expected = serial_reference_scan(&scan_dirs, &running);
    assert_eq!(
        expected[0].bundle_size_bytes, expected[1].bundle_size_bytes,
        "fixture 前提：两个 app 体积必须相同，否则测不到 tiebreaker"
    );

    invalidate_app_scan_cache();
    let first = scan_installed_apps_in_dirs(&scan_dirs, &running).apps;
    for _ in 0..5 {
        invalidate_app_scan_cache();
        let again = scan_installed_apps_in_dirs(&scan_dirs, &running).apps;
        assert_eq!(
            ids(&again),
            ids(&expected),
            "同体积 app 的顺序必须稳定，不能随线程调度漂移"
        );
    }
    assert_eq!(ids(&first), ids(&expected));
}

/// 测试 2：连续两次调用，第二次命中进程内缓存
#[test]
fn second_scan_of_the_same_dirs_hits_the_in_process_cache() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("cache_hit");
    fixture.write_bundle("Alpha", "com.example.alpha", 2048, false);
    let scan_dirs = fixture.scan_dirs();
    let running = HashSet::new();

    invalidate_app_scan_cache();
    let (hits_before, misses_before) = app_scan_cache_stats();

    let first = scan_installed_apps_in_dirs(&scan_dirs, &running);
    assert!(!first.cache_hit, "冷缓存首次调用必然是 miss");
    let (hits_after_first, misses_after_first) = app_scan_cache_stats();
    assert_eq!(hits_after_first, hits_before, "miss 不应增加命中计数");
    assert_eq!(misses_after_first, misses_before + 1);

    let second = scan_installed_apps_in_dirs(&scan_dirs, &running);
    assert!(second.cache_hit, "第二次调用必须命中缓存");
    let (hits_after_second, misses_after_second) = app_scan_cache_stats();
    assert_eq!(hits_after_second, hits_before + 1, "命中计数应 +1");
    assert_eq!(
        misses_after_second, misses_after_first,
        "命中时不应再产生一次全量扫描"
    );

    assert_eq!(ids(&first.apps), ids(&second.apps));
    assert_rows_equal(&second.apps, &first.apps, "命中缓存 vs 冷扫描");
}

/// 测试 3（核心防线）：缓存只保存磁盘派生数据，is_running 每次现算
///
/// 若把 is_running 一起缓存，第二次调用会返回首次扫描时的陈旧运行态，此测试变红。
#[test]
fn cached_disk_facts_still_report_live_running_state() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("live_running");
    fixture.write_bundle("Demo", "com.example.demo", 4096, true);
    fixture.write_bundle("Idle", "com.example.idle", 1024, false);
    let scan_dirs = fixture.scan_dirs();

    invalidate_app_scan_cache();

    // 第一次：Demo 未运行
    let first = scan_installed_apps_in_dirs(&scan_dirs, &HashSet::new());
    assert!(!first.cache_hit);
    let demo_first = first
        .apps
        .iter()
        .find(|a| a.bundle_id == "com.example.demo")
        .expect("存在 Demo");
    assert!(!demo_first.is_running, "首次调用时 Demo 未运行");

    // 第二次：磁盘数据走缓存，但运行态集合变了 —— Demo 已启动
    let running_now: HashSet<String> = ["com.example.demo".to_owned()].into_iter().collect();
    let second = scan_installed_apps_in_dirs(&scan_dirs, &running_now);
    assert!(
        second.cache_hit,
        "本测试的前提是第二次确实命中了缓存（否则测不到缓存陈旧问题）"
    );
    let demo_second = second
        .apps
        .iter()
        .find(|a| a.bundle_id == "com.example.demo")
        .expect("存在 Demo");
    assert!(
        demo_second.is_running,
        "命中缓存也必须用本次现读的运行态，is_running 不能被缓存住"
    );

    // 第三次：反向——Demo 又退出了
    let third = scan_installed_apps_in_dirs(&scan_dirs, &HashSet::new());
    assert!(third.cache_hit);
    let demo_third = third
        .apps
        .iter()
        .find(|a| a.bundle_id == "com.example.demo")
        .expect("存在 Demo");
    assert!(
        !demo_third.is_running,
        "运行态必须双向跟随实时进程，不能只增不减"
    );

    // 反向检查：磁盘派生的字段在多次调用间保持稳定（证明缓存确实被用上了）
    assert_eq!(demo_second.bundle_size_bytes, demo_first.bundle_size_bytes);
    assert_eq!(demo_second.icon_base64, demo_first.icon_base64);
}

/// 测试 4：TTL 到期后缓存失效（可控时钟，不真的等 10 分钟）
#[test]
fn app_scan_cache_expires_after_its_ttl() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("ttl");
    fixture.write_bundle("Alpha", "com.example.alpha", 2048, false);
    let scan_dirs = fixture.scan_dirs();

    invalidate_app_scan_cache();
    scan_installed_apps_in_dirs(&scan_dirs, &HashSet::new());

    // 未过期：仍可命中
    assert!(
        cached_apps(&scan_dirs, Instant::now()).is_some(),
        "TTL 内应命中缓存"
    );
    // 刚好卡在 TTL 边界之前
    assert!(
        cached_apps(
            &scan_dirs,
            Instant::now() + APP_SCAN_CACHE_TTL - Duration::from_secs(1)
        )
        .is_some(),
        "TTL 边界前 1 秒仍应命中"
    );
    // 过期：必须失效
    assert!(
        cached_apps(
            &scan_dirs,
            Instant::now() + APP_SCAN_CACHE_TTL + Duration::from_secs(1)
        )
        .is_none(),
        "超过 TTL 必须失效并重扫"
    );

    // 反向验证：上面的过期判定不是因为缓存本来就是空的
    assert!(
        cached_apps(&scan_dirs, Instant::now()).is_some(),
        "缓存确实已填充，TTL 判定才有意义"
    );
    assert_eq!(
        APP_SCAN_CACHE_TTL,
        Duration::from_secs(600),
        "TTL 为 10 分钟"
    );
}

/// 缓存键必须包含扫描目录列表：目录变了就得重扫
#[test]
fn app_scan_cache_is_keyed_on_the_scan_directory_list() {
    let _guard = cache_guard();
    let fixture_a = ScanFixture::new("key_a");
    let fixture_b = ScanFixture::new("key_b");
    fixture_a.write_bundle("Alpha", "com.example.alpha", 2048, false);
    fixture_b.write_bundle("Beta", "com.example.beta", 2048, false);

    let dirs_a = fixture_a.scan_dirs();
    let dirs_b = fixture_b.scan_dirs();
    assert_ne!(scan_cache_key(&dirs_a), scan_cache_key(&dirs_b));

    invalidate_app_scan_cache();
    let from_a = scan_installed_apps_in_dirs(&dirs_a, &HashSet::new());
    assert!(!from_a.cache_hit);
    assert_eq!(ids(&from_a.apps), vec!["com.example.alpha".to_owned()]);

    // 同目录 → 命中
    let from_a_again = scan_installed_apps_in_dirs(&dirs_a, &HashSet::new());
    assert!(from_a_again.cache_hit);
    assert_eq!(
        ids(&from_a_again.apps),
        vec!["com.example.alpha".to_owned()]
    );

    // 换目录 → 必须 miss，且不能返回上一个目录的数据
    let from_b = scan_installed_apps_in_dirs(&dirs_b, &HashSet::new());
    assert!(!from_b.cache_hit, "扫描目录不同，缓存键必须不同");
    assert_eq!(
        ids(&from_b.apps),
        vec!["com.example.beta".to_owned()],
        "不得返回上一个扫描目录的结果"
    );

    // 目录列表顺序不同 → 也算不同的键
    let multi_a = vec![fixture_a.root.clone(), fixture_b.root.clone()];
    let multi_b = vec![fixture_b.root.clone(), fixture_a.root.clone()];
    assert_ne!(scan_cache_key(&multi_a), scan_cache_key(&multi_b));
}

/// 强制失效入口真的清空缓存
#[test]
fn invalidating_the_app_scan_cache_forces_a_rescan() {
    let _guard = cache_guard();
    let fixture = ScanFixture::new("invalidate");
    fixture.write_bundle("Alpha", "com.example.alpha", 2048, false);
    let scan_dirs = fixture.scan_dirs();
    let running = HashSet::new();

    scan_installed_apps_in_dirs(&scan_dirs, &running);
    assert!(scan_installed_apps_in_dirs(&scan_dirs, &running).cache_hit);

    invalidate_app_scan_cache();
    assert!(
        !scan_installed_apps_in_dirs(&scan_dirs, &running).cache_hit,
        "invalidate 之后必须重新全量扫描"
    );
}

/// 图标：多个 app 共用同一图标文件名时，并行解码不能互相覆盖
///
/// 本机 /Applications 有 15 个 app 的 CFBundleIconFile 都叫 AppIcon.icns，
/// 若临时文件名不带唯一后缀，并行的 sips 会写坏彼此的输出。
#[test]
fn parallel_icon_decoding_survives_apps_sharing_one_icon_file_name() {
    // 这条测的是「多个 app 共用同一个图标文件名时，并行的 sips 不会互相覆盖」。
    // MAS 形态根本没有 sips（`icns_to_base64_png` 直接返回 None），这条测的
    // 对象不存在，跳过而不是让它必然失败。
    if cfg!(feature = "mas") {
        eprintln!("跳过：MAS 形态不调用 sips，无并行解码可测");
        return;
    }
    let _guard = cache_guard();
    let fixture = ScanFixture::new("icon_collision");
    for name in ["One", "Two", "Three", "Four", "Five", "Six"] {
        fixture.write_bundle(name, &format!("com.example.{name}"), 1024, true);
    }
    let scan_dirs = fixture.scan_dirs();
    let running = HashSet::new();

    for round in 0..3 {
        invalidate_app_scan_cache();
        let scanned = scan_installed_apps_in_dirs(&scan_dirs, &running);
        assert_eq!(scanned.apps.len(), 6, "第 {round} 轮：6 个 app 都要扫到");
        for app in &scanned.apps {
            assert!(
                app.icon_base64.is_some(),
                "第 {round} 轮：{} 的图标解码失败（并行下临时文件互相覆盖）",
                app.bundle_path
            );
        }
        let first_icon = scanned.apps[0].icon_base64.as_ref().expect("有图标");
        for app in &scanned.apps {
            assert_eq!(
                app.icon_base64.as_ref(),
                Some(first_icon),
                "同一份 icns 内容必须解出同一张图，拿到别的 app 的输出即为串味"
            );
        }
    }
}

/// 独立验证「图标临时文件用完即删」。
///
/// 这条**故意不放在** `parallel_icon_decoding_survives_apps_sharing_one_icon_file_name`
/// 末尾。原来的写法扫的是**全局** temp 目录，于是「这台机器上有没有别的 macslim
/// 跑过」也会算成失败 —— 实测开发期间反复 quit / pkill 掉 MacSlim 留下了 14 个
/// `macslim_icon_*` 文件，之后每次跑测试都红，而 `cargo test` 单跑又能过。
/// 那是环境脏，不是被测代码泄漏。
///
/// 拆出来之后用**调用方指定的目录**断言（`icns_to_base64_png` 的第二个参数就是
/// 为此加的），归属完全确定，跨进程、跨测试都不串味。
#[test]
fn icon_temp_file_is_removed_after_conversion() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "macslim_icon_leak_probe_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&scratch).expect("能建探针目录");

    // 一个最小可解的 icns：单张 32x32 纯色。写不出来就跳过，别让这条测试
    // 依赖 macOS 的 `sips` 之外的东西。
    let icns = scratch.join("probe.icns");
    match write_minimal_icns(&icns) {
        Ok(()) => {}
        Err(reason) => {
            let _ = std::fs::remove_dir_all(&scratch);
            eprintln!("跳过：造不出最小 icns（{reason}）");
            return;
        }
    }

    let result = icns_to_base64_png(&icns, &scratch);
    let leftovers: Vec<String> = std::fs::read_dir(&scratch)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("macslim_icon_"))
        .collect();
    let _ = std::fs::remove_dir_all(&scratch);

    // 转换本身可能因 sips 不可用而返回 None；那条分支下同样不能留垃圾。
    // 所以这里只断言「不留残留」，不断言一定解出图。
    if leftovers.is_empty() {
        return;
    }
    panic!(
        "图标转换后临时文件应被清理，残留：{leftovers:?}（转换结果 = {}）",
        result.is_some()
    );
}

/// 写一个最小的合法 icns：32x32、单张、真彩色。
fn write_minimal_icns(path: &Path) -> Result<(), String> {
    use std::io::Write;
    const SIZE: u32 = 32;
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"icns");
    bytes.extend_from_slice(&8u32.to_be_bytes()); // 总长，本函数会回填
    let table_len_pos = bytes.len() - 4;
    // type = ic07（32-bit ARGB + PNG）
    bytes.extend_from_slice(&32u32.to_be_bytes()); // 数据块长度（占位，下面回填）
    let data_len_pos = bytes.len() - 4;
    bytes.extend_from_slice(b"ic07");
    // ic07 结构：宽高(2B 零) + 平台(1) + 深度(1) + 颜色类型(1) + 零(3) + 长度(4)
    let mut header = [0u8; 8];
    header[0] = 0; // 宽高合并 = 32
    header[1] = 32;
    header[2] = 0; // platform
    header[3] = 0; // depth
    header[4] = 0; // color type
    header[5..8].copy_from_slice(&0x000000u32.to_be_bytes()[1..4]);
    bytes.extend_from_slice(&header);
    // ARGB 像素，全不透明
    for _ in 0..(SIZE * SIZE) {
        bytes.extend_from_slice(&[0x00, 0x33, 0x66, 0xCC]);
    }
    bytes.extend_from_slice(&0u32.to_be_bytes()); // is32bitLargest
    bytes.extend_from_slice(&0u32.to_be_bytes()); // is32bitSmallest
    let data_len = (bytes.len() - data_len_pos - 4) as u32;
    bytes[data_len_pos..data_len_pos + 4].copy_from_slice(&data_len.to_be_bytes());
    let total = bytes.len() as u32;
    bytes[table_len_pos..table_len_pos + 4].copy_from_slice(&total.to_be_bytes());
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())
}
