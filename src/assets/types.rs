use serde::{Serialize, Deserialize};

/// An in-memory only image candidate representing a source URL and its association.
/// This type does NOT derive Serialize or standard Debug to prevent URLs leaking into logs or database.
#[derive(Clone)]
pub struct AssetCandidate {
    pub url: String,
    pub dataset_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub role: String,
}

impl std::fmt::Debug for AssetCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetCandidate")
            .field("url", &"[REDACTED]")
            .field("dataset_id", &self.dataset_id)
            .field("entity_type", &self.entity_type)
            .field("entity_id", &self.entity_id)
            .field("role", &self.role)
            .finish()
    }
}

/// Metadata stored in the local SQLite database for ready assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    pub asset_id: String,
    pub content_hash: String,
    pub storage_key: String,
    pub mime_type: Option<String>,
    pub byte_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub provenance: String,
}
