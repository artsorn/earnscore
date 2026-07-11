use serde_json::Value;
use std::fmt;

mod basketball;
mod football;
pub use basketball::BasketballAdapter;
pub use football::FootballAdapter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLayer {
    Network,
    Store,
    Dom,
}
#[derive(Debug, Clone)]
pub struct SourceEnvelope {
    pub layer: SourceLayer,
    pub payload: Value,
}
impl SourceEnvelope {
    pub fn new(layer: SourceLayer, payload: Value) -> Self {
        Self { layer, payload }
    }
}
impl SourceLayer {
    pub fn priority(self) -> u8 {
        match self {
            Self::Network => 0,
            Self::Store => 1,
            Self::Dom => 2,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    WrongFilter,
    WrongSport,
    SourceChanged,
}
#[derive(Debug, Clone)]
pub struct NormalizedMatch {
    pub match_id: String,
    pub status_id: i32,
    pub home_score: Value,
    pub away_score: Value,
    pub period: Option<String>,
    pub clock: Option<String>,
    pub odds: Value,
    pub is_live: bool,
    pub source_timestamp: Option<String>,
    pub payload: Value,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    Browser(String),
    Disconnected,
    SourceChanged(String),
    WrongFilter,
    WrongSport,
    Invalid(String),
}
impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FeedError {}

pub trait FeedAdapter: Send + Sync + Sized + 'static {
    fn sport_id(&self) -> i32;
    fn name(&self) -> &'static str;
    fn target_url(&self) -> &'static str;
    fn extract(&self, envelope: &SourceEnvelope) -> Result<Vec<NormalizedMatch>, FeedError> {
        extract_matches(envelope, self.sport_id())
    }
}

pub(crate) fn extract_matches(
    envelope: &SourceEnvelope,
    sport_id: i32,
) -> Result<Vec<NormalizedMatch>, FeedError> {
    let value = &envelope.payload;
    if value["activeFilter"].as_str() != Some("live") {
        return Err(FeedError::WrongFilter);
    }
    if value["sportId"].as_i64() != Some(sport_id as i64) {
        return Err(FeedError::WrongSport);
    }
    let matches = value["matches"]
        .as_array()
        .ok_or_else(|| FeedError::SourceChanged("matches must be an array".into()))?;
    matches
        .iter()
        .map(|item| {
            let id = item["id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| FeedError::SourceChanged("match identity missing".into()))?
                .to_string();
            let status = item["statusId"]
                .as_i64()
                .or_else(|| item["status_id"].as_i64())
                .ok_or_else(|| FeedError::SourceChanged("match status missing".into()))?
                as i32;
            if item["competition"]["id"].as_str().is_none()
                || item["homeTeam"]["id"].as_str().is_none()
                || item["awayTeam"]["id"].as_str().is_none()
            {
                return Err(FeedError::SourceChanged("match relation missing".into()));
            }
            Ok(NormalizedMatch {
                match_id: id,
                status_id: status,
                home_score: first_value(item, &["homeScore", "home_score", "homeScores"]),
                away_score: first_value(item, &["awayScore", "away_score", "awayScores"]),
                period: first_string(item, &["period", "periodName", "period_name"]),
                clock: first_string(item, &["clock", "matchClock", "match_clock"]),
                odds: first_value(item, &["odds", "markets", "oddsData"]),
                is_live: live_status(sport_id, status),
                source_timestamp: value["timestamp"].as_i64().map(|v| v.to_string()),
                payload: item.clone(),
            })
        })
        .collect()
}

fn first_value(item: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| item.get(*key).filter(|value| !value.is_null()).cloned())
        .unwrap_or(Value::Null)
}

fn first_string(item: &Value, keys: &[&str]) -> Option<String> {
    first_value(item, keys).as_str().map(ToOwned::to_owned)
}

fn live_status(sport_id: i32, status_id: i32) -> bool {
    match sport_id {
        1 => (2..=7).contains(&status_id),
        2 => (2..=7).contains(&status_id) || status_id == 9,
        _ => false,
    }
}
