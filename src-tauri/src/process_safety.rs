//! 进程安全审计层 —— 所有会导致「误杀用户进程」的规则集中在这里。
//!
//! 设计原则：
//! 1. 「看起来像垃圾」≠「可以终止」。宁可保守放过一百个，也绝不错杀一个。
//! 2. 只有**完全没用**的进程才允许默认选中（目前只有僵尸 + 僵死孤儿）。
//! 3. 多进程族应用（Chrome / Electron / IDE 等）的子进程永远隐藏或标为 Low 风险。
//! 4. 有子进程的进程绝不碰 —— 它是某个活动应用的主进程。
//! 5. 新启动（< 10 分钟）的进程一律跳过 —— 用户刚启动的工具不算「残留」。

use std::collections::HashSet;
use sysinfo::{Process, System};

/// 已知多进程架构的应用族 —— 名字中包含这些子串的都是正常的多进程设计，
/// 绝不能因为「同名多实例」就当重复清理。
///
/// 数据基于 2026 年 macOS 上主流应用的进程命名惯例。
pub const MULTIPROCESS_FAMILIES: &[&str] = &[
    // Chrome / Chromium 系（包含所有基于 Chromium 的浏览器和 Electron 应用）
    "Google Chrome Helper",
    "Google Chrome Helper (Renderer)",
    "Google Chrome Helper (GPU)",
    "Google Chrome Helper (Plugin)",
    "Chromium Helper",
    "Microsoft Edge Helper",
    "Brave Browser Helper",
    "Arc Helper",
    "Opera Helper",
    "Vivaldi Helper",
    // Electron 通用
    "Electron Helper",
    "Electron Helper (Renderer)",
    "Electron Helper (GPU)",
    "Electron Helper (Plugin)",
    // VS Code / Cursor / Windsurf / Fork / Zed
    "Code Helper",
    "Code Helper (Renderer)",
    "Code Helper (GPU)",
    "Code Helper (Plugin)",
    "Code - Insiders Helper",
    "Cursor Helper",
    "Windsurf Helper",
    "Zed Helper",
    // 通讯类 Electron 应用
    "Slack Helper",
    "Slack Helper (Renderer)",
    "Slack Helper (GPU)",
    "Discord Helper",
    "Discord Helper (Renderer)",
    "Discord Helper (GPU)",
    "WhatsApp Helper",
    "Telegram",
    "QQ",
    "WeChat",
    "WeWorkMac",
    "DingTalk",
    "Lark",
    "Feishu",
    "飞书",
    // 笔记 / 文档类
    "Notion Helper",
    "Obsidian Helper",
    "Logseq Helper",
    "Craft Helper",
    "Linear Helper",
    "Figma Helper",
    "Raycast",
    "Alfred",
    // AI 客户端
    "ChatGPT Helper",
    "Claude Helper",
    "Perplexity Helper",
    // 浏览器主进程本身（虽然是单例，但不可当成残留）
    "Google Chrome",
    "Chromium",
    "Microsoft Edge",
    "Brave Browser",
    "Safari",
    "Arc",
    "Firefox",
    // 其他 Apple/macOS 多进程但不在核心白名单里的
    "com.apple.WebKit.WebContent",
    "com.apple.WebKit.Networking",
    "com.apple.WebKit.GPU",
    // 开发工具
    "docker",
    "com.docker.backend",
    "com.docker.build",
    "com.docker.dev-envs",
    "com.docker.virtualization",
    "Docker Desktop",
    "Docker Desktop Backend",
    // 音视频 / 娱乐
    "Spotify",
    "Spotify Helper",
    "Music",
    "Apple TV",
];

/// 判断进程是否属于多进程族（Helper / 子进程架构）。
pub fn is_multiprocess_family(name: &str) -> bool {
    MULTIPROCESS_FAMILIES.iter().any(|f| {
        // 精准匹配 + 子串匹配（兼容 "Google Chrome Helper (Renderer)" 和 "Google Chrome Helper"）
        name == *f || name.starts_with(f) || name.contains(f)
    })
}

/// 给定 System 快照，返回所有有子进程的 PID 集合。
/// 「父进程」= 在当前快照里，有任何其他进程的 parent_pid 指向它。
pub fn collect_parent_pids(sys: &System) -> HashSet<u32> {
    let mut out = HashSet::new();
    for proc in sys.processes().values() {
        if let Some(ppid) = proc.parent() {
            out.insert(ppid.as_u32());
        }
    }
    out
}

/// 获取进程的运行时长（秒）。失败返回 0。
pub fn process_uptime_secs(proc: &Process) -> u64 {
    let started = proc.start_time(); // Unix epoch seconds
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(started)
}

/// 「年轻」进程 —— 运行 < 10 分钟的任何进程都视为用户刚启动的东西，不清理。
pub fn is_young_process(proc: &Process) -> bool {
    process_uptime_secs(proc) < 600
}

/// 安全审计的否决**原因种类**。
///
/// 为什么必须是枚举而不是一段中文：
/// `scanner.rs` 里有一条真实的业务分支（「年轻进程要隐藏，父进程 / 多进程族要
/// 降级展示」），它过去靠 `veto_reason.contains("不足")` 这种中文子串匹配来
/// 判断。中文子串匹配既脆又不属于「安全语义」的一部分：改一次文案就可能悄悄
/// 改变判定结果。枚举把这个耦合从「字符串内容」降到「类型」。
///
/// 铁律：**枚举的判定顺序与命中条件一字不改**，i18n 改造不得改变任何一条
/// `safety_veto` 分支的行为。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyVeto {
    /// 该进程是别人的父进程 —— 它是某应用的主进程
    ParentOfOthers,
    /// 多进程架构应用的 Helper —— Chrome / Electron / IDE 等
    MultiProcessComponent,
    /// 运行时间过短 —— 可能是用户刚启动的
    YoungProcess,
}

impl SafetyVeto {
    /// 前端词典里的 key（纯 ASCII）。**不是文案**：译文只在 `src/i18n/*.ts`。
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::ParentOfOthers => "process.protect.parentOfOthers",
            Self::MultiProcessComponent => "process.protect.multiProcessComponent",
            Self::YoungProcess => "process.protect.youngProcess",
        }
    }
}

/// 最终安全判断 —— 任意一条命中就表示「不能默认选中」。
///
/// 返回 Some(种类) 表示必须降级/隐藏，None 表示通过安全审计。
pub fn safety_veto(proc: &Process, name: &str, parent_pids: &HashSet<u32>) -> Option<SafetyVeto> {
    // 1. 该进程是别人的父进程 —— 说明它是某应用的主进程，永远不碰
    if parent_pids.contains(&proc.pid().as_u32()) {
        return Some(SafetyVeto::ParentOfOthers);
    }

    // 2. 多进程架构应用的 Helper —— Chrome / Electron / IDE 等
    if is_multiprocess_family(name) {
        return Some(SafetyVeto::MultiProcessComponent);
    }

    // 3. 运行时间过短 —— 可能是用户刚启动的
    if is_young_process(proc) {
        return Some(SafetyVeto::YoungProcess);
    }

    // 4. 有活跃 IO / 文件句柄 太多 —— sysinfo 无直接支持；交给 port 检测层做类似效果

    None
}

/// PID 是否属于当前用户 —— 跨用户的进程一律不碰。
pub fn is_same_user(proc: &Process) -> bool {
    let current = nix::unistd::Uid::effective().as_raw();
    match proc.user_id() {
        Some(uid) => **uid == current,
        None => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessProtection {
    pub protected: bool,
    pub whitelisted: bool,
    /// 受保护原因的**待翻译文案**（i18n key + 插值参数），不是文案本身。
    ///
    /// 「受保护原因」纯粹是展示信息：`protected` 布尔才是判定输入，
    /// 把 `reason` 换成 key 不影响任何判定。
    pub reason: Option<crate::i18n_text::I18nText>,
}

pub fn evaluate_protection(
    proc: &Process,
    name: &str,
    parent_pids: &HashSet<u32>,
) -> ProcessProtection {
    evaluate_protection_with(
        proc,
        name,
        parent_pids,
        crate::whitelist::is_whitelisted(name),
    )
}

pub fn evaluate_protection_with(
    proc: &Process,
    name: &str,
    parent_pids: &HashSet<u32>,
    whitelisted: bool,
) -> ProcessProtection {
    let veto = safety_veto(proc, name, parent_pids);
    let protected = veto.is_some() || whitelisted;
    // 优先级与改造前逐字一致：命中白名单时显示白名单原因，不再看 veto。
    let reason = if whitelisted {
        Some(crate::i18n_text::I18nText::plain(
            "process.protect.whitelisted",
        ))
    } else {
        veto.map(|kind| crate::i18n_text::I18nText::plain(kind.i18n_key()))
    };
    ProcessProtection {
        protected,
        whitelisted,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use crate::whitelist::is_whitelisted;

    fn any_process(sys: &mut System) -> u32 {
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes()
            .values()
            .next()
            .expect("系统至少有一个进程")
            .pid()
            .as_u32()
    }

    fn reason_key(protection: &ProcessProtection) -> Option<&str> {
        protection.reason.as_ref().map(|text| text.key.as_str())
    }

    #[test]
    fn chrome_helper_is_multiprocess() {
        assert!(is_multiprocess_family("Google Chrome Helper"));
        assert!(is_multiprocess_family("Google Chrome Helper (Renderer)"));
        assert!(is_multiprocess_family("Google Chrome Helper (GPU)"));
    }

    #[test]
    fn electron_apps_are_multiprocess() {
        assert!(is_multiprocess_family("Slack Helper"));
        assert!(is_multiprocess_family("Discord Helper (Renderer)"));
        assert!(is_multiprocess_family("Code Helper"));
        assert!(is_multiprocess_family("Cursor Helper"));
    }

    #[test]
    fn cjk_communications_apps() {
        assert!(is_multiprocess_family("Lark"));
        assert!(is_multiprocess_family("Feishu"));
        assert!(is_multiprocess_family("WeChat"));
        assert!(is_multiprocess_family("飞书"));
    }

    #[test]
    fn random_process_not_multiprocess() {
        assert!(!is_multiprocess_family("my-custom-daemon"));
        assert!(!is_multiprocess_family("random-script"));
    }

    #[test]
    fn browser_main_process_also_protected() {
        assert!(is_multiprocess_family("Google Chrome"));
        assert!(is_multiprocess_family("Safari"));
        assert!(is_multiprocess_family("Firefox"));
    }

    #[test]
    fn process_protection_vetoes_multiprocess_family_members() {
        let mut sys = System::new_all();
        let pid = any_process(&mut sys);
        let proc = sys.process(sysinfo::Pid::from_u32(pid)).unwrap();
        let parents = HashSet::new();

        let protection = evaluate_protection(proc, "Google Chrome Helper (Renderer)", &parents);

        assert!(protection.protected);
        assert!(!protection.whitelisted);
        assert_eq!(
            reason_key(&protection),
            Some("process.protect.multiProcessComponent")
        );
    }

    #[test]
    fn process_protection_vetoes_parent_processes() {
        let mut sys = System::new_all();
        let pid = any_process(&mut sys);
        let proc = sys.process(sysinfo::Pid::from_u32(pid)).unwrap();
        let mut parents = HashSet::new();
        parents.insert(pid);

        let protection = evaluate_protection(proc, "plain-daemon", &parents);

        assert!(protection.protected);
        assert_eq!(
            reason_key(&protection),
            Some("process.protect.parentOfOthers")
        );
    }

    #[test]
    fn process_protection_flags_whitelisted_names() {
        let mut sys = System::new_all();
        let pid = any_process(&mut sys);
        let proc = sys.process(sysinfo::Pid::from_u32(pid)).unwrap();
        let parents = HashSet::new();

        let protection = evaluate_protection(proc, "launchd", &parents);

        assert!(protection.protected);
        assert!(protection.whitelisted);
        assert_eq!(reason_key(&protection), Some("process.protect.whitelisted"));
    }

    /// `protected` 与「有受保护原因」必须严格等价 —— 前端靠这个不变量决定
    /// 要不要渲染「受保护（…）」这一段，改一边不改另一边会让界面出现
    /// 「有盾牌但没有理由」或「没盾牌却写着理由」。
    #[test]
    fn protected_flag_and_reason_always_agree() {
        let mut sys = System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let parents = collect_parent_pids(&sys);
        for (pid, proc) in sys.processes() {
            assert!(pid.as_u32() > 0);
            for name in [
                proc.name().to_string_lossy().into_owned(),
                "Google Chrome Helper".to_owned(),
            ] {
                let protection = evaluate_protection_with(
                    proc,
                    &name,
                    &parents,
                    crate::whitelist::is_whitelisted(&name),
                );
                assert_eq!(
                    protection.protected,
                    protection.reason.is_some(),
                    "{name} 的 protected 与 reason 不一致",
                );
            }
        }
    }

    /// 每一种否决原因的 key 都必须落在 `process.protect.*` 命名空间且纯 ASCII。
    #[test]
    fn every_veto_kind_declares_a_process_protect_key() {
        for kind in [
            SafetyVeto::ParentOfOthers,
            SafetyVeto::MultiProcessComponent,
            SafetyVeto::YoungProcess,
        ] {
            let key = kind.i18n_key();
            assert!(key.starts_with("process.protect."), "{key:?} 命名空间不对");
            assert!(key.is_ascii(), "{key:?} 含非 ASCII 字符");
        }
    }

    /// `process.protect.*` 的全仓库字面量清单门禁。
    ///
    /// 受保护原因一共有三个产生点（安全审计三种否决 + 用户白名单），
    /// 分处两个文件（`process_safety.rs` 与 `operation_commands.rs` 的
    /// `apply_policy`），所以这里把两个文件都扫一遍。少扫一个，那个文件里
    /// 塞进 key 位的文案就没人管。
    #[test]
    fn protection_reasons_ship_only_ascii_i18n_keys() {
        let mut keys = crate::i18n_text::key_literals_before_tests(
            include_str!("process_safety.rs"),
            "process.protect.",
        );
        keys.extend(crate::i18n_text::key_literals_before_tests(
            include_str!("operation_commands.rs"),
            "process.protect.",
        ));
        keys.sort();
        keys.dedup();

        assert!(
            keys.len() >= 4,
            "抽取到的 process.protect.* 数量对不上（只有 {} 枚）：{keys:?}",
            keys.len(),
        );
        for key in &keys {
            crate::i18n_text::assert_ascii_key(key);
        }
        for expected in [
            "process.protect.parentOfOthers",
            "process.protect.multiProcessComponent",
            "process.protect.youngProcess",
            "process.protect.whitelisted",
        ] {
            assert!(
                keys.iter().any(|key| key == expected),
                "{expected} 必须被后端发出（否则界面上这一条永远是查不到翻译的裸 key）",
            );
        }
    }

    #[test]
    fn process_protection_allows_old_non_family_plain_process() {
        let mut sys = System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let parents = collect_parent_pids(&sys);
        let plain = sys.processes().values().find(|proc| {
            !is_young_process(proc)
                && !is_whitelisted(&proc.name().to_string_lossy())
                && !is_multiprocess_family(&proc.name().to_string_lossy())
                && !parents.contains(&proc.pid().as_u32())
        });

        if let Some(proc) = plain {
            let name = proc.name().to_string_lossy().to_string();
            let protection = evaluate_protection(proc, &name, &parents);
            assert!(!protection.protected, "{} 不应受保护", name);
            assert!(!protection.whitelisted);
            assert!(protection.reason.is_none());
        }
    }
}
