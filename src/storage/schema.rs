use rusqlite::{Connection, Result as SqlResult};

/// Logical schema version for the local SQLite store and the D1 bootstrap.
pub const V3_SCHEMA_VERSION: i64 = 3;
pub const LEGACY_DATASET_ID: &str = "legacy-dataset-id";

/// Create the additive v3 contract.  Existing tables are deliberately kept
/// compatible with the sync protocol from generations 1 and 2; the event,
/// detail, job, asset and outbox tables are append-only additions.
pub fn create_v3_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS datasets (
            dataset_id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            generation_order INTEGER NOT NULL DEFAULT 0,
            schema_generation INTEGER NOT NULL DEFAULT 3
        );

        CREATE TABLE IF NOT EXISTS dataset_sports (
            dataset_id TEXT NOT NULL,
            sport_id INTEGER NOT NULL CHECK (sport_id IN (1, 2)),
            captured_at TEXT NOT NULL,
            synced INTEGER NOT NULL DEFAULT 0,
            synced_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (dataset_id, sport_id)
        );

        CREATE TABLE IF NOT EXISTS sports (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            slug TEXT,
            asset_id TEXT,
            metadata_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS competitions (
            id TEXT,
            sport_id INTEGER,
            name TEXT,
            logo TEXT,
            slug TEXT,
            country_name TEXT,
            country_logo TEXT,
            raw_payload TEXT,
            synced INTEGER DEFAULT 0,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS teams (
            id TEXT,
            sport_id INTEGER,
            name TEXT,
            logo TEXT,
            slug TEXT,
            raw_payload TEXT,
            synced INTEGER DEFAULT 0,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS players (
            id TEXT NOT NULL,
            sport_id INTEGER,
            team_id TEXT,
            name TEXT,
            position TEXT,
            shirt_number TEXT,
            asset_id TEXT,
            raw_payload TEXT,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS coaches (
            id TEXT NOT NULL,
            sport_id INTEGER,
            team_id TEXT,
            name TEXT,
            asset_id TEXT,
            raw_payload TEXT,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS venues (
            id TEXT NOT NULL,
            sport_id INTEGER,
            name TEXT,
            city TEXT,
            country TEXT,
            asset_id TEXT,
            raw_payload TEXT,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS referees (
            id TEXT NOT NULL,
            sport_id INTEGER,
            name TEXT,
            role TEXT,
            asset_id TEXT,
            raw_payload TEXT,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS matches (
            id TEXT,
            sport_id INTEGER,
            competition_id TEXT,
            home_team_id TEXT,
            away_team_id TEXT,
            match_time INTEGER,
            status_id INTEGER,
            home_scores TEXT,
            away_scores TEXT,
            is_live INTEGER NOT NULL DEFAULT 0,
            raw_payload TEXT,
            synced INTEGER DEFAULT 0,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS match_state_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            sport_id INTEGER,
            state TEXT NOT NULL,
            status_id INTEGER,
            home_scores TEXT,
            away_scores TEXT,
            period TEXT,
            clock TEXT,
            source_timestamp TEXT NOT NULL DEFAULT '',
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            payload_hash TEXT NOT NULL DEFAULT '',
            provenance TEXT NOT NULL DEFAULT 'feed',
            UNIQUE (match_id, dataset_id, state, source_timestamp, payload_hash)
        );

        CREATE TABLE IF NOT EXISTS odds_current (
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            bookmaker_id TEXT NOT NULL,
            market_type TEXT NOT NULL,
            period TEXT NOT NULL DEFAULT '',
            selection_key TEXT NOT NULL,
            line_value TEXT NOT NULL DEFAULT '',
            odds_value TEXT NOT NULL,
            is_live INTEGER NOT NULL DEFAULT 0,
            source_timestamp TEXT NOT NULL DEFAULT '',
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            payload_hash TEXT NOT NULL DEFAULT '',
            provenance TEXT NOT NULL DEFAULT 'feed',
            PRIMARY KEY (
                match_id, dataset_id, bookmaker_id, market_type,
                period, selection_key, line_value
            )
        );

        CREATE TABLE IF NOT EXISTS odds_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            bookmaker_id TEXT NOT NULL,
            market_type TEXT NOT NULL,
            period TEXT NOT NULL DEFAULT '',
            selection_key TEXT NOT NULL,
            line_value TEXT NOT NULL DEFAULT '',
            odds_value TEXT NOT NULL,
            previous_odds_value TEXT,
            is_live INTEGER NOT NULL DEFAULT 0,
            source_timestamp TEXT NOT NULL DEFAULT '',
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            payload_hash TEXT NOT NULL DEFAULT '',
            provenance TEXT NOT NULL DEFAULT 'feed',
            event_key TEXT NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS match_details (
            match_id TEXT,
            sport_id INTEGER,
            incidents TEXT,
            stats TEXT,
            lineups TEXT,
            odds TEXT,
            h2h TEXT,
            raw_payload TEXT,
            synced INTEGER DEFAULT 0,
            last_updated INTEGER,
            updated_at TEXT,
            dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id',
            PRIMARY KEY (match_id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS match_detail_sections (
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            section_name TEXT NOT NULL,
            status TEXT NOT NULL,
            provenance TEXT NOT NULL,
            is_empty INTEGER NOT NULL DEFAULT 0,
            is_unparseable INTEGER NOT NULL DEFAULT 0,
            content_hash TEXT NOT NULL DEFAULT '',
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            completed_at TEXT,
            last_error TEXT,
            PRIMARY KEY (match_id, dataset_id, section_name)
        );

        CREATE TABLE IF NOT EXISTS match_detail_data (
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            section_name TEXT NOT NULL,
            data_json TEXT,
            provenance TEXT NOT NULL,
            content_hash TEXT NOT NULL DEFAULT '',
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (match_id, dataset_id, section_name)
        );

        CREATE TABLE IF NOT EXISTS match_incidents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            incident_key TEXT NOT NULL,
            incident_json TEXT NOT NULL,
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            provenance TEXT NOT NULL DEFAULT 'feed',
            UNIQUE (match_id, dataset_id, incident_key)
        );

        CREATE TABLE IF NOT EXISTS match_statistics (
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            period TEXT NOT NULL DEFAULT '',
            stat_key TEXT NOT NULL,
            side TEXT NOT NULL DEFAULT '',
            value_json TEXT,
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            provenance TEXT NOT NULL DEFAULT 'detail',
            PRIMARY KEY (match_id, dataset_id, period, stat_key, side)
        );

        CREATE TABLE IF NOT EXISTS match_lineups (
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            team_side TEXT NOT NULL,
            player_id TEXT NOT NULL,
            coach_id TEXT,
            position TEXT,
            shirt_number TEXT,
            starter INTEGER,
            lineup_json TEXT,
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            provenance TEXT NOT NULL DEFAULT 'detail',
            PRIMARY KEY (match_id, dataset_id, team_side, player_id)
        );

        CREATE TABLE IF NOT EXISTS match_h2h (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            related_match_id TEXT,
            home_name TEXT,
            away_name TEXT,
            played_at TEXT,
            result_json TEXT,
            is_canonical INTEGER NOT NULL DEFAULT 0,
            provenance TEXT NOT NULL DEFAULT 'detail',
            content_hash TEXT NOT NULL DEFAULT '',
            UNIQUE (match_id, dataset_id, related_match_id, content_hash)
        );

        CREATE TABLE IF NOT EXISTS match_h2h_references (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            reference_key TEXT NOT NULL,
            reference_json TEXT NOT NULL,
            provenance TEXT NOT NULL DEFAULT 'legacy',
            content_hash TEXT NOT NULL DEFAULT '',
            UNIQUE (match_id, dataset_id, reference_key, content_hash)
        );

        CREATE TABLE IF NOT EXISTS assets (
            asset_id TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL UNIQUE,
            storage_key TEXT NOT NULL,
            mime_type TEXT,
            byte_size INTEGER,
            width INTEGER,
            height INTEGER,
            status TEXT NOT NULL DEFAULT 'READY',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            provenance TEXT NOT NULL DEFAULT 'local'
        );

        CREATE TABLE IF NOT EXISTS asset_links (
            asset_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            role TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (asset_id, dataset_id, entity_type, entity_id, role)
        );

        CREATE TABLE IF NOT EXISTS feed_sessions (
            session_id TEXT PRIMARY KEY,
            sport_id INTEGER NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            status TEXT NOT NULL,
            source_page TEXT,
            filter_name TEXT,
            last_heartbeat_at TEXT,
            metadata_json TEXT
        );

        CREATE TABLE IF NOT EXISTS feed_events (
            event_id TEXT PRIMARY KEY,
            event_key TEXT NOT NULL UNIQUE,
            source_event_id TEXT,
            session_id TEXT,
            match_id TEXT,
            dataset_id TEXT,
            sport_id INTEGER,
            event_type TEXT NOT NULL,
            source_timestamp TEXT,
            received_at TEXT NOT NULL DEFAULT (datetime('now')),
            payload_hash TEXT NOT NULL DEFAULT '',
            payload_json TEXT NOT NULL,
            processed_at TEXT,
            processing_error TEXT
        );

        CREATE TABLE IF NOT EXISTS detail_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            section_name TEXT NOT NULL,
            load_phase TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'PENDING',
            scheduled_at TEXT NOT NULL DEFAULT (datetime('now')),
            started_at TEXT,
            completed_at TEXT,
            lease_owner TEXT,
            lease_expires_at TEXT,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            UNIQUE (match_id, dataset_id, section_name, load_phase)
        );

        CREATE TABLE IF NOT EXISTS asset_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            asset_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'PENDING',
            scheduled_at TEXT NOT NULL DEFAULT (datetime('now')),
            started_at TEXT,
            completed_at TEXT,
            lease_owner TEXT,
            lease_expires_at TEXT,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            UNIQUE (asset_id, dataset_id)
        );

        CREATE TABLE IF NOT EXISTS recovery_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            match_id TEXT NOT NULL,
            dataset_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'PENDING',
            previous_feed_session_id TEXT,
            scheduled_at TEXT NOT NULL DEFAULT (datetime('now')),
            started_at TEXT,
            completed_at TEXT,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            UNIQUE (match_id, dataset_id, reason, status)
        );

        CREATE TABLE IF NOT EXISTS sync_outbox (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset_id TEXT,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            claimed_at TEXT,
            claimed_by TEXT,
            sent_at TEXT,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            UNIQUE (dataset_id, entity_type, entity_id, event_type, payload_json)
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS migration_audit (
            migration_id TEXT PRIMARY KEY,
            version INTEGER NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            report_json TEXT NOT NULL DEFAULT '{}',
            error_text TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_matches_list
            ON matches (dataset_id, is_live, sport_id, status_id, match_time);
        CREATE INDEX IF NOT EXISTS idx_matches_competition
            ON matches (dataset_id, competition_id);
        CREATE INDEX IF NOT EXISTS idx_matches_home_team
            ON matches (dataset_id, home_team_id);
        CREATE INDEX IF NOT EXISTS idx_matches_away_team
            ON matches (dataset_id, away_team_id);
        CREATE INDEX IF NOT EXISTS idx_match_details_lookup
            ON match_details (dataset_id, match_id, sport_id);
        CREATE INDEX IF NOT EXISTS idx_competitions_sport
            ON competitions (dataset_id, sport_id);
        CREATE INDEX IF NOT EXISTS idx_teams_sport
            ON teams (dataset_id, sport_id);
        CREATE INDEX IF NOT EXISTS idx_dataset_sports_ready
            ON dataset_sports (dataset_id, sport_id, captured_at);
        CREATE INDEX IF NOT EXISTS idx_state_history_match
            ON match_state_history (dataset_id, match_id, received_at);
        CREATE INDEX IF NOT EXISTS idx_odds_current_match
            ON odds_current (dataset_id, match_id);
        CREATE INDEX IF NOT EXISTS idx_odds_history_match
            ON odds_history (dataset_id, match_id, received_at);
        CREATE INDEX IF NOT EXISTS idx_detail_sections_status
            ON match_detail_sections (dataset_id, match_id, status);
        CREATE INDEX IF NOT EXISTS idx_feed_events_session
            ON feed_events (session_id, received_at);
        CREATE INDEX IF NOT EXISTS idx_feed_events_match
            ON feed_events (dataset_id, match_id, received_at);
        CREATE INDEX IF NOT EXISTS idx_detail_jobs_ready
            ON detail_jobs (status, scheduled_at, lease_expires_at);
        CREATE INDEX IF NOT EXISTS idx_asset_jobs_ready
            ON asset_jobs (status, scheduled_at, lease_expires_at);
        CREATE INDEX IF NOT EXISTS idx_recovery_jobs_ready
            ON recovery_jobs (status, scheduled_at);
        CREATE INDEX IF NOT EXISTS idx_sync_outbox_ready
            ON sync_outbox (sent_at, claimed_at, created_at);
        "#,
    )?;

    Ok(())
}
