-- Migration 0003_event_driven_live.sql
-- Additive v3 objects and deterministic legacy conversion. This file uses
-- create/insert/update operations only; historical migrations are untouched.
-- EarnScore D1 v3 bootstrap.
-- Historical migrations 0001 and 0002 are intentionally not copied or rewritten.
-- Asset rows contain local storage keys and content hashes; no source image URL
-- is part of the v3 asset contract.

CREATE TABLE IF NOT EXISTS datasets (
    dataset_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    generation_order INTEGER NOT NULL DEFAULT 0,
    schema_generation INTEGER NOT NULL DEFAULT 3
);

INSERT OR IGNORE INTO datasets (dataset_id, created_at, generation_order, schema_generation)
VALUES ('legacy-dataset-id', datetime('now'), 0, 3);

UPDATE datasets SET schema_generation = 3
WHERE schema_generation IS NULL OR schema_generation < 3;

CREATE TABLE IF NOT EXISTS dataset_sports (
    dataset_id TEXT NOT NULL,
    sport_id INTEGER NOT NULL CHECK (sport_id IN (1, 2)),
    captured_at TEXT NOT NULL,
    synced_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (dataset_id, sport_id)
);

-- 0002 owns the original dataset_sports contract.  Keep this alteration in
-- 0003 so both a fresh bootstrap (followed by 0001/0002) and an existing D1
-- database converge on the v3 projection flag without rewriting history.
ALTER TABLE dataset_sports ADD COLUMN synced INTEGER NOT NULL DEFAULT 0;

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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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
    dataset_id TEXT NOT NULL,
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

INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5');
INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token');
INSERT OR IGNORE INTO settings (key, value) VALUES ('detail_update_interval_secs', '60');
INSERT OR IGNORE INTO settings (key, value) VALUES ('cf_worker_url', 'http://127.0.0.1:8080');

INSERT OR IGNORE INTO migration_audit
    (migration_id, version, status, started_at, completed_at, report_json)
VALUES
    ('d1-v3-bootstrap', 3, 'COMPLETED', datetime('now'), datetime('now'), '{}');



-- Convert the legacy JSON detail columns without removing the source row.
-- Invalid JSON is represented as UNPARSEABLE with a reportable error; it is
-- never coerced into fabricated structured data.
WITH legacy_sections AS (
    SELECT match_id, dataset_id, 'incidents' AS section_name, incidents AS section_value, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'stats', stats, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'lineups', lineups, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'odds', odds, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'h2h', h2h, last_updated FROM match_details
)
INSERT OR IGNORE INTO match_detail_sections
    (match_id, dataset_id, section_name, status, provenance, is_empty,
     is_unparseable, content_hash, source_timestamp, received_at,
     completed_at, last_error)
SELECT
    match_id,
    dataset_id,
    section_name,
    CASE
        WHEN section_value IS NULL OR trim(section_value) = ''
             OR trim(section_value) IN ('{}', '[]', 'null') THEN 'EMPTY'
        WHEN json_valid(section_value) = 1 THEN 'COMPLETED'
        ELSE 'UNPARSEABLE'
    END,
    'legacy:match_details.' || section_name,
    CASE
        WHEN section_value IS NULL OR trim(section_value) = ''
             OR trim(section_value) IN ('{}', '[]', 'null') THEN 1
        ELSE 0
    END,
    CASE
        WHEN section_value IS NULL OR trim(section_value) = ''
             OR trim(section_value) IN ('{}', '[]', 'null') THEN 0
        WHEN json_valid(section_value) = 1 THEN 0
        ELSE 1
    END,
    lower(hex(CAST(coalesce(section_value, '') AS BLOB))),
    CAST(last_updated AS TEXT),
    datetime('now'),
    CASE WHEN json_valid(section_value) = 1
              AND trim(section_value) NOT IN ('', '{}', '[]', 'null')
         THEN datetime('now') ELSE NULL END,
    CASE
        WHEN section_value IS NULL OR trim(section_value) = ''
             OR trim(section_value) IN ('{}', '[]', 'null') THEN NULL
        WHEN json_valid(section_value) = 1 THEN NULL
        ELSE 'legacy ' || section_name || ' is not valid JSON'
    END
FROM legacy_sections;

WITH legacy_sections AS (
    SELECT match_id, dataset_id, 'incidents' AS section_name, incidents AS section_value, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'stats', stats, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'lineups', lineups, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'odds', odds, last_updated FROM match_details
    UNION ALL
    SELECT match_id, dataset_id, 'h2h', h2h, last_updated FROM match_details
)
INSERT OR IGNORE INTO match_detail_data
    (match_id, dataset_id, section_name, data_json, provenance,
     content_hash, source_timestamp, received_at)
SELECT
    match_id,
    dataset_id,
    section_name,
    CASE
        WHEN section_value IS NULL OR trim(section_value) = '' THEN 'null'
        WHEN json_valid(section_value) = 1 THEN section_value
        ELSE NULL
    END,
    'legacy:match_details.' || section_name,
    lower(hex(CAST(coalesce(section_value, '') AS BLOB))),
    CAST(last_updated AS TEXT),
    datetime('now')
FROM legacy_sections;

INSERT OR IGNORE INTO match_state_history
    (match_id, dataset_id, sport_id, state, status_id, home_scores,
     away_scores, source_timestamp, received_at, payload_hash, provenance)
SELECT
    id,
    dataset_id,
    sport_id,
    CASE
        WHEN status_id = 8 THEN 'FINISHED'
        WHEN is_live = 1 THEN 'LIVE'
        ELSE 'LEGACY_IMPORTED'
    END,
    status_id,
    home_scores,
    away_scores,
    coalesce(updated_at, ''),
    datetime('now'),
    lower(hex(CAST(
        id || '|' || coalesce(dataset_id, '') || '|' ||
        coalesce(status_id, 0) || '|' ||
        coalesce(home_scores, '') || '|' || coalesce(away_scores, '')
        AS BLOB))),
    'legacy:matches'
FROM matches;

-- Convert canonical array/object records only when all identifying fields
-- are present. Other legacy odds remain in match_detail_data with an explicit
-- section status and are listed by the local conversion report.
WITH legacy_odds AS (
    SELECT
        d.match_id,
        d.dataset_id,
        CAST(d.last_updated AS TEXT) AS source_timestamp,
        json_extract(item.value, '$.bookmaker_id') AS bookmaker_id,
        json_extract(item.value, '$.market_type') AS market_type,
        coalesce(json_extract(item.value, '$.period'), '') AS period,
        json_extract(item.value, '$.selection_key') AS selection_key,
        coalesce(json_extract(item.value, '$.line_value'), '') AS line_value,
        CAST(json_extract(item.value, '$.odds_value') AS TEXT) AS odds_value,
        item.value AS payload_json
    FROM match_details AS d
    JOIN json_each(
        CASE
            WHEN json_valid(d.odds) = 1 AND json_type(d.odds) = 'array'
                THEN d.odds
            WHEN json_valid(d.odds) = 1 AND json_type(d.odds) = 'object'
                THEN json_array(json(d.odds))
            ELSE json('[]')
        END
    ) AS item
    WHERE json_valid(d.odds) = 1
)
INSERT OR IGNORE INTO odds_history
    (match_id, dataset_id, bookmaker_id, market_type, period, selection_key,
     line_value, odds_value, previous_odds_value, is_live, source_timestamp,
     received_at, payload_hash, provenance, event_key)
SELECT
    match_id,
    dataset_id,
    CAST(bookmaker_id AS TEXT),
    CAST(market_type AS TEXT),
    CAST(period AS TEXT),
    CAST(selection_key AS TEXT),
    CAST(line_value AS TEXT),
    odds_value,
    NULL,
    0,
    coalesce(source_timestamp, ''),
    datetime('now'),
    lower(hex(CAST(payload_json AS BLOB))),
    'legacy:match_details.odds',
    'legacy:' || match_id || ':' || dataset_id || ':' ||
        CAST(bookmaker_id AS TEXT) || ':' || CAST(market_type AS TEXT) || ':' ||
        CAST(period AS TEXT) || ':' || CAST(selection_key AS TEXT) || ':' ||
        CAST(line_value AS TEXT) || ':' || lower(hex(CAST(payload_json AS BLOB)))
FROM legacy_odds
WHERE bookmaker_id IS NOT NULL
  AND market_type IS NOT NULL
  AND selection_key IS NOT NULL
  AND odds_value IS NOT NULL;

WITH legacy_odds AS (
    SELECT
        d.match_id,
        d.dataset_id,
        CAST(d.last_updated AS TEXT) AS source_timestamp,
        json_extract(item.value, '$.bookmaker_id') AS bookmaker_id,
        json_extract(item.value, '$.market_type') AS market_type,
        coalesce(json_extract(item.value, '$.period'), '') AS period,
        json_extract(item.value, '$.selection_key') AS selection_key,
        coalesce(json_extract(item.value, '$.line_value'), '') AS line_value,
        CAST(json_extract(item.value, '$.odds_value') AS TEXT) AS odds_value,
        item.value AS payload_json
    FROM match_details AS d
    JOIN json_each(
        CASE
            WHEN json_valid(d.odds) = 1 AND json_type(d.odds) = 'array'
                THEN d.odds
            WHEN json_valid(d.odds) = 1 AND json_type(d.odds) = 'object'
                THEN json_array(json(d.odds))
            ELSE json('[]')
        END
    ) AS item
    WHERE json_valid(d.odds) = 1
)
INSERT OR REPLACE INTO odds_current
    (match_id, dataset_id, bookmaker_id, market_type, period, selection_key,
     line_value, odds_value, is_live, source_timestamp, received_at,
     payload_hash, provenance)
SELECT
    match_id,
    dataset_id,
    CAST(bookmaker_id AS TEXT),
    CAST(market_type AS TEXT),
    CAST(period AS TEXT),
    CAST(selection_key AS TEXT),
    CAST(line_value AS TEXT),
    odds_value,
    0,
    coalesce(source_timestamp, ''),
    datetime('now'),
    lower(hex(CAST(payload_json AS BLOB))),
    'legacy:match_details.odds'
FROM legacy_odds
WHERE bookmaker_id IS NOT NULL
  AND market_type IS NOT NULL
  AND selection_key IS NOT NULL
  AND odds_value IS NOT NULL;

INSERT OR REPLACE INTO migration_audit
    (migration_id, version, status, started_at, completed_at, report_json)
VALUES (
    'd1-v3-legacy-conversion',
    3,
    'COMPLETED',
    datetime('now'),
    datetime('now'),
    json_object(
        'matches_preserved', (SELECT count(*) FROM matches),
        'details_seen', (SELECT count(*) FROM match_details),
        'sections', (SELECT count(*) FROM match_detail_sections),
        'unparseable_sections',
            (SELECT count(*) FROM match_detail_sections WHERE is_unparseable = 1),
        'odds_converted', (SELECT count(*) FROM odds_history),
        'unmappable_fields', COALESCE(
            (SELECT json_group_array(match_id || ':' || section_name)
             FROM match_detail_sections WHERE is_unparseable = 1),
            json('[]')
        )
    )
);
