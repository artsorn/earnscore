# EarnScore v3 database ER model

The diagrams use the public identifiers used by both SQLite and D1. The
dataset_id column scopes local generations; the source match id is never
rewritten.

## Canonical and event data

    datasets 1 --- n dataset_sports
       |
       +--- n competitions
       +--- n teams
       +--- n matches
                       |
                       +--- n match_state_history
                       +--- n odds_current
                       +--- n odds_history
                       +--- 1 match_details
                                      |
                                      +--- n match_detail_sections
                                      +--- n match_detail_data
                                      +--- n match_incidents
                                      +--- n match_statistics
                                      +--- n match_lineups
                                      +--- n match_h2h
                                      +--- n match_h2h_references

Canonical entities include sports, competitions, teams, players, coaches,
venues and referees. Their raw payload is optional compatibility data; the
new normalized detail and event tables carry provenance and hashes.

## Sessions, jobs, assets and sync

    feed_sessions 1 --- n feed_events
    feed_events   n --- 1 matches (logical match_id + dataset_id)

    matches 1 --- n detail_jobs
    matches 1 --- n recovery_jobs
    detail_jobs n --- 1 match_detail_sections

    assets 1 --- n asset_links
    asset_jobs n --- 1 assets

    sync_outbox --> D1/R2 sync boundary
    settings    --> runtime configuration
    migration_audit --> schema/conversion evidence

feed_events.event_key is unique. detail_jobs is unique on
match_id/dataset_id/section_name/load_phase. odds_current is keyed by
match/bookmaker/market/period/selection/line, while odds_history.event_key
deduplicates replayed odds events.

The assets table stores asset_id, content_hash, storage_key, MIME/size and
processing status. Asset links point from a local asset to an entity and role;
there is no new source-image URL column.

