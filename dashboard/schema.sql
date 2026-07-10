CREATE TABLE IF NOT EXISTS competitions (
    id TEXT PRIMARY KEY,
    sport_id INTEGER,
    name TEXT,
    logo TEXT,
    slug TEXT,
    country_name TEXT,
    country_logo TEXT,
    raw_payload TEXT,
    synced INTEGER DEFAULT 0,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    sport_id INTEGER,
    name TEXT,
    logo TEXT,
    slug TEXT,
    raw_payload TEXT,
    synced INTEGER DEFAULT 0,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS matches (
    id TEXT PRIMARY KEY,
    sport_id INTEGER,
    competition_id TEXT,
    home_team_id TEXT,
    away_team_id TEXT,
    match_time INTEGER,
    status_id INTEGER,
    home_scores TEXT,
    away_scores TEXT,
    raw_payload TEXT,
    synced INTEGER DEFAULT 0,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT
);

CREATE TABLE IF NOT EXISTS match_details (
    match_id TEXT PRIMARY KEY,
    sport_id INTEGER,
    incidents TEXT,
    stats TEXT,
    lineups TEXT,
    odds TEXT,
    h2h TEXT,
    raw_payload TEXT,
    synced INTEGER DEFAULT 0,
    last_updated INTEGER,
    updated_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_matches_list ON matches (sport_id, status_id, match_time);
CREATE INDEX IF NOT EXISTS idx_matches_competition ON matches (competition_id);
CREATE INDEX IF NOT EXISTS idx_matches_home_team ON matches (home_team_id);
CREATE INDEX IF NOT EXISTS idx_matches_away_team ON matches (away_team_id);
CREATE INDEX IF NOT EXISTS idx_match_details_lookup ON match_details (match_id, sport_id);
CREATE INDEX IF NOT EXISTS idx_competitions_sport ON competitions (sport_id);
CREATE INDEX IF NOT EXISTS idx_teams_sport ON teams (sport_id);

INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5');
INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token');
INSERT OR IGNORE INTO settings (key, value) VALUES ('detail_update_interval_secs', '60');
INSERT OR IGNORE INTO settings (key, value) VALUES ('cf_worker_url', 'http://127.0.0.1:8080');
