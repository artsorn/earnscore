use super::events::{EventType, NormalizedEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InternalState {
    DiscoveredLive,
    Live,
    HalfTime,
    Paused,
    Finishing,
    Finished,
    Cancelled,
    Postponed,
    Abandoned,
    RecoveryPending,
    Finalized,
    Unknown,
}

impl InternalState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiscoveredLive => "DISCOVERED_LIVE",
            Self::Live => "LIVE",
            Self::HalfTime => "HALF_TIME",
            Self::Paused => "PAUSED",
            Self::Finishing => "FINISHING",
            Self::Finished => "FINISHED",
            Self::Cancelled => "CANCELLED",
            Self::Postponed => "POSTPONED",
            Self::Abandoned => "ABANDONED",
            Self::RecoveryPending => "RECOVERY_PENDING",
            Self::Finalized => "FINALIZED",
            Self::Unknown => "UNKNOWN",
        }
    }

    /// Read plain SQLite text, while accepting the quoted legacy form.
    pub fn from_storage(value: &str) -> Option<Self> {
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        match value.to_ascii_uppercase().as_str() {
            "DISCOVERED_LIVE" => Some(Self::DiscoveredLive),
            "LIVE" => Some(Self::Live),
            "HALF_TIME" | "HALFTIME" => Some(Self::HalfTime),
            "PAUSED" => Some(Self::Paused),
            "FINISHING" => Some(Self::Finishing),
            "FINISHED" => Some(Self::Finished),
            "CANCELLED" | "CANCELED" => Some(Self::Cancelled),
            "POSTPONED" => Some(Self::Postponed),
            "ABANDONED" => Some(Self::Abandoned),
            "RECOVERY_PENDING" => Some(Self::RecoveryPending),
            "FINALIZED" => Some(Self::Finalized),
            "UNKNOWN" => Some(Self::Unknown),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Finished | Self::Cancelled | Self::Postponed | Self::Abandoned | Self::Finalized
        )
    }

    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::DiscoveredLive | Self::Live | Self::HalfTime | Self::Paused | Self::Finishing
        )
    }

    fn rank(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::DiscoveredLive => 1,
            Self::Live => 2,
            Self::HalfTime => 3,
            Self::Paused => 4,
            Self::Finishing => 5,
            Self::Finished | Self::Cancelled | Self::Postponed | Self::Abandoned => 6,
            Self::RecoveryPending => 7,
            Self::Finalized => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSnapshot {
    pub match_id: String,
    pub sport_id: i32,
    pub competition_id: String,
    pub home_team_id: String,
    pub away_team_id: String,
    pub match_time: i64,
    pub status_id: Option<i32>,
    pub state: InternalState,
    pub home_scores: Value,
    pub away_scores: Value,
    pub period: Option<String>,
    pub clock: Option<String>,
    pub source_timestamp: Option<String>,
    pub received_at: String,
    pub payload_hash: String,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionEvidence {
    pub source_status: Option<String>,
    pub started: bool,
    pub clock: Option<String>,
    pub period: Option<String>,
    pub home_score: Option<i64>,
    pub away_score: Option<i64>,
}

impl AdmissionEvidence {
    pub fn live_status(status: impl Into<String>) -> Self {
        Self {
            source_status: Some(status.into()),
            ..Self::default()
        }
    }

    pub fn has_started(&self) -> bool {
        self.started
            || self
                .clock
                .as_deref()
                .is_some_and(|clock| !clock.trim().is_empty())
            || self
                .period
                .as_deref()
                .is_some_and(|period| !period.trim().is_empty())
            || self.home_score.is_some()
            || self.away_score.is_some()
    }

    pub fn is_live(&self) -> bool {
        self.source_status.as_deref().is_some_and(is_live_status) || self.has_started()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionResult {
    Admit,
    RejectScheduled,
    RejectNotStarted,
    RejectTerminalWithoutLiveHistory,
    RejectTerminalImmutable,
}

pub fn admission_result(evidence: &AdmissionEvidence, existing_live: bool) -> AdmissionResult {
    let status = evidence.source_status.as_deref().unwrap_or("");
    if is_scheduled_status(status) && !evidence.has_started() {
        return AdmissionResult::RejectScheduled;
    }
    if is_terminal_status(status) && !existing_live {
        return AdmissionResult::RejectTerminalWithoutLiveHistory;
    }
    // A terminal row is admissible only as the continuation of a match that
    // was already admitted while live.  Terminal rows do not themselves have
    // live evidence, so checking `is_live()` here would reject the valid
    // recovery/final update.
    if is_terminal_status(status) && existing_live {
        return AdmissionResult::Admit;
    }
    if evidence.is_live() {
        return AdmissionResult::Admit;
    }
    AdmissionResult::RejectNotStarted
}

pub fn next_state(
    current: Option<InternalState>,
    event_type: EventType,
    source_status: Option<&str>,
) -> InternalState {
    let candidate = if let Some(status) = source_status.and_then(state_from_status) {
        status
    } else {
        match event_type {
            EventType::MatchDiscoveredLive => InternalState::DiscoveredLive,
            EventType::MatchFinished => InternalState::Finished,
            EventType::MatchRemovedFromLive => InternalState::Finishing,
            EventType::MatchStatusChanged => current.unwrap_or(InternalState::Unknown),
            _ => match current {
                Some(state) if state.is_live() => state,
                Some(state) => state,
                None => InternalState::Live,
            },
        }
    };
    // A late status label must not move the canonical state backwards.  The
    // event is still retained in the append-only history by the repository.
    if is_monotonic(current, candidate) {
        candidate
    } else {
        current.unwrap_or(candidate)
    }
}

/// Returns whether applying `candidate` is allowed. Terminal state is
/// immutable and no event may move a snapshot backwards in its state order.
pub fn is_monotonic(current: Option<InternalState>, candidate: InternalState) -> bool {
    match current {
        None => true,
        Some(existing) if existing == candidate => true,
        Some(existing) if existing == InternalState::Finalized => false,
        Some(existing) if existing.is_terminal() => false,
        Some(existing) => candidate.rank() >= existing.rank(),
    }
}

pub fn compare_event_order(
    current_source_timestamp: Option<&str>,
    current_received_at: Option<&str>,
    event: &NormalizedEvent,
) -> Ordering {
    // Source timestamps are the primary clock when both events have one.
    // For timestamp-less sources, received_at is the deterministic fallback.
    // A missing timestamp must not automatically make an event stale.
    let event_time = event
        .source_timestamp
        .as_deref()
        .filter(|value| !value.is_empty());
    let current_time = current_source_timestamp.filter(|value| !value.is_empty());
    compare_order_parts(
        event_time,
        Some(event.received_at.as_str()),
        Some(event.payload_hash.as_str()),
        current_time,
        current_received_at,
        None,
    )
}

/// Compares an incoming event with an already persisted history row. Source
/// time is authoritative only when both values exist; otherwise receipt time
/// is used. Payload hashes make equal-time events deterministic.
pub fn compare_order_parts(
    incoming_source_timestamp: Option<&str>,
    incoming_received_at: Option<&str>,
    incoming_payload_hash: Option<&str>,
    current_source_timestamp: Option<&str>,
    current_received_at: Option<&str>,
    current_payload_hash: Option<&str>,
) -> Ordering {
    let incoming_source_timestamp = incoming_source_timestamp.filter(|value| !value.is_empty());
    let current_source_timestamp = current_source_timestamp.filter(|value| !value.is_empty());
    let ordering = match (incoming_source_timestamp, current_source_timestamp) {
        (Some(incoming), Some(current)) => incoming.cmp(current),
        _ => incoming_received_at
            .unwrap_or("")
            .cmp(current_received_at.unwrap_or("")),
    };
    if ordering != Ordering::Equal {
        ordering
    } else {
        incoming_payload_hash
            .unwrap_or("")
            .cmp(current_payload_hash.unwrap_or(""))
    }
}

fn is_live_status(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    [
        "live",
        "in play",
        "playing",
        "started",
        "halftime",
        "half-time",
        "paused",
    ]
    .iter()
    .any(|candidate| status == *candidate)
}

fn is_scheduled_status(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    ["scheduled", "upcoming", "not started", "ns"]
        .iter()
        .any(|candidate| status == *candidate)
}

fn is_terminal_status(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    [
        "finished",
        "full time",
        "ft",
        "cancelled",
        "canceled",
        "postponed",
        "abandoned",
    ]
    .iter()
    .any(|candidate| status == *candidate)
}

fn state_from_status(status: &str) -> Option<InternalState> {
    let normalized = status.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    Some(match normalized.as_str() {
        "live" | "in play" | "playing" | "started" => InternalState::Live,
        "halftime" | "half time" => InternalState::HalfTime,
        "paused" => InternalState::Paused,
        "finishing" => InternalState::Finishing,
        "finished" | "full time" | "ft" => InternalState::Finished,
        "cancelled" | "canceled" => InternalState::Cancelled,
        "postponed" => InternalState::Postponed,
        "abandoned" => InternalState::Abandoned,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_rejects_scheduled_and_unknown_terminal_rows() {
        assert_eq!(
            admission_result(&AdmissionEvidence::live_status("Scheduled"), false),
            AdmissionResult::RejectScheduled
        );
        assert_eq!(
            admission_result(&AdmissionEvidence::live_status("Finished"), false),
            AdmissionResult::RejectTerminalWithoutLiveHistory
        );
    }

    #[test]
    fn score_is_sufficient_started_evidence() {
        let evidence = AdmissionEvidence {
            home_score: Some(0),
            ..Default::default()
        };
        assert_eq!(admission_result(&evidence, false), AdmissionResult::Admit);
    }

    #[test]
    fn terminal_state_cannot_be_regressed() {
        assert!(!is_monotonic(
            Some(InternalState::Finished),
            InternalState::Live
        ));
        assert!(is_monotonic(
            Some(InternalState::Live),
            InternalState::Finished
        ));
    }

    #[test]
    fn event_order_compares_incoming_event_against_current_row() {
        let event = NormalizedEvent::new(
            EventType::MatchScoreChanged,
            Some("m1".into()),
            Some(1),
            Some("2026-01-01T00:02:00Z".into()),
            "2026-01-01T00:02:01Z".into(),
            serde_json::json!({"score": 2}),
        );
        assert_eq!(
            compare_event_order(
                Some("2026-01-01T00:01:00Z"),
                Some("2026-01-01T00:01:01Z"),
                &event,
            ),
            Ordering::Greater
        );
    }
}
