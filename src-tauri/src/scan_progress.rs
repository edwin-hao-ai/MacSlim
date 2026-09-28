use crate::cache_scanner::CacheItem;
use serde::Serialize;

/// 单个扫描阶段的进度更新。
/// 只允许携带阶段名、条目数与体积，不得加入路径、PID 等用户数据。
#[derive(Clone, Serialize)]
pub struct StageUpdate {
    pub stage: String,
    /// "running" | "done"
    pub state: &'static str,
    pub item_count: usize,
    pub found_bytes: u64,
}

impl StageUpdate {
    pub fn running(stage: &str) -> Self {
        Self {
            stage: stage.to_string(),
            state: "running",
            item_count: 0,
            found_bytes: 0,
        }
    }

    pub fn done(stage: &str, items: &[CacheItem]) -> Self {
        Self {
            stage: stage.to_string(),
            state: "done",
            item_count: items.len(),
            found_bytes: items.iter().map(|i| i.size_bytes).sum(),
        }
    }
}

pub type ProgressSink = std::sync::Arc<dyn Fn(StageUpdate) + Send + Sync>;
