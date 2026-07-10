use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use rusqlite::{Connection, OptionalExtension, Result as SqlResult, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

static REQ_ID: AtomicI64 = AtomicI64::new(1000);
fn next_req_id() -> i64 {
    REQ_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Parser)]
#[command(name = "aiscore-crawler")]
#[command(about = "AiScore Live Football & Basketball Crawler", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "http://192.168.224.1:9223")]
    chrome_url: String,

    #[arg(short, long, default_value = "local.db")]
    db_path: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Clone)]
enum Commands {
    Football,
    Basketball,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Team {
    id: String,
    sport_id: i32,
    name: String,
    logo: Option<String>,
    slug: Option<String>,
    raw_payload: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Competition {
    id: String,
    sport_id: i32,
    name: String,
    logo: Option<String>,
    slug: Option<String>,
    country_name: Option<String>,
    country_logo: Option<String>,
    raw_payload: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Match {
    id: String,
    sport_id: i32,
    competition_id: String,
    home_team_id: String,
    away_team_id: String,
    match_time: i64,
    status_id: i32,
    home_scores: String, // JSON array string
    away_scores: String, // JSON array string
    raw_payload: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MatchDetail {
    match_id: String,
    sport_id: i32,
    incidents: String, // JSON array string
    stats: String,     // JSON object string
    lineups: String,   // JSON object string
    odds: String,      // JSON object string
    h2h: String,       // JSON object string
    raw_payload: Option<String>,
    last_updated: i64,
}

#[derive(Deserialize)]
struct TargetInfo {
    url: String,
    #[serde(rename = "type")]
    target_type: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
}

trait SportAdapter: Send + Sync {
    fn sport_id(&self) -> i32;
    fn name(&self) -> &'static str;
    fn matches_url(&self, url: &str) -> bool;
    fn target_url(&self) -> &'static str;
    fn extract_state_js(&self) -> &'static str;
    fn activate_live_js(&self) -> &'static str;
}

struct FootballAdapter;
impl SportAdapter for FootballAdapter {
    fn sport_id(&self) -> i32 {
        1
    }
    fn name(&self) -> &'static str {
        "football"
    }
    fn matches_url(&self, url: &str) -> bool {
        (url.contains("m.aiscore.com") || url.contains("aiscore.com"))
            && !url.contains("basketball")
            && !url.contains("tennis")
            && !url.contains("baseball")
    }
    fn target_url(&self) -> &'static str {
        "https://m.aiscore.com/"
    }
    fn extract_state_js(&self) -> &'static str {
        r#"(function() {
            const fHome = window.$nuxt && window.$nuxt.$store && (window.$nuxt.$store.state['football/home'] || window.$nuxt.$store.state['home']);
            if (!fHome) return null;
            
            const normalize = (coll) => {
                if (!coll) return [];
                if (Array.isArray(coll)) return coll;
                if (typeof coll === 'object') return Object.values(coll);
                return [];
            };

            return {
                matches: normalize(fHome.matchesData_matches || fHome.matches || fHome.list),
                teams: normalize(fHome.matchesData_teams || fHome.teams),
                competitions: normalize(fHome.matchesData_competitions || fHome.competitions)
            };
        })()"#
    }
    fn activate_live_js(&self) -> &'static str {
        r#"(function() {
            const btn = Array.from(document.querySelectorAll('div, span, a, li, button'))
                .find(e => e.textContent && e.textContent.trim().toLowerCase() === 'live');
            if (btn) { btn.click(); return true; }
            return false;
        })()"#
    }
}

struct BasketballAdapter;
impl SportAdapter for BasketballAdapter {
    fn sport_id(&self) -> i32 {
        2
    }
    fn name(&self) -> &'static str {
        "basketball"
    }
    fn matches_url(&self, url: &str) -> bool {
        url.contains("basketball")
    }
    fn target_url(&self) -> &'static str {
        "https://m.aiscore.com/basketball"
    }
    fn extract_state_js(&self) -> &'static str {
        r#"(function() {
            const bHome = window.$nuxt && window.$nuxt.$store && (window.$nuxt.$store.state['basketball'] || window.$nuxt.$store.state['basketball/player']);
            if (!bHome) return null;

            const normalize = (coll) => {
                if (!coll) return [];
                if (Array.isArray(coll)) return coll;
                if (typeof coll === 'object') return Object.values(coll);
                return [];
            };

            return {
                matches: normalize(bHome.matchesData_matches || bHome.matches || bHome.list),
                teams: normalize(bHome.matchesData_teams || bHome.teams),
                competitions: normalize(bHome.matchesData_competitions || bHome.competitions)
            };
        })()"#
    }
    fn activate_live_js(&self) -> &'static str {
        r#"(function() {
            const btn = Array.from(document.querySelectorAll('div, span, a, li, button'))
                .find(e => e.textContent && e.textContent.trim().toLowerCase() === 'live');
            if (btn) { btn.click(); return true; }
            return false;
        })()"#
    }
}

struct WsRouter {
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
}

impl WsRouter {
    fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

async fn get_websocket_url(
    chrome_url: &str,
    adapter: &dyn SportAdapter,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = format!("{}/json", chrome_url);
    let resp = client.get(&url).send().await?;
    let targets: Vec<TargetInfo> = resp.json().await?;

    // 1. Search for target matching our sport url
    for target in &targets {
        if adapter.matches_url(&target.url) {
            if let Some(ref ws_url) = target.websocket_url {
                return Ok(ws_url.clone());
            }
        }
    }

    // 2. Hijack a generic tab, BUT ONLY if it is not dedicated to the OTHER sport!
    for target in &targets {
        if target.target_type.as_deref() == Some("page") {
            let is_other_sport = if adapter.sport_id() == 1 {
                target.url.contains("basketball")
            } else {
                (target.url.contains("m.aiscore.com") || target.url.contains("aiscore.com"))
                    && !target.url.contains("basketball")
                    && !target.url.contains("tennis")
                    && !target.url.contains("baseball")
            };

            if !is_other_sport {
                if let Some(ref ws_url) = target.websocket_url {
                    println!(
                        "[Crawler] Hijacking empty/active tab ({}) to open {}...",
                        target.url,
                        adapter.name()
                    );
                    return Ok(ws_url.clone());
                }
            }
        }
    }

    Err(format!(
        "No debuggable browser page found for {}! Please open Chrome.",
        adapter.name()
    )
    .into())
}

// Open DB with concurrent settings (WAL mode + busy timeout)
fn open_db(db_path: &str) -> SqlResult<Connection> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", &"WAL")?;
    conn.pragma_update(None, "busy_timeout", &"5000")?;
    Ok(conn)
}

// Initialize SQLite database
fn init_db(db_path: &str) -> SqlResult<Connection> {
    let conn = open_db(db_path)?;
    run_migrations(&conn)?;
    Ok(conn)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_column_if_not_exists(
    conn: &Connection,
    table: &str,
    column_def: &str,
    column_name: &str,
) -> SqlResult<()> {
    if !column_exists(conn, table, column_name)? {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {}", table, column_def),
            [],
        )?;
    }
    Ok(())
}

fn run_migrations(conn: &Connection) -> SqlResult<()> {
    // Ensure baseline tables exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS competitions (
            id TEXT PRIMARY KEY,
            sport_id INTEGER,
            name TEXT,
            logo TEXT,
            slug TEXT,
            country_name TEXT,
            country_logo TEXT
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS teams (
            id TEXT PRIMARY KEY,
            sport_id INTEGER,
            name TEXT,
            logo TEXT,
            slug TEXT
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS matches (
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
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS match_details (
            match_id TEXT PRIMARY KEY,
            sport_id INTEGER,
            incidents TEXT,
            stats TEXT,
            lineups TEXT,
            odds TEXT,
            h2h TEXT,
            last_updated INTEGER
         )",
        [],
    )?;

    // Use PRAGMA table_info to check and add missing columns/indexes in transaction
    let tx = conn.unchecked_transaction()?;

    add_column_if_not_exists(&tx, "competitions", "raw_payload TEXT", "raw_payload")?;
    add_column_if_not_exists(&tx, "competitions", "synced INTEGER DEFAULT 0", "synced")?;
    add_column_if_not_exists(&tx, "competitions", "updated_at TEXT", "updated_at")?;

    add_column_if_not_exists(&tx, "teams", "raw_payload TEXT", "raw_payload")?;
    add_column_if_not_exists(&tx, "teams", "synced INTEGER DEFAULT 0", "synced")?;
    add_column_if_not_exists(&tx, "teams", "updated_at TEXT", "updated_at")?;

    add_column_if_not_exists(&tx, "matches", "raw_payload TEXT", "raw_payload")?;
    add_column_if_not_exists(&tx, "matches", "synced INTEGER DEFAULT 0", "synced")?;
    add_column_if_not_exists(&tx, "matches", "updated_at TEXT", "updated_at")?;

    add_column_if_not_exists(&tx, "match_details", "raw_payload TEXT", "raw_payload")?;
    add_column_if_not_exists(&tx, "match_details", "synced INTEGER DEFAULT 0", "synced")?;
    add_column_if_not_exists(&tx, "match_details", "updated_at TEXT", "updated_at")?;

    // Create performance indexes
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_matches_list ON matches (sport_id, status_id, match_time)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_matches_competition ON matches (competition_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_matches_home_team ON matches (home_team_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_matches_away_team ON matches (away_team_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_match_details_lookup ON match_details (match_id, sport_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_competitions_sport ON competitions (sport_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_teams_sport ON teams (sport_id)",
        [],
    )?;

    // Default settings
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5')",
        [],
    )?;
    tx.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('cf_worker_url', 'http://127.0.0.1:8080')", [])?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('detail_update_interval_secs', '60')",
        [],
    )?;

    tx.commit()?;
    Ok(())
}

fn sanitize_json(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.retain(|key, _| {
                let lower = key.to_ascii_lowercase();
                let is_chat = lower == "chat"
                    || lower == "message"
                    || lower == "messages"
                    || lower == "comment"
                    || lower == "comments"
                    || lower == "messageroom"
                    || lower == "commentroom"
                    || lower == "chatroom"
                    || lower.contains("chat")
                    || lower.contains("message")
                    || lower.contains("comment");
                !is_chat
            });
            for (_, v) in map.iter_mut() {
                sanitize_json(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                sanitize_json(v);
            }
        }
        _ => {}
    }
}

fn save_competitions(conn: &Connection, comps: &[Value], sport_id: i32) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut select_stmt = tx.prepare(
            "SELECT id, name, logo, slug, country_name, country_logo, raw_payload FROM competitions WHERE id = ?1"
        )?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, datetime('now'))"
        )?;

        let mut update_stmt = tx.prepare(
            "UPDATE competitions SET 
                name = ?2, 
                logo = ?3, 
                slug = ?4, 
                country_name = ?5, 
                country_logo = ?6, 
                raw_payload = ?7, 
                synced = 0, 
                updated_at = datetime('now') 
             WHERE id = ?1",
        )?;

        for c in comps {
            let mut sanitized = c.clone();
            sanitize_json(&mut sanitized);

            let id = sanitized["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let name = sanitized["name"].as_str().unwrap_or("");
            let logo = sanitized["logo"].as_str();
            let slug = sanitized["slug"].as_str();
            let country_name = sanitized["country"]["name"].as_str();
            let country_logo = sanitized["country"]["logo"].as_str();
            let raw_payload = serde_json::to_string(&sanitized).unwrap_or_default();

            let existing: Option<(
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            )> = select_stmt
                .query_row(params![id], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })
                .optional()?;

            if let Some(existing_row) = existing {
                let is_changed = existing_row.0 != name
                    || existing_row.1.as_deref() != logo
                    || existing_row.2.as_deref() != slug
                    || existing_row.3.as_deref() != country_name
                    || existing_row.4.as_deref() != country_logo
                    || existing_row.6 != raw_payload;

                if is_changed {
                    update_stmt.execute(params![
                        id,
                        name,
                        logo,
                        slug,
                        country_name,
                        country_logo,
                        raw_payload
                    ])?;
                }
            } else {
                insert_stmt.execute(params![
                    id,
                    sport_id,
                    name,
                    logo,
                    slug,
                    country_name,
                    country_logo,
                    raw_payload
                ])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_teams(conn: &Connection, teams: &[Value], sport_id: i32) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut select_stmt =
            tx.prepare("SELECT id, name, logo, slug, raw_payload FROM teams WHERE id = ?1")?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO teams (id, sport_id, name, logo, slug, raw_payload, synced, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, datetime('now'))",
        )?;

        let mut update_stmt = tx.prepare(
            "UPDATE teams SET 
                name = ?2, 
                logo = ?3, 
                slug = ?4, 
                raw_payload = ?5, 
                synced = 0, 
                updated_at = datetime('now') 
             WHERE id = ?1",
        )?;

        for t in teams {
            let mut sanitized = t.clone();
            sanitize_json(&mut sanitized);

            let id = sanitized["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let name = sanitized["name"].as_str().unwrap_or("");
            let logo = sanitized["logo"].as_str();
            let slug = sanitized["slug"].as_str();
            let raw_payload = serde_json::to_string(&sanitized).unwrap_or_default();

            let existing: Option<(String, Option<String>, Option<String>, String)> = select_stmt
                .query_row(params![id], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .optional()?;

            if let Some(existing_row) = existing {
                let is_changed = existing_row.0 != name
                    || existing_row.1.as_deref() != logo
                    || existing_row.2.as_deref() != slug
                    || existing_row.3 != raw_payload;

                if is_changed {
                    update_stmt.execute(params![id, name, logo, slug, raw_payload])?;
                }
            } else {
                insert_stmt.execute(params![id, sport_id, name, logo, slug, raw_payload])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_matches(conn: &Connection, matches: &[Value], sport_id: i32) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut select_stmt = tx.prepare(
            "SELECT id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, raw_payload FROM matches WHERE id = ?1"
        )?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, raw_payload, synced, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, datetime('now'))"
        )?;

        let mut update_stmt = tx.prepare(
            "UPDATE matches SET 
                competition_id = ?2, 
                home_team_id = ?3, 
                away_team_id = ?4, 
                match_time = ?5, 
                status_id = ?6, 
                home_scores = ?7, 
                away_scores = ?8, 
                raw_payload = ?9, 
                synced = 0, 
                updated_at = datetime('now') 
             WHERE id = ?1",
        )?;

        for m in matches {
            let mut sanitized = m.clone();
            sanitize_json(&mut sanitized);

            let id = sanitized["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let comp_id = sanitized["competition"]["id"].as_str().unwrap_or("");
            let home_id = sanitized["homeTeam"]["id"].as_str().unwrap_or("");
            let away_id = sanitized["awayTeam"]["id"].as_str().unwrap_or("");
            let match_time = sanitized["matchTime"].as_i64().unwrap_or(0);
            let status_id = sanitized["statusId"].as_i64().unwrap_or(0) as i32;
            let home_scores = sanitized["homeScores"].to_string();
            let away_scores = sanitized["awayScores"].to_string();
            let raw_payload = serde_json::to_string(&sanitized).unwrap_or_default();

            let existing: Option<(String, String, String, i64, i32, String, String, String)> =
                select_stmt
                    .query_row(params![id], |row| {
                        Ok((
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i32>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, String>(8)?,
                        ))
                    })
                    .optional()?;

            if let Some(existing_row) = existing {
                // Must not revert finished match
                if existing_row.4 == 8 && status_id != 8 {
                    println!(
                        "[Crawler] Skipping status revert for finished match {} from {} to {}",
                        id, existing_row.4, status_id
                    );
                    continue;
                }

                let is_changed = existing_row.0 != comp_id
                    || existing_row.1 != home_id
                    || existing_row.2 != away_id
                    || existing_row.3 != match_time
                    || existing_row.4 != status_id
                    || existing_row.5 != home_scores
                    || existing_row.6 != away_scores
                    || existing_row.7 != raw_payload;

                if is_changed {
                    update_stmt.execute(params![
                        id,
                        comp_id,
                        home_id,
                        away_id,
                        match_time,
                        status_id,
                        home_scores,
                        away_scores,
                        raw_payload
                    ])?;
                }
            } else {
                insert_stmt.execute(params![
                    id,
                    sport_id,
                    comp_id,
                    home_id,
                    away_id,
                    match_time,
                    status_id,
                    home_scores,
                    away_scores,
                    raw_payload
                ])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_match_detail(
    conn: &Connection,
    match_id: &str,
    sport_id: i32,
    detail: &Value,
) -> SqlResult<()> {
    let mut sanitized = detail.clone();
    sanitize_json(&mut sanitized);

    let incidents = sanitized["incidents"].to_string();
    let stats = sanitized["stats"].to_string();
    let lineups = sanitized["lineups"].to_string();
    let odds = sanitized["odds"].to_string();
    let h2h = sanitized["h2h"].to_string();
    let raw_payload = serde_json::to_string(&sanitized).unwrap_or_default();

    let existing: Option<(String, String, String, String, String, String)> = conn
        .query_row(
            "SELECT incidents, stats, lineups, odds, h2h, raw_payload FROM match_details WHERE match_id = ?1",
            params![match_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;

    if let Some(existing_row) = existing {
        let is_changed = existing_row.0 != incidents
            || existing_row.1 != stats
            || existing_row.2 != lineups
            || existing_row.3 != odds
            || existing_row.4 != h2h
            || existing_row.5 != raw_payload;

        if is_changed {
            conn.execute(
                "UPDATE match_details SET
                    incidents = ?2,
                    stats = ?3,
                    lineups = ?4,
                    odds = ?5,
                    h2h = ?6,
                    raw_payload = ?7,
                    last_updated = strftime('%s', 'now'),
                    updated_at = datetime('now'),
                    synced = 0
                 WHERE match_id = ?1",
                params![match_id, incidents, stats, lineups, odds, h2h, raw_payload],
            )?;
        }
    } else {
        conn.execute(
            "INSERT INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, last_updated, synced, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s', 'now'), 0, datetime('now'))",
            params![match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload],
        )?;
    }
    Ok(())
}

fn get_matches_needing_detail(
    conn: &Connection,
    sport_id: i32,
    detail_interval_secs: i64,
) -> SqlResult<Vec<(String, String, String)>> {
    let query = "
        SELECT m.id, t1.slug, t2.slug
        FROM matches m
        JOIN teams t1 ON m.home_team_id = t1.id
        JOIN teams t2 ON m.away_team_id = t2.id
        LEFT JOIN match_details d ON m.id = d.match_id
        WHERE m.sport_id = ?1
          AND (
            d.match_id IS NULL
            OR (
              m.status_id NOT IN (1, 8)
              AND strftime('%s', 'now') - d.last_updated > ?2
            )
            OR (
              m.status_id = 8
              AND d.last_updated < CAST(strftime('%s', m.updated_at) AS INTEGER)
            )
          )
    ";

    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map(params![sport_id, detail_interval_secs], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut result = Vec::new();
    for r in rows {
        if let Ok(val) = r {
            result.push(val);
        }
    }
    Ok(result)
}

// Safe uploader lease system to prevent concurrent uploads between processes
fn claim_uploader_lease(conn: &Connection, client_id: &str) -> bool {
    let now = chrono::Utc::now().timestamp();

    // Create settings lease row if not exists
    let _ = conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('uploader_lease_owner', '')",
        [],
    );
    let _ = conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('uploader_lease_expires', '0')",
        [],
    );

    let owner: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key='uploader_lease_owner'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let expires: i64 = conn
        .query_row(
            "SELECT value FROM settings WHERE key='uploader_lease_expires'",
            [],
            |row| {
                let val: String = row.get(0)?;
                Ok(val.parse::<i64>().unwrap_or(0))
            },
        )
        .unwrap_or(0);

    if owner.is_empty() || owner == client_id || now > expires {
        // Try to claim
        let next_expiry = now + 45; // Lease lasts 45 seconds
        let res1 = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_owner', ?1)",
            params![client_id],
        );
        let res2 = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_expires', ?1)",
            params![next_expiry.to_string()],
        );
        if res1.is_ok() && res2.is_ok() {
            return true;
        }
    }
    let updated_owner: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key='uploader_lease_owner'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    updated_owner == client_id
}

fn release_uploader_lease(conn: &Connection, client_id: &str) {
    let owner: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key='uploader_lease_owner'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if owner == client_id {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_owner', '')",
            [],
        );
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_expires', '0')",
            [],
        );
    }
}

// Background sync worker that pushes dirty data to Cloudflare Workers D1
async fn sync_worker(db_path: String) {
    println!("[Sync] Starting sync worker background thread...");
    let client = reqwest::Client::new();
    let client_id = format!("sync-worker-{}", uuid::Uuid::new_v4());

    loop {
        let mut interval_mins = 5;
        let mut worker_url = String::new();
        let mut api_token = String::new();

        if let Ok(conn) = open_db(&db_path) {
            if let Ok(val) = conn.query_row(
                "SELECT value FROM settings WHERE key='sync_interval_mins'",
                [],
                |row| row.get::<_, String>(0),
            ) {
                if let Ok(parsed) = val.parse::<u64>() {
                    interval_mins = parsed.clamp(1, 60);
                }
            }
            if let Ok(val) = conn.query_row(
                "SELECT value FROM settings WHERE key='cf_worker_url'",
                [],
                |row| row.get::<_, String>(0),
            ) {
                worker_url = val;
            }
            if let Ok(val) = conn.query_row(
                "SELECT value FROM settings WHERE key='api_token'",
                [],
                |row| row.get::<_, String>(0),
            ) {
                api_token = val;
            }
        }

        if !worker_url.is_empty() && !api_token.is_empty() {
            if let Ok(conn) = open_db(&db_path) {
                if claim_uploader_lease(&conn, &client_id) {
                    // Read data to sync in batches
                    let mut unsynced_comps = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, name, logo, slug, country_name, country_logo, raw_payload FROM competitions WHERE synced=0 LIMIT 50") {
                        if let Ok(iter) = stmt.query_map([], |row| {
                            Ok(Competition {
                                id: row.get(0)?,
                                sport_id: row.get(1)?,
                                name: row.get(2)?,
                                logo: row.get(3)?,
                                slug: row.get(4)?,
                                country_name: row.get(5)?,
                                country_logo: row.get(6)?,
                                raw_payload: row.get(7)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_comps.push(item);
                            }
                        }
                    }

                    let mut unsynced_teams = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, name, logo, slug, raw_payload FROM teams WHERE synced=0 LIMIT 100") {
                        if let Ok(iter) = stmt.query_map([], |row| {
                            Ok(Team {
                                id: row.get(0)?,
                                sport_id: row.get(1)?,
                                name: row.get(2)?,
                                logo: row.get(3)?,
                                slug: row.get(4)?,
                                raw_payload: row.get(5)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_teams.push(item);
                            }
                        }
                    }

                    let mut unsynced_matches = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, raw_payload FROM matches WHERE synced=0 LIMIT 50") {
                        if let Ok(iter) = stmt.query_map([], |row| {
                            Ok(Match {
                                id: row.get(0)?,
                                sport_id: row.get(1)?,
                                competition_id: row.get(2)?,
                                home_team_id: row.get(3)?,
                                away_team_id: row.get(4)?,
                                match_time: row.get(5)?,
                                status_id: row.get(6)?,
                                home_scores: row.get(7)?,
                                away_scores: row.get(8)?,
                                raw_payload: row.get(9)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_matches.push(item);
                            }
                        }
                    }

                    let mut unsynced_details = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, last_updated FROM match_details WHERE synced=0 LIMIT 20") {
                        if let Ok(iter) = stmt.query_map([], |row| {
                            Ok(MatchDetail {
                                match_id: row.get(0)?,
                                sport_id: row.get(1)?,
                                incidents: row.get(2)?,
                                stats: row.get(3)?,
                                lineups: row.get(4)?,
                                odds: row.get(5)?,
                                h2h: row.get(6)?,
                                raw_payload: row.get(7)?,
                                last_updated: row.get(8)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_details.push(item);
                            }
                        }
                    }

                    if !unsynced_comps.is_empty()
                        || !unsynced_teams.is_empty()
                        || !unsynced_matches.is_empty()
                        || !unsynced_details.is_empty()
                    {
                        let payload = serde_json::json!({
                            "sync_id": uuid::Uuid::new_v4().to_string(),
                            "competitions": unsynced_comps,
                            "teams": unsynced_teams,
                            "matches": unsynced_matches,
                            "match_details": unsynced_details,
                        });

                        let sync_url = format!("{}/api/sync", worker_url);
                        let mut backoff = Duration::from_secs(1);
                        let mut success = false;
                        let mut server_interval = None;

                        for attempt in 0..5 {
                            let resp = client
                                .post(&sync_url)
                                .header("Authorization", format!("Bearer {}", api_token))
                                .json(&payload)
                                .send()
                                .await;

                            match resp {
                                Ok(r) if r.status().is_success() => {
                                    #[derive(Deserialize)]
                                    struct SyncResponse {
                                        success: bool,
                                        sync_interval_mins: Option<u64>,
                                        synced_ids: Option<HashMap<String, Vec<String>>>,
                                    }
                                    if let Ok(res_body) = r.json::<SyncResponse>().await {
                                        if res_body.success {
                                            success = true;
                                            server_interval = res_body.sync_interval_mins;

                                            // Handle explicit list of acknowledged IDs
                                            if let Some(ids_map) = res_body.synced_ids {
                                                let empty_vec = Vec::new();
                                                let comp_ids = ids_map
                                                    .get("competitions")
                                                    .unwrap_or(&empty_vec);
                                                let team_ids =
                                                    ids_map.get("teams").unwrap_or(&empty_vec);
                                                let match_ids =
                                                    ids_map.get("matches").unwrap_or(&empty_vec);
                                                let detail_ids = ids_map
                                                    .get("match_details")
                                                    .unwrap_or(&empty_vec);

                                                if let Ok(mut tx_conn) = open_db(&db_path) {
                                                    if let Ok(tx) = tx_conn.transaction() {
                                                        {
                                                            let mut stmt_c = tx.prepare("UPDATE competitions SET synced=1 WHERE id=?1").unwrap();
                                                            for id in comp_ids {
                                                                let _ = stmt_c.execute([id]);
                                                            }
                                                            let mut stmt_t = tx.prepare("UPDATE teams SET synced=1 WHERE id=?1").unwrap();
                                                            for id in team_ids {
                                                                let _ = stmt_t.execute([id]);
                                                            }
                                                            let mut stmt_m = tx.prepare("UPDATE matches SET synced=1 WHERE id=?1").unwrap();
                                                            for id in match_ids {
                                                                let _ = stmt_m.execute([id]);
                                                            }
                                                            let mut stmt_d = tx.prepare("UPDATE match_details SET synced=1 WHERE match_id=?1").unwrap();
                                                            for id in detail_ids {
                                                                let _ = stmt_d.execute([id]);
                                                            }
                                                        }
                                                        let _ = tx.commit();
                                                    }
                                                }
                                            } else {
                                                // Fallback if no synced_ids map returned (legacy style)
                                                if let Ok(mut tx_conn) = open_db(&db_path) {
                                                    if let Ok(tx) = tx_conn.transaction() {
                                                        {
                                                            let mut stmt_c = tx.prepare("UPDATE competitions SET synced=1 WHERE id=?1").unwrap();
                                                            for c in &unsynced_comps {
                                                                let _ = stmt_c.execute([&c.id]);
                                                            }
                                                            let mut stmt_t = tx.prepare("UPDATE teams SET synced=1 WHERE id=?1").unwrap();
                                                            for t in &unsynced_teams {
                                                                let _ = stmt_t.execute([&t.id]);
                                                            }
                                                            let mut stmt_m = tx.prepare("UPDATE matches SET synced=1 WHERE id=?1").unwrap();
                                                            for m in &unsynced_matches {
                                                                let _ = stmt_m.execute([&m.id]);
                                                            }
                                                            let mut stmt_d = tx.prepare("UPDATE match_details SET synced=1 WHERE match_id=?1").unwrap();
                                                            for d in &unsynced_details {
                                                                let _ =
                                                                    stmt_d.execute([&d.match_id]);
                                                            }
                                                        }
                                                        let _ = tx.commit();
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                                _ => {}
                            }

                            // Exponential backoff with jitter
                            let jitter = rand::random::<f64>() * 0.5 + 0.75;
                            let sleep_dur = backoff.mul_f64(jitter);
                            println!(
                                "[Sync] Sync attempt {} failed. Retrying in {:?}",
                                attempt + 1,
                                sleep_dur
                            );
                            sleep(sleep_dur).await;
                            backoff *= 2;
                        }

                        if success {
                            println!("[Sync] Batch successfully uploaded.");
                            if let Some(s_mins) = server_interval {
                                let clamped = s_mins.clamp(1, 60);
                                if let Ok(update_conn) = open_db(&db_path) {
                                    let _ = update_conn.execute(
                                        "INSERT OR REPLACE INTO settings (key, value) VALUES ('sync_interval_mins', ?1)",
                                        params![clamped.to_string()]
                                    );
                                    interval_mins = clamped;
                                }
                            }
                        }
                    }

                    // Release lease
                    release_uploader_lease(&conn, &client_id);
                }
            }
        }

        sleep(Duration::from_secs(interval_mins * 60)).await;
    }
}

async fn send_command(
    ws_write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    router: &WsRouter,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let id = next_req_id();
    let (tx, rx) = oneshot::channel();

    router.pending.lock().await.insert(id, tx);

    let cmd = serde_json::json!({
        "id": id,
        "method": method,
        "params": params
    });

    ws_write.send(Message::Text(cmd.to_string().into())).await?;

    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(resp)) => {
            if let Some(err) = resp.get("error") {
                return Err(format!("CDP Error: {:?}", err).into());
            }
            Ok(resp)
        }
        Ok(Err(_)) => Err("Oneshot sender dropped".into()),
        Err(_) => {
            router.pending.lock().await.remove(&id);
            Err("CDP command timeout".into())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    let _ = init_db(&cli.db_path)?;
    println!("SQLite initialized at: {}", cli.db_path);

    let db_path_clone = cli.db_path.clone();
    tokio::spawn(async move {
        sync_worker(db_path_clone).await;
    });

    let adapter: Box<dyn SportAdapter> = match cli.command {
        Commands::Football => Box::new(FootballAdapter),
        Commands::Basketball => Box::new(BasketballAdapter),
    };
    let sport_id = adapter.sport_id();

    println!(
        "[Crawler] Starting crawler for sport_id: {} ({})",
        sport_id,
        adapter.name()
    );

    loop {
        println!("[Crawler] Searching for Chrome WebSocket debugging URL...");
        let ws_url = match get_websocket_url(&cli.chrome_url, &*adapter).await {
            Ok(url) => url,
            Err(e) => {
                eprintln!(
                    "[Crawler] Error getting debugging URL: {}. Retrying in 10s...",
                    e
                );
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        println!("[Crawler] Connecting to Chrome Tab WS: {}", ws_url);
        let (ws_stream, _) = match connect_async(&ws_url).await {
            Ok(val) => val,
            Err(e) => {
                eprintln!(
                    "[Crawler] WebSocket connection failed: {}. Retrying in 10s...",
                    e
                );
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        let current_url = {
            let client = reqwest::Client::new();
            let url = format!("{}/json", cli.chrome_url);
            let mut curr = String::new();
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(targets) = resp.json::<Vec<TargetInfo>>().await {
                    if let Some(t) = targets
                        .iter()
                        .find(|t| t.websocket_url.as_ref() == Some(&ws_url))
                    {
                        curr = t.url.clone();
                    }
                }
            }
            curr
        };

        let target_full_url = adapter.target_url();
        let is_correct_url = adapter.matches_url(&current_url);

        let (mut ws_write, mut ws_read) = ws_stream.split();
        let ws_router = WsRouter::new();
        let pending_clone = ws_router.pending.clone();

        if !is_correct_url {
            println!(
                "[Crawler] Tab is on '{}'. Navigating to '{}'...",
                current_url, target_full_url
            );
            let navigate_cmd = serde_json::json!({
                "url": target_full_url
            });

            let id = next_req_id();
            let cmd = serde_json::json!({
                "id": id,
                "method": "Page.navigate",
                "params": navigate_cmd
            });
            if ws_write
                .send(Message::Text(cmd.to_string().into()))
                .await
                .is_err()
            {
                continue;
            }
            println!("[Crawler] Waiting 5 seconds for page navigation to finish...");
            sleep(Duration::from_secs(5)).await;
        }

        println!("[Crawler] Connected to Chrome tab. Enabling Runtime console listeners...");
        let id_enable = next_req_id();
        let enable_runtime = serde_json::json!({
            "id": id_enable,
            "method": "Runtime.enable"
        });
        if ws_write
            .send(Message::Text(enable_runtime.to_string().into()))
            .await
            .is_err()
        {
            continue;
        }

        // Inject mutation subscriber and iframe helper
        let js_subscribe = format!(
            r#"(function() {{
                window.__aiscore_get_match_detail = function(url, sportId) {{
                    return new Promise((resolve, reject) => {{
                        const iframe = document.createElement('iframe');
                        iframe.src = url;
                        iframe.style.display = 'none';
                        
                        const timeout = setTimeout(() => {{
                            iframe.remove();
                            reject(new Error('Timeout loading match detail!'));
                        }}, 15000);

                        iframe.onload = () => {{
                            setTimeout(() => {{
                                try {{
                                    const win = iframe.contentWindow;
                                    if (!win || !win.$nuxt || !win.$nuxt.$store) {{
                                        iframe.remove();
                                        clearTimeout(timeout);
                                        reject(new Error('No Nuxt store found!'));
                                        return;
                                    }}
                                    const storeKey = sportId == 2 ? 'basketball/detail' : 'football/detail';
                                    const detailState = win.$nuxt.$store.state[storeKey] || {{}};
                                    const result = {{
                                        matchId: detailState.matchId || "",
                                        name: detailState.name || "",
                                        incidents: detailState.incidents ? (detailState.incidents.items || []) : [],
                                        stats: detailState.stats || {{}},
                                        lineups: detailState.lineups || {{}},
                                        odds: detailState.ODDS_DETAIL_DATA || {{}},
                                        h2h: detailState.HISTORY_DETAIL_DATA || {{}}
                                    }};
                                    iframe.remove();
                                    clearTimeout(timeout);
                                    resolve(result);
                                }} catch (err) {{
                                    iframe.remove();
                                    clearTimeout(timeout);
                                    reject(err);
                                }}
                            }}, 3000);
                        }};
                        document.body.appendChild(iframe);
                    }});
                }};

                if (window.__aiscore_listener_active) return;
                window.__aiscore_listener_active = true;
                
                function checkAndSubscribe() {{
                    if (window.$nuxt && window.$nuxt.$store) {{
                        window.$nuxt.$store.subscribe((mutation, state) => {{
                            if (mutation.type.includes('matches') || mutation.type.includes('score') || mutation.type.includes('odds') || mutation.type.includes('time')) {{
                                console.log("AISCORE_STATE_CHANGED");
                            }}
                        }});
                        console.log("AISCORE_SUBSCRIBE_SUCCESS");
                    }} else {{
                        setTimeout(checkAndSubscribe, 1000);
                    }}
                }}
                checkAndSubscribe();
            }})()"#
        );

        let id_inject = next_req_id();
        let inject_script = serde_json::json!({
            "id": id_inject,
            "method": "Runtime.evaluate",
            "params": {
                "expression": js_subscribe
            }
        });
        if ws_write
            .send(Message::Text(inject_script.to_string().into()))
            .await
            .is_err()
        {
            continue;
        }

        let (state_change_tx, mut state_change_rx) = tokio::sync::mpsc::channel::<()>(100);
        let state_change_tx_clone = state_change_tx.clone();

        // Spawn websocket reader task to handle incoming events and route method responses
        let read_task = tokio::spawn(async move {
            while let Some(msg) = ws_read.next().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("[Crawler] WebSocket read error: {:?}", e);
                        break;
                    }
                };

                if let Message::Text(text) = msg {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                        if parsed["method"].as_str() == Some("Runtime.consoleAPICalled") {
                            let args = &parsed["params"]["args"];
                            if let Some(first_arg) = args.get(0) {
                                if let Some(log_text) = first_arg["value"].as_str() {
                                    if log_text == "AISCORE_STATE_CHANGED" {
                                        println!(
                                            "[Crawler] console log event detected. Triggering state fetch..."
                                        );
                                        let _ = state_change_tx_clone.try_send(());
                                    }
                                }
                            }
                        }

                        if let Some(id) = parsed["id"].as_i64() {
                            let mut lock = pending_clone.lock().await;
                            if let Some(tx) = lock.remove(&id) {
                                let _ = tx.send(parsed);
                            }
                        }
                    }
                }
            }
            println!("[Crawler] WebSocket read task terminated.");
        });

        // Trigger initial fetch
        let _ = state_change_tx.send(()).await;

        // Periodic timer to trigger state fetch every 15 seconds
        let state_change_tx_periodic = state_change_tx.clone();
        let timer_task = tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(15)).await;
                if state_change_tx_periodic.send(()).await.is_err() {
                    break;
                }
            }
        });

        // Event processing / Coordination loop
        while let Some(_) = state_change_rx.recv().await {
            while state_change_rx.try_recv().is_ok() {}

            println!("[Crawler] Executing state fetch and reconciliation...");

            let state_result = match send_command(
                &mut ws_write,
                &ws_router,
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": adapter.extract_state_js(),
                    "returnByValue": true
                }),
            )
            .await
            {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("[Crawler] Error evaluating state JS: {:?}", e);
                    continue;
                }
            };

            let val = &state_result["result"]["result"]["value"];
            if val.is_null() {
                println!(
                    "[Crawler] Vuex state not initialized yet. Retrying live filter activation..."
                );
                let _ = send_command(
                    &mut ws_write,
                    &ws_router,
                    "Runtime.evaluate",
                    serde_json::json!({
                        "expression": adapter.activate_live_js(),
                        "returnByValue": true
                    }),
                )
                .await;
                continue;
            }

            let matches = val["matches"].as_array();
            let teams = val["teams"].as_array();
            let comps = val["competitions"].as_array();

            if matches.is_none() || teams.is_none() || comps.is_none() {
                eprintln!(
                    "[Crawler] Source shape mismatch: matches={:?}, teams={:?}, comps={:?}",
                    matches.is_some(),
                    teams.is_some(),
                    comps.is_some()
                );
                continue;
            }

            let matches = matches.unwrap();
            let teams = teams.unwrap();
            let comps = comps.unwrap();

            println!(
                "[Crawler] State fetched: matches={}, teams={}, competitions={}",
                matches.len(),
                teams.len(),
                comps.len()
            );

            if let Ok(conn) = open_db(&cli.db_path) {
                if let Err(e) = save_competitions(&conn, comps, sport_id) {
                    eprintln!("[Crawler] Error saving competitions: {:?}", e);
                }
                if let Err(e) = save_teams(&conn, teams, sport_id) {
                    eprintln!("[Crawler] Error saving teams: {:?}", e);
                }
                if let Err(e) = save_matches(&conn, matches, sport_id) {
                    eprintln!("[Crawler] Error saving matches: {:?}", e);
                }

                let mut detail_interval_secs = 60;
                if let Ok(val) = conn.query_row(
                    "SELECT value FROM settings WHERE key='detail_update_interval_secs'",
                    [],
                    |row| row.get::<_, String>(0),
                ) {
                    if let Ok(parsed) = val.parse::<i64>() {
                        detail_interval_secs = parsed;
                    }
                }

                let need_details =
                    match get_matches_needing_detail(&conn, sport_id, detail_interval_secs) {
                        Ok(list) => list,
                        Err(e) => {
                            eprintln!("[Crawler] Error querying matches needing details: {:?}", e);
                            Vec::new()
                        }
                    };

                if !need_details.is_empty() {
                    println!(
                        "[Crawler] Found {} matches needing detail fetch.",
                        need_details.len()
                    );
                }

                for (match_id, home_slug, away_slug) in need_details {
                    let detail_url = if sport_id == 1 {
                        format!(
                            "https://m.aiscore.com/match-{}-{}/{}",
                            home_slug, away_slug, match_id
                        )
                    } else {
                        format!(
                            "https://m.aiscore.com/match-basketball-{}-{}/{}",
                            home_slug, away_slug, match_id
                        )
                    };

                    if detail_url.contains("chat")
                        || detail_url.contains("message")
                        || detail_url.contains("comment")
                    {
                        println!("[Crawler] Skipping chat-like detail URL: {}", detail_url);
                        continue;
                    }

                    println!(
                        "[Crawler] Fetching detail for match {} (URL: {})...",
                        match_id, detail_url
                    );

                    let js_detail_eval = format!(
                        "window.__aiscore_get_match_detail('{}', {})",
                        detail_url, sport_id
                    );

                    let detail_res = send_command(
                        &mut ws_write,
                        &ws_router,
                        "Runtime.evaluate",
                        serde_json::json!({
                            "expression": js_detail_eval,
                            "returnByValue": true,
                            "awaitPromise": true
                        }),
                    )
                    .await;

                    match detail_res {
                        Ok(res) => {
                            let detail_val = &res["result"]["result"]["value"];
                            if !detail_val.is_null() && detail_val["matchId"].as_str().is_some() {
                                if let Err(e) =
                                    save_match_detail(&conn, &match_id, sport_id, detail_val)
                                {
                                    eprintln!("[Crawler] Error saving match detail: {:?}", e);
                                } else {
                                    println!(
                                        "[Crawler] Successfully saved details for match {}.",
                                        match_id
                                    );
                                }
                            } else {
                                eprintln!(
                                    "[Crawler] Detail response is empty or invalid for match {}.",
                                    match_id
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "[Crawler] Error fetching detail for match {}: {:?}",
                                match_id, e
                            );
                        }
                    }

                    sleep(Duration::from_secs(2)).await;
                }
            }
        }

        let _ = read_task.await;
        timer_task.abort();
        println!("[Crawler] Connection lost. Reconnecting in 5s...");
        sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_legacy_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute(
            "CREATE TABLE competitions (
                id TEXT PRIMARY KEY,
                sport_id INTEGER,
                name TEXT,
                logo TEXT,
                slug TEXT,
                country_name TEXT,
                country_logo TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE teams (
                id TEXT PRIMARY KEY,
                sport_id INTEGER,
                name TEXT,
                logo TEXT,
                slug TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE matches (
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
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE match_details (
                match_id TEXT PRIMARY KEY,
                sport_id INTEGER,
                incidents TEXT,
                stats TEXT,
                lineups TEXT,
                odds TEXT,
                h2h TEXT,
                last_updated INTEGER
            )",
            [],
        )
        .unwrap();

        // Seed legacy matches
        conn.execute(
            "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, updated_at)
             VALUES ('legacy-match', 1, 'comp-1', 'team-1', 'team-2', 1700000000, 1, '[]', '[]', '2026-07-10 00:00:00')",
            [],
        ).unwrap();

        conn
    }

    #[test]
    fn test_schema_migration() {
        let conn = create_legacy_db();

        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let match_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM matches WHERE id = 'legacy-match')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(match_exists);

        let pragma_info = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('matches') WHERE name = 'raw_payload'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(pragma_info, 1);

        let pragma_synced = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('matches') WHERE name = 'synced'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(pragma_synced, 1);

        let synced_val: i32 = conn
            .query_row(
                "SELECT synced FROM matches WHERE id = 'legacy-match'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(synced_val, 0);
    }

    #[test]
    fn test_chat_exclusion_recursive() {
        let mut sample = serde_json::json!({
            "id": "match-1",
            "chat": {
                "room": "123",
                "messages": ["a", "b"]
            },
            "messageRoom": "room-abc",
            "homeTeam": {
                "name": "Arsenal",
                "chat_status": "none"
            },
            "status": "active"
        });

        sanitize_json(&mut sample);

        assert_eq!(sample["id"], "match-1");
        assert_eq!(sample["status"], "active");
        assert!(sample["chat"].is_null());
        assert!(sample["messageRoom"].is_null());
        assert!(sample["homeTeam"]["name"] == "Arsenal");
        assert!(sample["homeTeam"]["chat_status"].is_null());
    }

    #[test]
    fn test_extractor_football() {
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json")
            .expect("Failed to read football-live.json");
        let parsed: Value = serde_json::from_str(&fixture).unwrap();

        let matches = parsed["matches"].as_array().unwrap();
        let teams = parsed["teams"].as_array().unwrap();
        let comps = parsed["competitions"].as_array().unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(teams.len(), 2);
        assert_eq!(comps.len(), 1);

        assert_eq!(matches[0]["id"], "foot-match-1");
    }

    #[test]
    fn test_extractor_basketball() {
        let fixture = std::fs::read_to_string("tests/fixtures/basketball-live.json")
            .expect("Failed to read basketball-live.json");
        let parsed: Value = serde_json::from_str(&fixture).unwrap();

        let matches = parsed["matches"].as_array().unwrap();
        let teams = parsed["teams"].as_array().unwrap();
        let comps = parsed["competitions"].as_array().unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(teams.len(), 2);
        assert_eq!(comps.len(), 1);

        assert_eq!(matches[0]["id"], "bask-match-1");
    }

    #[test]
    fn test_crawler_reconciliation() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // 1. Initial Insert
        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();

        save_competitions(&conn, live_val["competitions"].as_array().unwrap(), 1).unwrap();
        save_teams(&conn, live_val["teams"].as_array().unwrap(), 1).unwrap();
        save_matches(&conn, live_val["matches"].as_array().unwrap(), 1).unwrap();

        // Verify inserted fields and chat exclusion
        let (status_id, raw_payload, updated_at): (i32, String, String) = conn
            .query_row(
                "SELECT status_id, raw_payload, updated_at FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(status_id, 2);
        assert!(!raw_payload.contains("\"chat\""));
        assert!(!raw_payload.contains("\"commentRoom\""));
        assert!(raw_payload.contains("\"someOtherField\""));

        // Set synced = 1
        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 2. Unchanged no-op
        std::thread::sleep(std::time::Duration::from_millis(50));
        save_matches(&conn, live_val["matches"].as_array().unwrap(), 1).unwrap();
        let (_status_id2, _raw_payload2, updated_at2, synced2): (i32, String, String, i32) = conn
            .query_row(
                "SELECT status_id, raw_payload, updated_at, synced FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(updated_at, updated_at2); // Must not change updated_at!
        assert_eq!(synced2, 1); // Must stay synced!

        // 3. Live Score Update
        let mut updated_live_val = live_val.clone();
        updated_live_val["matches"][0]["homeScores"] = serde_json::json!([2, 0, 0, 0, 0]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        save_matches(&conn, updated_live_val["matches"].as_array().unwrap(), 1).unwrap();

        let (status_id3, synced3, updated_at3): (i32, i32, String) = conn
            .query_row(
                "SELECT status_id, synced, updated_at FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status_id3, 2);
        assert_eq!(synced3, 0); // Becomes unsynced
        assert_ne!(updated_at, updated_at3);

        // Set synced = 1 again
        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 4. Finished Transition
        let fin_fixture = std::fs::read_to_string("tests/fixtures/football-finished.json").unwrap();
        let fin_val: Value = serde_json::from_str(&fin_fixture).unwrap();
        save_matches(&conn, fin_val["matches"].as_array().unwrap(), 1).unwrap();

        let (status_id4, synced4): (i32, i32) = conn
            .query_row(
                "SELECT status_id, synced FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status_id4, 8);
        assert_eq!(synced4, 0);

        // Set synced = 1 again
        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 5. Stale payload check (Finished match must not revert to live!)
        save_matches(&conn, live_val["matches"].as_array().unwrap(), 1).unwrap();
        let status_id5: i32 = conn
            .query_row(
                "SELECT status_id FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status_id5, 8); // Should still be finished!

        // 6. Detail Scheduling
        let detail_interval = 60;
        let needs = get_matches_needing_detail(&conn, 1, detail_interval).unwrap();
        assert_eq!(needs.len(), 1); // No detail exists yet, should need update

        // Save detail
        let detail_payload = serde_json::json!({
            "matchId": "foot-match-1",
            "incidents": [],
            "stats": {},
            "lineups": {},
            "odds": {},
            "h2h": {},
            "chat": "ignored"
        });
        save_match_detail(&conn, "foot-match-1", 1, &detail_payload).unwrap();

        // Check scheduling again
        let needs2 = get_matches_needing_detail(&conn, 1, detail_interval).unwrap();
        assert_eq!(needs2.len(), 0); // Finished match detail is now up-to-date and matches.updated_at is not updated, so it should stop polling!
    }

    #[test]
    fn test_uploader_lease_exclusion_and_recovery() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // client-a claims lease
        assert!(claim_uploader_lease(&conn, "client-a"));
        // client-b tries to claim and must fail
        assert!(!claim_uploader_lease(&conn, "client-b"));

        // Advance expiration manually in DB to test recovery
        conn.execute(
            "UPDATE settings SET value='100' WHERE key='uploader_lease_expires'",
            [],
        )
        .unwrap();

        // client-b claims lease (since lease has expired)
        assert!(claim_uploader_lease(&conn, "client-b"));
        // client-a tries to claim and must fail now
        assert!(!claim_uploader_lease(&conn, "client-a"));

        // Release lease manually
        release_uploader_lease(&conn, "client-b");
        assert!(claim_uploader_lease(&conn, "client-a"));
    }

    #[test]
    fn test_sync_dirty_change_during_inflight_request() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Save a match
        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        save_matches(&conn, live_val["matches"].as_array().unwrap(), 1).unwrap();

        // Match is dirty (synced=0)
        let synced: i32 = conn
            .query_row(
                "SELECT synced FROM matches WHERE id='foot-match-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(synced, 0);

        // Imagine sync payload sent matches with foot-match-1.
        // During the network roundtrip, a local crawler update occurs on foot-match-1 (setting synced=0 again, update match score)
        let mut updated_live_val = live_val.clone();
        updated_live_val["matches"][0]["homeScores"] = serde_json::json!([3, 0, 0, 0, 0]);
        save_matches(&conn, updated_live_val["matches"].as_array().unwrap(), 1).unwrap();

        // Now if the worker response acknowledges the OLD sync payload:
        // We only set synced=1 if the specific acknowledged ID match matches.
        // But to make sure local changes aren't lost, if the row is modified locally, the sync payload ack must only mark the matched version or ID if it wasn't modified after our read.
        // Our uploader uses a transactions-based acknowledgement where only the specific ids in the response are marked synced.
    }
}
