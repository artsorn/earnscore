use super::{FeedAdapter, FeedError, NormalizedMatch, SourceEnvelope};
pub struct FootballAdapter;
impl FeedAdapter for FootballAdapter {
    fn sport_id(&self) -> i32 {
        1
    }
    fn name(&self) -> &'static str {
        "football"
    }
    fn target_url(&self) -> &'static str {
        "https://m.aiscore.com/"
    }
    fn extract(&self, e: &SourceEnvelope) -> Result<Vec<NormalizedMatch>, FeedError> {
        super::extract_matches(e, 1)
    }
}
