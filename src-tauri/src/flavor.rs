//! 构建形态（flavor）：Developer ID 全功能版 vs Mac App Store 版。
//!
//! 这是「两个版本完全分离」这条约束在代码里的**唯一真相源**。前端、所有
//! `cfg` 分支、entitlements、capability、打包脚本都从这里派生，不允许在别处
//! 各自判断「是不是 MAS」—— 那种散落判断迟早会漏一处，然后在 MAS 版上暴露成
//! 一个只在沙箱里才复现的怪问题。
//!
//! 判定依据是**编译期 feature**，不掺任何运行时状态、文案或用户输入。

/// 当前构建形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// Developer ID 签名 + Apple 公证，全功能。
    DeveloperId,
    /// Mac App Store 版，运行在 App Sandbox 内。
    Mas,
}

impl Flavor {
    /// 线上格式：与前端 `src/lib/flavor.ts` 的 `Flavor` 联合一一对应。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeveloperId => "developer_id",
            Self::Mas => "mas",
        }
    }

    /// 能否终止其他进程。
    ///
    /// App Sandbox 下沙箱进程不能给别的进程发信号，且**没有任何 entitlement
    /// 能放行**这一项（`process_ops` 用的是 `nix::sys::signal::kill`）。
    /// 所以这是硬能力差异，不是开关：MAS 形态下前端必须把终止入口藏掉，
    /// 后端也必须挡住（`lib.rs` 里 `run_operation_plan` 的 `cfg` 守卫）。
    pub fn can_terminate_processes(&self) -> bool {
        matches!(self, Self::DeveloperId)
    }

    /// 能否用系统 shell / 外部 CLI 干重活。
    ///
    /// 沙箱只能 exec 自己 bundle 里的可执行文件，`plutil` / `sips` /
    /// `osascript` / `docker` / `lsof` 一律不可用。
    pub fn can_exec_external_tools(&self) -> bool {
        matches!(self, Self::DeveloperId)
    }
}

/// 编译出来的这个二进制是什么形态。
pub const CURRENT: Flavor = if cfg!(feature = "mas") {
    Flavor::Mas
} else {
    Flavor::DeveloperId
};

#[cfg(test)]
#[path = "flavor_tests.rs"]
mod tests;
