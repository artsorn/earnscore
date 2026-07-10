CREATE TABLE IF NOT EXISTS competitions (
    id TEXT PRIMARY KEY,
    sport_id INTEGER,
    name TEXT,
    logo TEXT,
    slug TEXT,
    country_name TEXT,
    country_logo TEXT
);

CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    sport_id INTEGER,
    name TEXT,
    logo TEXT,
    slug TEXT
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
    last_updated INTEGER
);

INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5');
INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token');
