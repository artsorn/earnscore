use crate::detail::jobs::FINAL_DETAIL_SECTIONS;
use rusqlite::{Connection, OptionalExtension, Result as SqlResult, params};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationPhase {
    Final,
    Manual,
}

impl FinalizationPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Final => "FINAL",
            Self::Manual => "MANUAL",
        }
    }
}

#[derive(Debug)]
pub enum FinalizationError {
    Sql(rusqlite::Error),
    Busy(String),
    NotReady(String),
}

impl fmt::Display for FinalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(error) => write!(f, "{error}"),
            Self::Busy(message) | Self::NotReady(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FinalizationError {}

impl From<rusqlite::Error> for FinalizationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizationVersion {
    pub match_id: String,
    pub dataset_id: String,
    pub version: i64,
    pub phase: String,
}

pub struct Finalizer;

impl Finalizer {
    pub fn ensure_support(conn: &Connection) -> SqlResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS recovery_phase_locks (
                match_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                phase TEXT NOT NULL,
                owner TEXT NOT NULL,
                lease_expires_at TEXT NOT NULL,
                PRIMARY KEY (match_id, dataset_id)
            );
            CREATE TABLE IF NOT EXISTS finalization_versions (
                match_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                phase TEXT NOT NULL,
                status TEXT NOT NULL,
                recovery_job_id INTEGER,
                actor TEXT NOT NULL,
                reason TEXT NOT NULL,
                started_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                PRIMARY KEY (match_id, dataset_id, version)
            );
            CREATE TABLE IF NOT EXISTS recovery_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                match_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                action TEXT NOT NULL,
                phase TEXT NOT NULL,
                version INTEGER,
                actor TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )?;
        for column in [
            "grace_until TEXT",
            "lease_owner TEXT",
            "lease_expires_at TEXT",
        ] {
            let name = column.split_whitespace().next().unwrap_or_default();
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('recovery_jobs') WHERE name=?1)",
                [name],
                |row| row.get(0),
            )?;
            if !exists {
                conn.execute(
                    &format!("ALTER TABLE recovery_jobs ADD COLUMN {column}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    pub fn begin(
        conn: &Connection,
        match_id: &str,
        dataset_id: &str,
        recovery_job_id: i64,
        actor: &str,
        reason: &str,
    ) -> Result<FinalizationVersion, FinalizationError> {
        Self::begin_phase(
            conn,
            match_id,
            dataset_id,
            FinalizationPhase::Final,
            Some(recovery_job_id),
            actor,
            reason,
        )
    }

    pub fn begin_manual(
        conn: &Connection,
        match_id: &str,
        dataset_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<FinalizationVersion, FinalizationError> {
        Self::begin_phase(
            conn,
            match_id,
            dataset_id,
            FinalizationPhase::Manual,
            None,
            actor,
            reason,
        )
    }

    /// Audited operator entry point. A manual action always gets a new
    /// version, including when an earlier FINAL version is already complete.
    pub fn force_finalize(
        conn: &Connection,
        match_id: &str,
        dataset_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<FinalizationVersion, FinalizationError> {
        let version = Self::begin_manual(conn, match_id, dataset_id, actor, reason)?;
        Self::plan_required_sections(conn, match_id, dataset_id, "MANUAL")?;
        Ok(version)
    }

    fn begin_phase(
        conn: &Connection,
        match_id: &str,
        dataset_id: &str,
        phase: FinalizationPhase,
        recovery_job_id: Option<i64>,
        actor: &str,
        reason: &str,
    ) -> Result<FinalizationVersion, FinalizationError> {
        Self::ensure_support(conn)?;
        let tx = conn.unchecked_transaction()?;
        let finalized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM finalization_versions WHERE match_id=?1 AND dataset_id=?2 AND status='COMPLETED')",
            params![match_id, dataset_id],
            |row| row.get(0),
        )?;
        if finalized && phase == FinalizationPhase::Final {
            return Err(FinalizationError::NotReady(
                "match already finalized; use an audited MANUAL version".into(),
            ));
        }
        tx.execute(
            "DELETE FROM recovery_phase_locks WHERE datetime(lease_expires_at) <= datetime('now')",
            [],
        )?;
        let lock = tx.execute(
            "INSERT OR IGNORE INTO recovery_phase_locks (match_id,dataset_id,phase,owner,lease_expires_at)
             VALUES (?1,?2,?3,?4,datetime('now','+60 seconds'))",
            params![match_id, dataset_id, phase.as_str(), actor],
        )?;
        if lock == 0 {
            return Err(FinalizationError::Busy(format!(
                "phase already running for match {match_id}"
            )));
        }
        let version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version),0)+1 FROM finalization_versions WHERE match_id=?1 AND dataset_id=?2",
            params![match_id, dataset_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO finalization_versions (match_id,dataset_id,version,phase,status,recovery_job_id,actor,reason)
             VALUES (?1,?2,?3,?4,'RUNNING',?5,?6,?7)",
            params![match_id, dataset_id, version, phase.as_str(), recovery_job_id, actor, reason],
        )?;
        tx.execute(
            "INSERT INTO recovery_audit (match_id,dataset_id,action,phase,version,actor,reason)
             VALUES (?1,?2,'VERSION_CREATED',?3,?4,?5,?6)",
            params![match_id, dataset_id, phase.as_str(), version, actor, reason],
        )?;
        tx.commit()?;
        Ok(FinalizationVersion {
            match_id: match_id.into(),
            dataset_id: dataset_id.into(),
            version,
            phase: phase.as_str().into(),
        })
    }

    pub fn plan_required_sections(
        conn: &Connection,
        match_id: &str,
        dataset_id: &str,
        phase: &str,
    ) -> SqlResult<usize> {
        Self::ensure_support(conn)?;
        let mut inserted = 0;
        for section in FINAL_DETAIL_SECTIONS {
            inserted += conn.execute(
                "INSERT OR IGNORE INTO detail_jobs (match_id,dataset_id,section_name,load_phase,status,scheduled_at,attempt_count)
                 VALUES (?1,?2,?3,?4,'PENDING',datetime('now'),0)",
                params![match_id, dataset_id, section, phase],
            )?;
        }
        // Period scores are already captured atomically with the final feed
        // state.  Materialize that state as an immutable detail section so a
        // final refresh has the same completion barrier as fetched sections.
        let period_scores: Option<(String, String)> = conn
            .query_row(
                "SELECT home_scores, away_scores FROM match_state_history
                 WHERE match_id=?1 AND dataset_id=?2 ORDER BY id DESC LIMIT 1",
                params![match_id, dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((home, away)) = period_scores {
            let data = serde_json::json!({"home_scores": serde_json::from_str::<serde_json::Value>(&home).unwrap_or_default(), "away_scores": serde_json::from_str::<serde_json::Value>(&away).unwrap_or_default()});
            conn.execute(
                "INSERT INTO match_detail_sections (match_id,dataset_id,section_name,status,provenance,is_empty,is_unparseable,content_hash,received_at,completed_at)
                 VALUES (?1,?2,'period_scores','COMPLETED','recovery',0,0,?3,datetime('now'),datetime('now'))
                 ON CONFLICT(match_id,dataset_id,section_name) DO UPDATE SET status='COMPLETED', provenance='recovery', content_hash=excluded.content_hash, completed_at=datetime('now')",
                params![match_id, dataset_id, crate::domain::events::payload_hash(&data)],
            )?;
            conn.execute(
                "INSERT INTO match_detail_data (match_id,dataset_id,section_name,data_json,provenance,content_hash,received_at)
                 VALUES (?1,?2,'period_scores',?3,'recovery',?4,datetime('now'))
                 ON CONFLICT(match_id,dataset_id,section_name) DO UPDATE SET data_json=excluded.data_json, provenance='recovery', content_hash=excluded.content_hash, received_at=datetime('now')",
                params![match_id, dataset_id, data.to_string(), crate::domain::events::payload_hash(&data)],
            )?;
        }
        Ok(inserted)
    }

    /// Rebuild the final plan after a process kill between version creation
    /// and job insertion. The version remains the source of truth; this is
    /// not a new refresh/version.
    pub fn resume_running(conn: &Connection, owner: &str) -> Result<usize, FinalizationError> {
        Self::ensure_support(conn)?;
        let versions = {
            let mut stmt = conn.prepare(
                "SELECT match_id,dataset_id,phase FROM finalization_versions WHERE status='RUNNING'",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<SqlResult<Vec<_>>>()?
        };
        let mut resumed = 0;
        for (match_id, dataset_id, phase) in versions {
            conn.execute(
                "INSERT OR IGNORE INTO recovery_phase_locks (match_id,dataset_id,phase,owner,lease_expires_at)
                 VALUES (?1,?2,?3,?4,datetime('now','+60 seconds'))",
                params![match_id, dataset_id, phase, owner],
            )?;
            resumed += Self::plan_required_sections(conn, &match_id, &dataset_id, &phase)?;
        }
        Ok(resumed)
    }

    pub fn complete_ready(conn: &Connection, owner: &str) -> Result<usize, FinalizationError> {
        Self::ensure_support(conn)?;
        let mut stmt = conn.prepare(
            "SELECT match_id,dataset_id,version,phase,recovery_job_id FROM finalization_versions
             WHERE status='RUNNING' ORDER BY started_at,version",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        drop(stmt);
        let mut completed = 0;
        for (match_id, dataset_id, version, phase, recovery_job_id) in rows {
            let pending: i64 = conn.query_row(
                "SELECT COUNT(*) FROM detail_jobs WHERE match_id=?1 AND dataset_id=?2 AND load_phase=?3
                 AND status NOT IN ('COMPLETED','EMPTY_CONFIRMED','FAILED_PERMANENT')",
                params![match_id, dataset_id, phase],
                |row| row.get(0),
            )?;
            if pending != 0 {
                continue;
            }
            let terminal: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM match_state_history WHERE match_id=?1 AND dataset_id=?2 AND state IN ('FINISHED','CANCELLED','POSTPONED','ABANDONED'))",
                params![match_id, dataset_id],
                |row| row.get(0),
            )?;
            if !terminal && phase == "FINAL" {
                continue;
            }
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE finalization_versions SET status='COMPLETED',completed_at=datetime('now')
                 WHERE match_id=?1 AND dataset_id=?2 AND version=?3 AND status='RUNNING'",
                params![match_id, dataset_id, version],
            )?;
            if phase == "FINAL" {
                tx.execute(
                    "INSERT OR IGNORE INTO match_state_history (match_id,dataset_id,state,source_timestamp,received_at,payload_hash,provenance)
                     VALUES (?1,?2,'FINALIZED','',datetime('now'),?3,'recovery')",
                    params![match_id, dataset_id, format!("finalized-{version}")],
                )?;
                tx.execute(
                    "UPDATE matches SET is_live=0,updated_at=datetime('now') WHERE id=?1 AND dataset_id=?2",
                    params![match_id, dataset_id],
                )?;
            }
            tx.execute(
                "INSERT INTO recovery_audit (match_id,dataset_id,action,phase,version,actor,reason)
                 SELECT match_id,dataset_id,'VERSION_COMPLETED',phase,version,?4,reason
                 FROM finalization_versions WHERE match_id=?1 AND dataset_id=?2 AND version=?3",
                params![match_id, dataset_id, version, owner],
            )?;
            if let Some(job_id) = recovery_job_id {
                tx.execute(
                    "UPDATE recovery_jobs SET status='COMPLETED',completed_at=datetime('now'),lease_owner=NULL,lease_expires_at=NULL WHERE id=?1",
                    [job_id],
                )?;
            }
            tx.execute(
                "DELETE FROM recovery_phase_locks WHERE match_id=?1 AND dataset_id=?2",
                params![match_id, dataset_id],
            )?;
            tx.commit()?;
            completed += 1;
        }
        Ok(completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    fn terminal_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        storage::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO matches (id,sport_id,is_live,dataset_id,home_scores,away_scores) VALUES ('final-1',1,0,'ds','[2]','[1]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO match_state_history (match_id,dataset_id,state,home_scores,away_scores,received_at,payload_hash) VALUES ('final-1','ds','FINISHED','[2]','[1]','2026-01-01T00:00:00Z','finished')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn finalization_version_and_phase_lock_are_idempotent() {
        let conn = terminal_db();
        let version = Finalizer::begin(&conn, "final-1", "ds", 1, "test", "offline").unwrap();
        assert_eq!(version.version, 1);
        assert!(matches!(
            Finalizer::begin(&conn, "final-1", "ds", 2, "other", "retry"),
            Err(FinalizationError::Busy(_))
        ));
    }

    #[test]
    fn finalization_completes_once_after_all_sections() {
        let conn = terminal_db();
        Finalizer::begin(&conn, "final-1", "ds", 1, "test", "offline").unwrap();
        Finalizer::plan_required_sections(&conn, "final-1", "ds", "FINAL").unwrap();
        conn.execute("UPDATE detail_jobs SET status='COMPLETED' WHERE match_id='final-1' AND load_phase='FINAL'", []).unwrap();
        assert_eq!(Finalizer::complete_ready(&conn, "test").unwrap(), 1);
        assert_eq!(Finalizer::complete_ready(&conn, "test").unwrap(), 0);
        let finalized: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM finalization_versions WHERE status='COMPLETED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(finalized, 1);
        let state: String = conn
            .query_row(
                "SELECT state FROM match_state_history WHERE state='FINALIZED'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "FINALIZED");
    }

    #[test]
    fn phase_lock_blocks_initial_and_manual_until_final_releases() {
        let conn = terminal_db();
        Finalizer::begin(&conn, "final-1", "ds", 1, "final", "offline").unwrap();
        let blocked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recovery_phase_locks WHERE match_id='final-1' AND phase='FINAL'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(blocked, 1);
        assert!(matches!(
            Finalizer::begin_manual(&conn, "final-1", "ds", "manual", "operator"),
            Err(FinalizationError::Busy(_))
        ));
    }

    #[test]
    fn restart_resumes_final_plan_without_new_version() {
        let conn = terminal_db();
        let first = Finalizer::begin(&conn, "final-1", "ds", 1, "first", "offline").unwrap();
        assert!(Finalizer::resume_running(&conn, "restart").unwrap() > 0);
        let versions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM finalization_versions WHERE match_id='final-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(versions, 1);
        assert_eq!(first.version, 1);
    }

    #[test]
    fn recovery_finished_offline_creates_one_final_plan() {
        let conn = terminal_db();
        let version = Finalizer::begin(&conn, "final-1", "ds", 1, "recovery", "offline").unwrap();
        let count = Finalizer::plan_required_sections(&conn, "final-1", "ds", "FINAL").unwrap();
        assert_eq!(version.version, 1);
        assert_eq!(count, 5);
        assert_eq!(
            Finalizer::plan_required_sections(&conn, "final-1", "ds", "FINAL").unwrap(),
            0
        );
    }
}
