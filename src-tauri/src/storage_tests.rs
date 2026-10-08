use super::*;

#[test]
fn legacy_rows_have_null_reclaimed_after_migration() {
    let storage = Storage::open_in_memory().expect("in-memory storage");
    storage
        .log_history(
            "cache",
            "2 项缓存",
            100,
            true,
            "旧数据",
            2,
            2,
            0,
            "",
            100,
            0,
            None,
        )
        .expect("log");
    let rows = storage.recent_history(10).expect("read");
    assert_eq!(rows.len(), 1);
    // 这条是接口新增字段后的写入：应落到主口径
    assert_eq!(rows[0].deleted_bytes, 100);
    assert_eq!(rows[0].trashed_bytes, 0);
    // 未提供实测值时是 None，绝不伪装 0
    assert_eq!(rows[0].reclaimed_bytes, None);
}

#[test]
fn migration_adds_structured_columns_to_legacy_history_table() {
    let conn = Connection::open_in_memory().expect("in-memory connection");
    conn.execute_batch(
        "CREATE TABLE history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            operation TEXT NOT NULL,
            target TEXT NOT NULL,
            freed_bytes INTEGER NOT NULL DEFAULT 0,
            success INTEGER NOT NULL DEFAULT 1,
            detail TEXT NOT NULL DEFAULT '',
            item_count INTEGER NOT NULL DEFAULT 0,
            ok_count INTEGER NOT NULL DEFAULT 0,
            fail_count INTEGER NOT NULL DEFAULT 0,
            reason_code TEXT NOT NULL DEFAULT ''
        );",
    )
    .expect("旧库 schema");

    Storage::migrate_history_columns(&conn).expect("迁移");

    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(history)")
        .expect("table_info")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    for name in ["reclaimed_bytes", "deleted_bytes", "trashed_bytes"] {
        assert!(
            columns.iter().any(|column| column == name),
            "迁移后缺少列 {name}，实际: {columns:?}"
        );
    }
}

#[test]
fn legacy_rows_read_back_as_zero_deleted_and_null_reclaimed() {
    let conn = Connection::open_in_memory().expect("in-memory connection");
    conn.execute_batch(
        "CREATE TABLE history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            operation TEXT NOT NULL,
            target TEXT NOT NULL,
            freed_bytes INTEGER NOT NULL DEFAULT 0,
            success INTEGER NOT NULL DEFAULT 1,
            detail TEXT NOT NULL DEFAULT '',
            item_count INTEGER NOT NULL DEFAULT 0,
            ok_count INTEGER NOT NULL DEFAULT 0,
            fail_count INTEGER NOT NULL DEFAULT 0,
            reason_code TEXT NOT NULL DEFAULT ''
        );
        INSERT INTO history
            (timestamp, operation, target, freed_bytes, success, detail,
             item_count, ok_count, fail_count, reason_code)
        VALUES ('2026-01-01T00:00:00+00:00', 'cache', '旧行', 42, 1, '', 1, 1, 0, '');",
    )
    .expect("旧库 + 旧行");
    Storage::migrate_history_columns(&conn).expect("迁移");

    let storage = Storage::from_connection(conn);
    let rows = storage.recent_history(10).expect("read");
    assert_eq!(rows.len(), 1);
    // NOT NULL DEFAULT 0 让旧行天然表示「没有结构化数据」
    assert_eq!(rows[0].deleted_bytes, 0);
    assert_eq!(rows[0].trashed_bytes, 0);
    // 可空列：旧行读回 None，而不是 0
    assert_eq!(rows[0].reclaimed_bytes, None);
}
