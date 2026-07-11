//! Source-neutral domain types used by the live feed and persistence layers.

pub mod events;
pub mod match_state;
pub mod odds;

pub use events::{EventType, NormalizedEvent, canonical_json, payload_hash};
pub use match_state::{AdmissionEvidence, AdmissionResult, InternalState, MatchSnapshot};
pub use odds::{OddsIdentity, OddsQuote, normalize_decimal};
