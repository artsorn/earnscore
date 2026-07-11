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

INSERT OR IGNORE INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo)
VALUES ('legacy-comp', 1, 'Premier League', 'logo.png', 'premier-league', 'England', 'flag.png');

INSERT OR IGNORE INTO teams (id, sport_id, name, logo, slug)
VALUES ('legacy-team', 1, 'Arsenal', 'logo.png', 'arsenal');

INSERT OR IGNORE INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, updated_at)
VALUES ('legacy-match', 1, 'legacy-comp', 'legacy-team', 'legacy-team', 1700000000, 1, '[]', '[]', '2026-07-10 00:00:00');

INSERT OR IGNORE INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated)
VALUES ('legacy-match', 1, '[]', '{}', '{}', '{}', '{}', 1700000000);

-- A second row makes preservation checks cover a terminal match and a
-- parseable legacy odds record without relying on a source URL.
INSERT OR IGNORE INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, updated_at)
VALUES ('legacy-finished-match', 1, 'legacy-comp', 'legacy-team', 'legacy-team', 1700000100, 8, '[2]', '[0]', '2026-07-10 00:01:00');

INSERT OR IGNORE INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated)
VALUES (
    'legacy-finished-match',
    1,
    '{"events":[]}',
    '{bad-json',
    '{}',
    '[{"bookmaker_id":"legacy-book","market_type":"1x2","period":"FULL","selection_key":"home","line_value":"","odds_value":"1.80"}]',
    '{"historical":[]}',
    1700000100
);

-- Migration assertions to run after 0001, 0002 and 0003:
-- 1. SELECT id FROM matches WHERE id IN ('legacy-match', 'legacy-finished-match');
-- 2. SELECT COUNT(*) FROM match_details WHERE match_id IN ('legacy-match', 'legacy-finished-match');
-- 3. SELECT section_name, status FROM match_detail_sections WHERE match_id='legacy-finished-match' ORDER BY section_name;
-- 4. SELECT odds_value, provenance FROM odds_history WHERE match_id='legacy-finished-match';
