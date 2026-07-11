-- Migration 0002_dataset_generation.sql
CREATE TABLE IF NOT EXISTS datasets (
    dataset_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    generation_order INTEGER NOT NULL,
    schema_generation INTEGER NOT NULL DEFAULT 2
);

-- Insert the legacy dataset ID so it exists in registry
INSERT OR IGNORE INTO datasets (dataset_id, created_at, generation_order, schema_generation)
VALUES ('legacy-dataset-id', datetime('now'), 0, 2);

CREATE TABLE IF NOT EXISTS dataset_sports (
    dataset_id TEXT NOT NULL,
    sport_id INTEGER NOT NULL CHECK (sport_id IN (1, 2)),
    captured_at TEXT NOT NULL,
    synced_at TEXT NOT NULL,
    PRIMARY KEY (dataset_id, sport_id)
);

-- competitions table
CREATE TABLE IF NOT EXISTS competitions_new (
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

INSERT INTO competitions_new (id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at, dataset_id)
SELECT id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at, 'legacy-dataset-id'
FROM competitions;

DROP TABLE competitions;
ALTER TABLE competitions_new RENAME TO competitions;

-- teams table
CREATE TABLE IF NOT EXISTS teams_new (
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

INSERT INTO teams_new (id, sport_id, name, logo, slug, raw_payload, synced, updated_at, dataset_id)
SELECT id, sport_id, name, logo, slug, raw_payload, synced, updated_at, 'legacy-dataset-id'
FROM teams;

DROP TABLE teams;
ALTER TABLE teams_new RENAME TO teams;

-- matches table
CREATE TABLE IF NOT EXISTS matches_new (
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

INSERT INTO matches_new (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload, synced, updated_at, dataset_id)
SELECT id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores,
       CASE
         WHEN sport_id = 1 AND status_id IN (2, 3, 4, 5, 6, 7) THEN 1
         WHEN sport_id = 2 AND status_id IN (2, 3, 4, 5, 6, 7, 9) THEN 1
         ELSE 0
       END,
       raw_payload, synced, updated_at, 'legacy-dataset-id'
FROM matches;

DROP TABLE matches;
ALTER TABLE matches_new RENAME TO matches;

-- match_details table
CREATE TABLE IF NOT EXISTS match_details_new (
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

INSERT INTO match_details_new (match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, synced, last_updated, updated_at, dataset_id)
SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, synced, last_updated, updated_at, 'legacy-dataset-id'
FROM match_details;

DROP TABLE match_details;
ALTER TABLE match_details_new RENAME TO match_details;

-- Recreate indexes
DROP INDEX IF EXISTS idx_matches_list;
DROP INDEX IF EXISTS idx_matches_competition;
DROP INDEX IF EXISTS idx_matches_home_team;
DROP INDEX IF EXISTS idx_matches_away_team;
DROP INDEX IF EXISTS idx_match_details_lookup;
DROP INDEX IF EXISTS idx_competitions_sport;
DROP INDEX IF EXISTS idx_teams_sport;

CREATE INDEX IF NOT EXISTS idx_matches_list ON matches (dataset_id, is_live, sport_id, status_id, match_time);
CREATE INDEX IF NOT EXISTS idx_matches_competition ON matches (dataset_id, competition_id);
CREATE INDEX IF NOT EXISTS idx_matches_home_team ON matches (dataset_id, home_team_id);
CREATE INDEX IF NOT EXISTS idx_matches_away_team ON matches (dataset_id, away_team_id);
CREATE INDEX IF NOT EXISTS idx_match_details_lookup ON match_details (dataset_id, match_id, sport_id);
CREATE INDEX IF NOT EXISTS idx_competitions_sport ON competitions (dataset_id, sport_id);
CREATE INDEX IF NOT EXISTS idx_teams_sport ON teams (dataset_id, sport_id);
CREATE INDEX IF NOT EXISTS idx_dataset_sports_ready ON dataset_sports (dataset_id, sport_id);
