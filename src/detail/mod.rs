pub mod extractor;
pub mod jobs;
pub mod types;

use crate::feed::browser::{OwnedBrowser, OwnedTarget, TargetRole};
use crate::storage::repositories::RepositoryUnitOfWork;
use rusqlite::Connection;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use types::{DetailJob, DetailSection};

pub struct DetailWorkerConfig {
    pub concurrency_limit: usize,
    pub lease_duration_secs: i64,
    pub base_delay_secs: u64,
    pub max_delay_secs: u64,
    pub max_attempts: i32,
    pub min_delay_ms: u64,
    pub readiness_timeout: Duration,
    pub probe_interval: Duration,
}

impl Default for DetailWorkerConfig {
    fn default() -> Self {
        Self {
            concurrency_limit: 1,
            lease_duration_secs: 30,
            base_delay_secs: 5,
            max_delay_secs: 300,
            max_attempts: 5,
            min_delay_ms: 1000,
            readiness_timeout: Duration::from_secs(25),
            probe_interval: Duration::from_millis(800),
        }
    }
}

pub struct DetailCoordinator {
    db_path: String,
    browser: Arc<OwnedBrowser>,
    config: DetailWorkerConfig,
    active_dataset_id: String,
    pub asset_coordinator: Arc<crate::assets::AssetCoordinator>,
}

impl DetailCoordinator {
    pub fn new(
        db_path: String,
        browser: Arc<OwnedBrowser>,
        config: DetailWorkerConfig,
        active_dataset_id: String,
        asset_coordinator: Arc<crate::assets::AssetCoordinator>,
    ) -> Self {
        Self {
            db_path,
            browser,
            config,
            active_dataset_id,
            asset_coordinator,
        }
    }

    pub async fn run(self: Arc<Self>, mut shutdown_rx: tokio::sync::oneshot::Receiver<()>) {
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency_limit));
        let worker_owner = format!("detail-worker-{}", uuid::Uuid::new_v4());

        loop {
            // Check shutdown
            if shutdown_rx.try_recv().is_ok() {
                break;
            }

            // 1. Reclaim expired leases
            if let Ok(conn) = Connection::open(&self.db_path) {
                let repo = RepositoryUnitOfWork::new(&conn);
                let _ = repo.reclaim_expired_leases();
            }

            // 2. Try to claim a job
            let mut job_opt = None;
            if let Ok(conn) = Connection::open(&self.db_path) {
                let repo = RepositoryUnitOfWork::new(&conn);
                if let Ok(Some(job)) =
                    repo.claim_next_job(&worker_owner, self.config.lease_duration_secs)
                {
                    job_opt = Some(job);
                }
            }

            if let Some(job) = job_opt {
                let this = self.clone();
                let sem = semaphore.clone();
                let owner = worker_owner.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.ok();
                    if let Err(err) = this.execute_job(&job, &owner).await {
                        eprintln!("[Detail Worker] Job {} failed: {:?}", job.id, err);
                    }
                });
            } else {
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    async fn execute_job(
        &self,
        job: &DetailJob,
        _owner: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 1. Retrieve slugs and sport_id in a short-lived connection
        let (home_slug, away_slug, sport_id) = {
            let conn = Connection::open(&self.db_path)?;
            crate::storage::repositories::get_match_team_slugs(
                &conn,
                &job.match_id,
                &job.dataset_id,
            )?
        };

        let detail_url = if sport_id == 1 {
            format!(
                "https://m.aiscore.com/match-{}-{}/{}",
                home_slug, away_slug, job.match_id
            )
        } else {
            format!(
                "https://m.aiscore.com/match-basketball-{}-{}/{}",
                home_slug, away_slug, job.match_id
            )
        };

        // Create dedicated detail target tab
        let owned_target = self
            .browser
            .create_target(TargetRole::Detail, sport_id, &detail_url)
            .await?;

        let result = self.fetch_and_save(&owned_target, job, sport_id).await;

        // Close the target
        let _ = self.browser.close_target(&owned_target).await;

        match result {
            Ok(_) => {
                println!(
                    "[Detail Worker] Job {} ({}) for match {} completed successfully",
                    job.id, job.section_name, job.match_id
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let permanent = job.attempt_count >= self.config.max_attempts;
                let delay = jobs::calculate_retry_delay(
                    job.attempt_count,
                    self.config.base_delay_secs,
                    self.config.max_delay_secs,
                );
                let conn = Connection::open(&self.db_path)?;
                let repo = RepositoryUnitOfWork::new(&conn);
                let _ = repo.fail_job(job.id, &err_msg, delay.as_secs() as i64, permanent)?;
            }
        }

        Ok(())
    }

    async fn fetch_and_save(
        &self,
        target: &OwnedTarget,
        job: &DetailJob,
        sport_id: i32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut cdp = self.browser.connect_target(target).await?;

        // 1. Minimum delay
        sleep(Duration::from_millis(self.config.min_delay_ms)).await;

        // 2. Activate tabs
        let activate_js = extractor::detail_activate_tabs_js(sport_id, &job.match_id);
        let _ = cdp.evaluate(&activate_js).await;

        // 3. Probe and extract
        let start_time = std::time::Instant::now();
        let extract_js = extractor::detail_extract_js(sport_id, &job.match_id);

        let mut full_payload = Value::Null;
        while start_time.elapsed() < self.config.readiness_timeout {
            if let Ok(val) = cdp.evaluate(&extract_js).await {
                if !val.is_null() {
                    let val_match_id = val["matchId"]
                        .as_str()
                        .or_else(|| val["match_id"].as_str())
                        .unwrap_or("");
                    let val_sport_id = val["sportId"]
                        .as_i64()
                        .or_else(|| val["sport_id"].as_i64())
                        .unwrap_or(0) as i32;
                    if val_match_id == job.match_id && val_sport_id == sport_id {
                        full_payload = val;
                        break;
                    }
                }
            }
            sleep(self.config.probe_interval).await;
        }

        if full_payload.is_null() {
            return Err(format!(
                "Detail readiness timeout or match ID mismatch for match {}",
                job.match_id
            )
            .into());
        }

        // 4. Extract section data and candidates
        let section = DetailSection::from_str(&job.section_name).ok_or("Invalid section name")?;
        let (section_data, image_candidates) =
            extractor::extract_section_data(sport_id, section, &full_payload);

        let is_empty = extractor::is_section_empty(section, &section_data);
        let hash = extractor::compute_hash(&section_data);

        // Extract and write image candidates to a separate location (in-memory candidate pool for Task 05)
        let downloaded_results = if !image_candidates.is_empty() {
            println!(
                "[Detail Worker] Found {} image candidates in section {}",
                image_candidates.len(),
                job.section_name
            );
            self.asset_coordinator
                .process_candidates(&image_candidates)
                .await
        } else {
            std::collections::HashMap::new()
        };

        // 5. Save section transactionally along with assets
        let conn = Connection::open(&self.db_path)?;
        let repo = RepositoryUnitOfWork::new(&conn);
        repo.save_detail_section_with_assets(
            &job.match_id,
            &job.dataset_id,
            &job.section_name,
            &section_data,
            is_empty,
            &hash,
            None,
            &image_candidates,
            &downloaded_results,
            &self.asset_coordinator.config.asset_root,
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn detail_section_conversions() {
        for s in DetailSection::all() {
            assert_eq!(DetailSection::from_str(s.as_str()), Some(*s));
        }
    }

    #[test]
    fn detail_section_empty() {
        assert!(extractor::is_section_empty(
            DetailSection::Stats,
            &json!({})
        ));
        assert!(!extractor::is_section_empty(
            DetailSection::Stats,
            &json!({"possession": [55, 45]})
        ));

        assert!(extractor::is_section_empty(
            DetailSection::Lineups,
            &json!({"home": [], "away": []})
        ));
        assert!(!extractor::is_section_empty(
            DetailSection::Lineups,
            &json!({"home": [{"id": "1"}], "away": []})
        ));
    }

    #[test]
    fn detail_job_planning() {
        let conn = setup_test_db();
        let tx = conn.unchecked_transaction().unwrap();
        crate::storage::repositories::plan_initial_detail_jobs(&tx, "dataset-1", "match-1")
            .unwrap();
        tx.commit().unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM detail_jobs WHERE match_id='match-1' AND dataset_id='dataset-1'",
            [],
            |r| r.get(0)
        ).unwrap();
        assert_eq!(count, 6);
    }

    #[test]
    fn detail_retry_delay() {
        let delay1 = jobs::calculate_retry_delay(1, 5, 300);
        let delay2 = jobs::calculate_retry_delay(2, 5, 300);
        let delay3 = jobs::calculate_retry_delay(3, 5, 300);

        assert!(delay2 >= delay1 || (delay2.as_secs_f64() - delay1.as_secs_f64()).abs() < 2.0);
        assert!(delay3 >= delay2 || (delay3.as_secs_f64() - delay2.as_secs_f64()).abs() < 4.0);
    }

    #[test]
    fn detail_lease_lifecycle() {
        let conn = setup_test_db();
        let repo = RepositoryUnitOfWork::new(&conn);

        {
            let tx = conn.unchecked_transaction().unwrap();
            crate::storage::repositories::plan_initial_detail_jobs(&tx, "dataset-1", "match-1")
                .unwrap();
            tx.commit().unwrap();
        }

        let job = repo.claim_next_job("worker-1", 30).unwrap().unwrap();
        assert_eq!(job.match_id, "match-1");
        assert_eq!(job.attempt_count, 1);

        conn.execute(
            "INSERT INTO detail_jobs (match_id, dataset_id, section_name, load_phase, status, scheduled_at)
             VALUES ('match-1', 'dataset-1', 'odds', 'NON_INITIAL', 'PENDING', datetime('now'))",
            []
        ).unwrap();

        let next_job = repo.claim_next_job("worker-1", 30).unwrap();
        if let Some(ref j) = next_job {
            assert_eq!(j.load_phase, "INITIAL");
        }

        conn.execute(
            "UPDATE detail_jobs SET lease_expires_at = datetime('now', '-10 seconds') WHERE status = 'LOADING'",
            []
        ).unwrap();

        let reclaimed = repo.reclaim_expired_leases().unwrap();
        assert!(reclaimed >= 1);

        let j = repo.claim_next_job("worker-2", 30).unwrap().unwrap();
        assert_eq!(j.attempt_count, 2);
    }

    #[test]
    fn h2h_reference_persistence() {
        let conn = setup_test_db();
        let repo = RepositoryUnitOfWork::new(&conn);

        let h2h_data = json!({
            "history": [
                {
                    "id": "h2h-match-1",
                    "homeTeam": { "id": "t1", "name": "Team 1" },
                    "awayTeam": { "id": "t2", "name": "Team 2" },
                    "played_at": "2026-07-10"
                }
            ]
        });

        repo.save_detail_section(
            "match-1",
            "dataset-1",
            "h2h",
            &h2h_data,
            false,
            "hash-1",
            None,
        )
        .unwrap();

        let count_ref: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM match_h2h_references WHERE match_id='match-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_ref, 1);

        let count_match: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM matches WHERE id='h2h-match-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_match, 0);
    }

    #[test]
    fn detail_no_repeat() {
        let conn = setup_test_db();

        {
            let tx = conn.unchecked_transaction().unwrap();
            crate::storage::repositories::plan_initial_detail_jobs(&tx, "dataset-1", "match-1")
                .unwrap();
            tx.commit().unwrap();
        }

        let repo = RepositoryUnitOfWork::new(&conn);
        repo.save_detail_section(
            "match-1",
            "dataset-1",
            "overview",
            &json!({}),
            false,
            "hash",
            None,
        )
        .unwrap();

        conn.execute("DELETE FROM detail_jobs", []).unwrap();

        // This plans missing sections again, but should not add completed overview section
        let tx = conn.unchecked_transaction().unwrap();
        crate::storage::repositories::plan_initial_detail_jobs(&tx, "dataset-1", "match-1")
            .unwrap();
        tx.commit().unwrap();

        let count_overview: i64 = conn.query_row(
            "SELECT COUNT(*) FROM detail_jobs WHERE match_id='match-1' AND section_name='overview'",
            [],
            |r| r.get(0)
        ).unwrap();
        assert_eq!(count_overview, 0);
    }

    #[test]
    fn source_url_in_memory_only() {
        let payload = json!({
            "matchId": "match-1",
            "name": "Arsenal vs Chelsea",
            "logo": "https://img.aiscore.com/team/logo/arsenal.png",
            "h2h": {},
            "odds": {},
            "lineups": {
                "home": [
                    { "id": "p1", "avatar": "https://img.aiscore.com/player/avatar.png" }
                ],
                "away": []
            },
            "stats": {},
            "incidents": []
        });

        let (_extracted_overview, _candidates) =
            extractor::extract_section_data(1, DetailSection::Overview, &payload);
        let (extracted_lineups, candidates_lineups) =
            extractor::extract_section_data(1, DetailSection::Lineups, &payload);

        assert_eq!(candidates_lineups.len(), 1);
        assert_eq!(
            candidates_lineups[0].url,
            "https://img.aiscore.com/player/avatar.png"
        );
        assert_eq!(candidates_lineups[0].entity_type, "lineups");

        let avatar_val = extracted_lineups["home"][0]["avatar"].as_str().unwrap();
        assert!(avatar_val.starts_with("asset-"));
        assert!(!avatar_val.contains("https://"));
    }
}
