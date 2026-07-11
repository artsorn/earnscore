use rand::Rng;
use std::time::Duration;

/// Sections that must be refreshed once a previously-live match finishes
/// while the feed is offline.  `period_scores` is materialized from the
/// append-only state history by the recovery finalizer.
pub const FINAL_DETAIL_SECTIONS: &[&str] = &["overview", "odds", "stats", "incidents", "lineups"];

pub const MANUAL_DETAIL_SECTIONS: &[&str] = FINAL_DETAIL_SECTIONS;

pub fn plan_final_sections(
    conn: &rusqlite::Connection,
    match_id: &str,
    dataset_id: &str,
) -> rusqlite::Result<usize> {
    crate::recovery::finalizer::Finalizer::plan_required_sections(
        conn, match_id, dataset_id, "FINAL",
    )
}

pub fn plan_manual_sections(
    conn: &rusqlite::Connection,
    match_id: &str,
    dataset_id: &str,
) -> rusqlite::Result<usize> {
    crate::recovery::finalizer::Finalizer::plan_required_sections(
        conn, match_id, dataset_id, "MANUAL",
    )
}

/// Calculate the exponential backoff delay with jitter (±15%).
pub fn calculate_retry_delay(
    attempt_count: i32,
    base_delay_secs: u64,
    max_delay_secs: u64,
) -> Duration {
    if attempt_count <= 0 {
        return Duration::from_secs(base_delay_secs);
    }
    let factor = 2_u64.pow(attempt_count.saturating_sub(1) as u32);
    let delay_secs = base_delay_secs.saturating_mul(factor).min(max_delay_secs);

    let mut rng = rand::thread_rng();
    let jitter_pct: f64 = rng.gen_range(-0.15..0.15);
    let delayed = (delay_secs as f64) * (1.0 + jitter_pct);
    Duration::from_secs_f64(delayed.max(1.0))
}

// NOTE: The section completion barrier integration for Task 05 is handled directly
// inside the DetailCoordinator's execute_job / fetch_and_save loops. It blocks on
// downloading and processing all extracted image candidates, and only persists
// the detail section once all assets have been stored or determined unavailable.
