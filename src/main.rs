use clap::{Parser, Subcommand};
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{SinkExt, StreamExt};

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

// Function to fetch the websocket URL for the target page from Chrome DevTools
async fn get_websocket_url(chrome_url: &str, _target_path: &str, sport_id: i32) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = format!("{}/json", chrome_url);
    let resp = client.get(&url).send().await?;
    let targets: Vec<TargetInfo> = resp.json().await?;

    // 1. Search for target matching our sport
    for target in &targets {
        let is_match = if sport_id == 1 {
            (target.url.contains("m.aiscore.com") || target.url.contains("aiscore.com"))
                && !target.url.contains("basketball")
                && !target.url.contains("tennis")
                && !target.url.contains("baseball")
        } else {
            target.url.contains("basketball")
        };

        if is_match {
            if let Some(ref ws_url) = target.websocket_url {
                return Ok(ws_url.clone());
            }
        }
    }

    // 2. Hijack any standard page tab
    for target in &targets {
        if target.target_type.as_deref() == Some("page") {
            if let Some(ref ws_url) = target.websocket_url {
                println!("[Crawler] Hijacking empty/active tab ({}) to open AiScore...", target.url);
                return Ok(ws_url.clone());
            }
        }
    }

    Err(format!("No debuggable browser page found in Chrome! Please open Chrome.").into())
}

// Initialize SQLite database
fn init_db(db_path: &str) -> SqlResult<Connection> {
    let conn = Connection::open(db_path)?;
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
            updated_at TEXT,
            synced INTEGER DEFAULT 0
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
            last_updated INTEGER,
            synced INTEGER DEFAULT 0
         )",
        [],
    )?;
    // Defaults
    conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_interval_mins', '5')", [])?;
    conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('cf_worker_url', 'http://127.0.0.1:8080')", [])?;
    conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('api_token', 'super-secret-token')", [])?;
    conn.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('detail_update_interval_secs', '60')", [])?;
    
    Ok(conn)
}

fn save_competitions(conn: &Connection, comps: &[Value], sport_id: i32) -> SqlResult<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            logo = excluded.logo,
            slug = excluded.slug,
            country_name = excluded.country_name,
            country_logo = excluded.country_logo"
    )?;
    
    for c in comps {
        let id = c["id"].as_str().unwrap_or_default();
        if id.is_empty() { continue; }
        let name = c["name"].as_str().unwrap_or("");
        let logo = c["logo"].as_str();
        let slug = c["slug"].as_str();
        let country_name = c["country"]["name"].as_str();
        let country_logo = c["country"]["logo"].as_str();

        stmt.execute(params![id, sport_id, name, logo, slug, country_name, country_logo])?;
    }
    Ok(())
}

fn save_teams(conn: &Connection, teams: &[Value], sport_id: i32) -> SqlResult<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO teams (id, sport_id, name, logo, slug)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            logo = excluded.logo,
            slug = excluded.slug"
    )?;
    
    for t in teams {
        let id = t["id"].as_str().unwrap_or_default();
        if id.is_empty() { continue; }
        let name = t["name"].as_str().unwrap_or("");
        let logo = t["logo"].as_str();
        let slug = t["slug"].as_str();

        stmt.execute(params![id, sport_id, name, logo, slug])?;
    }
    Ok(())
}

fn save_matches(conn: &Connection, matches: &[Value], sport_id: i32) -> SqlResult<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, updated_at, synced)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), 0)
         ON CONFLICT(id) DO UPDATE SET
            status_id = excluded.status_id,
            home_scores = excluded.home_scores,
            away_scores = excluded.away_scores,
            updated_at = datetime('now'),
            synced = CASE 
                WHEN status_id != excluded.status_id OR home_scores != excluded.home_scores OR away_scores != excluded.away_scores THEN 0 
                ELSE synced 
            END"
    )?;
    
    for m in matches {
        let id = m["id"].as_str().unwrap_or_default();
        if id.is_empty() { continue; }
        let comp_id = m["competition"]["id"].as_str().unwrap_or("");
        let home_id = m["homeTeam"]["id"].as_str().unwrap_or("");
        let away_id = m["awayTeam"]["id"].as_str().unwrap_or("");
        let match_time = m["matchTime"].as_i64().unwrap_or(0);
        let status_id = m["statusId"].as_i64().unwrap_or(0) as i32;
        let home_scores = m["homeScores"].to_string();
        let away_scores = m["awayScores"].to_string();

        stmt.execute(params![id, sport_id, comp_id, home_id, away_id, match_time, status_id, home_scores, away_scores])?;
    }
    Ok(())
}

// Background sync worker that pushes dirty data to Cloudflare Workers D1
async fn sync_worker(db_path: String) {
    println!("[Sync] Starting sync worker background thread...");
    let client = reqwest::Client::new();

    loop {
        // Scope settings query so Connection is dropped before await
        let (interval_mins, worker_url, api_token) = {
            let mut interval_mins = 5;
            let mut worker_url = String::new();
            let mut api_token = String::new();

            if let Ok(conn) = Connection::open(&db_path) {
                if let Ok(val) = conn.query_row("SELECT value FROM settings WHERE key='sync_interval_mins'", [], |row| row.get::<_, String>(0)) {
                    if let Ok(parsed) = val.parse::<u64>() {
                        interval_mins = parsed;
                    }
                }
                if let Ok(val) = conn.query_row("SELECT value FROM settings WHERE key='cf_worker_url'", [], |row| row.get::<_, String>(0)) {
                    worker_url = val;
                }
                if let Ok(val) = conn.query_row("SELECT value FROM settings WHERE key='api_token'", [], |row| row.get::<_, String>(0)) {
                    api_token = val;
                }
            }
            (interval_mins, worker_url, api_token)
        };

        if !worker_url.is_empty() && !api_token.is_empty() {
            // Scope matches, teams, comps fetching so Connection and Statement are dropped before await
            let sync_data = {
                let mut data = None;
                if let Ok(conn) = Connection::open(&db_path) {
                    let mut unsynced_matches = Vec::new();
                    if let Ok(mut stmt_matches) = conn.prepare("SELECT id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores FROM matches WHERE synced=0") {
                        let matches_iter = stmt_matches.query_map([], |row| {
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
                            })
                        });
                        if let Ok(iter) = matches_iter {
                            for m in iter {
                                if let Ok(match_obj) = m {
                                    unsynced_matches.push(match_obj);
                                }
                            }
                        }
                    }

                    let mut unsynced_details = Vec::new();
                    if let Ok(mut stmt_details) = conn.prepare("SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated FROM match_details WHERE synced=0") {
                        let details_iter = stmt_details.query_map([], |row| {
                            Ok(MatchDetail {
                                match_id: row.get(0)?,
                                sport_id: row.get(1)?,
                                incidents: row.get(2)?,
                                stats: row.get(3)?,
                                lineups: row.get(4)?,
                                odds: row.get(5)?,
                                h2h: row.get(6)?,
                                last_updated: row.get(7)?,
                            })
                        });
                        if let Ok(iter) = details_iter {
                            for d in iter {
                                if let Ok(detail_obj) = d {
                                    unsynced_details.push(detail_obj);
                                }
                            }
                        }
                    }

                    if !unsynced_matches.is_empty() || !unsynced_details.is_empty() {
                        let mut referenced_teams = Vec::new();
                        let mut referenced_comps = Vec::new();

                        for m in &unsynced_matches {
                            // Query home team
                            if let Ok(t) = conn.query_row("SELECT id, sport_id, name, logo, slug FROM teams WHERE id=?1", [&m.home_team_id], |row| {
                                Ok(Team { id: row.get(0)?, sport_id: row.get(1)?, name: row.get(2)?, logo: row.get(3)?, slug: row.get(4)? })
                            }) { referenced_teams.push(t); }

                            // Query away team
                            if let Ok(t) = conn.query_row("SELECT id, sport_id, name, logo, slug FROM teams WHERE id=?1", [&m.away_team_id], |row| {
                                Ok(Team { id: row.get(0)?, sport_id: row.get(1)?, name: row.get(2)?, logo: row.get(3)?, slug: row.get(4)? })
                            }) { referenced_teams.push(t); }

                            // Query competition
                            if let Ok(c) = conn.query_row("SELECT id, sport_id, name, logo, slug, country_name, country_logo FROM competitions WHERE id=?1", [&m.competition_id], |row| {
                                Ok(Competition { id: row.get(0)?, sport_id: row.get(1)?, name: row.get(2)?, logo: row.get(3)?, slug: row.get(4)?, country_name: row.get(5)?, country_logo: row.get(6)? })
                            }) { referenced_comps.push(c); }
                        }

                        // Deduplicate
                        referenced_teams.sort_by_key(|t| t.id.clone());
                        referenced_teams.dedup_by_key(|t| t.id.clone());
                        referenced_comps.sort_by_key(|c| c.id.clone());
                        referenced_comps.dedup_by_key(|c| c.id.clone());

                        data = Some((unsynced_matches, unsynced_details, referenced_teams, referenced_comps));
                    }
                }
                data
            };

            // Now perform async send without holding connection or statement
            if let Some((matches, details, teams, competitions)) = sync_data {
                println!("[Sync] Found {} unsynced matches and {} details. Pushing to Cloudflare Workers...", matches.len(), details.len());
                let payload = serde_json::json!({
                    "matches": matches,
                    "match_details": details,
                    "teams": teams,
                    "competitions": competitions
                });

                let sync_url = format!("{}/api/sync", worker_url);
                let resp = client.post(&sync_url)
                    .header("Authorization", format!("Bearer {}", api_token))
                    .json(&payload)
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        println!("[Sync] Successfully synced {} matches and {} details to Cloudflare D1.", matches.len(), details.len());
                        // Mark as synced (new scoped transaction connection)
                        if let Ok(mut conn) = Connection::open(&db_path) {
                            if let Ok(tx) = conn.transaction() {
                                {
                                    let mut stmt_update = tx.prepare("UPDATE matches SET synced=1 WHERE id=?1").unwrap();
                                    for m in &matches {
                                        let _ = stmt_update.execute([&m.id]);
                                    }
                                    let mut stmt_detail_update = tx.prepare("UPDATE match_details SET synced=1 WHERE match_id=?1").unwrap();
                                    for d in &details {
                                        let _ = stmt_detail_update.execute([&d.match_id]);
                                    }
                                }
                                let _ = tx.commit();
                            }
                        }
                    }
                    Ok(r) => {
                        eprintln!("[Sync] Failed to sync. Cloudflare Worker returned status: {}", r.status());
                    }
                    Err(e) => {
                        eprintln!("[Sync] Network error syncing: {:?}", e);
                    }
                }
            }
        }

        sleep(Duration::from_secs(interval_mins * 60)).await;
    }
}

// Function to fetch store data from browser page
async fn trigger_state_fetch(ws_url: &str, sport_id: i32, db_path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut ws_stream, _) = connect_async(ws_url).await?;

    let js_code = format!(
        r#"(function() {{
            const fHome = window.$nuxt.$store.state['football/home'] || window.$nuxt.$store.state['home'];
            const bHome = window.$nuxt.$store.state['basketball'] || window.$nuxt.$store.state['basketball/player'];
            const activeModule = {} == 1 ? fHome : bHome;
            if (!activeModule) return null;
            return {{
                matches: activeModule.matchesData_matches || [],
                teams: activeModule.matchesData_teams || [],
                competitions: activeModule.matchesData_competitions || []
            }};
        }})()"#,
        sport_id
    );

    let eval_msg = serde_json::json!({
        "id": 99,
        "method": "Runtime.evaluate",
        "params": {
            "expression": js_code,
            "returnByValue": true
        }
    });

    ws_stream.send(Message::Text(eval_msg.to_string().into())).await?;

    // Wait for response
    while let Some(msg) = ws_stream.next().await {
        let msg = msg?;
        if let Message::Text(text) = msg {
            let parsed: Value = serde_json::from_str(&text)?;
            if parsed["id"].as_i64() == Some(99) {
                let result = &parsed["result"]["result"]["value"];
                if result.is_null() {
                    println!("[Crawler] Nuxt Vuex module state is not initialized yet.");
                    break;
                }
                
                let matches = result["matches"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                let teams = result["teams"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                let comps = result["competitions"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);

                println!("[Crawler] State fetched successfully. Matches: {}, Teams: {}, Leagues: {}", matches.len(), teams.len(), comps.len());

                if let Ok(conn) = Connection::open(db_path) {
                    save_competitions(&conn, comps, sport_id)?;
                    save_teams(&conn, teams, sport_id)?;
                    save_matches(&conn, matches, sport_id)?;
                    println!("[Crawler] Saved to local SQLite database.");

                    // Query the update interval from settings table
                    let mut detail_interval_secs = 60;
                    if let Ok(val) = conn.query_row("SELECT value FROM settings WHERE key='detail_update_interval_secs'", [], |row| row.get::<_, String>(0)) {
                        if let Ok(parsed) = val.parse::<i64>() {
                            detail_interval_secs = parsed;
                        }
                    }

                    // Query matches that need detail updates
                    let live_matches = {
                        let mut list = Vec::new();
                        let query = "
                            SELECT matches.id, t1.slug, t2.slug 
                            FROM matches 
                            JOIN teams t1 ON matches.home_team_id = t1.id 
                            JOIN teams t2 ON matches.away_team_id = t2.id 
                            WHERE matches.sport_id = ?1 
                              AND matches.status_id != 1 
                              AND (
                                matches.id NOT IN (SELECT match_id FROM match_details)
                                OR
                                (
                                  matches.status_id != 8
                                  AND ?2 > 0
                                  AND matches.id IN (
                                    SELECT match_id FROM match_details WHERE strftime('%s', 'now') - last_updated > ?2
                                  )
                                )
                                OR
                                (
                                  matches.status_id = 8
                                  AND matches.id IN (
                                    SELECT match_id FROM match_details WHERE last_updated < matches.match_time + 10800
                                  )
                                )
                              )
                        ";
                        if let Ok(mut stmt) = conn.prepare(query) {
                            if let Ok(mut rows) = stmt.query(params![sport_id, detail_interval_secs]) {
                                while let Some(row) = rows.next().unwrap_or(None) {
                                    if let (Ok(id), Ok(home_slug), Ok(away_slug)) = (row.get::<_, String>(0), row.get::<_, String>(1), row.get::<_, String>(2)) {
                                        list.push((id, home_slug, away_slug));
                                    }
                                }
                            }
                        }
                        list
                    };

                    for (match_id, home_slug, away_slug) in live_matches {
                        let url = if sport_id == 1 {
                            format!("https://m.aiscore.com/match-{}-{}/{}", home_slug, away_slug, match_id)
                        } else {
                            format!("https://m.aiscore.com/match-basketball-{}-{}/{}", home_slug, away_slug, match_id)
                        };

                        println!("[Crawler] Fetching live details for match {} (URL: {})...", match_id, url);
                        let eval_js = format!("window.__aiscore_get_match_detail('{}', {})", url, sport_id);
                        let eval_msg = serde_json::json!({
                            "id": 100,
                            "method": "Runtime.evaluate",
                            "params": {
                                "expression": eval_js,
                                "returnByValue": true,
                                "awaitPromise": true
                            }
                        });

                        if ws_stream.send(Message::Text(eval_msg.to_string().into())).await.is_err() {
                            break;
                        }

                        // Wait for response 100
                        let mut detail_saved = false;
                        while let Some(msg) = ws_stream.next().await {
                            if let Ok(Message::Text(text)) = msg {
                                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                                    if parsed["id"].as_i64() == Some(100) {
                                        let result = &parsed["result"]["result"]["value"];
                                        if !result.is_null() && result["matchId"].as_str().is_some() {
                                            let incidents = result["incidents"].to_string();
                                            let stats = result["stats"].to_string();
                                            let lineups = result["lineups"].to_string();
                                            let odds = result["odds"].to_string();
                                            let h2h = result["h2h"].to_string();

                                            let _ = conn.execute(
                                                "INSERT INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated, synced)
                                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s', 'now'), 0)
                                                 ON CONFLICT(match_id) DO UPDATE SET
                                                    incidents = excluded.incidents,
                                                    stats = excluded.stats,
                                                    lineups = excluded.lineups,
                                                    odds = excluded.odds,
                                                    h2h = excluded.h2h,
                                                    last_updated = excluded.last_updated,
                                                    synced = 0",
                                                params![match_id, sport_id, incidents, stats, lineups, odds, h2h]
                                            );
                                            detail_saved = true;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        if detail_saved {
                            println!("[Crawler] Saved details for match {}.", match_id);
                        } else {
                            println!("[Crawler] Failed to fetch details for match {}.", match_id);
                        }
                        // Sleep 2 seconds between iframe requests to let Chrome breathe
                        sleep(Duration::from_secs(2)).await;
                    }
                }
                break;
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    
    // Initialize DB
    let _ = init_db(&cli.db_path)?;
    println!("SQLite initialized at: {}", cli.db_path);

    let db_path_clone = cli.db_path.clone();
    tokio::spawn(async move {
        sync_worker(db_path_clone).await;
    });

    let (sport_id, target_path) = match cli.command {
        Commands::Football => (1, "m.aiscore.com"),
        Commands::Basketball => (2, "m.aiscore.com/basketball"),
    };

    println!("[Crawler] Starting crawler for sport_id: {} on {}", sport_id, target_path);

    loop {
        println!("[Crawler] Searching for Chrome WebSocket debugging URL...");
        let ws_url = match get_websocket_url(&cli.chrome_url, target_path, sport_id).await {
            Ok(url) => url,
            Err(e) => {
                eprintln!("[Crawler] Error getting debugging URL: {}. Retrying in 10s...", e);
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        println!("[Crawler] Connecting to Chrome Tab WS: {}", ws_url);
        let (mut ws_stream, _) = match connect_async(&ws_url).await {
            Ok(val) => val,
            Err(e) => {
                eprintln!("[Crawler] WebSocket connection failed: {}. Retrying in 10s...", e);
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        // Check current URL of the page and navigate if it is not correct
        let current_url = {
            let client = reqwest::Client::new();
            let url = format!("{}/json", cli.chrome_url);
            let mut curr = String::new();
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(targets) = resp.json::<Vec<TargetInfo>>().await {
                    if let Some(t) = targets.iter().find(|t| t.websocket_url.as_ref() == Some(&ws_url)) {
                        curr = t.url.clone();
                    }
                }
            }
            curr
        };

        let target_full_url = if sport_id == 1 {
            "https://m.aiscore.com/"
        } else {
            "https://m.aiscore.com/basketball"
        };

        let is_correct_url = if sport_id == 1 {
            (current_url.contains("m.aiscore.com") || current_url.contains("aiscore.com"))
                && !current_url.contains("basketball")
                && !current_url.contains("tennis")
                && !current_url.contains("baseball")
        } else {
            current_url.contains("basketball")
        };

        if !is_correct_url {
            println!("[Crawler] Tab is on '{}'. Navigating to '{}'...", current_url, target_full_url);
            let navigate_cmd = serde_json::json!({
                "id": 10,
                "method": "Page.navigate",
                "params": {
                    "url": target_full_url
                }
            });
            if ws_stream.send(Message::Text(navigate_cmd.to_string().into())).await.is_err() {
                continue;
            }
            // Wait 5 seconds for page load
            println!("[Crawler] Waiting 5 seconds for page navigation to finish...");
            sleep(Duration::from_secs(5)).await;
        }

        println!("[Crawler] Connected to Chrome tab. Enabling Runtime console listeners...");
        let enable_runtime = serde_json::json!({
            "id": 1,
            "method": "Runtime.enable"
        });
        if ws_stream.send(Message::Text(enable_runtime.to_string().into())).await.is_err() {
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
            }})()"#,
        );

        let inject_script = serde_json::json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": js_subscribe
            }
        });
        if ws_stream.send(Message::Text(inject_script.to_string().into())).await.is_err() {
            continue;
        }

        // Create a channel to serialize and throttle trigger_state_fetch calls
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(100);
        
        let db_path_clone = cli.db_path.clone();
        let ws_url_clone = ws_url.clone();
        tokio::spawn(async move {
            while let Some(_) = rx.recv().await {
                // Drain any pending notifications to coalesce updates
                while rx.try_recv().is_ok() {}
                
                let _ = trigger_state_fetch(&ws_url_clone, sport_id, &db_path_clone).await;
                // Wait 10 seconds to throttle requests and avoid spamming Chrome
                sleep(Duration::from_secs(10)).await;
            }
        });

        // Trigger initial fetch of data
        let _ = tx.send(()).await;

        // Periodic timer to check for details updates every 15 seconds
        let tx_timer = tx.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(15)).await;
                let _ = tx_timer.try_send(());
            }
        });

        // Main event loop
        println!("[Crawler] Listening for live score updates from page...");
        while let Some(msg) = ws_stream.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[Crawler] WebSocket stream error: {}. Reconnecting...", e);
                    break;
                }
            };

            if let Message::Text(text) = msg {
                let parsed: Value = match serde_json::from_str(&text) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                if parsed["method"].as_str() == Some("Runtime.consoleAPICalled") {
                    let args = &parsed["params"]["args"];
                    if let Some(first_arg) = args.get(0) {
                        if let Some(log_text) = first_arg["value"].as_str() {
                            if log_text == "AISCORE_STATE_CHANGED" {
                                println!("[Crawler] Live state changed mutation detected. Queueing fetch...");
                                let _ = tx.try_send(());
                            }
                        }
                    }
                }
            }
        }

        println!("[Crawler] Connection lost. Reconnecting in 5s...");
        sleep(Duration::from_secs(5)).await;
    }
}
