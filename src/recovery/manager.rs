use crate::domain::events::NormalizedEvent;
use crate::domain::match_state::AdmissionEvidence;
use crate::recovery::finalizer::{FinalizationError, Finalizer};
use crate::storage::repositories::RepositoryUnitOfWork;
use rusqlite::{Result as SqlResult, params};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    pub grace_period: Duration,
    pub retry_delay: Duration,
    pub max_attempts: i32,
    pub lease_duration: Duration,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(15 * 60),
            retry_delay: Duration::from_secs(30),
            max_attempts: 5,
            lease_duration: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryJob {
    pub id: i64,
    pub match_id: String,
    pub dataset_id: String,
    pub reason: String,
    pub status: String,
    pub attempt_count: i32,
    pub grace_until: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RecoveryOutcome {
    StillLive {
        event: NormalizedEvent,
        evidence: AdmissionEvidence,
    },
    Finished {
        event: NormalizedEvent,
        evidence: AdmissionEvidence,
    },
    Cancelled {
        event: NormalizedEvent,
        evidence: AdmissionEvidence,
    },
    Postponed {
        event: NormalizedEvent,
        evidence: AdmissionEvidence,
    },
    Abandoned {
        event: NormalizedEvent,
        evidence: AdmissionEvidence,
    },
    NotFound,
}

pub struct RecoveryManager {
    pub db_path: String,
    pub dataset_id: String,
    pub config: RecoveryConfig,
    pub worker_id: String,
}

impl RecoveryManager {
    pub fn new(db_path: impl Into<String>, dataset_id: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
            dataset_id: dataset_id.into(),
            config: RecoveryConfig::default(),
            worker_id: format!("recovery-{}", uuid::Uuid::new_v4()),
        }
    }

    pub fn on_feed_disconnect(
        &self,
        session_id: Option<&str>,
        sport_id: Option<i32>,
    ) -> SqlResult<usize> {
        let conn = crate::storage::open_db(&self.db_path)?;
        RepositoryUnitOfWork::new(&conn).record_feed_disconnect(
            &self.dataset_id,
            session_id,
            sport_id,
        )
    }

    pub fn on_feed_reconnect(&self) -> SqlResult<usize> {
        let conn = crate::storage::open_db(&self.db_path)?;
        RepositoryUnitOfWork::new(&conn).enqueue_recovery_candidates(
            &self.dataset_id,
            "FEED_RECONNECT",
            self.config.grace_period,
        )
    }

    pub fn on_startup(&self) -> SqlResult<usize> {
        let conn = crate::storage::open_db(&self.db_path)?;
        let repo = RepositoryUnitOfWork::new(&conn);
        repo.reclaim_recovery_leases()?;
        let _ = Finalizer::resume_running(&conn, &self.worker_id);
        repo.enqueue_recovery_candidates(&self.dataset_id, "STARTUP", self.config.grace_period)
    }

    pub fn claim_next(&self) -> SqlResult<Option<RecoveryJob>> {
        let conn = crate::storage::open_db(&self.db_path)?;
        RepositoryUnitOfWork::new(&conn).claim_next_recovery_job(
            &self.dataset_id,
            &self.worker_id,
            self.config.lease_duration,
            self.config.grace_period,
        )
    }

    /// Apply one reacquired source result.  A source/parser failure must call
    /// neither this method nor `Finished`; it remains a retryable recovery job.
    pub fn reconcile(
        &self,
        job: &RecoveryJob,
        outcome: RecoveryOutcome,
    ) -> Result<(), FinalizationError> {
        let conn = crate::storage::open_db(&self.db_path)?;
        let repo = RepositoryUnitOfWork::new(&conn);
        match outcome {
            RecoveryOutcome::NotFound => {
                if repo.retry_recovery_job(
                    job.id,
                    &self.config.retry_delay,
                    self.config.max_attempts,
                )? {
                    repo.complete_recovery_as_unknown(job.id, &job.match_id, &job.dataset_id)?;
                }
            }
            RecoveryOutcome::StillLive { event, evidence } => {
                crate::storage::repositories::apply_event(&conn, &event, &evidence, &[])?;
                repo.complete_recovery_job(job.id)?;
            }
            RecoveryOutcome::Finished { event, evidence }
            | RecoveryOutcome::Cancelled { event, evidence }
            | RecoveryOutcome::Postponed { event, evidence }
            | RecoveryOutcome::Abandoned { event, evidence } => {
                crate::storage::repositories::apply_event(&conn, &event, &evidence, &[])?;
                Finalizer::begin(
                    &conn,
                    &job.match_id,
                    &job.dataset_id,
                    job.id,
                    "recovery",
                    "recovery",
                )?;
                Finalizer::plan_required_sections(&conn, &job.match_id, &job.dataset_id, "FINAL")?;
                repo.wait_for_finalization(job.id)?;
            }
        }
        Ok(())
    }

    /// Reconcile a still-live snapshot and preserve its normalized odds in
    /// the same Task 02 transaction. Completed detail sections are untouched.
    pub fn reconcile_still_live(
        &self,
        job: &RecoveryJob,
        event: &NormalizedEvent,
        evidence: &AdmissionEvidence,
        odds: &[crate::domain::odds::OddsQuote],
    ) -> Result<(), FinalizationError> {
        let conn = crate::storage::open_db(&self.db_path)?;
        crate::storage::repositories::apply_event(&conn, event, evidence, odds)?;
        RepositoryUnitOfWork::new(&conn).complete_recovery_job(job.id)?;
        Ok(())
    }

    pub fn finalize_ready(&self) -> Result<usize, FinalizationError> {
        let conn = crate::storage::open_db(&self.db_path)?;
        Finalizer::complete_ready(&conn, &self.worker_id)
    }

    /// Advance one durable job using only the canonical local state.  A live
    /// match is resolved by the next feed event; a terminal match enters the
    /// FINAL detail plan.  An unresolved candidate is left retryable rather
    /// than guessed as finished.
    pub fn dispatch_once(&self) -> Result<(), FinalizationError> {
        let Some(job) = self.claim_next()? else {
            let _ = self.finalize_ready()?;
            return Ok(());
        };
        let conn = crate::storage::open_db(&self.db_path)?;
        let repo = RepositoryUnitOfWork::new(&conn);
        let live: bool = conn.query_row(
            "SELECT COALESCE(is_live,0) FROM matches WHERE id=?1 AND dataset_id=?2",
            params![job.match_id, job.dataset_id],
            |row| row.get(0),
        )?;
        if live {
            repo.complete_recovery_job(job.id)?;
        } else {
            let terminal: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM match_state_history WHERE match_id=?1 AND dataset_id=?2 AND state IN ('FINISHED','CANCELLED','POSTPONED','ABANDONED'))",
                params![job.match_id, job.dataset_id],
                |row| row.get(0),
            )?;
            if terminal {
                Finalizer::begin(
                    &conn,
                    &job.match_id,
                    &job.dataset_id,
                    job.id,
                    &self.worker_id,
                    "offline-finish",
                )?;
                Finalizer::plan_required_sections(&conn, &job.match_id, &job.dataset_id, "FINAL")?;
                repo.wait_for_finalization(job.id)?;
            } else {
                let expired = repo.retry_recovery_job(
                    job.id,
                    &self.config.retry_delay,
                    self.config.max_attempts,
                )?;
                if expired {
                    repo.complete_recovery_as_unknown(job.id, &job.match_id, &job.dataset_id)?;
                }
            }
        }
        let _ = self.finalize_ready()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    fn live_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        storage::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO matches (id,sport_id,is_live,dataset_id,home_scores,away_scores) VALUES ('live-1',1,1,'ds','[0]','[0]'),('scheduled-1',1,0,'ds','[]','[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO match_state_history (match_id,dataset_id,state,received_at,payload_hash) VALUES ('live-1','ds','LIVE','2026-01-01T00:00:00Z','live')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn recovery_candidate_marks_only_previously_live_matches() {
        let conn = live_db();
        let repo = RepositoryUnitOfWork::new(&conn);
        assert_eq!(
            repo.record_feed_disconnect("ds", Some("session-1"), Some(1))
                .unwrap(),
            1
        );
        let jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recovery_jobs WHERE dataset_id='ds'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(jobs, 1);
        let live: i64 = conn
            .query_row("SELECT is_live FROM matches WHERE id='live-1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(live, 0);
        let scheduled_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recovery_jobs WHERE match_id='scheduled-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scheduled_jobs, 0);
    }

    #[test]
    fn recovery_candidate_query_is_idempotent_on_reconnect() {
        let conn = live_db();
        let repo = RepositoryUnitOfWork::new(&conn);
        assert_eq!(
            repo.enqueue_recovery_candidates("ds", "STARTUP", Duration::from_secs(60))
                .unwrap(),
            1
        );
        assert_eq!(
            repo.enqueue_recovery_candidates("ds", "STARTUP", Duration::from_secs(60))
                .unwrap(),
            0
        );
    }

    #[test]
    fn recovery_not_found_expires_to_unknown_without_replacement() {
        let conn = live_db();
        let repo = RepositoryUnitOfWork::new(&conn);
        repo.ensure_recovery_support().unwrap();
        conn.execute(
            "INSERT INTO recovery_jobs (match_id,dataset_id,reason,status,attempt_count,grace_until,scheduled_at) VALUES ('live-1','ds','NOT_FOUND','RUNNING',5,datetime('now','-1 second'),datetime('now'))",
            [],
        )
        .unwrap();
        assert!(
            repo.retry_recovery_job(1, &Duration::from_secs(1), 5)
                .unwrap()
        );
        repo.complete_recovery_as_unknown(1, "live-1", "ds")
            .unwrap();
        let state: String = conn
            .query_row(
                "SELECT state FROM match_state_history WHERE match_id='live-1' AND state='UNKNOWN'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "UNKNOWN");
        let matches: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM matches WHERE id='live-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matches, 1);
    }

    #[test]
    fn recovery_still_live_keeps_one_canonical_match() {
        let conn = live_db();
        let repo = RepositoryUnitOfWork::new(&conn);
        repo.record_feed_disconnect("ds", Some("session-1"), Some(1))
            .unwrap();
        conn.execute("UPDATE matches SET is_live=1 WHERE id='live-1'", [])
            .unwrap();
        let finalized: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM finalization_versions WHERE match_id='live-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(finalized, 0);
    }
}
