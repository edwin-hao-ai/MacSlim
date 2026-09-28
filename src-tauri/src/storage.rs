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
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                target TEXT NOT NULL,
                freed_bytes INTEGER NOT NULL DEFAULT 0,
                success INTEGER NOT NULL DEFAULT 1,
                detail TEXT NOT NULL DEFAULT ''
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

        Ok(Storage {
            conn: Mutex::new(conn),
        })
    }

    pub fn log_history(
        &self,
        operation: &str,
        target: &str,
        freed_bytes: u64,
        success: bool,
        detail: &str,
    ) -> Result<i64, UserError> {
        let now = Utc::now().to_rfc3339();
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO history (timestamp, operation, target, freed_bytes, success, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                now,
                operation,
                target,
                freed_bytes as i64,
                success as i32,
                detail
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(c.last_insert_rowid())
    }

    pub fn recent_history(&self, limit: usize) -> Result<Vec<HistoryEntry>, UserError> {
        let c = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = c
            .prepare(
                "SELECT id, timestamp, operation, target, freed_bytes, success, detail
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
