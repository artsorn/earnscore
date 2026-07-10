-- Migration from legacy schema to full payload schema

-- Alter tables to add new columns
ALTER TABLE competitions ADD COLUMN raw_payload TEXT;
ALTER TABLE competitions ADD COLUMN synced INTEGER DEFAULT 0;
ALTER TABLE competitions ADD COLUMN updated_at TEXT;

ALTER TABLE teams ADD COLUMN raw_payload TEXT;
ALTER TABLE teams ADD COLUMN synced INTEGER DEFAULT 0;
ALTER TABLE teams ADD COLUMN updated_at TEXT;

ALTER TABLE matches ADD COLUMN raw_payload TEXT;
ALTER TABLE matches ADD COLUMN synced INTEGER DEFAULT 0;

ALTER TABLE match_details ADD COLUMN raw_payload TEXT;
ALTER TABLE match_details ADD COLUMN synced INTEGER DEFAULT 0;
ALTER TABLE match_details ADD COLUMN updated_at TEXT;

-- Create indexes for performance optimization
CREATE INDEX IF NOT EXISTS idx_matches_list ON matches (sport_id, status_id, match_time);
CREATE INDEX IF NOT EXISTS idx_matches_competition ON matches (competition_id);
CREATE INDEX IF NOT EXISTS idx_matches_home_team ON matches (home_team_id);
CREATE INDEX IF NOT EXISTS idx_matches_away_team ON matches (away_team_id);
CREATE INDEX IF NOT EXISTS idx_match_details_lookup ON match_details (match_id, sport_id);
CREATE INDEX IF NOT EXISTS idx_competitions_sport ON competitions (sport_id);
CREATE INDEX IF NOT EXISTS idx_teams_sport ON teams (sport_id);
