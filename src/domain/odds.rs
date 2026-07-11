use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OddsIdentity {
    pub match_id: String,
    pub bookmaker_id: String,
    pub market_type: String,
    pub period: String,
    pub selection_key: String,
    pub line_value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OddsQuote {
    pub identity: OddsIdentity,
    pub odds_value: String,
    pub is_live: bool,
    pub source_timestamp: Option<String>,
    pub received_at: String,
    pub payload_hash: String,
    pub payload: Value,
}

impl OddsQuote {
    pub fn new(
        match_id: impl Into<String>,
        bookmaker_id: impl Into<String>,
        market_type: impl Into<String>,
        period: impl Into<String>,
        selection_key: impl Into<String>,
        line_value: Option<&str>,
        odds_value: &str,
        is_live: bool,
        source_timestamp: Option<String>,
        received_at: String,
        payload_hash: String,
        payload: Value,
    ) -> Self {
        Self {
            identity: OddsIdentity {
                match_id: match_id.into().trim().to_string(),
                bookmaker_id: bookmaker_id.into().trim().to_string(),
                market_type: market_type.into().trim().to_string(),
                period: period.into().trim().to_string(),
                selection_key: selection_key.into().trim().to_string(),
                line_value: normalize_decimal(line_value.unwrap_or("")),
            },
            odds_value: normalize_decimal(odds_value),
            is_live,
            source_timestamp: source_timestamp.and_then(|value| {
                let value = value.trim().to_string();
                (!value.is_empty()).then_some(value)
            }),
            received_at,
            payload_hash,
            payload,
        }
    }

    pub fn value_changed(&self, previous: Option<&str>) -> bool {
        previous.is_none_or(|value| normalize_decimal(value) != self.odds_value)
    }

    pub fn event_key(&self) -> String {
        let time_key = self
            .source_timestamp
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.identity.match_id);
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.identity.match_id,
            self.identity.bookmaker_id,
            self.identity.market_type,
            self.identity.period,
            self.identity.selection_key,
            self.identity.line_value,
            time_key,
            self.odds_value,
            self.payload_hash,
        )
    }
}

/// Canonical decimal text without floating-point rounding. Empty and invalid
/// values remain empty so malformed source quotes cannot become zero odds.
pub fn normalize_decimal(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let (sign, digits) = match value.as_bytes()[0] {
        b'+' => ("", &value[1..]),
        b'-' => ("-", &value[1..]),
        _ => ("", value),
    };
    let mut parts = digits.split('.');
    let integer = parts.next().unwrap_or("");
    let fraction = parts.next();
    if parts.next().is_some() || integer.is_empty() && fraction.is_none() {
        return String::new();
    }
    if !integer.chars().all(|c| c.is_ascii_digit())
        || fraction.is_some_and(|part| !part.chars().all(|c| c.is_ascii_digit()))
    {
        return String::new();
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.unwrap_or("").trim_end_matches('0');
    if fraction.is_empty() {
        if sign == "-" && integer != "0" {
            format!("-{integer}")
        } else {
            integer.to_string()
        }
    } else {
        format!("{sign}{integer}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decimal_normalization_avoids_false_changes() {
        assert_eq!(normalize_decimal("+01.5000"), "1.5");
        assert_eq!(normalize_decimal("1.5"), "1.5");
        assert_eq!(normalize_decimal("bad"), "");
    }

    #[test]
    fn identity_does_not_depend_on_bookmaker_names() {
        let quote = OddsQuote::new(
            "m",
            "any-bookmaker",
            "moneyline",
            "full",
            "home",
            None,
            "1.90",
            true,
            None,
            "r".into(),
            "h".into(),
            json!({}),
        );
        assert!(quote.event_key().contains("any-bookmaker"));
    }
}
