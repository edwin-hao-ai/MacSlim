use crate::user_error::UserError;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Storage {
    conn: Mutex<Connection>,
}

#[derive(Serialize, Clone, Debug)]
pub struct HistoryEntry {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub operation: String, // cache | process | app_terminate | uninstall | docker
    pub target: String,
    pub freed_bytes: u64,
    pub success: bool,
    pub detail: String,
    /// 结构化计数：界面据此本地化渲染。
    /// 旧数据为 0 —— 前端会回退到 `target` / `detail` 的原始文本。
    pub item_count: u64,
    pub ok_count: u64,
    pub fail_count: u64,
    /// 失败原因的错误码（`ErrorCode` 的序列化名）；空串表示没有失败原因。
    pub reason_code: String,
    /// 成功永久删除的体积之和（缓存主口径）。
    pub deleted_bytes: u64,
    /// 移入废纸篓的体积之和（卸载主口径，尚未释放）。
    pub trashed_bytes: u64,
    /// 卷可用空间的实测增量；`None` 表示未能测量（区别于 0）。
    pub reclaimed_bytes: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct WhitelistEntry {
    pub id: i64,
    pub kind: String, // "process" | "cache_path"
    pub value: String,
    pub added_at: DateTime<Utc>,
    pub note: String,
}

impl Storage {
    pub fn open() -> Result<Self, UserError> {
        let path = db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {}", e))?;
        }
        let conn = Connection::open(&path).map_err(|e| format!("打开 DB 失败: {}", e))?;
        Self::initialize(&conn)?;

        Ok(Storage {
            conn: Mutex::new(conn),
        })
    }

    /// 测试用：不落盘，避免污染真实 `macslim.db`。
    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, UserError> {
        let conn = Connection::open_in_memory().map_err(|e| format!("打开内存 DB 失败: {}", e))?;
        Self::initialize(&conn)?;
        Ok(Self::from_connection(conn))
    }

    /// 测试用：接管一个已就绪的连接（旧库迁移测试）。
    #[cfg(test)]
    pub(crate) fn from_connection(conn: Connection) -> Self {
        Storage {
            conn: Mutex::new(conn),
        }
    }

    fn initialize(conn: &Connection) -> Result<(), UserError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                target TEXT NOT NULL,
                freed_bytes INTEGER NOT NULL DEFAULT 0,
                success INTEGER NOT NULL DEFAULT 1,
                detail TEXT NOT NULL DEFAULT '',
                -- 结构化字段：界面据此本地化渲染（见 HistoryView）。
                -- target/detail 是拼好的中文，给 CLI 与旧数据兜底用。
                item_count INTEGER NOT NULL DEFAULT 0,
                ok_count INTEGER NOT NULL DEFAULT 0,
                fail_count INTEGER NOT NULL DEFAULT 0,
                reason_code TEXT NOT NULL DEFAULT '',
                -- 诚实口径：旧 freed_bytes 保留兜底，新列承载可审计的主口径。
                -- reclaimed_bytes 可空：读不到就是 NULL，绝不伪装 0。
                reclaimed_bytes INTEGER,
                deleted_bytes INTEGER NOT NULL DEFAULT 0,
                trashed_bytes INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_history_ts ON history(timestamp DESC);

            CREATE TABLE IF NOT EXISTS whitelist (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                value TEXT NOT NULL,
                added_at TEXT NOT NULL,
                note TEXT NOT NULL DEFAULT '',
                UNIQUE(kind, value)
            );
            "#,
        )
        .map_err(|e| format!("初始化 schema 失败: {}", e))?;
        Self::migrate_history_columns(conn)?;
        Ok(())
    }

    /// 给老库补上后加的结构化列。
    ///
    /// `CREATE TABLE IF NOT EXISTS` 对**已存在**的表什么都不做，所以新增列
    /// 必须显式 ALTER，否则升级上来的用户一读历史就报「no such column」。
    /// 默认值让旧行天然表示「没有结构化数据」，前端据此回退到 target/detail。
    fn migrate_history_columns(conn: &Connection) -> Result<(), UserError> {
        let existing: Vec<String> = conn
            .prepare("PRAGMA table_info(history)")
            .and_then(|mut stmt| {
                let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| e.to_string())?;
        for (name, ddl) in [
            (
                "item_count",
                "ALTER TABLE history ADD COLUMN item_count INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "ok_count",
                "ALTER TABLE history ADD COLUMN ok_count INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "fail_count",
                "ALTER TABLE history ADD COLUMN fail_count INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "reason_code",
                "ALTER TABLE history ADD COLUMN reason_code TEXT NOT NULL DEFAULT ''",
            ),
            (
                "reclaimed_bytes",
                "ALTER TABLE history ADD COLUMN reclaimed_bytes INTEGER",
            ),
            (
                "deleted_bytes",
                "ALTER TABLE history ADD COLUMN deleted_bytes INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "trashed_bytes",
                "ALTER TABLE history ADD COLUMN trashed_bytes INTEGER NOT NULL DEFAULT 0",
            ),
        ] {
            if !existing.iter().any(|c| c == name) {
                conn.execute(ddl, []).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn log_history(
        &self,
        operation: &str,
        target: &str,
        freed_bytes: u64,
        success: bool,
        detail: &str,
        item_count: u64,
        ok_count: u64,
        fail_count: u64,
        reason_code: &str,
        deleted_bytes: u64,
        trashed_bytes: u64,
        reclaimed_bytes: Option<u64>,
    ) -> Result<i64, UserError> {
        let now = Utc::now().to_rfc3339();
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO history
               (timestamp, operation, target, freed_bytes, success, detail,
                item_count, ok_count, fail_count, reason_code,
                reclaimed_bytes, deleted_bytes, trashed_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                now,
                operation,
                target,
                freed_bytes as i64,
                success as i32,
                detail,
                item_count as i64,
                ok_count as i64,
                fail_count as i64,
                reason_code,
                reclaimed_bytes.map(|v| v as i64),
                deleted_bytes as i64,
                trashed_bytes as i64
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(c.last_insert_rowid())
    }

    pub fn recent_history(&self, limit: usize) -> Result<Vec<HistoryEntry>, UserError> {
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = c
            .prepare(
                "SELECT id, timestamp, operation, target, freed_bytes, success, detail,
                        item_count, ok_count, fail_count, reason_code,
                        reclaimed_bytes, deleted_bytes, trashed_bytes
                 FROM history ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                let ts: String = r.get(1)?;
                let ts_parsed = DateTime::parse_from_rfc3339(&ts)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(HistoryEntry {
                    id: r.get(0)?,
                    timestamp: ts_parsed,
                    operation: r.get(2)?,
                    target: r.get(3)?,
                    freed_bytes: r.get::<_, i64>(4)? as u64,
                    success: r.get::<_, i32>(5)? != 0,
                    detail: r.get(6)?,
                    item_count: r.get::<_, i64>(7)? as u64,
                    ok_count: r.get::<_, i64>(8)? as u64,
                    fail_count: r.get::<_, i64>(9)? as u64,
                    reason_code: r.get(10)?,
                    reclaimed_bytes: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
                    deleted_bytes: r.get::<_, i64>(12)? as u64,
                    trashed_bytes: r.get::<_, i64>(13)? as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn add_whitelist(&self, kind: &str, value: &str, note: &str) -> Result<(), UserError> {
        let now = Utc::now().to_rfc3339();
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        c.execute(
            "INSERT OR IGNORE INTO whitelist (kind, value, added_at, note) VALUES (?1, ?2, ?3, ?4)",
            params![kind, value, now, note],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove_whitelist(&self, id: i64) -> Result<(), UserError> {
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        c.execute("DELETE FROM whitelist WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_whitelist(&self) -> Result<Vec<WhitelistEntry>, UserError> {
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = c
            .prepare("SELECT id, kind, value, added_at, note FROM whitelist ORDER BY id DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                let ts: String = r.get(3)?;
                Ok(WhitelistEntry {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    value: r.get(2)?,
                    added_at: DateTime::parse_from_rfc3339(&ts)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    note: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn is_whitelisted(&self, kind: &str, value: &str) -> bool {
        let Ok(c) = self.conn.lock() else {
            return false;
        };
        c.query_row(
            "SELECT 1 FROM whitelist WHERE kind = ?1 AND value = ?2",
            params![kind, value],
            |_| Ok(()),
        )
        .is_ok()
    }
}

fn db_path() -> Result<PathBuf, UserError> {
    let base = dirs::config_dir().ok_or("无法获取配置目录")?;
    Ok(base.join("MacSlim").join("macslim.db"))
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
