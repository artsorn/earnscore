use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Events emitted by a feed adapter.  The names deliberately do not contain
/// source-specific status codes or payload shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    FeedConnected,
    FeedDisconnected,
    FeedHeartbeat,
    MatchDiscoveredLive,
    MatchScoreChanged,
    MatchClockChanged,
    MatchPeriodChanged,
    MatchStatusChanged,
    MatchOddsChanged,
    MatchRemovedFromLive,
    MatchFinished,
}

impl EventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FeedConnected => "FEED_CONNECTED",
            Self::FeedDisconnected => "FEED_DISCONNECTED",
            Self::FeedHeartbeat => "FEED_HEARTBEAT",
            Self::MatchDiscoveredLive => "MATCH_DISCOVERED_LIVE",
            Self::MatchScoreChanged => "MATCH_SCORE_CHANGED",
            Self::MatchClockChanged => "MATCH_CLOCK_CHANGED",
            Self::MatchPeriodChanged => "MATCH_PERIOD_CHANGED",
            Self::MatchStatusChanged => "MATCH_STATUS_CHANGED",
            Self::MatchOddsChanged => "MATCH_ODDS_CHANGED",
            Self::MatchRemovedFromLive => "MATCH_REMOVED_FROM_LIVE",
            Self::MatchFinished => "MATCH_FINISHED",
        }
    }
}

/// A normalized event envelope.  `payload` is sanitized before it is hashed
/// and persisted, so incidental source metadata cannot create fake changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub event_id: String,
    pub source_event_id: Option<String>,
    pub match_id: Option<String>,
    pub sport_id: Option<i32>,
    pub event_type: EventType,
    pub source_timestamp: Option<String>,
    pub received_at: String,
    pub payload_hash: String,
    pub payload: Value,
    pub feed_session_id: Option<String>,
    pub dataset_id: Option<String>,
}

impl NormalizedEvent {
    pub fn new(
        event_type: EventType,
        match_id: Option<String>,
        sport_id: Option<i32>,
        source_timestamp: Option<String>,
        received_at: String,
        payload: Value,
    ) -> Self {
        let payload = sanitize_payload(payload);
        let payload_hash = payload_hash(&payload);
        let source_timestamp = source_timestamp.and_then(|value| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        });
        let event_id = format!(
            "evt-{}",
            event_key_digest(&event_key_parts(
                match_id.as_deref(),
                event_type,
                source_timestamp.as_deref(),
                &payload_hash,
            ))
        );
        Self {
            event_id,
            source_event_id: None,
            match_id,
            sport_id,
            event_type,
            source_timestamp,
            received_at,
            payload_hash,
            payload,
            feed_session_id: None,
            dataset_id: None,
        }
    }

    /// Stable idempotency key.  For sources without timestamps, the match id
    /// is the explicit fallback required by the feed contract.
    pub fn event_key(&self) -> String {
        event_key_parts(
            self.match_id.as_deref(),
            self.event_type,
            self.source_timestamp.as_deref(),
            &self.payload_hash,
        )
    }

    pub fn ordering_key(&self) -> EventOrderingKey {
        EventOrderingKey {
            source_timestamp: self.source_timestamp.clone(),
            received_at: self.received_at.clone(),
            event_id: self.event_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventOrderingKey {
    pub source_timestamp: Option<String>,
    pub received_at: String,
    pub event_id: String,
}

fn event_key_parts(
    match_id: Option<&str>,
    event_type: EventType,
    source_timestamp: Option<&str>,
    payload_hash: &str,
) -> String {
    let match_key = match_id.unwrap_or("");
    let time_key = source_timestamp
        .filter(|value| !value.is_empty())
        .unwrap_or(match_key);
    format!(
        "{match_key}|{}|{time_key}|{payload_hash}",
        event_type.as_str()
    )
}

/// Recursively sorts object keys and removes chat-like fields from a payload.
pub fn sanitize_payload(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sorted = Map::new();
            for (key, value) in object {
                let lower = key.to_ascii_lowercase();
                if lower == "chat"
                    || lower == "message"
                    || lower == "messages"
                    || lower == "comment"
                    || lower == "comments"
                    || lower.contains("chat")
                    || lower.contains("message")
                    || lower.contains("comment")
                {
                    continue;
                }
                sorted.insert(key, sanitize_payload(value));
            }
            let mut ordered = Map::new();
            for (key, value) in sorted.into_iter() {
                ordered.insert(key, value);
            }
            Value::Object(ordered)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize_payload).collect()),
        other => other,
    }
}

/// JSON serialization of `serde_json::Map` is ordered by insertion here;
/// build a sorted representation first to make this canonical across input
/// key orderings.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let fields = keys
                .into_iter()
                .map(|key| format!("{}:{}", quote_json(key), canonical_json(&object[key])))
                .collect::<Vec<_>>();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::String(value) => quote_json(value),
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
    }
}

fn quote_json(value: &str) -> String {
    serde_json::to_string(value).expect("JSON strings are serializable")
}

/// A small, dependency-free stable digest.  It is used as an identity hash,
/// not as a security primitive.
pub fn payload_hash(value: &Value) -> String {
    event_key_digest(&canonical_json(value))
}

pub fn event_key_digest(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_key_is_stable_when_object_keys_are_reordered() {
        let a = NormalizedEvent::new(
            EventType::MatchScoreChanged,
            Some("m1".into()),
            Some(1),
            Some("2026-01-01T00:00:00Z".into()),
            "received".into(),
            json!({"b": 2, "a": 1, "chat": "discard"}),
        );
        let b = NormalizedEvent::new(
            EventType::MatchScoreChanged,
            Some("m1".into()),
            Some(1),
            Some("2026-01-01T00:00:00Z".into()),
            "received".into(),
            json!({"a": 1, "b": 2}),
        );
        assert_eq!(a.payload_hash, b.payload_hash);
        assert_eq!(a.event_key(), b.event_key());
    }

    #[test]
    fn missing_timestamp_uses_match_id_fallback() {
        let event = NormalizedEvent::new(
            EventType::MatchDiscoveredLive,
            Some("m1".into()),
            Some(1),
            None,
            "received".into(),
            json!({"live": true}),
        );
        assert!(event.event_key().contains("|m1|"));
    }
}
