use crate::detail::types::{DetailSection, ImageCandidate};
use serde_json::Value;

pub const DETAIL_DATA_TAB_LABELS: [&str; 7] = [
    "Overview",
    "Odds",
    "Stats",
    "H2H",
    "Lineups",
    "Standings",
    "Prediction",
];

pub fn detail_activate_tabs_js(sport_id: i32, match_id: &str) -> String {
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

pub fn detail_extract_js(sport_id: i32, requested_match_id: &str) -> String {
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

pub fn extract_section_data(
    sport_id: i32,
    section: DetailSection,
    full_payload: &Value,
) -> (Value, Vec<ImageCandidate>) {
    let mut raw_data = match section {
        DetailSection::Overview => {
            let match_id = full_payload["matchId"]
                .as_str()
                .or_else(|| full_payload["match_id"].as_str())
                .unwrap_or("");
            serde_json::json!({
                "matchId": match_id,
                "sportId": sport_id,
                "name": full_payload["name"].as_str().unwrap_or(""),
            })
        }
        DetailSection::Odds => full_payload.get("odds").cloned().unwrap_or_else(|| serde_json::json!({})),
        DetailSection::H2H => full_payload.get("h2h").cloned().unwrap_or_else(|| serde_json::json!({})),
        DetailSection::Lineups => full_payload.get("lineups").cloned().unwrap_or_else(|| serde_json::json!({})),
        DetailSection::Stats => full_payload.get("stats").cloned().unwrap_or_else(|| serde_json::json!({})),
        DetailSection::Incidents => full_payload.get("incidents").cloned().unwrap_or_else(|| serde_json::json!([])),
    };

    let mut candidates = Vec::new();
    sanitize_and_extract_images(&mut raw_data, &mut candidates, section.as_str());

    (raw_data, candidates)
}

pub fn compute_hash(val: &Value) -> String {
    let s = serde_json::to_string(val).unwrap_or_default();
    format!("{:x}", md5::compute(s))
}

pub fn is_section_empty(section: DetailSection, val: &Value) -> bool {
    match val {
        Value::Null => true,
        Value::Array(arr) => arr.is_empty(),
        Value::Object(map) => {
            if map.is_empty() {
                return true;
            }
            match section {
                DetailSection::Lineups => {
                    let home_empty = map.get("home").map(|v| v.as_array().map_or(true, |a| a.is_empty())).unwrap_or(true);
                    let away_empty = map.get("away").map(|v| v.as_array().map_or(true, |a| a.is_empty())).unwrap_or(true);
                    home_empty && away_empty
                }
                DetailSection::H2H => {
                    let history_empty = map.get("history").map(|v| v.as_array().map_or(true, |a| a.is_empty())).unwrap_or(true);
                    history_empty
                }
                _ => false,
            }
        }
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

fn sanitize_and_extract_images(val: &mut Value, candidates: &mut Vec<ImageCandidate>, section_name: &str) {
    match val {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(v) = map.get_mut(&key) {
                    if let Some(s) = v.as_str() {
                        if is_image_url(s) {
                            let hash = format!("{:x}", md5::compute(s));
                            let asset_id = format!("asset-{}", hash);
                            candidates.push(ImageCandidate {
                                url: s.to_string(),
                                entity_type: section_name.to_string(),
                                entity_id: key.clone(),
                                role: key.clone(),
                            });
                            *v = Value::String(asset_id);
                        }
                    } else {
                        sanitize_and_extract_images(v, candidates, section_name);
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                sanitize_and_extract_images(item, candidates, section_name);
            }
        }
        _ => {}
    }
}

fn is_image_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && (s.contains("/logo/")
            || s.contains("/team/")
            || s.contains("/player/")
            || s.contains("/image/")
            || s.ends_with(".png")
            || s.ends_with(".jpg")
            || s.ends_with(".jpeg")
            || s.ends_with(".webp")
            || s.ends_with(".gif"))
}
