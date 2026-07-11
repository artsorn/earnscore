use super::{FeedAdapter, FeedError, NormalizedMatch, SourceEnvelope};
pub struct BasketballAdapter;
impl FeedAdapter for BasketballAdapter {
    fn sport_id(&self) -> i32 {
        2
    }
    fn name(&self) -> &'static str {
        "basketball"
    }
    fn target_url(&self) -> &'static str {
        "https://m.aiscore.com/basketball"
    }
    fn extract(&self, e: &SourceEnvelope) -> Result<Vec<NormalizedMatch>, FeedError> {
        super::extract_matches(e, 2)
    }
}
