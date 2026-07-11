use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailSection {
    Overview,
    Odds,
    H2H,
    Lineups,
    Stats,
    Incidents,
}

impl DetailSection {
    pub fn as_str(&self) -> &'static str {
        match self {
            DetailSection::Overview => "overview",
            DetailSection::Odds => "odds",
            DetailSection::H2H => "h2h",
            DetailSection::Lineups => "lineups",
            DetailSection::Stats => "stats",
            DetailSection::Incidents => "incidents",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "overview" => Some(DetailSection::Overview),
            "odds" => Some(DetailSection::Odds),
            "h2h" => Some(DetailSection::H2H),
            "lineups" => Some(DetailSection::Lineups),
            "stats" => Some(DetailSection::Stats),
            "incidents" => Some(DetailSection::Incidents),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Overview,
            Self::Odds,
            Self::H2H,
            Self::Lineups,
            Self::Stats,
            Self::Incidents,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadPhase {
    Initial,
    NonInitial,
}

impl LoadPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoadPhase::Initial => "INITIAL",
            LoadPhase::NonInitial => "NON_INITIAL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "LOADING")]
    Loading,
    #[serde(rename = "COMPLETED")]
    Completed,
    #[serde(rename = "FAILED_RETRYABLE")]
    FailedRetryable,
    #[serde(rename = "FAILED_PERMANENT")]
    FailedPermanent,
    #[serde(rename = "EMPTY_CONFIRMED")]
    EmptyConfirmed,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "PENDING",
            JobStatus::Loading => "LOADING",
            JobStatus::Completed => "COMPLETED",
            JobStatus::FailedRetryable => "FAILED_RETRYABLE",
            JobStatus::FailedPermanent => "FAILED_PERMANENT",
            JobStatus::EmptyConfirmed => "EMPTY_CONFIRMED",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetailJob {
    pub id: i64,
    pub match_id: String,
    pub dataset_id: String,
    pub section_name: String,
    pub load_phase: String,
    pub attempt_count: i32,
}

/// An image URL candidate extracted from the detail payload.
/// Note that it explicitly DOES NOT implement serde::Serialize to prevent
/// database, jobs, outbox, or log serialization.
#[derive(Debug, Clone)]
pub struct ImageCandidate {
    pub url: String,
    pub entity_type: String,
    pub entity_id: String,
    pub role: String,
}
