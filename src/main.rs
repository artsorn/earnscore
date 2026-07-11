use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use rusqlite::{Connection, OptionalExtension, Result as SqlResult, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub mod storage;

static REQ_ID: AtomicI64 = AtomicI64::new(1000);
fn next_req_id() -> i64 {
    REQ_ID.fetch_add(1, Ordering::SeqCst)
}

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "aiscore-crawler")]
#[command(about = "AiScore Live Football & Basketball Crawler", long_about = None)]
struct Cli {
    /// Chrome remote debugging HTTP URL (e.g. http://127.0.0.1:9223)
    #[arg(short, long, default_value = "http://192.168.224.1:9223")]
    chrome_url: String,

    /// SQLite database path
    #[arg(short, long, default_value = "local.db")]
    db_path: String,

    /// Minimum delay (ms) after list-page navigation before probing readiness
    #[arg(long, default_value = "1500", value_parser = parse_ready_delay_ms)]
    page_ready_delay_ms: u64,

    /// Minimum delay (ms) after detail-page navigation before extracting data
    #[arg(long, default_value = "2000", value_parser = parse_ready_delay_ms)]
    detail_ready_delay_ms: u64,

    /// Maximum number of detail fetch tabs open simultaneously (1..=10)
    #[arg(long, default_value = "3", value_parser = parse_detail_concurrency)]
    detail_concurrency: usize,

    #[command(subcommand)]
    command: Commands,
}

fn parse_ready_delay_ms(s: &str) -> Result<u64, String> {
    let v: u64 = s
        .parse()
        .map_err(|_| format!("'{}' is not a valid number", s))?;
    if v == 0 {
        return Err("page-ready-delay-ms must be > 0 ms".to_string());
    }
    if v > 30_000 {
        return Err("page-ready-delay-ms must be <= 30000 ms (30 s)".to_string());
    }
    Ok(v)
}

fn parse_detail_concurrency(s: &str) -> Result<usize, String> {
    let v: usize = s
        .parse()
        .map_err(|_| format!("'{}' is not a valid number", s))?;
    if v == 0 {
        return Err("detail-concurrency must be >= 1".to_string());
    }
    if v > 10 {
        return Err("detail-concurrency must be <= 10 to avoid tab explosion".to_string());
    }
    Ok(v)
}

#[derive(Subcommand, Clone)]
enum Commands {
    Football,
    Basketball,
    /// Apply the local v3 migration, optionally after creating a verified backup.
    Migrate {
        #[arg(long)]
        backup_destination: Option<String>,
    },
    /// Create a verified SQLite backup without mutating the source database.
    Backup {
        #[arg(long)]
        destination: String,
    },
    /// Restore a verified backup through a temporary integrity-checked copy.
    Restore {
        #[arg(long)]
        backup: String,
        #[arg(long)]
        destination: String,
    },
}

// ── Path / file helpers ───────────────────────────────────────────────────────

fn get_absolute_normalized_path(path_str: &str) -> std::io::Result<String> {
    let path = std::path::Path::new(path_str);
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    use std::path::{Component, PathBuf};
    let mut normalized = PathBuf::new();
    for component in absolute_path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(c) => {
                normalized.push(c);
            }
            Component::CurDir => {}
            other => {
                normalized.push(other.as_os_str());
            }
        }
    }

    Ok(normalized.to_string_lossy().to_string())
}

#[cfg(unix)]
fn get_file_identity(path: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn get_file_identity(path: &str) -> Option<(u64, u64)> {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path).ok().and_then(|m| {
        let mod_time = m
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs();
        let size = m.len();
        Some((mod_time, size))
    })
}

fn verify_db_identity(
    path: &str,
    expected_dataset_id: &str,
    expected_identity: Option<(u64, u64)>,
) -> bool {
    if !std::path::Path::new(path).exists() {
        return false;
    }
    let current_identity = get_file_identity(path);
    if current_identity != expected_identity {
        return false;
    }
    if let Ok(conn) = open_db(path) {
        let stored_dataset_id: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'active_dataset_id'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None);
        if let Some(stored_id) = stored_dataset_id {
            if stored_id == expected_dataset_id {
                return true;
            }
        }
    }
    false
}

fn init_dataset_id(conn: &Connection) -> SqlResult<String> {
    storage::init_dataset_id(conn)
}

// ── Data models ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Team {
    id: String,
    sport_id: i32,
    name: String,
    logo: Option<String>,
    slug: Option<String>,
    raw_payload: Option<String>,
    dataset_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Competition {
    id: String,
    sport_id: i32,
    name: String,
    logo: Option<String>,
    slug: Option<String>,
    country_name: Option<String>,
    country_logo: Option<String>,
    raw_payload: Option<String>,
    dataset_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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
    is_live: bool,
    raw_payload: Option<String>,
    dataset_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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
    dataset_id: String,
}

// ── Chrome target info (from /json endpoint) ──────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
struct TargetInfo {
    #[serde(default)]
    url: String,
    #[serde(rename = "type")]
    target_type: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
    #[serde(default)]
    id: String,
}

// ── Owned target registry ─────────────────────────────────────────────────────

/// Tracks a tab that was created by this crawler session.
#[derive(Debug, Clone)]
struct OwnedTarget {
    target_id: String,
    websocket_url: String,
    role: TargetRole,
    sport_id: i32,
    /// Requested match ID for detail targets.
    requested_match_id: Option<String>,
    created_at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
enum TargetRole {
    List,
    Detail,
}

/// Close an owned target via the Chrome HTTP API (best-effort).
async fn close_target(chrome_url: &str, target_id: &str) {
    let client = reqwest::Client::new();
    let url = format!("{}/json/close/{}", chrome_url, target_id);
    let _ = client.get(&url).send().await;
}

/// Open a new Chrome tab via `PUT /json/new?url=<url>` and return the target info.
/// This NEVER hijacks an existing user tab.
async fn create_dedicated_target(
    chrome_url: &str,
    initial_url: &str,
) -> Result<TargetInfo, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = format!("{}/json/new?{}", chrome_url, initial_url);
    let resp = client.put(&url).send().await?;
    let target: TargetInfo = resp.json().await?;
    if target.websocket_url.is_none() {
        return Err("Created target has no webSocketDebuggerUrl".into());
    }
    Ok(target)
}

// ── Sport adapters ────────────────────────────────────────────────────────────

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

            const terminal = /(^|\b)(ft|aet|full[\s-]*time|finished|ended|after penalties|cancel(?:led|ed)|postponed|abandoned|awarded|walkover)(\b|$)/i;
            const statusId = match => Number(match?.statusId ?? match?.status_id ?? match?.status?.id ?? 0);
            const hasTerminalStatus = match => [
                match?.statusText, match?.status_text, match?.statusLabel, match?.status_label,
                match?.statusName, match?.stateName, match?.matchStatus, match?.state,
                typeof match?.status === 'object' ? (match.status.name || match.status.label || match.status.text) : match?.status
            ].some(value => typeof value === 'string' && terminal.test(value.trim()));
            const isLiveMatch = match => [2, 3, 4, 5, 6, 7].includes(statusId(match)) && !hasTerminalStatus(match);

            const matches = normalize(fHome.matchesData_matches || fHome.matches || fHome.list).filter(isLiveMatch);
            const teamIds = new Set(matches.flatMap(match => [
                match?.homeTeam?.id || match?.homeTeamId || match?.home_team_id,
                match?.awayTeam?.id || match?.awayTeamId || match?.away_team_id
            ].filter(Boolean).map(String)));
            const competitionIds = new Set(matches.map(match =>
                match?.competition?.id || match?.competitionId || match?.competition_id
            ).filter(Boolean).map(String));
            const teams = normalize(fHome.matchesData_teams || fHome.teams)
                .filter(team => team && teamIds.has(String(team.id)));
            const competitions = normalize(fHome.matchesData_competitions || fHome.competitions)
                .filter(competition => competition && competitionIds.has(String(competition.id)));

            const getActiveFilter = () => {
                const path = (window.$nuxt && window.$nuxt.$route && window.$nuxt.$route.path) || window.location.pathname;
                if (path.includes('/live')) return 'live';
                if (fHome.filter === 'live' || fHome.dataType === 'live' || fHome.tab === 'live') return 'live';
                const btn = Array.from(document.querySelectorAll('div, span, a, li, button'))
                    .find(e => e.textContent && e.textContent.trim().toLowerCase() === 'live');
                if (btn) {
                    const cls = (btn.className || '').toString().toLowerCase();
                    const parentCls = (btn.parentElement?.className || '').toString().toLowerCase();
                    if (cls.includes('active') || cls.includes('selected') || cls.includes('cur') || cls.includes('current') || cls.includes('on')
                        || parentCls.includes('active') || parentCls.includes('selected') || parentCls.includes('cur') || parentCls.includes('current') || parentCls.includes('on')) {
                        return 'live';
                    }
                }
                return 'all';
            };

            return {
                matches,
                teams,
                competitions,
                activeFilter: getActiveFilter(),
                sportId: 1,
                timestamp: Date.now()
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

            const terminal = /(^|\b)(ft|aet|full[\s-]*time|finished|ended|after penalties|cancel(?:led|ed)|postponed|abandoned|awarded|walkover)(\b|$)/i;
            const statusId = match => Number(match?.statusId ?? match?.status_id ?? match?.status?.id ?? 0);
            const hasTerminalStatus = match => [
                match?.statusText, match?.status_text, match?.statusLabel, match?.status_label,
                match?.statusName, match?.stateName, match?.matchStatus, match?.state,
                typeof match?.status === 'object' ? (match.status.name || match.status.label || match.status.text) : match?.status
            ].some(value => typeof value === 'string' && terminal.test(value.trim()));
            const isLiveMatch = match => [2, 3, 4, 5, 6, 7, 9].includes(statusId(match)) && !hasTerminalStatus(match);

            const matches = normalize(bHome.matchesData_matches || bHome.matches || bHome.list).filter(isLiveMatch);
            const teamIds = new Set(matches.flatMap(match => [
                match?.homeTeam?.id || match?.homeTeamId || match?.home_team_id,
                match?.awayTeam?.id || match?.awayTeamId || match?.away_team_id
            ].filter(Boolean).map(String)));
            const competitionIds = new Set(matches.map(match =>
                match?.competition?.id || match?.competitionId || match?.competition_id
            ).filter(Boolean).map(String));
            const teams = normalize(bHome.matchesData_teams || bHome.teams)
                .filter(team => team && teamIds.has(String(team.id)));
            const competitions = normalize(bHome.matchesData_competitions || bHome.competitions)
                .filter(competition => competition && competitionIds.has(String(competition.id)));

            const getActiveFilter = () => {
                const path = (window.$nuxt && window.$nuxt.$route && window.$nuxt.$route.path) || window.location.pathname;
                if (path.includes('/live')) return 'live';
                if (bHome.filter === 'live' || bHome.dataType === 'live' || bHome.tab === 'live') return 'live';
                const btn = Array.from(document.querySelectorAll('div, span, a, li, button'))
                    .find(e => e.textContent && e.textContent.trim().toLowerCase() === 'live');
                if (btn) {
                    const cls = (btn.className || '').toString().toLowerCase();
                    const parentCls = (btn.parentElement?.className || '').toString().toLowerCase();
                    if (cls.includes('active') || cls.includes('selected') || cls.includes('cur') || cls.includes('current') || cls.includes('on')
                        || parentCls.includes('active') || parentCls.includes('selected') || parentCls.includes('cur') || parentCls.includes('current') || parentCls.includes('on')) {
                        return 'live';
                    }
                }
                return 'all';
            };

            return {
                matches,
                teams,
                competitions,
                activeFilter: getActiveFilter(),
                sportId: 2,
                timestamp: Date.now()
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

// ── Source / detail snapshot types ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SourceSnapshot {
    matches: Vec<Value>,
    teams: Vec<Value>,
    competitions: Vec<Value>,
    #[serde(rename = "activeFilter")]
    active_filter: String,
    #[serde(rename = "sportId")]
    sport_id: i32,
    timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtractionError {
    FilterNotLive,
    CrossSportMismatch {
        expected: i32,
        found: i32,
    },
    EmptyMatches,
    IncompleteRelations {
        missing_competitions: Vec<String>,
        missing_teams: Vec<String>,
    },
    MissingRequiredField(String),
    JsonError(String),
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractionError::FilterNotLive => write!(f, "filter_not_live"),
            ExtractionError::CrossSportMismatch { expected, found } => {
                write!(
                    f,
                    "cross_sport_mismatch: expected {}, found {}",
                    expected, found
                )
            }
            ExtractionError::EmptyMatches => write!(f, "empty_matches"),
            ExtractionError::IncompleteRelations {
                missing_competitions,
                missing_teams,
            } => {
                write!(
                    f,
                    "incomplete_relations: missing_competitions={:?}, missing_teams={:?}",
                    missing_competitions, missing_teams
                )
            }
            ExtractionError::MissingRequiredField(field) => {
                write!(f, "missing_required_field: {}", field)
            }
            ExtractionError::JsonError(err) => write!(f, "json_error: {}", err),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetailDecoderError {
    MatchIdMismatch { expected: String, found: String },
    SportIdMismatch { expected: i32, found: i32 },
    EmptyMatchId,
    JsonError(String),
}

impl std::fmt::Display for DetailDecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetailDecoderError::MatchIdMismatch { expected, found } => {
                write!(
                    f,
                    "match_id_mismatch: expected {}, found {}",
                    expected, found
                )
            }
            DetailDecoderError::SportIdMismatch { expected, found } => {
                write!(
                    f,
                    "sport_id_mismatch: expected {}, found {}",
                    expected, found
                )
            }
            DetailDecoderError::EmptyMatchId => write!(f, "empty_match_id"),
            DetailDecoderError::JsonError(err) => write!(f, "json_error: {}", err),
        }
    }
}

fn parse_list_or_dict(val: &Value) -> Result<Vec<Value>, ExtractionError> {
    match val {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => Ok(arr.clone()),
        Value::Object(obj) => Ok(obj.values().cloned().collect()),
        _ => Err(ExtractionError::JsonError(
            "Expected array or dictionary".to_string(),
        )),
    }
}

fn is_live_status(sport_id: i32, status_id: i32) -> bool {
    match sport_id {
        1 => matches!(status_id, 2..=7),
        2 => matches!(status_id, 2..=7 | 9),
        _ => false,
    }
}

fn is_terminal_status_text(text: &str) -> bool {
    let normalized = text
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    matches!(normalized.as_str(), "ft" | "aet")
        || [
            "full time",
            "finished",
            "ended",
            "after penalties",
            "cancelled",
            "canceled",
            "postponed",
            "abandoned",
            "awarded",
            "walkover",
        ]
        .iter()
        .any(|terminal| normalized.contains(terminal))
}

fn match_has_terminal_status(match_value: &Value) -> bool {
    const STATUS_KEYS: [&str; 9] = [
        "statusText",
        "status_text",
        "statusLabel",
        "status_label",
        "statusName",
        "stateName",
        "matchStatus",
        "state",
        "status",
    ];
    const NESTED_KEYS: [&str; 5] = ["name", "label", "text", "shortName", "statusName"];

    STATUS_KEYS.iter().any(|key| match match_value.get(*key) {
        Some(Value::String(text)) => is_terminal_status_text(text),
        Some(Value::Object(status)) => NESTED_KEYS.iter().any(|nested_key| {
            status
                .get(*nested_key)
                .and_then(Value::as_str)
                .is_some_and(is_terminal_status_text)
        }),
        _ => false,
    })
}

fn match_is_currently_live(match_value: &Value, sport_id: i32) -> bool {
    let status_id = match_value["statusId"]
        .as_i64()
        .or_else(|| match_value["status_id"].as_i64())
        .or_else(|| match_value["status"]["id"].as_i64())
        .unwrap_or_default() as i32;
    is_live_status(sport_id, status_id) && !match_has_terminal_status(match_value)
}

fn decode_and_validate_snapshot(
    val: &Value,
    expected_sport_id: i32,
) -> Result<SourceSnapshot, ExtractionError> {
    let active_filter = val["activeFilter"].as_str().unwrap_or("all");
    if active_filter != "live" {
        return Err(ExtractionError::FilterNotLive);
    }

    let sport_id = val["sportId"].as_i64().unwrap_or(0) as i32;
    if sport_id != expected_sport_id {
        return Err(ExtractionError::CrossSportMismatch {
            expected: expected_sport_id,
            found: sport_id,
        });
    }

    let matches = parse_list_or_dict(&val["matches"])?;
    let teams = parse_list_or_dict(&val["teams"])?;
    let competitions = parse_list_or_dict(&val["competitions"])?;

    let mut team_ids = std::collections::HashSet::new();
    for team in &teams {
        let id = team["id"]
            .as_str()
            .ok_or_else(|| ExtractionError::MissingRequiredField("team.id".to_string()))?;
        if id.is_empty() {
            return Err(ExtractionError::MissingRequiredField("team.id".to_string()));
        }
        team_ids.insert(id.to_string());
    }

    let mut comp_ids = std::collections::HashSet::new();
    for comp in &competitions {
        let id = comp["id"]
            .as_str()
            .ok_or_else(|| ExtractionError::MissingRequiredField("competition.id".to_string()))?;
        if id.is_empty() {
            return Err(ExtractionError::MissingRequiredField(
                "competition.id".to_string(),
            ));
        }
        comp_ids.insert(id.to_string());
    }

    let mut missing_competitions = Vec::new();
    let mut missing_teams = Vec::new();

    for m in &matches {
        let match_id = m["id"]
            .as_str()
            .ok_or_else(|| ExtractionError::MissingRequiredField("match.id".to_string()))?;
        if match_id.is_empty() {
            return Err(ExtractionError::MissingRequiredField(
                "match.id".to_string(),
            ));
        }

        let comp_id = m["competition"]["id"]
            .as_str()
            .or_else(|| m["competitionId"].as_str())
            .unwrap_or("");
        if comp_id.is_empty() {
            return Err(ExtractionError::MissingRequiredField(
                "match.competition.id".to_string(),
            ));
        }

        let home_team_id = m["homeTeam"]["id"]
            .as_str()
            .or_else(|| m["homeTeamId"].as_str())
            .unwrap_or("");
        if home_team_id.is_empty() {
            return Err(ExtractionError::MissingRequiredField(
                "match.homeTeam.id".to_string(),
            ));
        }

        let away_team_id = m["awayTeam"]["id"]
            .as_str()
            .or_else(|| m["awayTeamId"].as_str())
            .unwrap_or("");
        if away_team_id.is_empty() {
            return Err(ExtractionError::MissingRequiredField(
                "match.awayTeam.id".to_string(),
            ));
        }

        if !comp_ids.contains(comp_id) {
            missing_competitions.push(comp_id.to_string());
        }
        if !team_ids.contains(home_team_id) {
            missing_teams.push(home_team_id.to_string());
        }
        if !team_ids.contains(away_team_id) {
            missing_teams.push(away_team_id.to_string());
        }
    }

    if !missing_competitions.is_empty() || !missing_teams.is_empty() {
        return Err(ExtractionError::IncompleteRelations {
            missing_competitions,
            missing_teams,
        });
    }

    Ok(SourceSnapshot {
        matches,
        teams,
        competitions,
        active_filter: active_filter.to_string(),
        sport_id,
        timestamp: val["timestamp"].as_i64(),
    })
}

fn decode_and_validate_detail(
    detail: &Value,
    requested_match_id: &str,
    expected_sport_id: i32,
) -> Result<Value, DetailDecoderError> {
    let match_id = detail["matchId"]
        .as_str()
        .or_else(|| detail["match_id"].as_str())
        .unwrap_or("");
    if match_id.is_empty() {
        return Err(DetailDecoderError::EmptyMatchId);
    }
    if match_id != requested_match_id {
        return Err(DetailDecoderError::MatchIdMismatch {
            expected: requested_match_id.to_string(),
            found: match_id.to_string(),
        });
    }

    if let Some(sport_id_val) = detail["sportId"]
        .as_i64()
        .or_else(|| detail["sport_id"].as_i64())
    {
        if sport_id_val as i32 != expected_sport_id {
            return Err(DetailDecoderError::SportIdMismatch {
                expected: expected_sport_id,
                found: sport_id_val as i32,
            });
        }
    }

    let mut sanitized = detail.clone();
    sanitize_json(&mut sanitized);
    Ok(sanitized)
}

// ── WebSocket router ──────────────────────────────────────────────────────────

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

// ── Chrome target management ──────────────────────────────────────────────────

/// Create a dedicated list target for this session.
/// Never hijacks or navigates existing user tabs.
async fn create_list_target(
    chrome_url: &str,
    adapter: &dyn SportAdapter,
) -> Result<OwnedTarget, Box<dyn std::error::Error + Send + Sync>> {
    let target_info = create_dedicated_target(chrome_url, adapter.target_url()).await?;
    let ws_url = target_info.websocket_url.clone().unwrap();
    println!(
        "[Crawler] Created dedicated {} list target: id={} ws={}",
        adapter.name(),
        target_info.id,
        ws_url
    );
    Ok(OwnedTarget {
        target_id: target_info.id,
        websocket_url: ws_url,
        role: TargetRole::List,
        sport_id: adapter.sport_id(),
        requested_match_id: None,
        created_at: Instant::now(),
    })
}

/// Create a dedicated detail target for a single match.
async fn create_detail_target(
    chrome_url: &str,
    detail_url: &str,
    match_id: &str,
    sport_id: i32,
) -> Result<OwnedTarget, Box<dyn std::error::Error + Send + Sync>> {
    let target_info = create_dedicated_target(chrome_url, detail_url).await?;
    let ws_url = target_info.websocket_url.clone().unwrap();
    Ok(OwnedTarget {
        target_id: target_info.id,
        websocket_url: ws_url,
        role: TargetRole::Detail,
        sport_id,
        requested_match_id: Some(match_id.to_string()),
        created_at: Instant::now(),
    })
}

// ── Readiness probing ─────────────────────────────────────────────────────────

/// Result from a single readiness probe attempt.
#[derive(Debug, Clone, PartialEq)]
enum ReadinessProbe {
    /// Not yet initialised (Vuex not ready).
    NotReady,
    /// State available but filter is not "live".
    NotLive,
    /// State is live but snapshot not yet stable (first probe).
    LiveUnstable(SourceSnapshot),
    /// Two consecutive identical live snapshots — ready.
    LiveStable(SourceSnapshot),
}

/// Wait for a stable Live snapshot on an already-connected websocket.
///
/// Protocol:
/// 1. Apply `min_delay_ms` unconditional sleep first.
/// 2. Always attempt `activate_live_js`.
/// 3. Loop: `extract_state_js` → compare two consecutive identical live results.
/// 4. `not_live` / `not_ready` states → retry/reactivate until timeout.
/// 5. Returns `None` on timeout.
async fn wait_for_live_stable(
    ws_write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    router: &WsRouter,
    adapter: &dyn SportAdapter,
    min_delay_ms: u64,
    readiness_timeout: Duration,
    probe_interval: Duration,
) -> Option<SourceSnapshot> {
    // Step 1: unconditional minimum delay
    sleep(Duration::from_millis(min_delay_ms)).await;

    // Step 2: activate Live filter
    let _ = send_command(
        ws_write,
        router,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": adapter.activate_live_js(),
            "returnByValue": true
        }),
    )
    .await;

    let deadline = Instant::now() + readiness_timeout;
    let mut prev_snapshot: Option<SourceSnapshot> = None;

    loop {
        if Instant::now() >= deadline {
            println!("[Crawler] Readiness timeout waiting for stable Live snapshot.");
            return None;
        }

        let state_result = match send_command(
            ws_write,
            router,
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
                eprintln!("[Crawler] Error probing state: {}", e);
                sleep(probe_interval).await;
                continue;
            }
        };

        let val = &state_result["result"]["result"]["value"];

        if val.is_null() {
            // Vuex not ready — re-activate and retry
            let _ = send_command(
                ws_write,
                router,
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": adapter.activate_live_js(),
                    "returnByValue": true
                }),
            )
            .await;
            sleep(probe_interval).await;
            continue;
        }

        match decode_and_validate_snapshot(val, adapter.sport_id()) {
            Ok(snap) => {
                // Compare with previous snapshot for stability
                if let Some(prev) = &prev_snapshot {
                    let prev_json = serde_json::to_string(&prev.matches).unwrap_or_default();
                    let curr_json = serde_json::to_string(&snap.matches).unwrap_or_default();
                    if prev_json == curr_json {
                        println!("[Crawler] Live snapshot stable (two consecutive matches).");
                        return Some(snap);
                    }
                }
                // First live probe or changed — keep as candidate
                prev_snapshot = Some(snap);
                sleep(probe_interval).await;
            }
            Err(ExtractionError::FilterNotLive) => {
                // Non-null but not live — retry activation
                let _ = send_command(
                    ws_write,
                    router,
                    "Runtime.evaluate",
                    serde_json::json!({
                        "expression": adapter.activate_live_js(),
                        "returnByValue": true
                    }),
                )
                .await;
                prev_snapshot = None;
                sleep(probe_interval).await;
            }
            Err(e) => {
                eprintln!("[Crawler] Readiness probe error: {}", e);
                prev_snapshot = None;
                sleep(probe_interval).await;
            }
        }
    }
}

// ── Detail extraction via dedicated Chrome tab ────────────────────────────────

/// Known data-bearing tabs on the reference detail page. Overview is included
/// because incidents and summary metadata can be lazy-loaded there. Chat is
/// deliberately absent.
const DETAIL_DATA_TAB_LABELS: [&str; 7] = [
    "Overview",
    "Odds",
    "Stats",
    "H2H",
    "Lineups",
    "Standings",
    "Prediction",
];

/// JS that activates only detail data tabs. Each click is followed by a short
/// settle delay so lazy Vuex/API data has time to hydrate. Chat is intentionally
/// absent from the allowlist and therefore is never activated.
fn detail_activate_tabs_js(sport_id: i32, match_id: &str) -> String {
    let labels = serde_json::to_string(&DETAIL_DATA_TAB_LABELS).unwrap();
    let store_key = if sport_id == 2 {
        "basketball/detail"
    } else {
        "football/detail"
    };
    let requested_match_id = serde_json::to_string(match_id).unwrap();
    format!(
        r#"(async function() {{
            const allowed = {labels};
            const requestedMatchId = {requested_match_id};
            const normalize = value => String(value || "")
                .replace(/\s+/g, " ").trim().toLowerCase();
            const clicked = [];
            const snapshots = {{}};
            const clone = value => {{
                try {{ return JSON.parse(JSON.stringify(value)); }} catch (_) {{ return {{}}; }}
            }};
            const capture = label => {{
                const store = window.$nuxt && window.$nuxt.$store;
                const state = store && store.state && store.state['{store_key}'];
                if (state && typeof state === 'object') snapshots[label] = clone(state);
            }};
            capture('Initial');
            for (const label of allowed) {{
                const wanted = normalize(label);
                const candidates = Array.from(document.querySelectorAll(
                    '[role="tab"], button, a, [class*="tab"], [class*="menu"]'
                ));
                const element = candidates.find(node => {{
                    const text = normalize(node.innerText || node.textContent);
                    const aria = normalize(node.getAttribute && node.getAttribute('aria-label'));
                    const visible = node.getClientRects && node.getClientRects().length > 0;
                    return visible && (text === wanted || aria === wanted);
                }});
                if (!element) continue;
                element.click();
                clicked.push(label);
                await new Promise(resolve => setTimeout(resolve, 900));
                capture(label);
            }}
            window.__crawlerActivatedDetailTabs = clicked;
            window.__crawlerDetailTabSnapshots = snapshots;
            return {{ matchId: requestedMatchId, clicked, captured: Object.keys(snapshots) }};
        }})()"#,
        labels = labels,
        requested_match_id = requested_match_id,
        store_key = store_key,
    )
}

fn detail_extract_js(sport_id: i32, requested_match_id: &str) -> String {
    let store_key = if sport_id == 2 {
        "basketball/detail"
    } else {
        "football/detail"
    };
    let requested_match_id = serde_json::to_string(requested_match_id).unwrap();
    format!(
        r#"(function() {{
            if (!window.$nuxt || !window.$nuxt.$store) return null;
            const rootState = window.$nuxt.$store.state || {{}};
            const requestedMatchId = {requested_match_id};
            const readMatchId = value => {{
                if (!value || typeof value !== 'object') return '';
                return String(value.matchId || value.match_id ||
                    (value.match && value.match.id) ||
                    (value.matchInfo && value.matchInfo.id) || '');
            }};
            const preferred = rootState['{store_key}'];
            const candidates = [preferred, ...Object.entries(rootState)
                .filter(([key, value]) => value && typeof value === 'object' &&
                    (key.toLowerCase().includes('detail') || key.toLowerCase().includes('match')))
                .map(([, value]) => value)]
                .filter(Boolean);
            const detailState = candidates.find(value => readMatchId(value) === requestedMatchId) || preferred || {{}};
            const matchId = readMatchId(detailState);
            if (!matchId) return null;
            const tabSnapshots = window.__crawlerDetailTabSnapshots || {{}};
            const sources = [detailState, ...Object.values(tabSnapshots)].filter(value => value && typeof value === 'object');
            const meaningful = value => value !== undefined && value !== null && value !== '' &&
                (!Array.isArray(value) || value.length > 0) &&
                (typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length > 0);
            const pick = keys => {{
                for (const source of sources) {{
                    for (const key of keys) {{
                        if (meaningful(source[key])) return source[key];
                    }}
                }}
                return undefined;
            }};
            const blocked = key => {{
                const lower = String(key || '').toLowerCase();
                return lower.includes('ch' + 'at') || lower.includes('mes' + 'sage') || lower.includes('com' + 'ment');
            }};
            const stripBlocked = (value, depth) => {{
                if (depth > 12 || value === null || value === undefined) return value;
                if (Array.isArray(value)) return value.map(item => stripBlocked(item, depth + 1));
                if (typeof value !== 'object') return value;
                const cleaned = {{}};
                for (const [key, item] of Object.entries(value)) {{
                    if (!blocked(key)) cleaned[key] = stripBlocked(item, depth + 1);
                }}
                return cleaned;
            }};
            return {{
                matchId: matchId,
                sportId: {sport_id},
                name: detailState.name || "",
                incidents: stripBlocked(pick(['incidents', 'INCIDENTS_DETAIL_DATA', 'incidentList', 'events']) || [], 0),
                stats: stripBlocked(pick(['stats', 'STATS_DETAIL_DATA', 'statistics']) || {{}}, 0),
                lineups: stripBlocked(pick(['lineups', 'LINEUPS_DETAIL_DATA', 'lineup']) || {{}}, 0),
                odds: stripBlocked(pick(['ODDS_DETAIL_DATA', 'odds', 'oddsData']) || {{}}, 0),
                h2h: stripBlocked(pick(['HISTORY_DETAIL_DATA', 'h2h', 'history', 'headToHead']) || {{}}, 0),
                activatedTabs: window.__crawlerActivatedDetailTabs || [],
                tabData: stripBlocked(tabSnapshots, 0),
                sourceDetail: stripBlocked(detailState, 0)
            }};
        }})()"#,
        store_key = store_key,
        sport_id = sport_id,
        requested_match_id = requested_match_id,
    )
}

/// Fetch detail from a dedicated owned target tab.
/// Applies minimum delay, then probes until `matchId` is hydrated.
/// Returns validated detail JSON, or an error.
async fn fetch_detail_from_target(
    ws_url: &str,
    match_id: &str,
    sport_id: i32,
    min_delay_ms: u64,
    readiness_timeout: Duration,
    probe_interval: Duration,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let (ws_stream, _) = connect_async(ws_url).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let router = WsRouter::new();
    let pending_clone = router.pending.clone();

    // Spawn a read pump for this connection
    let read_task = tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            if let Ok(Message::Text(text)) = msg {
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    if let Some(id) = parsed["id"].as_i64() {
                        let mut lock = pending_clone.lock().await;
                        if let Some(tx) = lock.remove(&id) {
                            let _ = tx.send(parsed);
                        }
                    }
                }
            }
        }
    });

    // Enable Runtime domain
    let _ = send_command(
        &mut ws_write,
        &router,
        "Runtime.enable",
        serde_json::json!({}),
    )
    .await;

    // Minimum delay
    sleep(Duration::from_millis(min_delay_ms)).await;

    // Activate lazy-loaded data sections before probing the Vuex detail state.
    // This is an explicit allowlist; Chat is never clicked.
    match send_command(
        &mut ws_write,
        &router,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": detail_activate_tabs_js(sport_id, match_id),
            "returnByValue": true,
            "awaitPromise": true
        }),
    )
    .await
    {
        Ok(res) => {
            let clicked = &res["result"]["result"]["value"];
            println!(
                "[Crawler] Detail data tabs activated for match {}: {}",
                match_id, clicked
            );
        }
        Err(e) => {
            eprintln!(
                "[Crawler] Could not activate optional detail tabs for match {}: {}",
                match_id, e
            );
        }
    }

    let deadline = Instant::now() + readiness_timeout;
    let js = detail_extract_js(sport_id, match_id);

    loop {
        if Instant::now() >= deadline {
            read_task.abort();
            return Err(format!("Detail readiness timeout for match {}", match_id).into());
        }

        let result = send_command(
            &mut ws_write,
            &router,
            "Runtime.evaluate",
            serde_json::json!({
                "expression": js,
                "returnByValue": true
            }),
        )
        .await;

        match result {
            Ok(res) => {
                let val = &res["result"]["result"]["value"];
                if val.is_null() {
                    sleep(probe_interval).await;
                    continue;
                }
                // Validate: must have correct matchId and sportId
                match decode_and_validate_detail(val, match_id, sport_id) {
                    Ok(validated) => {
                        read_task.abort();
                        return Ok(validated);
                    }
                    Err(e) => {
                        read_task.abort();
                        return Err(format!("Detail validation failed: {}", e).into());
                    }
                }
            }
            Err(e) => {
                eprintln!("[Crawler] Detail probe command error: {}", e);
                sleep(probe_interval).await;
            }
        }
    }
}

// ── Database ──────────────────────────────────────────────────────────────────

fn open_db(db_path: &str) -> SqlResult<Connection> {
    storage::open_db(db_path)
}

fn init_db(db_path: &str) -> SqlResult<Connection> {
    storage::init_db(db_path)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    storage::column_exists(conn, table, column)
}

fn run_migrations(conn: &Connection) -> SqlResult<()> {
    storage::run_migrations(conn)
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

fn save_competitions(
    conn: &Connection,
    comps: &[Value],
    sport_id: i32,
    dataset_id: &str,
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut select_stmt = tx.prepare(
            "SELECT id, name, logo, slug, country_name, country_logo, raw_payload FROM competitions WHERE id = ?1 AND dataset_id = ?2"
        )?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at, dataset_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, datetime('now'), ?9)"
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
             WHERE id = ?1 AND dataset_id = ?8",
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
                String,
            )> = select_stmt
                .query_row(params![id, dataset_id], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .optional()?;

            if let Some(existing_row) = existing {
                let is_changed = existing_row.0 != name
                    || existing_row.1.as_deref() != logo
                    || existing_row.2.as_deref() != slug
                    || existing_row.3.as_deref() != country_name
                    || existing_row.4.as_deref() != country_logo
                    || existing_row.5 != raw_payload;

                if is_changed {
                    update_stmt.execute(params![
                        id,
                        name,
                        logo,
                        slug,
                        country_name,
                        country_logo,
                        raw_payload,
                        dataset_id
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
                    raw_payload,
                    dataset_id
                ])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_teams(
    conn: &Connection,
    teams: &[Value],
    sport_id: i32,
    dataset_id: &str,
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut select_stmt = tx.prepare(
            "SELECT id, name, logo, slug, raw_payload FROM teams WHERE id = ?1 AND dataset_id = ?2",
        )?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO teams (id, sport_id, name, logo, slug, raw_payload, synced, updated_at, dataset_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, datetime('now'), ?7)",
        )?;

        let mut update_stmt = tx.prepare(
            "UPDATE teams SET
                name = ?2,
                logo = ?3,
                slug = ?4,
                raw_payload = ?5,
                synced = 0,
                updated_at = datetime('now')
             WHERE id = ?1 AND dataset_id = ?6",
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
                .query_row(params![id, dataset_id], |row| {
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
                    update_stmt.execute(params![id, name, logo, slug, raw_payload, dataset_id])?;
                }
            } else {
                insert_stmt.execute(params![
                    id,
                    sport_id,
                    name,
                    logo,
                    slug,
                    raw_payload,
                    dataset_id
                ])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn save_matches(
    conn: &Connection,
    matches: &[Value],
    sport_id: i32,
    dataset_id: &str,
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let live_ids = matches
            .iter()
            .filter(|item| match_is_currently_live(item, sport_id))
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect::<std::collections::HashSet<_>>();

        let mut previous_live_stmt =
            tx.prepare("SELECT id FROM matches WHERE dataset_id=?1 AND sport_id=?2 AND is_live=1")?;
        let previous_live = previous_live_stmt
            .query_map(params![dataset_id, sport_id], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        drop(previous_live_stmt);
        let mut hide_stmt = tx.prepare(
            "UPDATE matches SET is_live=0, synced=0, updated_at=datetime('now')
             WHERE id=?1 AND dataset_id=?2 AND sport_id=?3 AND is_live=1",
        )?;
        for id in previous_live {
            if !live_ids.contains(&id) {
                hide_stmt.execute(params![id, dataset_id, sport_id])?;
            }
        }
        drop(hide_stmt);

        let mut select_stmt = tx.prepare(
            "SELECT id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload FROM matches WHERE id = ?1 AND dataset_id = ?2"
        )?;

        let mut insert_stmt = tx.prepare(
            "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload, synced, updated_at, dataset_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, datetime('now'), ?12)"
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
                is_live = ?9,
                raw_payload = ?10,
                synced = 0,
                updated_at = datetime('now')
             WHERE id = ?1 AND dataset_id = ?11",
        )?;

        for m in matches {
            let mut sanitized = m.clone();
            sanitize_json(&mut sanitized);

            let id = sanitized["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let comp_id = sanitized["competition"]["id"]
                .as_str()
                .or_else(|| sanitized["competitionId"].as_str())
                .or_else(|| sanitized["competition_id"].as_str())
                .unwrap_or("");
            let home_id = sanitized["homeTeam"]["id"]
                .as_str()
                .or_else(|| sanitized["homeTeamId"].as_str())
                .or_else(|| sanitized["home_team_id"].as_str())
                .unwrap_or("");
            let away_id = sanitized["awayTeam"]["id"]
                .as_str()
                .or_else(|| sanitized["awayTeamId"].as_str())
                .or_else(|| sanitized["away_team_id"].as_str())
                .unwrap_or("");
            let match_time = sanitized["matchTime"]
                .as_i64()
                .or_else(|| sanitized["match_time"].as_i64())
                .unwrap_or(0);
            let status_id = sanitized["statusId"]
                .as_i64()
                .or_else(|| sanitized["status_id"].as_i64())
                .unwrap_or(0) as i32;
            let is_live = match_is_currently_live(&sanitized, sport_id);
            let home_scores = sanitized
                .get("homeScores")
                .or_else(|| sanitized.get("home_scores"))
                .map(Value::to_string)
                .unwrap_or_else(|| "[]".to_string());
            let away_scores = sanitized
                .get("awayScores")
                .or_else(|| sanitized.get("away_scores"))
                .map(Value::to_string)
                .unwrap_or_else(|| "[]".to_string());
            let raw_payload = serde_json::to_string(&sanitized).unwrap_or_default();

            let existing: Option<(
                String,
                String,
                String,
                i64,
                i32,
                String,
                String,
                bool,
                String,
            )> = select_stmt
                .query_row(params![id, dataset_id], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i32>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, bool>(8)?,
                        row.get::<_, String>(9)?,
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
                    || existing_row.7 != is_live
                    || existing_row.8 != raw_payload;

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
                        is_live,
                        raw_payload,
                        dataset_id
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
                    is_live,
                    raw_payload,
                    dataset_id
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
    dataset_id: &str,
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
            "SELECT incidents, stats, lineups, odds, h2h, raw_payload FROM match_details WHERE match_id = ?1 AND dataset_id = ?2",
            params![match_id, dataset_id],
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
                 WHERE match_id = ?1 AND dataset_id = ?8",
                params![
                    match_id,
                    incidents,
                    stats,
                    lineups,
                    odds,
                    h2h,
                    raw_payload,
                    dataset_id
                ],
            )?;
        }
    } else {
        conn.execute(
            "INSERT INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, last_updated, synced, updated_at, dataset_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s', 'now'), 0, datetime('now'), ?9)",
            params![match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, dataset_id],
        )?;
    }
    Ok(())
}

fn get_matches_needing_detail(
    conn: &Connection,
    sport_id: i32,
    detail_interval_secs: i64,
    dataset_id: &str,
) -> SqlResult<Vec<(String, String, String)>> {
    let query = "
        SELECT m.id, t1.slug, t2.slug
        FROM matches m
        JOIN teams t1 ON m.home_team_id = t1.id AND m.dataset_id = t1.dataset_id
        JOIN teams t2 ON m.away_team_id = t2.id AND m.dataset_id = t2.dataset_id
        LEFT JOIN match_details d ON m.id = d.match_id AND m.dataset_id = d.dataset_id
        WHERE m.sport_id = ?1
          AND m.dataset_id = ?3
          AND (
            (
              m.is_live = 1
              AND (
                d.match_id IS NULL
                OR strftime('%s', 'now') - d.last_updated > ?2
              )
            )
            OR (
              m.status_id = 8
              AND (
                d.match_id IS NULL
                OR d.last_updated < CAST(strftime('%s', m.updated_at) AS INTEGER)
              )
            )
          )
    ";

    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map(params![sport_id, detail_interval_secs, dataset_id], |row| {
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

fn mark_sport_snapshot_ready(
    conn: &Connection,
    dataset_id: &str,
    sport_id: i32,
    captured_at: &str,
) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO dataset_sports (dataset_id, sport_id, captured_at, synced)
         VALUES (?1, ?2, ?3, 0)
         ON CONFLICT(dataset_id, sport_id) DO UPDATE SET captured_at=excluded.captured_at, synced=0",
        params![dataset_id, sport_id, captured_at],
    )?;
    Ok(())
}

fn acknowledge_match_if_unchanged(
    conn: &Connection,
    row: &Match,
    dataset_id: &str,
) -> SqlResult<usize> {
    conn.execute(
        "UPDATE matches SET synced=1
         WHERE id=?1 AND dataset_id=?2 AND sport_id=?3 AND competition_id=?4
           AND home_team_id=?5 AND away_team_id=?6 AND match_time=?7 AND status_id=?8
           AND home_scores=?9 AND away_scores=?10 AND is_live=?11 AND raw_payload IS ?12",
        params![
            row.id,
            dataset_id,
            row.sport_id,
            row.competition_id,
            row.home_team_id,
            row.away_team_id,
            row.match_time,
            row.status_id,
            row.home_scores,
            row.away_scores,
            row.is_live,
            row.raw_payload
        ],
    )
}

// ── Uploader lease (sync worker) ──────────────────────────────────────────────

fn claim_uploader_lease(conn: &Connection, client_id: &str) -> bool {
    let now = chrono::Utc::now().timestamp();
    let tx = match rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(_) => return false,
    };
    let _ = tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('uploader_lease_owner', '')",
        [],
    );
    let _ = tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('uploader_lease_expires', '0')",
        [],
    );

    let owner: String = tx
        .query_row(
            "SELECT value FROM settings WHERE key='uploader_lease_owner'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let expires: i64 = tx
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
        let next_expiry = now + 45;
        let res1 = tx.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_owner', ?1)",
            params![client_id],
        );
        let res2 = tx.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('uploader_lease_expires', ?1)",
            params![next_expiry.to_string()],
        );
        if res1.is_ok() && res2.is_ok() && tx.commit().is_ok() {
            return true;
        }
        return false;
    }
    let _ = tx.rollback();
    false
}

fn release_uploader_lease(conn: &Connection, client_id: &str) {
    for attempt in 0..3 {
        if let Ok(tx) = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate) {
            let released = tx
                .execute(
                    "UPDATE settings SET value='' WHERE key='uploader_lease_owner' AND value=?1",
                    params![client_id],
                )
                .unwrap_or(0);
            if released == 1 {
                let _ = tx.execute(
                    "UPDATE settings SET value='0' WHERE key='uploader_lease_expires'",
                    [],
                );
                if tx.commit().is_ok() {
                    return;
                }
            } else {
                let _ = tx.rollback();
                return;
            }
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    eprintln!(
        "[Sync] Could not release uploader lease after retries; it will expire automatically."
    );
}

// ── Background sync worker ────────────────────────────────────────────────────

async fn sync_worker(db_path: String) {
    println!("[Sync] Starting sync worker background thread...");
    let client = reqwest::Client::new();
    let client_id = format!("sync-worker-{}", uuid::Uuid::new_v4());

    loop {
        let mut interval_mins = 5;
        let mut uploaded_batch = false;
        let mut retry_soon = false;
        let mut worker_url = String::new();
        let mut api_token = String::new();
        let mut active_dataset_id = String::new();

        let file_identity = get_file_identity(&db_path);

        if let Ok(conn) = open_db(&db_path) {
            if let Ok(ds_id) = conn.query_row(
                "SELECT value FROM settings WHERE key='active_dataset_id'",
                [],
                |row| row.get::<_, String>(0),
            ) {
                active_dataset_id = ds_id;
            }

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

        if !worker_url.is_empty() && !api_token.is_empty() && !active_dataset_id.is_empty() {
            if let Ok(conn) = open_db(&db_path) {
                if claim_uploader_lease(&conn, &client_id) {
                    let pending_sport: Option<(i32, String, bool)> = conn
                        .query_row(
                            "SELECT sport_id, captured_at, 1 FROM dataset_sports WHERE dataset_id=?1 AND synced=0 ORDER BY captured_at LIMIT 1",
                            params![active_dataset_id],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i32>(2)? != 0)),
                        )
                        .optional()
                        .unwrap_or(None)
                        .or_else(|| {
                            conn.query_row(
                                "SELECT sport_id FROM (
                                   SELECT sport_id FROM competitions WHERE dataset_id=?1 AND synced=0
                                   UNION SELECT sport_id FROM teams WHERE dataset_id=?1 AND synced=0
                                   UNION SELECT sport_id FROM matches WHERE dataset_id=?1 AND synced=0
                                   UNION SELECT sport_id FROM match_details WHERE dataset_id=?1 AND synced=0
                                 ) ORDER BY sport_id LIMIT 1",
                                params![active_dataset_id],
                                |row| row.get::<_, i32>(0),
                            )
                            .optional()
                            .unwrap_or(None)
                            .map(|sport| {
                                let captured = conn
                                    .query_row(
                                        "SELECT captured_at FROM dataset_sports WHERE dataset_id=?1 AND sport_id=?2",
                                        params![active_dataset_id, sport],
                                        |row| row.get(0),
                                    )
                                    .unwrap_or_else(|_| chrono::Utc::now().to_rfc3339());
                                (sport, captured, false)
                            })
                        });

                    let Some((batch_sport_id, captured_at, readiness_pending)) = pending_sport
                    else {
                        release_uploader_lease(&conn, &client_id);
                        // The crawler may publish its first snapshot immediately
                        // after this worker starts. Poll briefly while idle so a
                        // new generation does not remain invisible for the full
                        // configured sync interval.
                        sleep(Duration::from_secs(2)).await;
                        continue;
                    };

                    let mut unsynced_comps = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, dataset_id FROM competitions WHERE synced=0 AND dataset_id=?1 AND sport_id=?2 ORDER BY updated_at ASC, id ASC LIMIT 50") {
                        if let Ok(iter) = stmt.query_map(params![active_dataset_id, batch_sport_id], |row| {
                            Ok(Competition {
                                id: row.get(0)?,
                                sport_id: row.get(1)?,
                                name: row.get(2)?,
                                logo: row.get(3)?,
                                slug: row.get(4)?,
                                country_name: row.get(5)?,
                                country_logo: row.get(6)?,
                                raw_payload: row.get(7)?,
                                dataset_id: row.get(8)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_comps.push(item);
                            }
                        }
                    }

                    let mut unsynced_teams = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, name, logo, slug, raw_payload, dataset_id FROM teams WHERE synced=0 AND dataset_id=?1 AND sport_id=?2 ORDER BY updated_at ASC, id ASC LIMIT 100") {
                        if let Ok(iter) = stmt.query_map(params![active_dataset_id, batch_sport_id], |row| {
                            Ok(Team {
                                id: row.get(0)?,
                                sport_id: row.get(1)?,
                                name: row.get(2)?,
                                logo: row.get(3)?,
                                slug: row.get(4)?,
                                raw_payload: row.get(5)?,
                                dataset_id: row.get(6)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_teams.push(item);
                            }
                        }
                    }

                    let mut unsynced_matches = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload, dataset_id FROM matches WHERE synced=0 AND dataset_id=?1 AND sport_id=?2 ORDER BY updated_at ASC, id ASC LIMIT 50") {
                        if let Ok(iter) = stmt.query_map(params![active_dataset_id, batch_sport_id], |row| {
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
                                is_live: row.get(9)?,
                                raw_payload: row.get(10)?,
                                dataset_id: row.get(11)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_matches.push(item);
                            }
                        }
                    }

                    let mut unsynced_details = Vec::new();
                    if let Ok(mut stmt) = conn.prepare("SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, last_updated, dataset_id FROM match_details WHERE synced=0 AND dataset_id=?1 AND sport_id=?2 ORDER BY updated_at ASC, match_id ASC LIMIT 20") {
                        if let Ok(iter) = stmt.query_map(params![active_dataset_id, batch_sport_id], |row| {
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
                                dataset_id: row.get(9)?,
                            })
                        }) {
                            for item in iter.flatten() {
                                unsynced_details.push(item);
                            }
                        }
                    }

                    if readiness_pending
                        || !unsynced_comps.is_empty()
                        || !unsynced_teams.is_empty()
                        || !unsynced_matches.is_empty()
                        || !unsynced_details.is_empty()
                    {
                        let sync_id = uuid::Uuid::new_v4().to_string();
                        let dataset_meta: Option<(String, i64)> = conn.query_row(
                            "SELECT created_at, generation_order FROM datasets WHERE dataset_id=?1",
                            params![active_dataset_id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        ).optional().unwrap_or(None);
                        let Some((dataset_created_at, generation_order)) = dataset_meta else {
                            release_uploader_lease(&conn, &client_id);
                            continue;
                        };
                        let payload = serde_json::json!({
                            "protocol_version": 2,
                            "sync_id": sync_id,
                            "dataset_id": active_dataset_id,
                            "dataset_created_at": dataset_created_at,
                            "generation_order": generation_order,
                            "sport_id": batch_sport_id,
                            "captured_at": captured_at,
                            "competitions": unsynced_comps,
                            "teams": unsynced_teams,
                            "matches": unsynced_matches,
                            "match_details": unsynced_details,
                        });

                        let sync_url = format!("{}/api/sync", worker_url);
                        println!(
                            "[Sync] Uploading sport {} batch: competitions={}, teams={}, matches={}, details={}, readiness_pending={}",
                            batch_sport_id,
                            unsynced_comps.len(),
                            unsynced_teams.len(),
                            unsynced_matches.len(),
                            unsynced_details.len(),
                            readiness_pending
                        );
                        let mut backoff = Duration::from_secs(1);
                        let mut success = false;
                        let mut server_interval = None;

                        for attempt in 0..5 {
                            if !verify_db_identity(&db_path, &active_dataset_id, file_identity) {
                                println!(
                                    "[Sync] DB identity changed before HTTP request! Aborting sync attempt."
                                );
                                break;
                            }

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
                                        dataset_id: Option<String>,
                                        sync_id: Option<String>,
                                        sync_interval_mins: Option<u64>,
                                        synced_ids: Option<HashMap<String, Vec<String>>>,
                                    }
                                    if let Ok(res_body) = r.json::<SyncResponse>().await {
                                        if res_body.success
                                            && res_body.dataset_id.as_deref()
                                                == Some(active_dataset_id.as_str())
                                            && res_body.sync_id.as_deref() == Some(sync_id.as_str())
                                            && res_body.synced_ids.is_some()
                                        {
                                            server_interval = res_body.sync_interval_mins;
                                            if verify_db_identity(
                                                &db_path,
                                                &active_dataset_id,
                                                file_identity,
                                            ) {
                                                if let Some(ids_map) = res_body.synced_ids {
                                                    let empty_vec = Vec::new();
                                                    let comp_ids = ids_map
                                                        .get("competitions")
                                                        .unwrap_or(&empty_vec);
                                                    let team_ids =
                                                        ids_map.get("teams").unwrap_or(&empty_vec);
                                                    let match_ids = ids_map
                                                        .get("matches")
                                                        .unwrap_or(&empty_vec);
                                                    let detail_ids = ids_map
                                                        .get("match_details")
                                                        .unwrap_or(&empty_vec);

                                                    if let Ok(mut tx_conn) = open_db(&db_path) {
                                                        if let Ok(tx) = tx_conn.transaction() {
                                                            {
                                                                let mut stmt_c = tx.prepare("UPDATE competitions SET synced=1 WHERE id=?1 AND dataset_id=?2 AND sport_id=?3 AND name=?4 AND logo IS ?5 AND slug IS ?6 AND country_name IS ?7 AND country_logo IS ?8 AND raw_payload IS ?9").unwrap();
                                                                for row in &unsynced_comps {
                                                                    if comp_ids.contains(&row.id) {
                                                                        let _ = stmt_c.execute(
                                                                            params![
                                                                                row.id,
                                                                                active_dataset_id,
                                                                                row.sport_id,
                                                                                row.name,
                                                                                row.logo,
                                                                                row.slug,
                                                                                row.country_name,
                                                                                row.country_logo,
                                                                                row.raw_payload
                                                                            ],
                                                                        );
                                                                    }
                                                                }
                                                                let mut stmt_t = tx.prepare("UPDATE teams SET synced=1 WHERE id=?1 AND dataset_id=?2 AND sport_id=?3 AND name=?4 AND logo IS ?5 AND slug IS ?6 AND raw_payload IS ?7").unwrap();
                                                                for row in &unsynced_teams {
                                                                    if team_ids.contains(&row.id) {
                                                                        let _ = stmt_t.execute(
                                                                            params![
                                                                                row.id,
                                                                                active_dataset_id,
                                                                                row.sport_id,
                                                                                row.name,
                                                                                row.logo,
                                                                                row.slug,
                                                                                row.raw_payload
                                                                            ],
                                                                        );
                                                                    }
                                                                }
                                                                for row in &unsynced_matches {
                                                                    if match_ids.contains(&row.id) {
                                                                        let _ = acknowledge_match_if_unchanged(
                                                                            &tx,
                                                                            row,
                                                                            &active_dataset_id,
                                                                        );
                                                                    }
                                                                }
                                                                let mut stmt_d = tx.prepare("UPDATE match_details SET synced=1 WHERE match_id=?1 AND dataset_id=?2 AND sport_id=?3 AND incidents=?4 AND stats=?5 AND lineups=?6 AND odds=?7 AND h2h=?8 AND raw_payload IS ?9 AND last_updated=?10").unwrap();
                                                                for row in &unsynced_details {
                                                                    if detail_ids
                                                                        .contains(&row.match_id)
                                                                    {
                                                                        let _ = stmt_d.execute(
                                                                            params![
                                                                                row.match_id,
                                                                                active_dataset_id,
                                                                                row.sport_id,
                                                                                row.incidents,
                                                                                row.stats,
                                                                                row.lineups,
                                                                                row.odds,
                                                                                row.h2h,
                                                                                row.raw_payload,
                                                                                row.last_updated
                                                                            ],
                                                                        );
                                                                    }
                                                                }
                                                                if readiness_pending {
                                                                    let _ = tx.execute(
                                                                        "UPDATE dataset_sports SET synced=1 WHERE dataset_id=?1 AND sport_id=?2 AND captured_at=?3",
                                                                        params![active_dataset_id, batch_sport_id, captured_at],
                                                                    );
                                                                }
                                                            }
                                                            if tx.commit().is_ok() {
                                                                success = true;
                                                            }
                                                        }
                                                    }
                                                }
                                            } else {
                                                println!(
                                                    "[Sync] DB identity changed during network roundtrip, or dataset_id mismatch! Aborting mark synced."
                                                );
                                            }
                                            break;
                                        }
                                    }
                                }
                                Ok(r) => {
                                    let status = r.status();
                                    let body = r.text().await.unwrap_or_else(|e| {
                                        format!("<failed to read response: {}>", e)
                                    });
                                    eprintln!(
                                        "[Sync] POST {} returned HTTP {}: {}",
                                        sync_url, status, body
                                    );
                                }
                                Err(e) => {
                                    eprintln!("[Sync] POST {} failed: {}", sync_url, e);
                                }
                            }

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
                            uploaded_batch = true;
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

                    release_uploader_lease(&conn, &client_id);
                } else {
                    retry_soon = true;
                }
            }
        }

        if uploaded_batch || retry_soon {
            // Drain remaining dirty rows immediately. A generation can contain
            // hundreds of matches while each request is deliberately bounded.
            sleep(Duration::from_millis(250)).await;
        } else {
            sleep(Duration::from_secs(interval_mins * 60)).await;
        }
    }
}

// ── WebSocket command helper ──────────────────────────────────────────────────

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

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    let resolved_db_path = get_absolute_normalized_path(&cli.db_path)?;

    match &cli.command {
        Commands::Backup { destination } => {
            let destination = get_absolute_normalized_path(destination)?;
            let manifest = storage::create_verified_backup(&resolved_db_path, destination)?;
            println!(
                "[Storage] Verified backup created: {} ({} bytes, checksum={})",
                manifest.destination.display(),
                manifest.size_bytes,
                manifest.checksum
            );
            return Ok(());
        }
        Commands::Restore {
            backup,
            destination,
        } => {
            let backup = get_absolute_normalized_path(backup)?;
            let destination = get_absolute_normalized_path(destination)?;
            let manifest = storage::restore_verified_backup(&backup, &destination)?;
            println!(
                "[Storage] Verified restore completed: {} ({} bytes, checksum={})",
                manifest.destination.display(),
                manifest.size_bytes,
                manifest.checksum
            );
            return Ok(());
        }
        Commands::Migrate { backup_destination } => {
            if let Some(destination) = backup_destination {
                let destination = get_absolute_normalized_path(destination)?;
                let manifest = storage::create_verified_backup(&resolved_db_path, &destination)?;
                println!(
                    "[Storage] Pre-migration backup verified: {} ({} bytes, checksum={})",
                    manifest.destination.display(),
                    manifest.size_bytes,
                    manifest.checksum
                );
            }
            let _ = init_db(&resolved_db_path)?;
            println!(
                "[Storage] SQLite v3 migration completed: {}",
                resolved_db_path
            );
            return Ok(());
        }
        Commands::Football | Commands::Basketball => {}
    }
    let _ = init_db(&resolved_db_path)?;

    let mut dataset_id = {
        let conn = open_db(&resolved_db_path)?;
        init_dataset_id(&conn)?
    };
    let mut file_identity = get_file_identity(&resolved_db_path);

    let adapter: Box<dyn SportAdapter> = match cli.command {
        Commands::Football => Box::new(FootballAdapter),
        Commands::Basketball => Box::new(BasketballAdapter),
        Commands::Migrate { .. } | Commands::Backup { .. } | Commands::Restore { .. } => {
            unreachable!("storage subcommands return before crawler startup")
        }
    };
    let sport_id = adapter.sport_id();

    println!(
        "[Crawler] Diagnostics: db_path=\"{}\" dataset_id=\"{}\" sport=\"{}\" page_ready_delay={}ms detail_ready_delay={}ms detail_concurrency={}",
        resolved_db_path,
        dataset_id,
        adapter.name(),
        cli.page_ready_delay_ms,
        cli.detail_ready_delay_ms,
        cli.detail_concurrency
    );

    let db_path_clone = resolved_db_path.clone();
    tokio::spawn(async move {
        sync_worker(db_path_clone).await;
    });

    println!(
        "[Crawler] Starting crawler for sport_id: {} ({})",
        sport_id,
        adapter.name()
    );

    // Readiness constants
    let readiness_timeout = Duration::from_secs(30);
    let probe_interval = Duration::from_millis(800);

    // Detail concurrency semaphore
    let detail_sem = Arc::new(Semaphore::new(cli.detail_concurrency));

    // Track owned list target across reconnects
    let mut owned_list_target: Option<OwnedTarget> = None;

    loop {
        // ── Create or validate the dedicated list target ──────────────────────
        let list_target = loop {
            // Check if we already own a live target
            if let Some(ref ot) = owned_list_target {
                // Verify it still exists in Chrome's target list
                let client = reqwest::Client::new();
                let url = format!("{}/json", cli.chrome_url);
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(targets) = resp.json::<Vec<TargetInfo>>().await {
                        if targets.iter().any(|t| t.id == ot.target_id) {
                            break ot.websocket_url.clone();
                        }
                    }
                }
                println!(
                    "[Crawler] Owned list target {} disappeared; creating a new one.",
                    ot.target_id
                );
                owned_list_target = None;
            }

            // Create a dedicated new tab
            match create_list_target(&cli.chrome_url, &*adapter).await {
                Ok(ot) => {
                    let ws = ot.websocket_url.clone();
                    owned_list_target = Some(ot);
                    break ws;
                }
                Err(e) => {
                    eprintln!(
                        "[Crawler] Failed to create dedicated list target: {}. Retrying in 10s...",
                        e
                    );
                    sleep(Duration::from_secs(10)).await;
                }
            }
        };

        // ── Connect to the list target ────────────────────────────────────────
        println!(
            "[Crawler] Connecting to dedicated list target WS: {}",
            list_target
        );
        let ws_stream = match connect_async(&list_target).await {
            Ok((s, _)) => s,
            Err(e) => {
                eprintln!(
                    "[Crawler] WebSocket connection failed: {}. Retrying in 10s...",
                    e
                );
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        let (mut ws_write, mut ws_read) = ws_stream.split();
        let ws_router = WsRouter::new();
        let pending_clone = ws_router.pending.clone();
        let (state_change_tx, mut state_change_rx) = tokio::sync::mpsc::channel::<()>(100);
        let state_change_tx_clone = state_change_tx.clone();

        // The response router must be running before the first send_command call.
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
                            if let Some(log_text) = parsed["params"]["args"]
                                .get(0)
                                .and_then(|arg| arg["value"].as_str())
                            {
                                if log_text == "AISCORE_STATE_CHANGED" {
                                    let _ = state_change_tx_clone.try_send(());
                                }
                            }
                        }
                        if let Some(id) = parsed["id"].as_i64() {
                            if let Some(tx) = pending_clone.lock().await.remove(&id) {
                                let _ = tx.send(parsed);
                            }
                        }
                    }
                }
            }
            println!("[Crawler] WebSocket read task terminated.");
        });

        // Enable Runtime domain
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

        // Inject state-change subscription listener
        let js_subscribe = r#"(function() {
                if (window.__aiscore_listener_active) return;
                window.__aiscore_listener_active = true;

                function checkAndSubscribe() {
                    if (window.$nuxt && window.$nuxt.$store) {
                        window.$nuxt.$store.subscribe((mutation, state) => {
                            if (mutation.type.includes('matches') || mutation.type.includes('score') || mutation.type.includes('odds') || mutation.type.includes('time')) {
                                console.log("AISCORE_STATE_CHANGED");
                            }
                        });
                        console.log("AISCORE_SUBSCRIBE_SUCCESS");
                    } else {
                        setTimeout(checkAndSubscribe, 1000);
                    }
                }
                checkAndSubscribe();
            })()"#;

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

        // ── Wait for stable Live readiness before starting the event loop ────
        println!(
            "[Crawler] Waiting for stable Live snapshot (min_delay={}ms)...",
            cli.page_ready_delay_ms
        );
        let initial_snapshot = wait_for_live_stable(
            &mut ws_write,
            &ws_router,
            &*adapter,
            cli.page_ready_delay_ms,
            readiness_timeout,
            probe_interval,
        )
        .await;

        if initial_snapshot.is_none() {
            eprintln!("[Crawler] Could not get stable Live snapshot; reconnecting...");
            read_task.abort();
            // Close and recreate the target on next iteration
            if let Some(ref ot) = owned_list_target {
                close_target(&cli.chrome_url, &ot.target_id).await;
            }
            owned_list_target = None;
            sleep(Duration::from_secs(5)).await;
            continue;
        }

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

        // ── Event processing / Coordination loop ──────────────────────────────
        while let Some(_) = state_change_rx.recv().await {
            // Drain queued events (coalesce)
            while state_change_rx.try_recv().is_ok() {}

            // Settle delay
            sleep(Duration::from_millis(500)).await;

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

            let decoded = match decode_and_validate_snapshot(val, sport_id) {
                Ok(snap) => snap,
                Err(e) => {
                    eprintln!("[Crawler] Extraction error: {}", e);
                    // Re-activate Live filter if filter not live
                    if matches!(e, ExtractionError::FilterNotLive) {
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
                    }
                    continue;
                }
            };

            let matches = &decoded.matches;
            let teams = &decoded.teams;
            let comps = &decoded.competitions;

            println!(
                "[Crawler] State fetched and validated: matches={}, teams={}, competitions={}",
                matches.len(),
                teams.len(),
                comps.len()
            );

            // DB identity guard
            if !verify_db_identity(&resolved_db_path, &dataset_id, file_identity) {
                println!(
                    "[Crawler] DB unlink or replacement detected! Recovering and generating new dataset ID..."
                );
                if let Err(e) = init_db(&resolved_db_path) {
                    eprintln!(
                        "[Crawler] Actionable recovery error: failed to re-initialize SQLite schema: {:?}",
                        e
                    );
                    break;
                }
                match open_db(&resolved_db_path) {
                    Ok(conn) => match init_dataset_id(&conn) {
                        Ok(new_ds_id) => {
                            dataset_id = new_ds_id;
                            file_identity = get_file_identity(&resolved_db_path);
                            println!(
                                "[Crawler] Diagnostics: db_path=\"{}\" dataset_id=\"{}\" sport=\"{}\"",
                                resolved_db_path,
                                dataset_id,
                                adapter.name()
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "[Crawler] Actionable recovery error: failed to generate new dataset ID: {:?}",
                                e
                            );
                            break;
                        }
                    },
                    Err(e) => {
                        eprintln!(
                            "[Crawler] Actionable recovery error: failed to open SQLite database: {:?}",
                            e
                        );
                        break;
                    }
                }
                println!("[Crawler] Pending in-memory work for old generation cleared.");
                continue;
            }

            // Persist list data
            if let Ok(conn) = open_db(&resolved_db_path) {
                let list_saved = save_competitions(&conn, comps, sport_id, &dataset_id)
                    .and_then(|_| save_teams(&conn, teams, sport_id, &dataset_id))
                    .and_then(|_| save_matches(&conn, matches, sport_id, &dataset_id));
                if let Err(e) = list_saved {
                    eprintln!("[Crawler] Error saving validated list snapshot: {:?}", e);
                    continue;
                }
                let captured_at = decoded
                    .timestamp
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .unwrap_or_else(chrono::Utc::now)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                if let Err(e) =
                    mark_sport_snapshot_ready(&conn, &dataset_id, sport_id, &captured_at)
                {
                    eprintln!("[Crawler] Error recording sport readiness: {:?}", e);
                    continue;
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

                let need_details = match get_matches_needing_detail(
                    &conn,
                    sport_id,
                    detail_interval_secs,
                    &dataset_id,
                ) {
                    Ok(list) => list,
                    Err(e) => {
                        eprintln!("[Crawler] Error querying matches needing details: {:?}", e);
                        Vec::new()
                    }
                };

                if !need_details.is_empty() {
                    println!(
                        "[Crawler] Found {} matches needing detail fetch (concurrency cap={}).",
                        need_details.len(),
                        cli.detail_concurrency
                    );
                }

                // Launch detail fetches concurrently, bounded by semaphore.
                // Each detail fetch runs independently — a single failure does not block others.
                let chrome_url_clone = cli.chrome_url.clone();
                let detail_ready_delay = cli.detail_ready_delay_ms;
                let dataset_id_clone = dataset_id.clone();
                let db_path_clone2 = resolved_db_path.clone();
                let sem_clone = detail_sem.clone();

                for (match_id, home_slug, away_slug) in need_details {
                    // DB identity check before launching
                    if !verify_db_identity(&resolved_db_path, &dataset_id, file_identity) {
                        println!(
                            "[Crawler] DB replaced during details launch! Aborting details iteration."
                        );
                        break;
                    }

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
                        "[Crawler] Scheduling detail fetch for match {} (URL: {})...",
                        match_id, detail_url
                    );

                    let chrome_url2 = chrome_url_clone.clone();
                    let ds_id = dataset_id_clone.clone();
                    let db_path3 = db_path_clone2.clone();
                    let sem2 = sem_clone.clone();
                    let mid = match_id.clone();
                    let sp = sport_id;

                    tokio::spawn(async move {
                        // Acquire concurrency permit — this enforces the cap
                        let _permit = sem2.acquire().await.ok()?;

                        println!(
                            "[Crawler] Fetching detail for match {} via dedicated tab...",
                            mid
                        );

                        // Create dedicated Chrome detail tab
                        let owned_detail =
                            match create_detail_target(&chrome_url2, &detail_url, &mid, sp).await {
                                Ok(ot) => ot,
                                Err(e) => {
                                    eprintln!(
                                        "[Crawler] Failed to create detail target for match {}: {}",
                                        mid, e
                                    );
                                    return None;
                                }
                            };

                        let result = fetch_detail_from_target(
                            &owned_detail.websocket_url,
                            &mid,
                            sp,
                            detail_ready_delay,
                            Duration::from_secs(25),
                            Duration::from_millis(800),
                        )
                        .await;

                        // Always close the detail target after use
                        close_target(&chrome_url2, &owned_detail.target_id).await;

                        match result {
                            Ok(validated_detail) => {
                                // Verify DB identity before writing
                                let fi = get_file_identity(&db_path3);
                                if let Ok(conn) = open_db(&db_path3) {
                                    // Confirm dataset_id still matches
                                    let current_ds: Option<String> = conn
                                        .query_row(
                                            "SELECT value FROM settings WHERE key='active_dataset_id'",
                                            [],
                                            |row| row.get(0),
                                        )
                                        .optional()
                                        .unwrap_or(None);
                                    if current_ds.as_deref() != Some(&ds_id) {
                                        eprintln!(
                                            "[Crawler] dataset_id changed before saving detail for match {}; discarding.",
                                            mid
                                        );
                                        return None;
                                    }
                                    if fi != get_file_identity(&db_path3) {
                                        eprintln!(
                                            "[Crawler] DB identity changed before saving detail for match {}; discarding.",
                                            mid
                                        );
                                        return None;
                                    }
                                    if let Err(e) = save_match_detail(
                                        &conn,
                                        &mid,
                                        sp,
                                        &validated_detail,
                                        &ds_id,
                                    ) {
                                        eprintln!(
                                            "[Crawler] Error saving match detail for {}: {:?}",
                                            mid, e
                                        );
                                    } else {
                                        println!(
                                            "[Crawler] Successfully saved details for match {}.",
                                            mid
                                        );
                                    }
                                }
                                Some(())
                            }
                            Err(e) => {
                                eprintln!("[Crawler] Detail fetch failed for match {}: {}", mid, e);
                                None
                            }
                        }
                    });
                }
            }
        }

        // Cleanup: close our owned list target (but not other user tabs)
        if let Some(ref ot) = owned_list_target {
            println!(
                "[Crawler] Closing owned list target {} on disconnect.",
                ot.target_id
            );
            close_target(&cli.chrome_url, &ot.target_id).await;
        }
        owned_list_target = None;

        let _ = read_task.await;
        timer_task.abort();
        println!("[Crawler] Connection lost. Reconnecting in 5s...");
        sleep(Duration::from_secs(5)).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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

        let pragma_is_live = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('matches') WHERE name = 'is_live'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(pragma_is_live, 1);

        let synced_val: i32 = conn
            .query_row(
                "SELECT synced FROM matches WHERE id = 'legacy-match'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(synced_val, 0);

        let dataset_id_val: String = conn
            .query_row(
                "SELECT dataset_id FROM matches WHERE id = 'legacy-match'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dataset_id_val, "legacy-dataset-id");
    }

    #[test]
    fn test_dataset_registry_migration_adds_generation_columns_before_use() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE datasets (
                dataset_id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO datasets (dataset_id, created_at)
             VALUES ('existing-dataset', '2026-07-10T00:00:00Z')",
            [],
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert!(column_exists(&conn, "datasets", "generation_order").unwrap());
        assert!(column_exists(&conn, "datasets", "schema_generation").unwrap());
        let existing_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM datasets WHERE dataset_id='existing-dataset'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(existing_count, 1);
        let generation_order: i64 = conn
            .query_row(
                "SELECT generation_order FROM datasets WHERE dataset_id='existing-dataset'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(generation_order > 0);
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

        let decoded = decode_and_validate_snapshot(&parsed, 1).unwrap();
        assert_eq!(decoded.matches.len(), 1);
        assert_eq!(decoded.teams.len(), 2);
        assert_eq!(decoded.competitions.len(), 1);
        assert_eq!(decoded.matches[0]["id"], "foot-match-1");
    }

    #[test]
    fn test_extractor_basketball() {
        let fixture = std::fs::read_to_string("tests/fixtures/basketball-live.json")
            .expect("Failed to read basketball-live.json");
        let parsed: Value = serde_json::from_str(&fixture).unwrap();

        let decoded = decode_and_validate_snapshot(&parsed, 2).unwrap();
        assert_eq!(decoded.matches.len(), 1);
        assert_eq!(decoded.teams.len(), 2);
        assert_eq!(decoded.competitions.len(), 1);
        assert_eq!(decoded.matches[0]["id"], "bask-match-1");
    }

    #[test]
    fn test_source_contract_football() {
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        let decoded = decode_and_validate_snapshot(&parsed, 1).unwrap();

        let match_item = &decoded.matches[0];
        let comp_id = match_item["competition"]["id"].as_str().unwrap();
        let home_id = match_item["homeTeam"]["id"].as_str().unwrap();
        let away_id = match_item["awayTeam"]["id"].as_str().unwrap();

        assert!(
            decoded
                .competitions
                .iter()
                .any(|c| c["id"].as_str().unwrap() == comp_id)
        );
        assert!(
            decoded
                .teams
                .iter()
                .any(|t| t["id"].as_str().unwrap() == home_id)
        );
        assert!(
            decoded
                .teams
                .iter()
                .any(|t| t["id"].as_str().unwrap() == away_id)
        );
    }

    #[test]
    fn test_source_contract_basketball() {
        let fixture = std::fs::read_to_string("tests/fixtures/basketball-live.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        let decoded = decode_and_validate_snapshot(&parsed, 2).unwrap();

        let match_item = &decoded.matches[0];
        let comp_id = match_item["competition"]["id"].as_str().unwrap();
        let home_id = match_item["homeTeam"]["id"].as_str().unwrap();
        let away_id = match_item["awayTeam"]["id"].as_str().unwrap();

        assert!(
            decoded
                .competitions
                .iter()
                .any(|c| c["id"].as_str().unwrap() == comp_id)
        );
        assert!(
            decoded
                .teams
                .iter()
                .any(|t| t["id"].as_str().unwrap() == home_id)
        );
        assert!(
            decoded
                .teams
                .iter()
                .any(|t| t["id"].as_str().unwrap() == away_id)
        );
    }

    #[test]
    fn test_source_filter_rejection() {
        let fixture = std::fs::read_to_string("tests/fixtures/source-filter-states.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();

        let all_state = &parsed["all_state"];
        let res = decode_and_validate_snapshot(all_state, 1);
        assert!(res.is_err());
        assert!(matches!(res.err().unwrap(), ExtractionError::FilterNotLive));

        let live_state = &parsed["live_state"];
        let res_live = decode_and_validate_snapshot(live_state, 1);
        assert!(res_live.is_ok());
    }

    #[test]
    fn test_detail_decoder_validation() {
        let foot_fixture = std::fs::read_to_string("tests/fixtures/football-detail.json").unwrap();
        let foot_parsed: Value = serde_json::from_str(&foot_fixture).unwrap();

        let res1 = decode_and_validate_detail(&foot_parsed, "foot-match-1", 1);
        assert!(res1.is_ok());

        let res2 = decode_and_validate_detail(&foot_parsed, "wrong-match-id", 1);
        assert!(res2.is_err());
        assert!(matches!(
            res2.err().unwrap(),
            DetailDecoderError::MatchIdMismatch { .. }
        ));

        let mut foot_empty = foot_parsed.clone();
        foot_empty["matchId"] = serde_json::json!("");
        let res3 = decode_and_validate_detail(&foot_empty, "foot-match-1", 1);
        assert!(res3.is_err());
        assert!(matches!(
            res3.err().unwrap(),
            DetailDecoderError::EmptyMatchId
        ));

        let res4 = decode_and_validate_detail(&foot_parsed, "foot-match-1", 2);
        assert!(res4.is_err());
        assert!(matches!(
            res4.err().unwrap(),
            DetailDecoderError::SportIdMismatch { .. }
        ));
    }

    #[test]
    fn test_status_score_mapping_football() {
        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        let live_snap = decode_and_validate_snapshot(&live_val, 1).unwrap();
        let live_match = &live_snap.matches[0];

        assert_eq!(live_match["statusId"].as_i64().unwrap(), 2);
        assert_eq!(
            live_match["homeScores"].as_array().unwrap(),
            &vec![
                serde_json::json!(1),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0)
            ]
        );

        let finished_fixture =
            std::fs::read_to_string("tests/fixtures/football-finished.json").unwrap();
        let finished_val: Value = serde_json::from_str(&finished_fixture).unwrap();
        let finished_snap = decode_and_validate_snapshot(&finished_val, 1).unwrap();
        let finished_match = &finished_snap.matches[0];

        assert_eq!(live_match["id"], finished_match["id"]);
        assert_eq!(finished_match["statusId"].as_i64().unwrap(), 8);
        assert_eq!(
            finished_match["homeScores"].as_array().unwrap(),
            &vec![
                serde_json::json!(2),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0)
            ]
        );
    }

    #[test]
    fn test_status_score_mapping_basketball() {
        let live_fixture = std::fs::read_to_string("tests/fixtures/basketball-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        let live_snap = decode_and_validate_snapshot(&live_val, 2).unwrap();
        let live_match = &live_snap.matches[0];

        assert_eq!(live_match["statusId"].as_i64().unwrap(), 2);
        assert_eq!(
            live_match["homeScores"].as_array().unwrap(),
            &vec![
                serde_json::json!(25),
                serde_json::json!(25),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0)
            ]
        );

        let finished_fixture =
            std::fs::read_to_string("tests/fixtures/basketball-finished.json").unwrap();
        let finished_val: Value = serde_json::from_str(&finished_fixture).unwrap();
        let finished_snap = decode_and_validate_snapshot(&finished_val, 2).unwrap();
        let finished_match = &finished_snap.matches[0];

        assert_eq!(live_match["id"], finished_match["id"]);
        assert_eq!(finished_match["statusId"].as_i64().unwrap(), 8);
        assert_eq!(
            finished_match["homeScores"].as_array().unwrap(),
            &vec![serde_json::json!(105), serde_json::json!(100)]
        );
    }

    #[test]
    fn test_crawler_reconciliation() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();

        // 1. Initial Insert
        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        let live_snap = decode_and_validate_snapshot(&live_val, 1).unwrap();

        save_competitions(&conn, &live_snap.competitions, 1, &dataset_id).unwrap();
        save_teams(&conn, &live_snap.teams, 1, &dataset_id).unwrap();
        save_matches(&conn, &live_snap.matches, 1, &dataset_id).unwrap();

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

        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 2. Unchanged no-op
        std::thread::sleep(std::time::Duration::from_millis(50));
        save_matches(&conn, &live_snap.matches, 1, &dataset_id).unwrap();
        let (_status_id2, _raw_payload2, updated_at2, synced2): (i32, String, String, i32) = conn
            .query_row(
                "SELECT status_id, raw_payload, updated_at, synced FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(updated_at, updated_at2);
        assert_eq!(synced2, 1);

        // 3. Live Score Update
        let mut updated_live_snap = live_snap.clone();
        updated_live_snap.matches[0]["homeScores"] = serde_json::json!([2, 0, 0, 0, 0]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        save_matches(&conn, &updated_live_snap.matches, 1, &dataset_id).unwrap();

        let (status_id3, synced3, updated_at3): (i32, i32, String) = conn
            .query_row(
                "SELECT status_id, synced, updated_at FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status_id3, 2);
        assert_eq!(synced3, 0);
        assert_ne!(updated_at, updated_at3);

        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 4. Finished Transition
        let fin_fixture = std::fs::read_to_string("tests/fixtures/football-finished.json").unwrap();
        let fin_val: Value = serde_json::from_str(&fin_fixture).unwrap();
        let fin_snap = decode_and_validate_snapshot(&fin_val, 1).unwrap();
        save_matches(&conn, &fin_snap.matches, 1, &dataset_id).unwrap();

        let (status_id4, synced4): (i32, i32) = conn
            .query_row(
                "SELECT status_id, synced FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status_id4, 8);
        assert_eq!(synced4, 0);

        conn.execute(
            "UPDATE matches SET synced = 1 WHERE id = 'foot-match-1'",
            [],
        )
        .unwrap();

        // 5. Stale payload check (Finished match must not revert to live!)
        save_matches(&conn, &live_snap.matches, 1, &dataset_id).unwrap();
        let status_id5: i32 = conn
            .query_row(
                "SELECT status_id FROM matches WHERE id = 'foot-match-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status_id5, 8);

        // 6. Detail Scheduling
        let detail_interval = 60;
        let needs = get_matches_needing_detail(&conn, 1, detail_interval, &dataset_id).unwrap();
        assert_eq!(needs.len(), 1);

        let detail_payload = serde_json::json!({
            "matchId": "foot-match-1",
            "incidents": [],
            "stats": {},
            "lineups": {},
            "odds": {},
            "h2h": {},
            "chat": "ignored"
        });
        save_match_detail(&conn, "foot-match-1", 1, &detail_payload, &dataset_id).unwrap();

        let needs2 = get_matches_needing_detail(&conn, 1, detail_interval, &dataset_id).unwrap();
        assert_eq!(needs2.len(), 0);
    }

    #[test]
    fn live_snapshot_membership_hides_absent_match_and_allows_reappearance() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let value: Value = serde_json::from_str(&fixture).unwrap();
        let snapshot = decode_and_validate_snapshot(&value, 1).unwrap();

        save_matches(&conn, &snapshot.matches, 1, &dataset_id).unwrap();
        let first_live: bool = conn
            .query_row(
                "SELECT is_live FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(first_live);

        conn.execute(
            "UPDATE matches SET synced=1 WHERE id='foot-match-1' AND dataset_id=?1",
            params![dataset_id],
        )
        .unwrap();
        save_matches(&conn, &[], 1, &dataset_id).unwrap();
        let hidden: (bool, i32) = conn
            .query_row(
                "SELECT is_live, synced FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(hidden, (false, 0));

        save_matches(&conn, &snapshot.matches, 1, &dataset_id).unwrap();
        let live_again: bool = conn
            .query_row(
                "SELECT is_live FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(live_again);
    }

    #[test]
    fn terminal_status_label_overrides_live_numeric_status() {
        let live = serde_json::json!({ "statusId": 2, "statusText": "First Half" });
        let full_time = serde_json::json!({ "statusId": 2, "statusText": "Full Time" });
        let nested_ft = serde_json::json!({
            "statusId": 2,
            "status": { "id": 2, "name": "FT" }
        });

        assert!(match_is_currently_live(&live, 1));
        assert!(!match_is_currently_live(&full_time, 1));
        assert!(!match_is_currently_live(&nested_ft, 1));
    }

    #[test]
    fn terminal_match_is_persisted_outside_live_feed() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let value: Value = serde_json::from_str(&fixture).unwrap();
        let mut snapshot = decode_and_validate_snapshot(&value, 1).unwrap();
        snapshot.matches[0]["statusText"] = serde_json::json!("Full Time");

        save_matches(&conn, &snapshot.matches, 1, &dataset_id).unwrap();
        let is_live: bool = conn
            .query_row(
                "SELECT is_live FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!is_live);
    }

    #[test]
    fn source_extractors_filter_to_live_matches_and_relations() {
        for js in [
            FootballAdapter.extract_state_js(),
            BasketballAdapter.extract_state_js(),
        ] {
            assert!(js.contains(".filter(isLiveMatch)"));
            assert!(js.contains("teamIds.has"));
            assert!(js.contains("competitionIds.has"));
            assert!(js.contains("full[\\s-]*time"));
            assert!(js.contains("timestamp: Date.now()"));
        }
    }

    #[test]
    fn test_uploader_lease_exclusion_and_recovery() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        assert!(claim_uploader_lease(&conn, "client-a"));
        assert!(!claim_uploader_lease(&conn, "client-b"));

        conn.execute(
            "UPDATE settings SET value='100' WHERE key='uploader_lease_expires'",
            [],
        )
        .unwrap();

        assert!(claim_uploader_lease(&conn, "client-b"));
        assert!(!claim_uploader_lease(&conn, "client-a"));

        release_uploader_lease(&conn, "client-b");
        assert!(claim_uploader_lease(&conn, "client-a"));
    }

    #[test]
    fn lease_concurrent_claim_has_single_winner() {
        let path = std::env::temp_dir().join(format!("lease-race-{}.db", uuid::Uuid::new_v4()));
        let path_string = path.to_string_lossy().to_string();
        init_db(&path_string).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut handles = Vec::new();
        for client in ["client-a", "client-b"] {
            let db_path = path_string.clone();
            let start = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let conn = open_db(&db_path).unwrap();
                start.wait();
                claim_uploader_lease(&conn, client)
            }));
        }
        barrier.wait();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sync_generation_empty_live_snapshot_records_ready_sport() {
        let snapshot = serde_json::json!({
            "activeFilter": "live",
            "sportId": 2,
            "matches": [],
            "teams": [],
            "competitions": []
        });
        let decoded = decode_and_validate_snapshot(&snapshot, 2).unwrap();
        assert!(decoded.matches.is_empty());

        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        mark_sport_snapshot_ready(&conn, &dataset_id, 2, "2026-07-10T09:00:00.000Z").unwrap();
        let ready: (String, i32) = conn
            .query_row(
                "SELECT captured_at, synced FROM dataset_sports WHERE dataset_id=?1 AND sport_id=2",
                params![dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(ready, ("2026-07-10T09:00:00.000Z".to_string(), 0));
    }

    #[test]
    fn sync_generation_fixtures_define_ordered_single_sport_batches() {
        let generation_a: Value = serde_json::from_str(
            &std::fs::read_to_string("tests/fixtures/sync-batch.json").unwrap(),
        )
        .unwrap();
        let generation_b: Value = serde_json::from_str(
            &std::fs::read_to_string("tests/fixtures/sync-batch-next-generation.json").unwrap(),
        )
        .unwrap();

        assert_eq!(generation_a["protocol_version"], 2);
        assert_eq!(generation_b["protocol_version"], 2);
        assert_eq!(generation_a["sport_id"], 1);
        assert_eq!(generation_b["sport_id"], 1);
        assert!(
            generation_b["generation_order"].as_i64().unwrap()
                > generation_a["generation_order"].as_i64().unwrap()
        );
        for payload in [&generation_a, &generation_b] {
            let dataset_id = payload["dataset_id"].as_str().unwrap();
            for collection in ["competitions", "teams", "matches", "match_details"] {
                for row in payload[collection].as_array().unwrap() {
                    assert_eq!(row["dataset_id"], dataset_id);
                    assert_eq!(row["sport_id"], payload["sport_id"]);
                }
            }
        }
    }

    #[test]
    fn test_sync_dirty_change_during_inflight_request() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();

        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        let live_snap = decode_and_validate_snapshot(&live_val, 1).unwrap();
        save_matches(&conn, &live_snap.matches, 1, &dataset_id).unwrap();

        let synced: i32 = conn
            .query_row(
                "SELECT synced FROM matches WHERE id='foot-match-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(synced, 0);

        let sent_row = conn
            .query_row(
                "SELECT id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload, dataset_id FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| {
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
                        is_live: row.get(9)?,
                        raw_payload: row.get(10)?,
                        dataset_id: row.get(11)?,
                    })
                },
            )
            .unwrap();

        let mut updated_live_snap = live_snap.clone();
        updated_live_snap.matches[0]["homeScores"] = serde_json::json!([3, 0, 0, 0, 0]);
        save_matches(&conn, &updated_live_snap.matches, 1, &dataset_id).unwrap();

        assert_eq!(
            acknowledge_match_if_unchanged(&conn, &sent_row, &dataset_id).unwrap(),
            0
        );
        let still_dirty: i32 = conn
            .query_row(
                "SELECT synced FROM matches WHERE id='foot-match-1' AND dataset_id=?1",
                params![dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_dirty, 0);
    }

    #[test]
    fn test_dataset_fresh_db_uuid_persistence() {
        let tempdir = std::env::temp_dir();
        let db_file = tempdir.join(format!("test_persist_{}.db", uuid::Uuid::new_v4()));
        let db_path = db_file.to_str().unwrap();

        {
            let _ = init_db(db_path).unwrap();
            let conn = open_db(db_path).unwrap();
            let id1 = init_dataset_id(&conn).unwrap();
            let id2 = init_dataset_id(&conn).unwrap();
            assert_eq!(id1, id2);
        }

        {
            let conn = open_db(db_path).unwrap();
            let id3 = init_dataset_id(&conn).unwrap();
            let conn2 = open_db(db_path).unwrap();
            let id4 = init_dataset_id(&conn2).unwrap();
            assert_eq!(id3, id4);
        }

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_dataset_concurrent_init_convergence() {
        let tempdir = std::env::temp_dir();
        let db_file = tempdir.join(format!("test_concurrent_{}.db", uuid::Uuid::new_v4()));
        let db_path = db_file.to_str().unwrap();
        let _ = init_db(db_path).unwrap();

        let db_path_clone1 = db_path.to_string();
        let db_path_clone2 = db_path.to_string();

        let t1 = std::thread::spawn(move || {
            let conn = open_db(&db_path_clone1).unwrap();
            init_dataset_id(&conn).unwrap()
        });

        let t2 = std::thread::spawn(move || {
            let conn = open_db(&db_path_clone2).unwrap();
            init_dataset_id(&conn).unwrap()
        });

        let id1 = t1.join().unwrap();
        let id2 = t2.join().unwrap();
        assert_eq!(id1, id2);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_dataset_different_files_produce_different_ids() {
        let tempdir = std::env::temp_dir();
        let db_file1 = tempdir.join(format!("test_diff1_{}.db", uuid::Uuid::new_v4()));
        let db_file2 = tempdir.join(format!("test_diff2_{}.db", uuid::Uuid::new_v4()));
        let db_path1 = db_file1.to_str().unwrap();
        let db_path2 = db_file2.to_str().unwrap();

        let _ = init_db(db_path1).unwrap();
        let conn1 = open_db(db_path1).unwrap();
        let id1 = init_dataset_id(&conn1).unwrap();

        let _ = init_db(db_path2).unwrap();
        let conn2 = open_db(db_path2).unwrap();
        let id2 = init_dataset_id(&conn2).unwrap();

        assert_ne!(id1, id2);

        let _ = std::fs::remove_file(db_path1);
        let _ = std::fs::remove_file(db_path2);
    }

    #[test]
    fn test_db_identity_delete_recreate_simulation() {
        let tempdir = std::env::temp_dir();
        let db_file = tempdir.join(format!("test_identity_{}.db", uuid::Uuid::new_v4()));
        let db_path = db_file.to_str().unwrap();

        let _ = init_db(db_path).unwrap();
        let conn = open_db(db_path).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        let identity = get_file_identity(db_path);

        assert!(verify_db_identity(db_path, &dataset_id, identity));

        let _ = std::fs::remove_file(db_path);
        let _ = init_db(db_path).unwrap();

        assert!(!verify_db_identity(db_path, &dataset_id, identity));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_reset_recovery_simulation() {
        let tempdir = std::env::temp_dir();
        let db_file = tempdir.join(format!("test_reset_{}.db", uuid::Uuid::new_v4()));
        let db_path = db_file.to_str().unwrap();

        let _ = init_db(db_path).unwrap();
        let conn = open_db(db_path).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        let identity = get_file_identity(db_path);

        assert!(verify_db_identity(db_path, &dataset_id, identity));

        let _ = std::fs::remove_file(db_path);

        assert!(!verify_db_identity(db_path, &dataset_id, identity));

        let _ = init_db(db_path).unwrap();
        let conn2 = open_db(db_path).unwrap();
        let new_dataset_id = init_dataset_id(&conn2).unwrap();
        let new_identity = get_file_identity(db_path);

        assert_ne!(dataset_id, new_dataset_id);
        assert!(verify_db_identity(db_path, &new_dataset_id, new_identity));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_dataset_generation_scoped_relation_queries() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let ds1 = "dataset-1";
        let ds2 = "dataset-2";

        conn.execute(
            "INSERT INTO teams (id, sport_id, name, dataset_id) VALUES ('team-1', 1, 'Team A', ?1)",
            params![ds1],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO teams (id, sport_id, name, dataset_id) VALUES ('team-1', 1, 'Team B', ?1)",
            params![ds2],
        )
        .unwrap();

        let name_ds1: String = conn
            .query_row(
                "SELECT name FROM teams WHERE id='team-1' AND dataset_id=?1",
                params![ds1],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name_ds1, "Team A");

        let name_ds2: String = conn
            .query_row(
                "SELECT name FROM teams WHERE id='team-1' AND dataset_id=?1",
                params![ds2],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name_ds2, "Team B");
    }

    #[test]
    fn test_migration_idempotence() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
    }

    // ── Task 07 new deterministic tests ──────────────────────────────────────

    /// target_ownership_: each sport session's adapter produces a different target URL
    /// (no shared tab hijacking). Football → aiscore.com, Basketball → basketball.
    #[test]
    fn target_ownership_different_sport_adapters_have_different_urls() {
        let foot = FootballAdapter;
        let bask = BasketballAdapter;
        assert_ne!(foot.target_url(), bask.target_url());
        assert_ne!(foot.sport_id(), bask.sport_id());
        // football adapter must NOT match basketball URL
        assert!(!foot.matches_url("https://m.aiscore.com/basketball"));
        // basketball adapter must NOT match plain football URL
        assert!(!bask.matches_url("https://m.aiscore.com/"));
    }

    /// target_ownership_: OwnedTarget fields are correct after construction.
    #[test]
    fn target_ownership_owned_target_fields() {
        let ot = OwnedTarget {
            target_id: "tab-abc".to_string(),
            websocket_url: "ws://127.0.0.1:9223/devtools/page/tab-abc".to_string(),
            role: TargetRole::List,
            sport_id: 1,
            requested_match_id: None,
            created_at: Instant::now(),
        };
        assert_eq!(ot.target_id, "tab-abc");
        assert_eq!(ot.role, TargetRole::List);
        assert_eq!(ot.sport_id, 1);
        assert!(ot.requested_match_id.is_none());
    }

    /// target_ownership_: detail OwnedTarget carries the requested match ID.
    #[test]
    fn target_ownership_detail_target_carries_match_id() {
        let ot = OwnedTarget {
            target_id: "tab-detail-1".to_string(),
            websocket_url: "ws://127.0.0.1:9223/devtools/page/tab-detail-1".to_string(),
            role: TargetRole::Detail,
            sport_id: 2,
            requested_match_id: Some("bask-match-1".to_string()),
            created_at: Instant::now(),
        };
        assert_eq!(ot.role, TargetRole::Detail);
        assert_eq!(ot.requested_match_id.as_deref(), Some("bask-match-1"));
    }

    /// readiness_: FilterNotLive state is correctly rejected by decode_and_validate_snapshot.
    #[test]
    fn readiness_filter_not_live_rejected() {
        let fixture = std::fs::read_to_string("tests/fixtures/source-filter-states.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        let all_state = &parsed["all_state"];
        // "all" filter must be rejected
        let result = decode_and_validate_snapshot(all_state, 1);
        assert!(matches!(result.err(), Some(ExtractionError::FilterNotLive)));
    }

    /// readiness_: Live state is accepted after filter activation.
    #[test]
    fn readiness_live_state_accepted() {
        let fixture = std::fs::read_to_string("tests/fixtures/source-filter-states.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        let live_state = &parsed["live_state"];
        let result = decode_and_validate_snapshot(live_state, 1);
        assert!(result.is_ok());
        let snap = result.unwrap();
        assert_eq!(snap.active_filter, "live");
    }

    /// readiness_: Two identical snapshots are needed for stability — simulated
    /// by checking that the same JSON serialises identically.
    #[test]
    fn readiness_stable_snapshot_requires_two_identical_probes() {
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let val: Value = serde_json::from_str(&fixture).unwrap();
        let snap1 = decode_and_validate_snapshot(&val, 1).unwrap();
        let snap2 = decode_and_validate_snapshot(&val, 1).unwrap();
        // Same source → identical JSON → would be considered stable
        let j1 = serde_json::to_string(&snap1.matches).unwrap();
        let j2 = serde_json::to_string(&snap2.matches).unwrap();
        assert_eq!(j1, j2);

        // Different matches → unstable
        let fixture2 = std::fs::read_to_string("tests/fixtures/football-finished.json").unwrap();
        let val2: Value = serde_json::from_str(&fixture2).unwrap();
        let snap3 = decode_and_validate_snapshot(&val2, 1).unwrap();
        let j3 = serde_json::to_string(&snap3.matches).unwrap();
        // Different status/scores → not equal
        assert_ne!(j1, j3);
    }

    /// readiness_: null Vuex state returns FilterNotLive (indirectly) — the null check
    /// upstream prevents calling decode_and_validate_snapshot, so here we confirm that
    /// a null value returns the correct parse-side behavior.
    #[test]
    fn readiness_null_state_produces_filter_not_live_or_json_error() {
        let null_val = serde_json::json!(null);
        // In runtime, null is caught before decode_and_validate_snapshot;
        // if it did reach it, activeFilter would be "all".
        let result = decode_and_validate_snapshot(&null_val, 1);
        // activeFilter defaults to "all" → FilterNotLive
        assert!(matches!(result.err(), Some(ExtractionError::FilterNotLive)));
    }

    /// detail_concurrency_: semaphore correctly caps simultaneous acquisitions.
    #[tokio::test]
    async fn detail_concurrency_semaphore_caps_simultaneous_tasks() {
        let cap = 3usize;
        let sem = Arc::new(Semaphore::new(cap));
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max_observed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let sem2 = sem.clone();
            let ctr = counter.clone();
            let mo = max_observed.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem2.acquire().await.unwrap();
                let cur = ctr.fetch_add(1, Ordering::SeqCst) + 1;
                // Track peak
                let mut prev = mo.load(Ordering::SeqCst);
                while cur > prev {
                    match mo.compare_exchange(prev, cur, Ordering::SeqCst, Ordering::SeqCst) {
                        Ok(_) => break,
                        Err(x) => prev = x,
                    }
                }
                // Simulate short work
                tokio::time::sleep(Duration::from_millis(10)).await;
                ctr.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Max concurrent must not exceed cap
        assert!(max_observed.load(Ordering::SeqCst) <= cap);
    }

    /// detail_concurrency_: concurrent limit of 1 means strictly serial execution.
    #[tokio::test]
    async fn detail_concurrency_cap_one_is_serial() {
        let sem = Arc::new(Semaphore::new(1));
        let order = Arc::new(Mutex::new(Vec::<usize>::new()));

        let mut handles = Vec::new();
        for i in 0..4 {
            let sem2 = sem.clone();
            let ord = order.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem2.acquire().await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
                ord.lock().await.push(i);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let final_order = order.lock().await;
        assert_eq!(final_order.len(), 4);
    }

    /// wrong_match_: detail decoder rejects wrong match ID.
    #[test]
    fn wrong_match_detail_decoder_rejects_wrong_match_id() {
        let foot_fixture = std::fs::read_to_string("tests/fixtures/football-detail.json").unwrap();
        let foot_parsed: Value = serde_json::from_str(&foot_fixture).unwrap();
        let res = decode_and_validate_detail(&foot_parsed, "wrong-match-999", 1);
        assert!(matches!(
            res.err(),
            Some(DetailDecoderError::MatchIdMismatch { .. })
        ));
    }

    /// wrong_match_: detail decoder rejects wrong sport ID.
    #[test]
    fn wrong_match_detail_decoder_rejects_wrong_sport_id() {
        let foot_fixture = std::fs::read_to_string("tests/fixtures/football-detail.json").unwrap();
        let foot_parsed: Value = serde_json::from_str(&foot_fixture).unwrap();
        // Correct match but wrong sport (basketball = 2)
        let res = decode_and_validate_detail(&foot_parsed, "foot-match-1", 2);
        assert!(matches!(
            res.err(),
            Some(DetailDecoderError::SportIdMismatch { .. })
        ));
    }

    /// wrong_match_: basketball detail fixture accepted for basketball sport ID.
    #[test]
    fn wrong_match_basketball_detail_accepted_for_correct_sport() {
        let fixture = std::fs::read_to_string("tests/fixtures/basketball-detail.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        let res = decode_and_validate_detail(&parsed, "bask-match-1", 2);
        assert!(res.is_ok());
    }

    /// wrong_match_: cross-sport mismatch in list snapshot is rejected.
    #[test]
    fn wrong_match_cross_sport_mismatch_in_snapshot() {
        let fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let parsed: Value = serde_json::from_str(&fixture).unwrap();
        // Football snapshot asked for as basketball (sport_id=2)
        let res = decode_and_validate_snapshot(&parsed, 2);
        assert!(matches!(
            res.err(),
            Some(ExtractionError::CrossSportMismatch { .. })
        ));
    }

    /// crawler_: save then retrieve detail — save_match_detail / get_matches_needing_detail lifecycle.
    #[test]
    fn crawler_detail_save_and_scheduling_lifecycle() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();

        // Set up a match
        let live_fixture = std::fs::read_to_string("tests/fixtures/football-live.json").unwrap();
        let live_val: Value = serde_json::from_str(&live_fixture).unwrap();
        let snap = decode_and_validate_snapshot(&live_val, 1).unwrap();
        save_competitions(&conn, &snap.competitions, 1, &dataset_id).unwrap();
        save_teams(&conn, &snap.teams, 1, &dataset_id).unwrap();
        save_matches(&conn, &snap.matches, 1, &dataset_id).unwrap();

        // Needs detail before saving
        let needs_before = get_matches_needing_detail(&conn, 1, 60, &dataset_id).unwrap();
        assert_eq!(needs_before.len(), 1);

        // Save detail
        let detail_val = serde_json::json!({
            "matchId": "foot-match-1",
            "sportId": 1,
            "incidents": [],
            "stats": {},
            "lineups": {},
            "odds": {},
            "h2h": {}
        });
        save_match_detail(&conn, "foot-match-1", 1, &detail_val, &dataset_id).unwrap();

        // No longer needs detail (match is finished per fixture, last_updated >= updated_at)
        let needs_after = get_matches_needing_detail(&conn, 1, 60, &dataset_id).unwrap();
        assert_eq!(needs_after.len(), 0);
    }

    /// crawler_: detail save is scoped to dataset_id; different dataset sees no detail.
    #[test]
    fn crawler_detail_scoped_to_dataset_id() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let ds1 = "ds-alpha";
        let ds2 = "ds-beta";

        let detail_val = serde_json::json!({
            "matchId": "match-xyz",
            "sportId": 1,
            "incidents": [{"time": 5}],
            "stats": {},
            "lineups": {},
            "odds": {},
            "h2h": {}
        });
        save_match_detail(&conn, "match-xyz", 1, &detail_val, ds1).unwrap();

        // ds2 should not find the detail
        let row: Option<String> = conn
            .query_row(
                "SELECT incidents FROM match_details WHERE match_id='match-xyz' AND dataset_id=?1",
                params![ds2],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(row.is_none());

        // ds1 should find it
        let row2: Option<String> = conn
            .query_row(
                "SELECT incidents FROM match_details WHERE match_id='match-xyz' AND dataset_id=?1",
                params![ds1],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(row2.is_some());
    }

    /// crawler_: football and basketball sessions have isolated sport_id scopes in DB queries.
    #[test]
    fn crawler_two_sport_sessions_isolation_in_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();

        // Insert a football match and a basketball match
        let football_match = serde_json::json!({
            "id": "foot-isolate-1",
            "competition": {"id": "comp-foot"},
            "homeTeam": {"id": "team-foot-1"},
            "awayTeam": {"id": "team-foot-2"},
            "matchTime": 1700000000_i64,
            "statusId": 2,
            "homeScores": [1],
            "awayScores": [0]
        });
        let bask_match = serde_json::json!({
            "id": "bask-isolate-1",
            "competition": {"id": "comp-bask"},
            "homeTeam": {"id": "team-bask-1"},
            "awayTeam": {"id": "team-bask-2"},
            "matchTime": 1700000000_i64,
            "statusId": 2,
            "homeScores": [25],
            "awayScores": [20]
        });

        // We need comp + team rows too for the JOIN in get_matches_needing_detail
        conn.execute(
            "INSERT OR IGNORE INTO competitions (id, sport_id, name, dataset_id) VALUES ('comp-foot', 1, 'PL', ?1)",
            params![dataset_id],
        ).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO competitions (id, sport_id, name, dataset_id) VALUES ('comp-bask', 2, 'NBA', ?1)",
            params![dataset_id],
        ).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO teams (id, sport_id, name, slug, dataset_id) VALUES ('team-foot-1', 1, 'Arsenal', 'arsenal', ?1)",
            params![dataset_id],
        ).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO teams (id, sport_id, name, slug, dataset_id) VALUES ('team-foot-2', 1, 'Chelsea', 'chelsea', ?1)",
            params![dataset_id],
        ).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO teams (id, sport_id, name, slug, dataset_id) VALUES ('team-bask-1', 2, 'Lakers', 'lakers', ?1)",
            params![dataset_id],
        ).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO teams (id, sport_id, name, slug, dataset_id) VALUES ('team-bask-2', 2, 'Celtics', 'celtics', ?1)",
            params![dataset_id],
        ).unwrap();

        save_matches(&conn, &[football_match], 1, &dataset_id).unwrap();
        save_matches(&conn, &[bask_match], 2, &dataset_id).unwrap();

        // Football needs detail, basketball also needs detail
        let foot_needs = get_matches_needing_detail(&conn, 1, 60, &dataset_id).unwrap();
        let bask_needs = get_matches_needing_detail(&conn, 2, 60, &dataset_id).unwrap();

        // Each sport sees only its own match
        assert!(foot_needs.iter().all(|(id, _, _)| id == "foot-isolate-1"));
        assert!(bask_needs.iter().all(|(id, _, _)| id == "bask-isolate-1"));
        // They should not cross-pollute
        assert!(!foot_needs.iter().any(|(id, _, _)| id == "bask-isolate-1"));
        assert!(!bask_needs.iter().any(|(id, _, _)| id == "foot-isolate-1"));
    }

    /// crawler_: CLI argument validation — zero delay is rejected.
    #[test]
    fn crawler_cli_zero_delay_rejected() {
        assert!(parse_ready_delay_ms("0").is_err());
        assert!(parse_ready_delay_ms("1").is_ok());
        assert!(parse_ready_delay_ms("30000").is_ok());
        assert!(parse_ready_delay_ms("30001").is_err());
    }

    /// crawler_: CLI argument validation — zero concurrency is rejected, and >10 is rejected.
    #[test]
    fn crawler_cli_concurrency_bounds() {
        assert!(parse_detail_concurrency("0").is_err());
        assert!(parse_detail_concurrency("1").is_ok());
        assert!(parse_detail_concurrency("10").is_ok());
        assert!(parse_detail_concurrency("11").is_err());
    }

    /// target_ownership_: TargetRole enum equality works for clone comparisons.
    #[test]
    fn target_ownership_role_equality() {
        assert_eq!(TargetRole::List, TargetRole::List);
        assert_eq!(TargetRole::Detail, TargetRole::Detail);
        assert_ne!(TargetRole::List, TargetRole::Detail);
    }

    /// readiness_: detail_extract_js produces different store keys per sport.
    #[test]
    fn readiness_detail_extract_js_sport_specific() {
        let foot_js = detail_extract_js(1, "foot-match-1");
        let bask_js = detail_extract_js(2, "bask-match-1");
        assert!(foot_js.contains("football/detail"));
        assert!(!foot_js.contains("basketball/detail"));
        assert!(bask_js.contains("basketball/detail"));
        assert!(!bask_js.contains("football/detail"));
    }

    #[test]
    fn detail_tab_activation_uses_data_allowlist_and_excludes_chat() {
        assert_eq!(
            DETAIL_DATA_TAB_LABELS,
            [
                "Overview",
                "Odds",
                "Stats",
                "H2H",
                "Lineups",
                "Standings",
                "Prediction"
            ]
        );
        let js = detail_activate_tabs_js(1, "foot-match-1");
        for label in DETAIL_DATA_TAB_LABELS {
            assert!(js.contains(label));
        }
        assert!(!js.to_ascii_lowercase().contains("chat"));
        assert!(js.contains("await new Promise"));
        assert!(js.contains("element.click()"));
    }

    #[test]
    fn detail_database_persists_all_data_sections_and_sanitized_source() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let dataset_id = init_dataset_id(&conn).unwrap();
        let detail = serde_json::json!({
            "matchId": "all-sections-match",
            "sportId": 1,
            "incidents": [{"type": "goal"}],
            "stats": {"possession": [55, 45]},
            "lineups": {"home": ["p1"]},
            "odds": {"market": "1x2"},
            "h2h": {"matches": ["previous"]},
            "activatedTabs": ["Odds", "Stats", "H2H", "Lineups"],
            "sourceDetail": {
                "extraSection": {"value": 42},
                "chat": {"messages": ["must-not-persist"]}
            }
        });

        save_match_detail(&conn, "all-sections-match", 1, &detail, &dataset_id).unwrap();

        let stored: (String, String, String, String, String, String) = conn
            .query_row(
                "SELECT incidents, stats, lineups, odds, h2h, raw_payload
                 FROM match_details WHERE match_id=?1 AND dataset_id=?2",
                params!["all-sections-match", dataset_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert!(stored.0.contains("goal"));
        assert!(stored.1.contains("possession"));
        assert!(stored.2.contains("p1"));
        assert!(stored.3.contains("1x2"));
        assert!(stored.4.contains("previous"));
        assert!(stored.5.contains("extraSection"));
        assert!(stored.5.contains("activatedTabs"));
        assert!(!stored.5.to_ascii_lowercase().contains("chat"));
        assert!(!stored.5.contains("must-not-persist"));
    }
}
