pub mod backup;
pub mod migration;
pub mod schema;

pub use backup::{
    BackupError, BackupManifest, BackupResult, BackupVerification, create_backup,
    create_verified_backup, restore_backup, restore_verified_backup, verify_backup,
    verify_backup_file,
};
pub use migration::LegacyConversionReport;

use rusqlite::{Connection, Result as SqlResult};

/// Open every SQLite connection with the durability/concurrency pragmas that
/// the crawler relies on. Foreign-key enforcement is enabled even though the
/// v3 contract intentionally avoids hard constraints for legacy compatibility.
pub fn open_db(db_path: &str) -> SqlResult<Connection> {
    let conn = Connection::open(db_path)?;
    configure_connection(&conn)?;
    Ok(conn)
}

pub fn configure_connection(conn: &Connection) -> SqlResult<()> {
    conn.pragma_update(None, "journal_mode", &"WAL")?;
    conn.pragma_update(None, "foreign_keys", &"ON")?;
    conn.pragma_update(None, "busy_timeout", &5000_i64)?;
    Ok(())
}

pub fn init_db(db_path: &str) -> SqlResult<Connection> {
    let conn = open_db(db_path)?;
    run_migrations(&conn)?;
    Ok(conn)
}

pub fn run_migrations(conn: &Connection) -> SqlResult<()> {
    configure_connection(conn)?;
    migration::run_migrations(conn)
}

pub fn init_dataset_id(conn: &Connection) -> SqlResult<String> {
    migration::init_dataset_id(conn)
}

pub fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    migration::column_exists(conn, table, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn schema_fresh_init_is_v3_and_configured() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, schema::V3_SCHEMA_VERSION);
        assert!(
            journal_mode.eq_ignore_ascii_case("memory") || journal_mode.eq_ignore_ascii_case("wal")
        );
        assert_eq!(foreign_keys, 1);
        for table in [
            "sports",
            "match_state_history",
            "odds_current",
            "odds_history",
            "match_detail_sections",
            "assets",
            "feed_events",
            "detail_jobs",
            "asset_jobs",
            "recovery_jobs",
            "sync_outbox",
            "migration_audit",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing v3 table {table}");
        }
    }

    #[test]
    fn schema_concurrent_connections_use_bounded_busy_timeout() {
        let path = std::env::temp_dir().join(format!("storage-schema-{}.db", uuid::Uuid::now_v7()));
        let path_string = path.to_string_lossy().to_string();
        let _ = init_db(&path_string).unwrap();
        let first = open_db(&path_string).unwrap();
        let second = open_db(&path_string).unwrap();
        let first_timeout: i64 = first
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        let second_timeout: i64 = second
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(first_timeout, 5000);
        assert_eq!(second_timeout, 5000);
        drop(first);
        drop(second);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{path_string}-wal"));
        let _ = std::fs::remove_file(format!("{path_string}-shm"));
    }
}
