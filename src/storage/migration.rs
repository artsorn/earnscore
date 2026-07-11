use super::schema::{self, LEGACY_DATASET_ID, V3_SCHEMA_VERSION};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Result as SqlResult, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const MIGRATION_ID: &str = "sqlite-v3-event-driven";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyConversionReport {
    pub matches_seen: usize,
    pub matches_preserved: usize,
    pub details_seen: usize,
    pub sections_converted: usize,
    pub empty_sections: usize,
    pub unparseable_sections: usize,
    pub state_history_rows: usize,
    pub odds_converted: usize,
    pub preserved_match_ids: Vec<String>,
    pub unmappable_fields: Vec<String>,
}

#[derive(Debug, Clone)]
struct LegacyDetailRow {
    match_id: String,
    incidents: Option<String>,
    stats: Option<String>,
    lineups: Option<String>,
    odds: Option<String>,
    h2h: Option<String>,
    last_updated: Option<i64>,
    dataset_id: String,
}

#[derive(Debug, Clone)]
struct LegacyOdd {
    bookmaker_id: String,
    market_type: String,
    period: String,
    selection_key: String,
    line_value: String,
    odds_value: String,
    payload_json: String,
    payload_hash: String,
}

#[derive(Debug, Clone)]
struct SectionValue {
    status: &'static str,
    is_empty: bool,
    is_unparseable: bool,
    data_json: Option<String>,
    content_hash: String,
    error: Option<String>,
}

pub fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    let table = quote_identifier(table);
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_exists(conn: &Connection, table: &str) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master
            WHERE type IN ('table', 'view') AND name = ?1
        )",
        params![table],
        |row| row.get(0),
    )
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column_definition: &str,
    column: &str,
) -> SqlResult<bool> {
    if column_exists(conn, table, column)? {
        return Ok(false);
    }
    conn.execute(
        &format!(
            "ALTER TABLE {} ADD COLUMN {}",
            quote_identifier(table),
            column_definition
        ),
        [],
    )?;
    Ok(true)
}

fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn drop_legacy_indexes(conn: &Connection) -> SqlResult<()> {
    for name in [
        "idx_matches_list",
        "idx_matches_competition",
        "idx_matches_home_team",
        "idx_matches_away_team",
        "idx_match_details_lookup",
        "idx_competitions_sport",
        "idx_teams_sport",
        "idx_dataset_sports_ready",
    ] {
        conn.execute(
            &format!("DROP INDEX IF EXISTS {}", quote_identifier(name)),
            [],
        )?;
    }
    Ok(())
}

fn next_archive_name(conn: &Connection, table: &str) -> SqlResult<String> {
    let base = format!("{table}_legacy_v1");
    if !table_exists(conn, &base)? {
        return Ok(base);
    }
    for suffix in 2..10_000 {
        let candidate = format!("{base}_{suffix}");
        if !table_exists(conn, &candidate)? {
            return Ok(candidate);
        }
    }
    Err(rusqlite::Error::InvalidParameterName(format!(
        "could not allocate legacy archive name for {table}"
    )))
}

/// Move an unscoped legacy table to a retained archive. No legacy table is
/// dropped: the new v3 table is populated from this archive below.
fn archive_unscoped_tables(conn: &Connection) -> SqlResult<()> {
    for table in ["competitions", "teams", "matches", "match_details"] {
        if table_exists(conn, table)? && !column_exists(conn, table, "dataset_id")? {
            drop_legacy_indexes(conn)?;
            let archive = next_archive_name(conn, table)?;
            conn.execute(
                &format!(
                    "ALTER TABLE {} RENAME TO {}",
                    quote_identifier(table),
                    quote_identifier(&archive)
                ),
                [],
            )?;
        }
    }
    Ok(())
}

fn ensure_compatibility_columns(conn: &Connection) -> SqlResult<()> {
    add_column_if_missing(
        conn,
        "datasets",
        "generation_order INTEGER NOT NULL DEFAULT 0",
        "generation_order",
    )?;
    add_column_if_missing(
        conn,
        "datasets",
        "schema_generation INTEGER NOT NULL DEFAULT 3",
        "schema_generation",
    )?;

    add_column_if_missing(
        conn,
        "dataset_sports",
        "synced INTEGER NOT NULL DEFAULT 0",
        "synced",
    )?;
    add_column_if_missing(
        conn,
        "dataset_sports",
        "synced_at TEXT NOT NULL DEFAULT ''",
        "synced_at",
    )?;

    add_column_if_missing(conn, "competitions", "raw_payload TEXT", "raw_payload")?;
    add_column_if_missing(
        conn,
        "competitions",
        "synced INTEGER NOT NULL DEFAULT 0",
        "synced",
    )?;
    add_column_if_missing(conn, "competitions", "updated_at TEXT", "updated_at")?;
    add_column_if_missing(
        conn,
        "competitions",
        "dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id'",
        "dataset_id",
    )?;

    add_column_if_missing(conn, "teams", "raw_payload TEXT", "raw_payload")?;
    add_column_if_missing(conn, "teams", "synced INTEGER NOT NULL DEFAULT 0", "synced")?;
    add_column_if_missing(conn, "teams", "updated_at TEXT", "updated_at")?;
    add_column_if_missing(
        conn,
        "teams",
        "dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id'",
        "dataset_id",
    )?;

    let had_is_live = column_exists(conn, "matches", "is_live")?;
    add_column_if_missing(
        conn,
        "matches",
        "is_live INTEGER NOT NULL DEFAULT 0",
        "is_live",
    )?;
    if !had_is_live {
        conn.execute(
            "UPDATE matches
             SET is_live = CASE
                 WHEN sport_id = 1 AND status_id IN (2, 3, 4, 5, 6, 7) THEN 1
                 WHEN sport_id = 2 AND status_id IN (2, 3, 4, 5, 6, 7, 9) THEN 1
                 ELSE 0
             END",
            [],
        )?;
    }
    add_column_if_missing(conn, "matches", "raw_payload TEXT", "raw_payload")?;
    add_column_if_missing(
        conn,
        "matches",
        "synced INTEGER NOT NULL DEFAULT 0",
        "synced",
    )?;
    add_column_if_missing(conn, "matches", "updated_at TEXT", "updated_at")?;
    add_column_if_missing(
        conn,
        "matches",
        "dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id'",
        "dataset_id",
    )?;

    add_column_if_missing(conn, "match_details", "raw_payload TEXT", "raw_payload")?;
    add_column_if_missing(
        conn,
        "match_details",
        "synced INTEGER NOT NULL DEFAULT 0",
        "synced",
    )?;
    add_column_if_missing(conn, "match_details", "updated_at TEXT", "updated_at")?;
    add_column_if_missing(
        conn,
        "match_details",
        "dataset_id TEXT NOT NULL DEFAULT 'legacy-dataset-id'",
        "dataset_id",
    )?;

    conn.execute(
        "UPDATE datasets SET generation_order = COALESCE(
             NULLIF(generation_order, 0),
             CAST(strftime('%s', created_at) AS INTEGER) * 1000 + rowid
         ) WHERE dataset_id <> ?1",
        params![LEGACY_DATASET_ID],
    )?;
    conn.execute(
        "UPDATE datasets SET schema_generation = ?1
         WHERE schema_generation IS NULL OR schema_generation < ?1",
        params![V3_SCHEMA_VERSION],
    )?;
    conn.execute(
        "UPDATE competitions SET dataset_id = ?1 WHERE dataset_id IS NULL OR dataset_id = ''",
        params![LEGACY_DATASET_ID],
    )?;
    conn.execute(
        "UPDATE teams SET dataset_id = ?1 WHERE dataset_id IS NULL OR dataset_id = ''",
        params![LEGACY_DATASET_ID],
    )?;
    conn.execute(
        "UPDATE matches SET dataset_id = ?1 WHERE dataset_id IS NULL OR dataset_id = ''",
        params![LEGACY_DATASET_ID],
    )?;
    conn.execute(
        "UPDATE match_details SET dataset_id = ?1 WHERE dataset_id IS NULL OR dataset_id = ''",
        params![LEGACY_DATASET_ID],
    )?;
    Ok(())
}

fn archive_names(conn: &Connection, table: &str) -> SqlResult<Vec<String>> {
    let pattern = format!("{table}_legacy_v1%");
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name LIKE ?1 ORDER BY name",
    )?;
    let names = stmt
        .query_map(params![pattern], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

fn copy_archived_rows(conn: &Connection) -> SqlResult<(usize, BTreeSet<String>)> {
    let mut preserved = BTreeSet::new();
    let mut matches_seen = 0;

    for archive in archive_names(conn, "competitions")? {
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO competitions
                 (id, sport_id, name, logo, slug, country_name, country_logo,
                  raw_payload, synced, updated_at, dataset_id)
                 SELECT id, sport_id, name, logo, slug, country_name, country_logo,
                        NULL, 0, NULL, ?1 FROM {}",
                quote_identifier(&archive)
            ),
            params![LEGACY_DATASET_ID],
        )?;
    }

    for archive in archive_names(conn, "teams")? {
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO teams
                 (id, sport_id, name, logo, slug, raw_payload, synced, updated_at, dataset_id)
                 SELECT id, sport_id, name, logo, slug, NULL, 0, NULL, ?1 FROM {}",
                quote_identifier(&archive)
            ),
            params![LEGACY_DATASET_ID],
        )?;
    }

    for archive in archive_names(conn, "matches")? {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, sport_id, competition_id, home_team_id, away_team_id,
                    match_time, status_id, home_scores, away_scores, updated_at
             FROM {} ORDER BY id",
            quote_identifier(&archive)
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i32>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                row.get::<_, Option<i64>>(5)?.unwrap_or_default(),
                row.get::<_, Option<i32>>(6)?.unwrap_or_default(),
                row.get::<_, Option<String>>(7)?
                    .unwrap_or_else(|| "[]".to_string()),
                row.get::<_, Option<String>>(8)?
                    .unwrap_or_else(|| "[]".to_string()),
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        for row in rows {
            let (
                id,
                sport_id,
                competition_id,
                home_team_id,
                away_team_id,
                match_time,
                status_id,
                home_scores,
                away_scores,
                updated_at,
            ) = row?;
            matches_seen += 1;
            preserved.insert(id.clone());
            let is_live = match sport_id {
                1 => [2, 3, 4, 5, 6, 7].contains(&status_id),
                2 => [2, 3, 4, 5, 6, 7, 9].contains(&status_id),
                _ => false,
            };
            conn.execute(
                "INSERT OR IGNORE INTO matches
                 (id, sport_id, competition_id, home_team_id, away_team_id,
                  match_time, status_id, home_scores, away_scores, is_live,
                  raw_payload, synced, updated_at, dataset_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, 0, ?11, ?12)",
                params![
                    id,
                    sport_id,
                    competition_id,
                    home_team_id,
                    away_team_id,
                    match_time,
                    status_id,
                    home_scores,
                    away_scores,
                    is_live,
                    updated_at,
                    LEGACY_DATASET_ID
                ],
            )?;
        }
    }

    for archive in archive_names(conn, "match_details")? {
        let mut stmt = conn.prepare(&format!(
            "SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated
             FROM {} ORDER BY match_id",
            quote_identifier(&archive)
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i32>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })?;
        for row in rows {
            let (match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated) = row?;
            conn.execute(
                "INSERT OR IGNORE INTO match_details
                 (match_id, sport_id, incidents, stats, lineups, odds, h2h,
                  raw_payload, synced, last_updated, updated_at, dataset_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 0, ?8, NULL, ?9)",
                params![
                    match_id,
                    sport_id,
                    incidents,
                    stats,
                    lineups,
                    odds,
                    h2h,
                    last_updated,
                    LEGACY_DATASET_ID
                ],
            )?;
        }
    }

    Ok((matches_seen, preserved))
}

fn fnv1a(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn json_hash(value: &str) -> String {
    fnv1a(value.as_bytes())
}

fn classify_section(raw: Option<&str>, section: &str) -> SectionValue {
    let text = raw.unwrap_or("").trim();
    let content_hash = json_hash(text);
    if text.is_empty() {
        return SectionValue {
            status: "EMPTY",
            is_empty: true,
            is_unparseable: false,
            data_json: Some("null".to_string()),
            content_hash,
            error: None,
        };
    }

    match serde_json::from_str::<Value>(text) {
        Ok(value)
            if value.is_null()
                || value.as_array().is_some_and(Vec::is_empty)
                || value.as_object().is_some_and(Map::is_empty) =>
        {
            SectionValue {
                status: "EMPTY",
                is_empty: true,
                is_unparseable: false,
                data_json: Some(text.to_string()),
                content_hash,
                error: None,
            }
        }
        Ok(_) => SectionValue {
            status: "COMPLETED",
            is_empty: false,
            is_unparseable: false,
            data_json: Some(text.to_string()),
            content_hash,
            error: None,
        },
        Err(error) => SectionValue {
            status: "UNPARSEABLE",
            is_empty: false,
            is_unparseable: true,
            data_json: None,
            content_hash,
            error: Some(format!("legacy {section} JSON: {error}")),
        },
    }
}

fn scalar_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Object(object)) => object
            .get("id")
            .or_else(|| object.get("key"))
            .and_then(|value| scalar_text(Some(value))),
        _ => None,
    }
}

fn first_scalar(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| scalar_text(object.get(*key)))
}

fn collect_odds_objects<'a>(value: &'a Value, output: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Object(object) => {
            let has_odd_fields = [
                "bookmaker_id",
                "bookmakerId",
                "bookmaker",
                "market_type",
                "marketType",
                "selection_key",
                "selectionKey",
                "odds_value",
                "oddsValue",
            ]
            .iter()
            .any(|key| object.contains_key(*key));
            if has_odd_fields {
                output.push(object);
                return;
            }
            for child in object.values() {
                collect_odds_objects(child, output);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_odds_objects(child, output);
            }
        }
        _ => {}
    }
}

fn parse_legacy_odds(raw: &str) -> (Vec<LegacyOdd>, bool) {
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(_) => return (Vec::new(), false),
    };
    let mut objects = Vec::new();
    collect_odds_objects(&value, &mut objects);
    let had_objects = !objects.is_empty();
    let mut parsed = Vec::new();

    for object in objects {
        let bookmaker_id = match first_scalar(
            object,
            &[
                "bookmaker_id",
                "bookmakerId",
                "bookmaker",
                "company_id",
                "companyId",
            ],
        ) {
            Some(value) => value,
            None => continue,
        };
        let market_type = match first_scalar(
            object,
            &[
                "market_type",
                "marketType",
                "market",
                "market_name",
                "marketName",
            ],
        ) {
            Some(value) => value,
            None => continue,
        };
        let selection_key = match first_scalar(
            object,
            &[
                "selection_key",
                "selectionKey",
                "selection",
                "outcome",
                "outcome_key",
            ],
        ) {
            Some(value) => value,
            None => continue,
        };
        let odds_value = match first_scalar(
            object,
            &["odds_value", "oddsValue", "odds", "value", "price"],
        ) {
            Some(value) if value.parse::<f64>().is_ok() => value,
            _ => continue,
        };
        let period =
            first_scalar(object, &["period", "period_name", "periodName"]).unwrap_or_default();
        let line_value =
            first_scalar(object, &["line_value", "lineValue", "line"]).unwrap_or_default();
        let payload_json = Value::Object(object.clone()).to_string();
        let payload_hash = json_hash(&payload_json);
        parsed.push(LegacyOdd {
            bookmaker_id,
            market_type,
            period,
            selection_key,
            line_value,
            odds_value,
            payload_json,
            payload_hash,
        });
    }

    (parsed, had_objects)
}

fn add_unmappable(set: &mut BTreeSet<String>, value: String) {
    set.insert(value);
}

fn convert_detail_rows(
    conn: &Connection,
    rows: &[LegacyDetailRow],
    report: &mut LegacyConversionReport,
    unmappable: &mut BTreeSet<String>,
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    let now = utc_now();

    for row in rows {
        let fields = [
            ("incidents", row.incidents.as_deref()),
            ("stats", row.stats.as_deref()),
            ("lineups", row.lineups.as_deref()),
            ("odds", row.odds.as_deref()),
            ("h2h", row.h2h.as_deref()),
        ];
        let source_timestamp = row.last_updated.map(|value| value.to_string());

        for (section_name, raw) in fields {
            let section = classify_section(raw, section_name);
            report.sections_converted += 1;
            if section.is_empty {
                report.empty_sections += 1;
            }
            if section.is_unparseable {
                report.unparseable_sections += 1;
                add_unmappable(
                    unmappable,
                    format!("{}:{section_name}:unparseable", row.match_id),
                );
            }

            tx.execute(
                "INSERT OR IGNORE INTO match_detail_sections
                 (match_id, dataset_id, section_name, status, provenance,
                  is_empty, is_unparseable, content_hash, source_timestamp,
                  received_at, completed_at, last_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.match_id,
                    row.dataset_id,
                    section_name,
                    section.status,
                    format!("legacy:match_details.{section_name}"),
                    section.is_empty,
                    section.is_unparseable,
                    section.content_hash,
                    source_timestamp,
                    now,
                    if section.status == "COMPLETED" {
                        Some(now.clone())
                    } else {
                        None
                    },
                    section.error,
                ],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO match_detail_data
                 (match_id, dataset_id, section_name, data_json, provenance,
                  content_hash, source_timestamp, received_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.match_id,
                    row.dataset_id,
                    section_name,
                    section.data_json,
                    format!("legacy:match_details.{section_name}"),
                    section.content_hash,
                    source_timestamp,
                    now,
                ],
            )?;
        }

        if let Some(raw_odds) = row.odds.as_deref() {
            let (odds, had_records) = parse_legacy_odds(raw_odds);
            if odds.is_empty() && !classify_section(Some(raw_odds), "odds").is_empty {
                if had_records {
                    add_unmappable(unmappable, format!("{}:odds:unmappable", row.match_id));
                } else {
                    add_unmappable(
                        unmappable,
                        format!("{}:odds:no-parseable-record", row.match_id),
                    );
                }
            }

            for odd in odds {
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT odds_value FROM odds_current
                         WHERE match_id=?1 AND dataset_id=?2 AND bookmaker_id=?3
                           AND market_type=?4 AND period=?5 AND selection_key=?6
                           AND line_value=?7",
                        params![
                            row.match_id,
                            row.dataset_id,
                            odd.bookmaker_id,
                            odd.market_type,
                            odd.period,
                            odd.selection_key,
                            odd.line_value
                        ],
                        |query_row| query_row.get(0),
                    )
                    .optional()?;
                let source_timestamp = row
                    .last_updated
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                let event_key = format!(
                    "legacy:{}:{}:{}:{}:{}:{}:{}:{}",
                    row.match_id,
                    row.dataset_id,
                    odd.bookmaker_id,
                    odd.market_type,
                    odd.period,
                    odd.selection_key,
                    odd.line_value,
                    odd.payload_hash
                );

                tx.execute(
                    "INSERT OR IGNORE INTO odds_history
                     (match_id, dataset_id, bookmaker_id, market_type, period,
                      selection_key, line_value, odds_value, previous_odds_value,
                      is_live, source_timestamp, received_at, payload_hash,
                      provenance, event_key)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        row.match_id,
                        row.dataset_id,
                        odd.bookmaker_id,
                        odd.market_type,
                        odd.period,
                        odd.selection_key,
                        odd.line_value,
                        odd.odds_value,
                        previous,
                        source_timestamp,
                        now,
                        odd.payload_hash,
                        "legacy:match_details.odds",
                        event_key
                    ],
                )?;
                tx.execute(
                    "INSERT INTO odds_current
                     (match_id, dataset_id, bookmaker_id, market_type, period,
                      selection_key, line_value, odds_value, is_live,
                      source_timestamp, received_at, payload_hash, provenance)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12)
                     ON CONFLICT(match_id, dataset_id, bookmaker_id, market_type,
                                 period, selection_key, line_value)
                     DO UPDATE SET odds_value=excluded.odds_value,
                                   source_timestamp=excluded.source_timestamp,
                                   received_at=excluded.received_at,
                                   payload_hash=excluded.payload_hash,
                                   provenance=excluded.provenance",
                    params![
                        row.match_id,
                        row.dataset_id,
                        odd.bookmaker_id,
                        odd.market_type,
                        odd.period,
                        odd.selection_key,
                        odd.line_value,
                        odd.odds_value,
                        source_timestamp,
                        now,
                        odd.payload_hash,
                        "legacy:match_details.odds"
                    ],
                )?;
                report.odds_converted += 1;
                let _ = &odd.payload_json;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn collect_detail_rows(conn: &Connection) -> SqlResult<Vec<LegacyDetailRow>> {
    let mut stmt = conn.prepare(
        "SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h,
                last_updated, dataset_id
         FROM match_details ORDER BY dataset_id, match_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LegacyDetailRow {
            match_id: row.get(0)?,
            incidents: row.get(2)?,
            stats: row.get(3)?,
            lineups: row.get(4)?,
            odds: row.get(5)?,
            h2h: row.get(6)?,
            last_updated: row.get(7)?,
            dataset_id: row
                .get::<_, Option<String>>(8)?
                .unwrap_or_else(|| LEGACY_DATASET_ID.to_string()),
        })
    })?;
    rows.collect()
}

fn create_legacy_state_rows(conn: &Connection) -> SqlResult<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, sport_id, status_id, home_scores, away_scores, is_live,
                dataset_id, updated_at
         FROM matches ORDER BY dataset_id, id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i32>>(1)?.unwrap_or_default(),
                row.get::<_, Option<i32>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<bool>>(5)?.unwrap_or(false),
                row.get::<_, Option<String>>(6)?
                    .unwrap_or_else(|| LEGACY_DATASET_ID.to_string()),
                row.get::<_, Option<String>>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let tx = conn.unchecked_transaction()?;
    for (id, sport_id, status_id, home_scores, away_scores, is_live, dataset_id, updated_at) in rows
    {
        let state = if status_id == 8 {
            "FINISHED"
        } else if is_live {
            "LIVE"
        } else {
            "LEGACY_IMPORTED"
        };
        let source_timestamp = updated_at.unwrap_or_default();
        let payload_hash = fnv1a(
            format!(
                "{id}|{dataset_id}|{status_id}|{}|{}",
                home_scores.as_deref().unwrap_or(""),
                away_scores.as_deref().unwrap_or("")
            )
            .as_bytes(),
        );
        tx.execute(
            "INSERT OR IGNORE INTO match_state_history
             (match_id, dataset_id, sport_id, state, status_id, home_scores,
              away_scores, source_timestamp, received_at, payload_hash, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'legacy:matches')",
            params![
                id,
                dataset_id,
                sport_id,
                state,
                status_id,
                home_scores,
                away_scores,
                source_timestamp,
                utc_now(),
                payload_hash
            ],
        )?;
    }
    tx.commit()?;
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM match_state_history WHERE provenance='legacy:matches'",
        [],
        |row| row.get::<_, i64>(0),
    )? as usize;
    Ok(total)
}

/// Convert legacy detail columns into section/data rows and parse only odds
/// records that carry all identifying fields. The source columns remain
/// untouched, and every insert is conflict-safe for repeat migrations.
pub fn convert_legacy_data(conn: &Connection) -> SqlResult<LegacyConversionReport> {
    let (matches_seen, preserved_ids) = copy_archived_rows(conn)?;
    let rows = collect_detail_rows(conn)?;
    let mut report = LegacyConversionReport {
        matches_seen,
        matches_preserved: preserved_ids.len(),
        details_seen: rows.len(),
        preserved_match_ids: preserved_ids.into_iter().collect(),
        ..LegacyConversionReport::default()
    };
    let mut unmappable = BTreeSet::new();
    convert_detail_rows(conn, &rows, &mut report, &mut unmappable)?;
    report.state_history_rows = create_legacy_state_rows(conn)?;
    report.unmappable_fields = unmappable.into_iter().collect();
    Ok(report)
}

fn write_audit(conn: &Connection, report: &LegacyConversionReport) -> SqlResult<()> {
    let now = utc_now();
    let report_json = serde_json::to_string(report).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO migration_audit
         (migration_id, version, status, started_at, completed_at, report_json, error_text)
         VALUES (?1, ?2, 'COMPLETED', ?3, ?4, ?5, NULL)
         ON CONFLICT(migration_id) DO UPDATE SET
             version=excluded.version,
             status=excluded.status,
             completed_at=excluded.completed_at,
             report_json=excluded.report_json,
             error_text=NULL",
        params![MIGRATION_ID, V3_SCHEMA_VERSION, now, now, report_json],
    )?;
    conn.pragma_update(None, "user_version", &V3_SCHEMA_VERSION)?;
    Ok(())
}

/// Apply the local v3 schema and deterministic legacy conversion. The
/// operation is repeat-safe: old tables are archived once, rows are copied
/// with INSERT OR IGNORE, and section/odds/audit keys are unique.
pub fn run_migrations(conn: &Connection) -> SqlResult<()> {
    archive_unscoped_tables(conn)?;
    schema::create_v3_schema(conn)?;
    ensure_compatibility_columns(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO datasets
         (dataset_id, created_at, generation_order, schema_generation)
         VALUES (?1, datetime('now'), 0, ?2)",
        params![LEGACY_DATASET_ID, V3_SCHEMA_VERSION],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5')",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('cf_worker_url', 'http://127.0.0.1:8080')",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token')",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('detail_update_interval_secs', '60')",
        [],
    )?;

    let report = convert_legacy_data(conn)?;
    write_audit(conn, &report)?;
    Ok(())
}

pub fn init_dataset_id(conn: &Connection) -> SqlResult<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'active_dataset_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(dataset_id) = existing {
        conn.execute(
            "INSERT OR IGNORE INTO datasets
             (dataset_id, created_at, generation_order, schema_generation)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                dataset_id,
                utc_now(),
                Utc::now().timestamp_millis(),
                V3_SCHEMA_VERSION
            ],
        )?;
        return Ok(dataset_id);
    }

    let now = Utc::now();
    let dataset_id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO datasets
         (dataset_id, created_at, generation_order, schema_generation)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            dataset_id,
            now.to_rfc3339_opts(SecondsFormat::Millis, true),
            now.timestamp_millis(),
            V3_SCHEMA_VERSION
        ],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('active_dataset_id', ?1)",
        params![dataset_id],
    )?;
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'active_dataset_id'",
        [],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE competitions (
                id TEXT PRIMARY KEY, sport_id INTEGER, name TEXT, logo TEXT,
                slug TEXT, country_name TEXT, country_logo TEXT
            );
            CREATE TABLE teams (
                id TEXT PRIMARY KEY, sport_id INTEGER, name TEXT, logo TEXT, slug TEXT
            );
            CREATE TABLE matches (
                id TEXT PRIMARY KEY, sport_id INTEGER, competition_id TEXT,
                home_team_id TEXT, away_team_id TEXT, match_time INTEGER,
                status_id INTEGER, home_scores TEXT, away_scores TEXT, updated_at TEXT
            );
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE match_details (
                match_id TEXT PRIMARY KEY, sport_id INTEGER, incidents TEXT,
                stats TEXT, lineups TEXT, odds TEXT, h2h TEXT, last_updated INTEGER
            );
            INSERT INTO matches
              (id, sport_id, competition_id, home_team_id, away_team_id,
               match_time, status_id, home_scores, away_scores, updated_at)
            VALUES ('legacy-id', 1, 'comp', 'home', 'away', 1700000000, 8,
                    '[2]', '[1]', '2026-07-10T00:00:00Z');
            INSERT INTO match_details
              (match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated)
            VALUES ('legacy-id', 1, '[]', '{bad', '{}',
                    '{"bookmaker_id":"book-1","market_type":"1x2","selection_key":"home","odds_value":"1.75"}',
                    '{}', 1700000000);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn migration_preserves_legacy_tables_and_ids() {
        let conn = legacy_connection();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let current: String = conn
            .query_row("SELECT id FROM matches WHERE id='legacy-id'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(current, "legacy-id");
        let archived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'matches_legacy_v1%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(archived, 1);
        let audit_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM migration_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audit_count, 1);
    }

    #[test]
    fn legacy_conversion_is_repeat_safe_and_reports_unparseable_sections() {
        let conn = legacy_connection();
        run_migrations(&conn).unwrap();
        let first_sections: i64 = conn
            .query_row("SELECT COUNT(*) FROM match_detail_sections", [], |row| {
                row.get(0)
            })
            .unwrap();
        let first_odds: i64 = conn
            .query_row("SELECT COUNT(*) FROM odds_history", [], |row| row.get(0))
            .unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(first_sections, 5);
        assert_eq!(first_odds, 1);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM match_detail_sections", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            first_sections
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM odds_history", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            first_odds
        );
        let status: String = conn
            .query_row(
                "SELECT status FROM match_detail_sections WHERE section_name='stats'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "UNPARSEABLE");
        let provenance: String = conn
            .query_row("SELECT provenance FROM odds_history", [], |row| row.get(0))
            .unwrap();
        assert!(provenance.contains("legacy"));
    }
}
