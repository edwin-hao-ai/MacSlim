use super::*;
use crate::operations::OperationStore;

/// 构造一个 `ProcessRow` 测试样本，`name`（展示名）与 `full_name`（原始名）分开给。
fn sample_process_row(display: &str, raw: &str) -> ProcessRow {
    ProcessRow {
        pid: 4242,
        parent_pid: Some(1),
        name: display.to_owned(),
        full_name: raw.to_owned(),
        exe: format!("/usr/local/bin/{raw}"),
        start_time: 1_000,
        cpu_percent: 0.5,
        memory_mb: 12.0,
        uptime_secs: 3_600,
        name_key: None,
        status_key: "process.status.sleep".to_owned(),
        ports: Vec::new(),
        icon_base64: None,
        protected: false,
        protected_reason_key: None,
        protected_reason_params: Vec::new(),
        whitelisted: false,
        selection_key: String::new(),
    }
}

/// 把「key + 插值参数」拼成可读字符串，供文本级护栏（复述检测）使用。
///
/// 改造后 reason 本身已经是一枚 key，用户看到的句子由前端拼。文案级的不变式
/// （不复述 CPU / 内存列的数值、不复述进程名）随文案一起搬到了前端词典门禁；
/// 但这两条护栏本身仍然有效：key 或参数里**夹带**「108.7% CPU」「内存 1894MB」
/// 这类内容一样会被它们抓住。
fn reason_debug(key: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return key.to_owned();
    }
    let rendered: Vec<String> = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{key}({})", rendered.join(", "))
}

#[test]
fn idle_memory_threshold_scales_with_total_memory() {
    assert_eq!(idle_memory_threshold(512.0), 80.0);
    assert_eq!(idle_memory_threshold(8_000.0), 80.0);
    assert_eq!(idle_memory_threshold(32_000.0), 320.0);
}

/// 对真实系统扫描结果的关键不变式检查 —— 在 CI 上跑，自证不误杀。
#[test]
fn real_scan_has_no_default_selected_non_zombie() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);

    for p in &result.processes {
        if p.default_select {
            assert_eq!(
                p.kind,
                ProcessKind::Zombie,
                "PID {} ({}) 被默认选中但不是僵尸进程！这会误杀用户进程！",
                p.pid,
                p.name
            );
        }
    }
}

#[test]
fn chrome_helpers_never_default_selected() {
    // 注：多进程族进程现在允许显示（作为 Low 风险，方便用户看到内存占用）
    // 但绝不允许默认选中。
    let mut sys = System::new_all();
    let result = scan(&mut sys);

    for p in &result.processes {
        let is_mp = p.name.contains("Chrome Helper")
            || p.name.contains("Slack Helper")
            || p.name.starts_with("Code Helper")
            || p.name.contains("Electron Helper");
        if is_mp {
            assert!(!p.default_select, "多进程族 Helper {} 不应默认选中", p.name);
            assert_ne!(
                p.risk,
                Risk::Safe,
                "多进程族 Helper {} 不应标记为 Safe",
                p.name
            );
        }
    }
}

#[test]
fn real_scan_has_no_listening_port_processes_default_selected() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);
    for p in &result.processes {
        if !p.ports.is_empty() {
            assert!(
                !p.default_select,
                "PID {} 监听端口 {:?} 但被默认选中！",
                p.pid, p.ports
            );
        }
    }
}

/// 铁律不变式：`default_select => !protected && !whitelisted`。
///
/// 这条不变式必须在**真实系统扫描**上成立：任何「先给 default_select=true、
/// 之后才算出 protected」的分支（例如僵尸分支）都会在这里暴露。
#[test]
fn real_scan_never_default_selects_a_protected_or_whitelisted_process() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);

    for p in &result.processes {
        if p.default_select {
            assert!(
                !p.protected,
                "PID {} ({}) 既受保护又被默认选中 —— 主 CTA 会整批失败：{}",
                p.pid, p.name, p.reason_key
            );
            assert!(
                !p.whitelisted,
                "PID {} ({}) 命中白名单却被默认选中",
                p.pid, p.name
            );
        }
    }
}

/// 纯函数不变式：不依赖真实进程，直接钉住「受保护/白名单 → 不可默认选中」。
#[test]
fn default_selection_rule_rejects_protected_and_whitelisted() {
    use crate::process_safety::ProcessProtection;

    let plain = ProcessProtection {
        protected: false,
        whitelisted: false,
        reason: None,
    };
    let guarded = ProcessProtection {
        protected: true,
        whitelisted: false,
        reason: Some(I18nText::plain("process.protect.parentOfOthers")),
    };
    let listed = ProcessProtection {
        protected: true,
        whitelisted: true,
        reason: Some(I18nText::plain("process.protect.whitelisted")),
    };

    assert!(default_selectable(&plain));
    assert!(!default_selectable(&guarded));
    assert!(!default_selectable(&listed));
}

/// 僵尸分支的确定性回归：veto 一旦命中就绝不能 default_select。
///
/// 真实系统上僵尸进程可能一个都没有，所以这条不变式必须用纯函数钉住，
/// 否则「僵尸分支先 return 再算保护」这个 bug 会等到用户机器上才暴露。
#[test]
fn zombie_classification_only_default_selects_after_the_safety_audit() {
    let clean = zombie_classification(None);
    assert!(clean.default_select);
    assert_eq!(clean.kind, ProcessKind::Zombie);
    assert_eq!(clean.risk, Risk::Safe);

    assert_eq!(clean.reason.key, "process.reason.zombie");
    assert!(clean.reason.params.is_empty());

    for (veto, expected_key) in [
        (
            SafetyVeto::ParentOfOthers,
            "process.reason.zombieVetoedParentOfOthers",
        ),
        (
            SafetyVeto::MultiProcessComponent,
            "process.reason.zombieVetoedMultiProcessComponent",
        ),
        (
            SafetyVeto::YoungProcess,
            "process.reason.zombieVetoedYoungProcess",
        ),
    ] {
        let guarded = zombie_classification(Some(veto));
        assert!(!guarded.default_select, "veto={veto:?} 仍被默认选中");
        assert_eq!(guarded.kind, ProcessKind::Zombie);
        assert_ne!(guarded.risk, Risk::Safe, "veto={veto:?} 仍被标为 Safe");
        // 每种否决种类一条独立 key：句子里的「原因」不是运行时拼的，
        // 所以前端词典门禁能逐条核对，不需要任何嵌套插值约定。
        assert_eq!(guarded.reason.key, expected_key);
        assert!(
            guarded.reason.params.is_empty(),
            "僵尸 reason 不该带插值参数：{:?}",
            guarded.reason.params
        );
    }
}

/// `classify_processes` 的保护闸门必须真的把 default_select 翻掉。
#[test]
fn protection_gate_clears_default_selection() {
    use crate::process_safety::ProcessProtection;

    let mut classification = Classification {
        kind: ProcessKind::Zombie,
        risk: Risk::Safe,
        default_select: true,
        reason: I18nText::plain("process.reason.zombie"),
    };

    apply_protection_gate(
        &mut classification,
        &ProcessProtection {
            protected: true,
            whitelisted: false,
            reason: Some(I18nText::plain("process.protect.youngProcess")),
        },
    );

    assert!(!classification.default_select);
    // 「受保护（原因）」那一段改由前端按 protected + protected_reason_key 拼装。
    // 闸门因此**只**碰 default_select —— 这正是本条要钉住的新不变式。
    assert_eq!(classification.reason.key, "process.reason.zombie");

    let mut untouched = Classification {
        kind: ProcessKind::Zombie,
        risk: Risk::Safe,
        default_select: true,
        reason: I18nText::plain("process.reason.zombie"),
    };
    apply_protection_gate(
        &mut untouched,
        &ProcessProtection {
            protected: false,
            whitelisted: false,
            reason: None,
        },
    );
    assert!(untouched.default_select);
    assert_eq!(untouched.reason.key, "process.reason.zombie");
}

/// reason 里出现「数字 + %」或「数字 + MB」即视为复述了列里已有的 CPU / 内存数值。
/// 扫描页（`ProcessList.tsx`）在行尾单独渲染 `{cpu_percent.toFixed(1)}% CPU` 与
/// `{Math.round(memory_mb)}MB`，reason 再写一遍就是同一屏出现两遍同一个数。
fn leaks_column_number(reason: &str) -> bool {
    let chars: Vec<char> = reason.chars().collect();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        // 数字串（允许小数点），然后允许空格
        let mut i = start;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        let is_pct = i < chars.len() && chars[i] == '%';
        let is_mb = i + 1 < chars.len() && chars[i] == 'M' && chars[i + 1] == 'B';
        if is_pct || is_mb {
            return true;
        }
    }
    false
}

/// reason 里出现该行自己的进程名，即判定为复述。
/// 扫描页把 `{p.name}` 渲染在行首、`{p.reason}` 渲染在同一块的下一行（`ProcessList.tsx:96` / `:109`），
/// 名字进 reason 就是同一屏出现两遍。
///
/// 名字短于 3 个字符时不做判定：`e` / `1` 这类名字在任何文案里都可能偶然出现，
/// 断言会退化成随机误报。短名由下面的 `classify_one` 定向护栏覆盖，不靠这条兜。
fn leaks_process_name(reason: &str, name: &str) -> bool {
    name.chars().count() >= 3 && reason.contains(name)
}

/// 挑一个能稳定走进 `classify_one` 开发工具分支的**真实**进程作为探针：
/// 非僵尸、已运行超过 10 分钟（避开 `safety_veto` 的「进程刚启动」否决）、
/// 内存 < 200MB（避开 `DevToolHighMemory` 分支）。取内存最小的一个以保证在任何
/// 机器上都能挑到。
///
/// `classify_one` 的 `name` 是**入参**、与 `proc` 自身无关，所以可以给任意真实进程
/// 喂 `"node"` 来确定性地触发开发工具分支 —— 不依赖本机是否真在跑 node。
fn dev_branch_probe(sys: &System) -> Option<&Process> {
    sys.processes()
        .values()
        .filter(|p| {
            p.status() != ProcessStatus::Zombie
                && p.memory() < 200 * 1024 * 1024
                && crate::process_safety::process_uptime_secs(p) > 600
        })
        .min_by_key(|p| p.memory())
}

/// 驱动**真实** `classify_one`，断言开发工具分支产出的 reason 不复述进程名。
///
/// 这条取代了原先那个同义反复版本：原版只对测试自己 `format!` 出来的字面量做断言，
/// 与 `scanner.rs` 没有任何调用关系，把 `classify_one` 改回 `format!("{} 开发工具进程…", name)`
/// 它照样绿 —— 正是那个空洞让进程名重复活到了今天。
#[test]
fn dev_tool_reason_does_not_repeat_the_process_name() {
    let sys = System::new_all();
    let probe =
        dev_branch_probe(&sys).expect("找不到探针进程（非僵尸 + 已运行 > 10 分钟 + 内存 < 200MB）");
    // 名字是入参：任意真实进程 + "node" 即可确定性地走进开发工具分支
    let name = "node";
    let no_parents = std::collections::HashSet::new();
    let c = classify_one(probe, name, &no_parents, 64_000.0);

    // 先证明探针真的进了开发工具分支，否则下面的断言会空转放行
    assert_eq!(
        c.kind,
        ProcessKind::Dev,
        "探针没走进开发工具分支，reason={:?} —— 本条护栏会空转，需修探针条件",
        c.reason
    );
    assert!(!c.default_select, "开发工具进程不得默认选中");
    let shown = reason_debug(&c.reason.key, &c.reason.params);
    assert!(
        !leaks_process_name(&shown, name),
        "reason 复述了进程名 {name}：{shown}"
    );
    assert!(
        !leaks_column_number(&shown),
        "reason 复述了 CPU / 内存数值：{shown}"
    );
    assert_eq!(c.reason.key, "process.reason.devTool");
}

/// `classify_one` 走**安全审计否决**分支（父进程 / 多进程族）时，
/// reason 同样不得复述进程名。这一条顺带锁住 `resource_reason(VetoedHighUsage)`。
#[test]
fn vetoed_reason_does_not_repeat_the_process_name() {
    let sys = System::new_all();
    let Some(probe) = dev_branch_probe(&sys) else {
        panic!("找不到探针进程");
    };
    let name = "node";
    // 把探针自己的 pid 塞进父进程集合 → `safety_veto` 命中「是其他进程的父进程」
    let parents = std::collections::HashSet::from([probe.pid().as_u32()]);
    let c = classify_one(probe, name, &parents, 8_000.0);

    // 这条只守住「veto 分支的 reason 不复述进程名、且必须是一枚合法 key」。
    // 它**不**钉住具体分支：`VetoedHighUsage` 要求探针内存 ≥ 100MB，而探针条件
    // 恰好要求 < 200MB 且取内存最小者，所以本机多数时候落进 Hidden。
    // 换句话说改造前这条断言对 Hidden 分支恒真 —— 它从来没有真的护住过
    // Hog 分支。Hog 分支的确定性护栏在
    // `resource_reasons_never_carry_cpu_or_memory_values`（穷举两个变体）。
    let shown = reason_debug(&c.reason.key, &c.reason.params);
    assert!(
        !leaks_process_name(&shown, "node"),
        "veto 分支的 reason 复述了进程名：{shown}"
    );
    if c.risk != Risk::Hidden {
        assert!(
            c.reason.key.starts_with("process.reason."),
            "非隐藏行的 reason 必须是 process.reason.* 的 key：{shown}"
        );
    }
}

/// 确定性护栏：veto 且占用达门槛时，两种进程身份必须各自选到自己那枚 key。
///
/// 这条不依赖任何真实进程状态，因此「veto → Hog」这条路径在任何机器上都被覆盖。
#[test]
fn vetoed_high_usage_picks_a_distinct_key_per_process_identity() {
    for variant in [
        VetoedHighUsageVariant::MultiProcessComponent,
        VetoedHighUsageVariant::MainProcess,
    ] {
        let reason = resource_reason(ResourceNote::VetoedHighUsage { variant });
        assert!(reason.key.starts_with("process.reason."));
        assert!(reason.params.is_empty());
    }
    assert_ne!(
        resource_reason(ResourceNote::VetoedHighUsage {
            variant: VetoedHighUsageVariant::MultiProcessComponent
        })
        .key,
        resource_reason(ResourceNote::VetoedHighUsage {
            variant: VetoedHighUsageVariant::MainProcess
        })
        .key,
    );
}

/// **确定性**护栏：穷举 `resource_reason` 的全部变体，断言一个都不带列里已有的数值。
///
/// 这条是主力护栏。上一轮只靠 live `scan()` 的护栏，变异验证时被证伪过一次：
/// 内存 hog 分支改回 `内存占用 {mem}MB` 后 live 护栏依然绿（测试机上那一刻没有
/// CPU ≤ 20% 且内存达 hog 门槛的进程，分支根本没被走到 → 空转放行）。
/// 纯函数版本与机器负载无关，回退必红。
#[test]
fn resource_reasons_never_carry_cpu_or_memory_values() {
    let notes = [
        ResourceNote::HighCpu,
        ResourceNote::HighMemory,
        ResourceNote::Idle { uptime_min: 0 },
        ResourceNote::Idle { uptime_min: 378 },
        ResourceNote::Idle {
            uptime_min: 100_000,
        },
        ResourceNote::DevToolHighMemory,
        ResourceNote::VetoedHighUsage {
            variant: VetoedHighUsageVariant::MultiProcessComponent,
        },
        ResourceNote::VetoedHighUsage {
            variant: VetoedHighUsageVariant::MainProcess,
        },
    ];

    for note in notes {
        let reason = resource_reason(note);
        let shown = reason_debug(&reason.key, &reason.params);
        assert!(
            !leaks_column_number(&shown),
            "{note:?} 的 reason 复述了 CPU / 内存列里的数值：{shown}"
        );
    }

    // key 本身也钉住，防止有人把语义改得含混不清或塞进夹带文案的 key
    assert_eq!(
        resource_reason(ResourceNote::HighCpu).key,
        "process.reason.highCpu"
    );
    assert_eq!(
        resource_reason(ResourceNote::HighMemory).key,
        "process.reason.highMemory"
    );
    let idle = resource_reason(ResourceNote::Idle { uptime_min: 378 });
    assert_eq!(idle.key, "process.reason.idle");
    // 运行时长走插值参数，不进 key
    assert_eq!(
        idle.params,
        vec![("uptime_min".to_owned(), "378".to_owned())]
    );
    assert_eq!(
        resource_reason(ResourceNote::DevToolHighMemory).key,
        "process.reason.devToolHighMemory"
    );
    assert_eq!(
        resource_reason(ResourceNote::VetoedHighUsage {
            variant: VetoedHighUsageVariant::MainProcess
        })
        .key,
        "process.reason.vetoedHighUsageMainProcess"
    );
}

/// 真实扫描结果的 reason 不得复述「CPU 百分比」与「内存 MB」这两个列里已有的数值。
/// 这是第二层护栏：覆盖 `resource_reason` 之外的 reason 拼装点
/// （`apply_protection_gate` 的「受保护」后缀、端口追加）以及任何将来新增的分支。
#[test]
fn scanned_reasons_never_restate_cpu_or_memory_values() {
    // 先自证探测器本身有效：这些正是改造前的拼装形态，必须被判为泄漏
    assert!(leaks_column_number("CPU 占用 22.6%"));
    assert!(leaks_column_number("内存占用 1894MB"));
    assert!(leaks_column_number("已运行 378 分钟 · 98MB 无活动"));
    assert!(leaks_column_number(
        "OpenCode Helper · 487MB · 71.2% CPU（多进程应用组件，仅供参考，清理会导致应用崩溃）"
    ));
    // 不含数值的语义文案不得被误判
    assert!(!leaks_column_number("CPU 占用偏高"));
    assert!(!leaks_column_number("内存占用偏高"));
    assert!(!leaks_column_number("开发工具进程，内存占用较高"));
    assert!(!leaks_column_number(
        "应用主进程，仅供参考，清理会导致应用崩溃 · 受保护（是其他进程的父进程（某应用的主进程））"
    ));
    // 端口号不在列里，属于新增信息，不算泄漏
    assert!(!leaks_column_number(
        "应用主进程，仅供参考，清理会导致应用崩溃 · 端口 3000/8080（运行中的服务，请确认）"
    ));

    let mut sys = System::new_all();
    let result = scan(&mut sys);

    for p in &result.processes {
        let shown = reason_debug(&p.reason_key, &p.reason_params);
        assert!(
            !leaks_column_number(&shown),
            "PID {} ({}) 的 reason 复述了 CPU / 内存列里的数值：{shown}",
            p.pid,
            p.name
        );
        assert!(
            !leaks_process_name(&shown, &p.name),
            "PID {} ({}) 的 reason 复述了进程名：{shown}",
            p.pid,
            p.name
        );
    }
}

/// 进程名探测器自身的有效性自证。探测器写错就会静默恒假、把回归全放行。
#[test]
fn process_name_detector_flags_the_legacy_dev_reason_only() {
    // 改造前的拼装形态，必须被判为泄漏
    assert!(leaks_process_name(
        "node 开发工具进程（建议手动确认）",
        "node"
    ));
    assert!(leaks_process_name(
        "opencode · 330MB · 108.7% CPU（应用主进程，仅供参考，清理会导致应用崩溃）",
        "opencode"
    ));
    // 改造后的语义文案，必须不泄漏
    assert!(!leaks_process_name("开发工具进程（建议手动确认）", "node"));
    // 「应用主进程」/「多进程应用组件」是类别词、不含具体进程名，不得误判
    assert!(!leaks_process_name(
        "应用主进程，仅供参考，清理会导致应用崩溃 · 受保护（是其他进程的父进程（某应用的主进程））",
        "OpenCode Helper"
    ));
    assert!(!leaks_process_name(
        "多进程应用组件，仅供参考，清理会导致应用崩溃 · 受保护（已知多进程架构应用的组件，属正常设计）",
        "OpenCode Helper (Renderer)"
    ));
    assert!(!leaks_process_name("CPU 占用偏高", "Otty"));
    assert!(!leaks_process_name("内存占用偏高", "Otty"));
    assert!(!leaks_process_name(
        "已运行 378 分钟 · 长时间无活动",
        "七七 Helper (Renderer)"
    ));
    // 短名不做判定（否则退化成随机误报）
    assert!(!leaks_process_name("已运行 378 分钟 · 长时间无活动", "e"));
    assert!(!leaks_process_name("CPU 占用偏高", "1"));
}

#[test]
fn real_scan_results_are_reasonable() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);
    // 结果上限 40
    assert!(result.processes.len() <= 40);
    // 每个进程都有 name
    for p in &result.processes {
        assert!(!p.name.is_empty());
        assert!(p.pid > 0);
    }
}

#[test]
fn process_rows_expose_backend_start_time_and_empty_selection_key() {
    let mut sys = System::new_all();
    let rows = list_all(&mut sys);
    assert!(!rows.is_empty());

    for row in &rows {
        assert!(row.start_time > 0, "PID {} 缺少后端 start_time", row.pid);
        assert!(row.selection_key.is_empty());
        if row.whitelisted {
            assert!(row.protected);
        }
    }
}

#[test]
fn process_targets_from_rows_keep_the_scanned_identity() {
    let mut sys = System::new_all();
    let rows = list_all(&mut sys);
    let targets = process_targets_from_rows(&rows);

    assert_eq!(targets.len(), rows.len());
    for (row, target) in rows.iter().zip(targets.iter()) {
        assert_eq!(target.identity.pid, row.pid);
        // 执行身份跟的是原始名，不是展示名
        assert_eq!(target.identity.name, row.full_name);
        assert_eq!(target.identity.exe, row.exe);
        assert_eq!(target.identity.start_time, row.start_time);
        assert_eq!(target.protected, row.protected);
        assert_eq!(target.whitelisted, row.whitelisted);
    }
}

#[test]
fn register_process_rows_binds_one_opaque_key_per_row() {
    let mut sys = System::new_all();
    let mut rows = list_all(&mut sys);
    let expected = process_targets_from_rows(&rows);
    let mut store = OperationStore::new();

    let registration = register_process_rows(&mut store, &mut rows).unwrap();

    assert_eq!(registration.selection_keys.len(), rows.len());
    let keys: std::collections::HashSet<&str> =
        rows.iter().map(|row| row.selection_key.as_str()).collect();
    assert_eq!(keys.len(), rows.len());
    assert!(rows.iter().all(|row| row.selection_key.len() == 64
        && row
            .selection_key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))));

    let selectable: Vec<&str> = rows
        .iter()
        .filter(|row| !row.whitelisted)
        .map(|row| row.selection_key.as_str())
        .collect();
    let mut expected_scanned: Vec<crate::operations::ProcessTarget> = expected
        .into_iter()
        .filter(|target| !target.whitelisted)
        .collect();
    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            selectable,
            crate::operations::ProcessMode::Force,
            "main",
        )
        .unwrap();
    let crate::operations::OperationPlan::Process { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected process plan")
    };
    assert!(!targets.iter().any(|target| target.whitelisted));
    let mut planned = targets.clone();
    planned.sort_by_key(|target| target.identity.pid);
    expected_scanned.sort_by_key(|target| target.identity.pid);
    assert_eq!(planned, expected_scanned);
}

#[test]
fn register_process_rows_rejects_whitelisted_rows_at_prepare_time() {
    let mut sys = System::new_all();
    let mut rows = list_all(&mut sys);
    let mut store = OperationStore::new();
    let registration = register_process_rows(&mut store, &mut rows).unwrap();
    let whitelisted: Vec<&str> = rows
        .iter()
        .filter(|row| row.whitelisted)
        .map(|row| row.selection_key.as_str())
        .collect();
    if whitelisted.is_empty() {
        return;
    }

    let error = store
        .prepare_process(
            &registration.snapshot_id,
            whitelisted,
            crate::operations::ProcessMode::Force,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "白名单进程不能终止");
}

#[test]
fn scan_process_targets_match_the_same_sysinfo_sample() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);
    let targets = scan_targets_from_infos(&result.processes);

    assert_eq!(targets.len(), result.processes.len());
    for (info, target) in result.processes.iter().zip(targets.iter()) {
        assert_eq!(target.identity.pid, info.pid);
        assert_eq!(target.identity.name, info.name);
        assert_eq!(target.identity.exe, info.exe);
        assert_eq!(target.identity.start_time, info.start_time);
        assert_eq!(target.protected, info.protected);
        assert!(!info.whitelisted, "扫描结果不应包含白名单进程");
    }
}

#[test]
fn bundle_display_name_wins_over_raw_process_name() {
    assert_eq!(
        display_name_for_process("Helper", Some("Google Chrome".into())),
        ("Google Chrome".to_owned(), None),
    );
}

#[test]
fn bundle_id_shaped_names_use_the_fixed_mapping() {
    // key 与原始进程名都要对：key 供前端翻译，原始名保证「漏翻时看到的是
    // com.apple.WebKit.WebContent 这种不好看但无害的东西」而不是裸 key。
    assert_eq!(
        display_name_for_process("com.apple.WebKit.WebContent", None),
        (
            "com.apple.WebKit.WebContent".to_owned(),
            Some("process.name.webkitWebContent".to_owned())
        ),
    );
    assert_eq!(
        display_name_for_process("com.apple.WebKit.Networking", None)
            .1
            .as_deref(),
        Some("process.name.webkitNetworking")
    );
    assert_eq!(
        display_name_for_process("com.apple.WebKit.GPU", None)
            .1
            .as_deref(),
        Some("process.name.webkitGpu")
    );
}

#[test]
fn unmapped_bundle_id_shaped_names_fall_back_to_the_last_segment() {
    assert_eq!(
        display_name_for_process("com.vendor.unknown.Agent", None),
        ("Agent".to_owned(), None),
    );
}

#[test]
fn plain_names_are_returned_unchanged() {
    assert_eq!(
        display_name_for_process("esbuild", None),
        ("esbuild".to_owned(), None)
    );
    assert_eq!(
        display_name_for_process("Doubaolme", None),
        ("Doubaolme".to_owned(), None)
    );
}

#[test]
fn empty_bundle_id_is_returned_verbatim() {
    assert_eq!(
        display_name_for_process("com.", None),
        ("com.".to_owned(), None)
    );
}

/// 子进程必须保住角色后缀。
///
/// 直接用 bundle 显示名会把主进程和所有 helper 压成同一个名字
/// （"Google Chrome" × 6），用户无法分辨谁是谁 —— 比修复前更难读。
#[test]
fn bundle_child_processes_keep_their_role_suffix() {
    assert_eq!(
        display_name_for_process(
            "Google Chrome Helper (Renderer)",
            Some("Google Chrome".into()),
        ),
        ("Google Chrome Helper (Renderer)".to_owned(), None),
    );
}

/// 反过来：原始名**不比** bundle 名长（或不是它的扩展）时，仍然用 bundle 名。
/// `Helper` 是最常见的形态（Safari / WebKit 的 XPC 服务），必须被归到宿主应用名下。
#[test]
fn non_descendant_process_names_still_fall_back_to_the_bundle_name() {
    assert_eq!(
        display_name_for_process("Helper", Some("Safari".into())),
        ("Safari".to_owned(), None),
    );
    assert_eq!(
        display_name_for_process("Google Chrome", Some("Google Chrome".into())),
        ("Google Chrome".to_owned(), None),
    );
}

/// 空白 bundle 名必须 fallthrough，不能被当成有效显示名返回。
///
/// 这条路径**实际可达**：`extract_plist_string`（`applications.rs:369-382`）只对
/// `<string></string>` 返回 `None`，`<string>   </string>` 会原样返回空白。
#[test]
fn blank_bundle_display_name_falls_through_to_the_later_branches() {
    // bundle-id 形态：落穿后应命中固定映射，而不是返回空白
    assert_eq!(
        display_name_for_process("com.apple.WebKit.WebContent", Some("   ".into()))
            .1
            .as_deref(),
        Some("process.name.webkitWebContent")
    );
    // 普通名字：落穿后应原样返回，而不是返回 "   "
    assert_eq!(
        display_name_for_process("Helper", Some("   ".into())),
        ("Helper".to_owned(), None)
    );
    // 空串同样落穿
    assert_eq!(
        display_name_for_process("esbuild", Some(String::new())),
        ("esbuild".to_owned(), None)
    );
}

/// 判定输入不变式的实机复检：那些「原始名与展示名会得出不同结论」的行，
/// 必须在用原始名判定的前提下**保持受保护 / 命中白名单**。
///
/// 这批行是整套改动里最敏感的地雷。实测（2026-09-27，本机 229 个进程，
/// 22 行展示名 ≠ 原始名）里有 4 行两个名字结论不同：
/// - `com.apple.SafariPlatformSupport.Helper` → 展示名 `"Helper"`
/// - `com.apple.Safari.SafeBrowsing.Service` → 展示名 `"Service"`
///   上面两类的 `is_multiprocess_family` 原始名 true / 展示名 false
/// - `com.apple.dock.extra` → 展示名 `"Dock"`，白名单 false / true
///
/// 前两类一旦让判定看到展示名，就会**失去保护**（从不可终止变成可终止）。
/// 这条测试把「它们仍然受保护」钉住。
#[test]
fn rows_whose_display_name_would_flip_the_verdict_stay_protected() {
    let mut sys = System::new_all();
    let rows = list_all(&mut sys);

    let sensitive: Vec<&ProcessRow> = rows
        .iter()
        .filter(|row| {
            crate::process_safety::is_multiprocess_family(&row.full_name)
                != crate::process_safety::is_multiprocess_family(&row.name)
                || is_whitelisted(&row.full_name) != is_whitelisted(&row.name)
        })
        .collect();

    // 判别力自检：没有这种行就说明本测试对回归是哑弹，必须显式失败
    assert!(
        !sensitive.is_empty(),
        "NO-COVERAGE：本机没有「原始名与展示名得出不同结论」的行，\
         本测试无法证明敏感进程仍然受保护"
    );

    for row in &sensitive {
        if crate::process_safety::is_multiprocess_family(&row.full_name) {
            assert!(
                row.protected,
                "PID {}（原始名={} 展示名={}）按原始名是多进程族组件，却没受保护",
                row.pid, row.full_name, row.name
            );
        }
        if is_whitelisted(&row.full_name) {
            assert!(
                row.whitelisted,
                "PID {}（原始名={} 展示名={}）按原始名命中白名单，却没标记",
                row.pid, row.full_name, row.name
            );
        }
    }
}

#[test]
fn display_name_resolution_never_changes_protection_or_whitelist_input() {
    // 展示名解析只影响 row.name；判定必须仍用原始 proc.name()
    let raw = "com.apple.WebKit.WebContent";
    let (display, _) = display_name_for_process(raw, Some("Safari".into()));
    assert_ne!(display, raw, "展示名确实被改写了");
    // 判定入口拿到的仍是原始名
    assert!(!is_whitelisted(raw));
    assert!(!is_whitelisted(&display) || raw == display);
    // 更强的保证：判定函数签名不变，调用点传原始名（由 Step 4 的结构保证）
}

/// 判定一旦被喂展示名，内置系统核心白名单会静默失效。
///
/// `Finder` 命中内置 `SYSTEM_CORE_NAMES`；如果 `is_whitelisted` 收到的是
/// 本地化后的展示名（"访达"），结论会翻成 false —— 一个系统核心进程就变成
/// 可终止目标。这条测试把「展示名不能进判定」的后果固定下来。
#[test]
fn a_localized_display_name_would_break_system_core_whitelisting() {
    let raw = "Finder";
    assert!(is_whitelisted(raw), "前提：Finder 命中内置系统核心白名单");

    let (display, _) = display_name_for_process(raw, Some("访达".into()));
    assert_ne!(display, raw, "前提：展示名确实被改写");
    assert!(
        !is_whitelisted(&display),
        "前提：展示名不再命中白名单 —— 这就是把展示名喂进判定的危险"
    );
}

/// 铁律（判定输入不变式）：`list_all` 每一行的**白名单结论**与**保护判定**
/// 都必须能由**原始进程名**重算出来。展示名解析只能影响 `row.name`。
///
/// 参考基准刻意取自 `proc.name()` 的**现场读取**，而不是 `row.full_name`：
/// 「把展示名重绑定到判定之前」这个变异会同时把 `full_name` 也改写成展示名，
/// 拿 `full_name` 当基准的断言在变异下依然自洽 —— 天然抓不到那个变异。
/// 现场读取的原始名不受变异影响，才是真正独立的基准。
///
/// 判别力不足时（本次扫描里没有任何「原始名与展示名会得出不同结论」的行）
/// 本测试**显式失败**，不允许静静通过。确定性机制覆盖见
/// `a_localized_display_name_would_break_system_core_whitelisting`。
#[test]
fn list_all_derives_protection_and_whitelist_from_the_live_raw_name() {
    let mut sys = System::new_all();
    let rows = list_all(&mut sys);
    assert!(!rows.is_empty(), "list_all 没有产出任何行");
    let parent_pids = collect_parent_pids(&sys);

    let mut compared = 0usize;
    for row in &rows {
        let Some(proc) = sys.process(sysinfo::Pid::from_u32(row.pid)) else {
            continue;
        };
        let raw = proc.name().to_string_lossy().to_string();
        compared += 1;

        assert_eq!(
            row.full_name, raw,
            "PID {} 的 full_name 与现场 proc.name() 不一致（{} vs {}）",
            row.pid, row.full_name, raw
        );
        assert_eq!(
            row.whitelisted,
            is_whitelisted(&raw),
            "PID {}（展示名={}）的白名单结论不是由原始名 {raw} 推出的",
            row.pid,
            row.name
        );
        // 保护判定：跳过「年轻进程」，因为 `safety_veto` 的第三条（运行不足 10 分钟）
        // 会随时间推进自然失效。一个进程完全可能在本测试重算的瞬间跨过 600 秒
        // 边界 —— 此时 list_all 算出 protected=true，重算却得到 false。
        // 所以这里跳过的不是「当前年轻」，而是「**在 list_all 那一刻可能年轻**」：
        // 阈值取 600 秒 + 5 分钟余量，覆盖 list_all 到本次重算之间的全部耗时。
        // 剩下的进程里，前两条 veto（是别人的父进程 / 多进程族组件）纯由名称驱动，
        // 正是要守的那部分。
        if crate::process_safety::process_uptime_secs(proc) >= 600 + 300 {
            let recomputed = crate::process_safety::evaluate_protection(proc, &raw, &parent_pids);
            assert_eq!(
                row.protected, recomputed.protected,
                "PID {}（展示名={}）的保护判定不是由原始名 {raw} 推出的",
                row.pid, row.name
            );
            let (recomputed_key, recomputed_params) = split_optional(&recomputed.reason);
            assert_eq!(
                row.protected_reason_key, recomputed_key,
                "PID {}（展示名={}）的受保护原因 key 不是由原始名 {raw} 推出的",
                row.pid, row.name
            );
            assert_eq!(
                row.protected_reason_params, recomputed_params,
                "PID {}（展示名={}）的受保护原因参数不是由原始名 {raw} 推出的",
                row.pid, row.name
            );
        }
    }
    assert!(
        compared > 0,
        "没有任何行能与现场 sysinfo 比对，不变式形同虚设"
    );

    // —— 判别力审计：不允许静默通过 ——
    // 上面的断言只有在「原始名与展示名会得出不同结论」时，才抓得住
    // 「判定输入被换成展示名」这个变异。没有这种行时必须显式失败。
    let flipped_whitelist = rows
        .iter()
        .filter(|row| is_whitelisted(&row.name) != is_whitelisted(&row.full_name))
        .count();
    let flipped_protection = rows
        .iter()
        .filter(|row| {
            sys.process(sysinfo::Pid::from_u32(row.pid))
                .map(|_| {
                    crate::process_safety::is_multiprocess_family(&row.name)
                        != crate::process_safety::is_multiprocess_family(&row.full_name)
                })
                .unwrap_or(false)
        })
        .count();

    assert!(
        flipped_whitelist > 0 || flipped_protection > 0,
        "NO-COVERAGE：本次扫描结果里没有任何「原始名与展示名得出不同判定结论」的行 \
         （共 {} 行，其中展示名≠原始名的有 {} 行），本测试对「判定输入被换成展示名」\
         这个变异没有判别力，属于静默通过。请人工确认 scanner::list_all 中 \
         `is_whitelisted` / `evaluate_protection` 仍在展示名计算之前。",
        rows.len(),
        rows.iter().filter(|row| row.name != row.full_name).count(),
    );
}

/// `ProcessRow` 的两个名字字段必须一起序列化：
/// `name` 给用户看，`full_name` 给 tooltip 与排查用。
#[test]
fn process_row_serializes_both_the_display_name_and_the_raw_name() {
    let mut row = sample_process_row("com.apple.WebKit.WebContent", "com.apple.WebKit.WebContent");
    row.name_key = Some("process.name.webkitWebContent".to_owned());
    let json = serde_json::to_value(&row).expect("ProcessRow 必须可序列化");

    assert_eq!(json["name"], "com.apple.WebKit.WebContent");
    assert_eq!(json["name_key"], "process.name.webkitWebContent");
    assert_eq!(json["full_name"], "com.apple.WebKit.WebContent");
}

/// 铁律：执行身份必须永远是**原始名**。
///
/// `revalidate_targets`（operation_executor.rs）会把扫描时登记的
/// `identity.name` 与终止前重新读取的 `proc.name()` 逐字比较。只要
/// `process_target_from_row` 改用展示名，任何名字被解析过的进程都会在
/// 终止瞬间报「进程身份已变化」，一键清理直接失效。
#[test]
fn process_targets_from_rows_register_the_raw_name_not_the_display_name() {
    let row = sample_process_row("Google Chrome", "Google Chrome Helper (Renderer)");
    let target = process_target_from_row(&row);

    assert_eq!(target.identity.name, "Google Chrome Helper (Renderer)");
    assert_eq!(row.name, "Google Chrome");
}

/// 端到端复核：`list_all` 登记出来的执行身份，必须与 `revalidate_targets`
/// 终止前会读到的 `proc.name()` 逐字一致。
///
/// 这条同时覆盖两处：`ProcessRow.full_name` 的构造，和
/// `process_target_from_row` 选哪个字段进 identity。任一处拿展示名，
/// 名字被解析过的进程就会在终止瞬间报「进程身份已变化」。
#[test]
fn real_rows_register_identities_that_match_a_live_sysinfo_read() {
    let mut sys = System::new_all();
    let rows = list_all(&mut sys);
    assert!(!rows.is_empty());

    let targets = process_targets_from_rows(&rows);
    // 只投影 (pid, name, full_name, exe)：`ProcessRow` 的 Debug 带着 `icon_base64`
    // （几十 KB base64），直接 `{mismatched:?}` 会把失败日志膨胀到 MB 级。
    let mismatched: Vec<(u32, &str, &str, &str)> = rows
        .iter()
        .zip(targets.iter())
        .filter_map(|(row, target)| {
            let live = sys.process(sysinfo::Pid::from_u32(row.pid))?;
            if live.name().to_string_lossy() == *target.identity.name {
                return None;
            }
            Some((
                row.pid,
                row.name.as_str(),
                row.full_name.as_str(),
                row.exe.as_str(),
            ))
        })
        .collect();

    assert!(
        mismatched.is_empty(),
        "这些行登记的执行名与现场 proc.name() 不一致，终止时会被判为身份变化 \
         （pid, name, full_name, exe）：{mismatched:?}"
    );
}

#[test]
fn register_scan_processes_binds_keys_to_scanned_identities() {
    let mut sys = System::new_all();
    let mut result = scan(&mut sys);
    let mut store = OperationStore::new();

    let registration = register_scan_processes(&mut store, &mut result.processes).unwrap();

    assert_eq!(registration.selection_keys.len(), result.processes.len());
    assert!(result
        .processes
        .iter()
        .zip(registration.selection_keys.iter())
        .all(|(info, key)| &info.selection_key == key));
    assert!(result
        .processes
        .iter()
        .all(|info| !info.selection_key.is_empty() && info.start_time > 0));
}

// ── i18n key 门禁：进程文案只发 key，不发文案 ──

/// 「`reason` / `status` / `protected_reason` 这些位上只放 key」的源码层门禁。
///
/// 改字段名（`reason` → `reason_key`）本身由编译器守住「不得再塞文案」；
/// 这道门禁补的是编译器管不到的一半：
///
/// 1. key 落在正确命名空间（`process.reason.` / `process.status.` /
///    `process.protect.` / `process.name.`）
/// 2. key 是纯 ASCII —— 想把中文塞进 key 位，赋值处当场就被这道断言抓住
///
/// 之所以能在源码层断言（而不是只靠运行时抽真实进程）：分类的阈值分支在测试机上
/// 未必被命中，纯 live-scan 护栏会空转放行。`include_str!` 读自己的源码，
/// 逐条列出**编译进二进制的所有 key**，结果在任何机器上都确定。
#[test]
fn process_text_ships_only_ascii_i18n_keys() {
    let source = include_str!("scanner.rs");
    let reason_keys = crate::i18n_text::key_literals_before_tests(source, "process.reason.");
    let status_keys = crate::i18n_text::key_literals_before_tests(source, "process.status.");
    let name_keys = crate::i18n_text::key_literals_before_tests(source, "process.name.");

    // 抽取本身必须有效：正则/切分失效会让下面所有断言空转成「全过」
    assert!(
        reason_keys.len() >= 10 && status_keys.len() >= 13,
        "抽取到的 key 数量对不上：reason {} 个、status {} 个",
        reason_keys.len(),
        status_keys.len(),
    );
    for (label, keys) in [
        ("reason", &reason_keys),
        ("status", &status_keys),
        ("name", &name_keys),
    ] {
        for key in keys {
            crate::i18n_text::assert_ascii_key(key);
            assert!(
                key.starts_with(&format!("process.{label}.")),
                "{label}_key 必须落在 process.{label}..* 命名空间，实际是 {key:?}",
            );
        }
    }
    // 「数量下限」不够：把某个 key 改成别的命名空间，它就从抽取结果里消失了，
    // 下限照样满足。所以必须**逐条**钉住完整集合 —— 否则「写错命名空间」这一类
    // 变异在源码层是隐形的（只靠前端孤儿门禁与端到端门禁兜底）。
    assert_eq!(
        reason_keys,
        vec![
            "process.reason.devTool",
            "process.reason.devToolHighMemory",
            "process.reason.highCpu",
            "process.reason.highMemory",
            "process.reason.idle",
            "process.reason.vetoedHighUsageMainProcess",
            "process.reason.vetoedHighUsageMultiProcess",
            "process.reason.zombie",
            "process.reason.zombieVetoedMultiProcessComponent",
            "process.reason.zombieVetoedParentOfOthers",
            "process.reason.zombieVetoedYoungProcess",
        ],
        "process.reason.* 必须恰好是这 11 枚",
    );
    // 13 个状态一个都不能少：少一个就有一个状态在界面上直接显示裸 key
    assert_eq!(
        status_keys.len(),
        13,
        "process.status.* 必须恰好 13 枚（与 sysinfo 的 12 个分支 + 兜底一一对应），\
         实际 {} 枚：{status_keys:?}",
        status_keys.len(),
    );
    // 3 个 WebKit 固定映射名
    assert_eq!(
        name_keys,
        vec![
            "process.name.webkitGpu",
            "process.name.webkitNetworking",
            "process.name.webkitWebContent",
        ],
        "process.name.* 必须恰好是这 3 枚",
    );
}

/// 端到端：真实扫描出来的每一行，reason / status / 受保护原因都必须是合法 key。
///
/// 这条与上面的源码门禁互补：源码门禁管「key 写对了吗」，这条管
/// 「实际序列化出去的东西对吗」（包括 `I18nText::default()` 的空 key 会不会
/// 因为某天忘了过滤 Hidden 分支而漏到前端）。
#[test]
fn scanned_processes_expose_ascii_keys_end_to_end() {
    let mut sys = System::new_all();
    let result = scan(&mut sys);

    for p in &result.processes {
        assert!(
            p.reason_key.starts_with("process.reason."),
            "PID {} 的 reason_key 非法：{:?}",
            p.pid,
            p.reason_key
        );
        assert!(
            p.reason_key.is_ascii(),
            "PID {} 的 reason_key 非 ASCII",
            p.pid
        );
        // 隐藏分支的 reason 是空 key，但这类行根本不该出现在结果里
        assert!(
            !p.reason_key.is_empty(),
            "PID {} 的 reason_key 是空的",
            p.pid
        );
        for (name, _) in &p.reason_params {
            assert!(
                name.is_ascii(),
                "PID {} 的 reason 参数名非 ASCII：{name:?}",
                p.pid
            );
        }
        // 受保护 ⟺ 有受保护原因：前端靠这个不变量决定要不要渲染「受保护（…）」
        assert_eq!(
            p.protected,
            p.protected_reason_key.is_some(),
            "PID {} 的 protected 与受保护原因不一致",
            p.pid,
        );
        if let Some(key) = &p.protected_reason_key {
            assert!(
                key.starts_with("process.protect."),
                "PID {} 的受保护原因非法：{key:?}",
                p.pid
            );
        }
    }

    let mut sys = System::new_all();
    let rows = list_all(&mut sys);
    for row in &rows {
        assert!(
            row.status_key.starts_with("process.status."),
            "PID {} 的 status_key 非法：{:?}",
            row.pid,
            row.status_key,
        );
        if let Some(key) = &row.name_key {
            assert!(
                key.starts_with("process.name."),
                "PID {} 的 name_key 非法：{key:?}",
                row.pid,
            );
            // name 里绝不能是 key —— 漏翻时用户看到的应该是原始进程名
            assert_ne!(row.name, *key, "PID {} 把 i18n key 塞进了 name", row.pid);
        }
    }
}
