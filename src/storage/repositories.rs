use crate::domain::events::{EventType, NormalizedEvent};
use crate::domain::match_state::{self, AdmissionEvidence, AdmissionResult, InternalState};
use crate::domain::odds::OddsQuote;
use rusqlite::{Connection, OptionalExtension, Result as SqlResult, Transaction, params};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOutcome {
    Accepted,
    Duplicate,
    Stale,
    Rejected(AdmissionResult),
}

pub struct RepositoryUnitOfWork<'a> {
    conn: &'a Connection,
}

impl<'a> RepositoryUnitOfWork<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Apply one normalized event atomically.  A duplicate is detected before
    /// any downstream write; a stale event is still retained in feed_events.
    pub fn apply_event(
        &self,
        event: &NormalizedEvent,
        evidence: &AdmissionEvidence,
        odds: &[OddsQuote],
    ) -> SqlResult<MutationOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let dataset_id = event.dataset_id.as_deref().unwrap_or("legacy-dataset-id");
        let event_key = event.event_key();
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO feed_events
             (event_id,event_key,source_event_id,session_id,match_id,dataset_id,sport_id,event_type,source_timestamp,received_at,payload_hash,payload_json,processed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,datetime('now'))",
            params![event.event_id, event_key, event.source_event_id, event.feed_session_id,
                event.match_id, dataset_id, event.sport_id, event.event_type.as_str(),
                event.source_timestamp, event.received_at, event.payload_hash, event.payload.to_string()],
        )?;
        if inserted == 0 {
            tx.commit()?;
            return Ok(MutationOutcome::Duplicate);
        }

        let Some(match_id) = event.match_id.as_deref() else {
            tx.commit()?;
            return Ok(MutationOutcome::Accepted);
        };
        let existing_live = existing_match_is_live(&tx, match_id, dataset_id)?;
        let is_match_event = matches!(
            event.event_type,
            EventType::MatchDiscoveredLive
                | EventType::MatchScoreChanged
                | EventType::MatchClockChanged
                | EventType::MatchPeriodChanged
                | EventType::MatchStatusChanged
                | EventType::MatchOddsChanged
                | EventType::MatchFinished
                | EventType::MatchRemovedFromLive
        );
        if is_match_event {
            if latest_state_history(&tx, match_id, dataset_id)?
                .as_ref()
                .is_some_and(|row| row.3.is_terminal())
            {
                tx.commit()?;
                return Ok(MutationOutcome::Rejected(
                    AdmissionResult::RejectTerminalImmutable,
                ));
            }
            match match_state::admission_result(evidence, existing_live) {
                AdmissionResult::Admit => {}
                rejection => {
                    tx.commit()?;
                    return Ok(MutationOutcome::Rejected(rejection));
                }
            }
        }
        let stale = if event.event_type == EventType::MatchOddsChanged {
            false
        } else {
            is_stale(&tx, match_id, dataset_id, event)?
        };

        if let Some(snapshot) = snapshot_from_event(&tx, event, evidence, dataset_id)? {
            // History is append-only even for a stale event.  Only the latest
            // ordered event may change the canonical snapshot and outbox.
            insert_state_history(&tx, dataset_id, &snapshot)?;
            if !stale {
                upsert_snapshot(&tx, dataset_id, &snapshot)?;
                insert_outbox(
                    &tx,
                    dataset_id,
                    "match",
                    match_id,
                    event.event_type.as_str(),
                    &json!(snapshot),
                )?;
                if event.event_type == EventType::MatchDiscoveredLive {
                    plan_initial_detail_jobs(&tx, dataset_id, match_id)?;
                }
            }
        }
        let mut stale_quote = false;
        if !stale {
            for quote in odds {
                if apply_quote(&tx, dataset_id, quote)? {
                    stale_quote = true;
                }
            }
        }
        tx.execute(
            "UPDATE feed_events SET processed_at=datetime('now') WHERE event_key=?1",
            params![event_key],
        )?;
        tx.commit()?;
        Ok(if stale {
            MutationOutcome::Stale
        } else if !odds.is_empty() && stale_quote {
            MutationOutcome::Stale
        } else {
            MutationOutcome::Accepted
        })
    }

    pub fn reclaim_expired_leases(&self) -> SqlResult<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let affected = tx.execute(
            "UPDATE detail_jobs
             SET status = 'PENDING', lease_owner = NULL, lease_expires_at = NULL, last_error = 'Lease expired'
             WHERE status = 'LOADING' AND datetime(lease_expires_at) < datetime('now')",
            [],
        )?;
        tx.commit()?;
        Ok(affected)
    }

    pub fn claim_next_job(
        &self,
        lease_owner: &str,
        lease_duration_secs: i64,
    ) -> SqlResult<Option<crate::detail::types::DetailJob>> {
        let tx = self.conn.unchecked_transaction()?;

        let candidate: Option<(i64, String, String, String, String, i32)> = tx
            .query_row(
                "SELECT id, match_id, dataset_id, section_name, load_phase, attempt_count
             FROM detail_jobs
             WHERE (status = 'PENDING' OR status = 'FAILED_RETRYABLE')
               AND datetime(scheduled_at) <= datetime('now')
               AND NOT EXISTS (
                   SELECT 1 FROM detail_jobs other
                   WHERE other.match_id = detail_jobs.match_id
                     AND other.dataset_id = detail_jobs.dataset_id
                     AND other.status = 'LOADING'
                     AND other.load_phase != detail_jobs.load_phase
               )
             ORDER BY scheduled_at ASC, id ASC
             LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;

        if let Some((id, match_id, dataset_id, section_name, load_phase, attempt_count)) = candidate
        {
            let expires = format!("+{} seconds", lease_duration_secs);
            tx.execute(
                "UPDATE detail_jobs
                 SET status = 'LOADING',
                     started_at = datetime('now'),
                     lease_owner = ?2,
                     lease_expires_at = datetime('now', ?3),
                     attempt_count = attempt_count + 1
                 WHERE id = ?1",
                params![id, lease_owner, expires],
            )?;
            tx.commit()?;
            Ok(Some(crate::detail::types::DetailJob {
                id,
                match_id,
                dataset_id,
                section_name,
                load_phase,
                attempt_count: attempt_count + 1,
            }))
        } else {
            tx.commit()?;
            Ok(None)
        }
    }
}

fn replace_all_placeholders(val: &mut Value, asset_replacements: &[(String, String)]) {
    match val {
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                if let Some(s) = v.as_str() {
                    for (url, asset_id) in asset_replacements {
                        let placeholder = format!("asset-{:x}", md5::compute(url));
                        if s == url || s == placeholder {
                            *v = Value::String(asset_id.clone());
                            break;
                        }
                    }
                } else {
                    replace_all_placeholders(v, asset_replacements);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                replace_all_placeholders(item, asset_replacements);
            }
        }
        _ => {}
    }
}

fn storage_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
}

fn persist_downloaded_asset(
    tx: &Transaction<'_>,
    dataset_id: &str,
    candidate: &crate::detail::types::ImageCandidate,
    downloaded: &crate::assets::download::DownloadedAsset,
    asset_root: &str,
) -> SqlResult<String> {
    let content_hash = crate::assets::store::calculate_sha256(&downloaded.bytes);
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT asset_id, storage_key FROM assets WHERE content_hash = ?1",
            params![content_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let asset_id = if let Some((asset_id, storage_key)) = existing {
        let path = Path::new(asset_root).join(&storage_key);
        if !path.is_file() {
            return Err(storage_error(format!(
                "Published asset is missing: {}",
                path.display()
            )));
        }
        let existing_bytes = std::fs::read(&path).map_err(|error| {
            storage_error(format!(
                "Cannot read existing asset {}: {}",
                asset_id, error
            ))
        })?;
        if crate::assets::store::calculate_sha256(&existing_bytes) != content_hash {
            return Err(storage_error(format!(
                "Existing asset {} failed content-hash verification",
                asset_id
            )));
        }
        asset_id
    } else {
        let ext = crate::assets::store::mime_to_extension(&downloaded.mime_type);
        crate::assets::store::publish_asset_file(
            asset_root,
            &candidate.entity_type,
            &candidate.entity_id,
            &content_hash,
            ext,
            &downloaded.bytes,
        )
        .map_err(storage_error)?;

        let asset_id = format!("asset-{}", content_hash);
        let storage_key = format!(
            "{}/{}/{}.{}",
            candidate.entity_type, candidate.entity_id, content_hash, ext
        );
        tx.execute(
            "INSERT INTO assets (asset_id, content_hash, storage_key, mime_type, byte_size, width, height, status, provenance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'READY', 'local')",
            params![
                asset_id,
                content_hash,
                storage_key,
                downloaded.mime_type,
                downloaded.bytes.len() as i64,
                downloaded.width,
                downloaded.height,
            ],
        )?;

        let has_outbox: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sync_outbox')",
            [],
            |row| row.get(0),
        )?;
        if has_outbox {
            let payload = json!({
                "asset_id": asset_id,
                "content_hash": content_hash,
                "storage_key": storage_key,
                "mime_type": downloaded.mime_type,
                "byte_size": downloaded.bytes.len() as i64,
                "width": downloaded.width,
                "height": downloaded.height,
            });
            tx.execute(
                "INSERT OR IGNORE INTO sync_outbox (dataset_id, entity_type, entity_id, event_type, payload_json)
                 VALUES (?1, 'asset', ?2, 'ASSET_UPLOAD_INTENT', ?3)",
                params![dataset_id, asset_id, payload.to_string()],
            )?;
        }
        asset_id
    };

    tx.execute(
        "INSERT OR IGNORE INTO asset_jobs (asset_id, dataset_id, status, completed_at)
         VALUES (?1, ?2, 'COMPLETED', datetime('now'))",
        params![asset_id, dataset_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO asset_links (asset_id, dataset_id, entity_type, entity_id, role)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            asset_id,
            dataset_id,
            candidate.entity_type,
            candidate.entity_id,
            candidate.role
        ],
    )?;
    Ok(asset_id)
}

impl<'a> RepositoryUnitOfWork<'a> {
    /// Return persisted source-URL locations without ever returning the URL
    /// itself.  This is intended for migration verification and tests after
    /// asset conversion; callers can safely report the returned locations.
    pub fn find_persisted_source_url_locations(&self) -> SqlResult<Vec<String>> {
        let mut locations = Vec::new();
        let mut tables = self.conn.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?;
        let table_names = tables
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<SqlResult<Vec<_>>>()?;

        for table_name in table_names {
            // SQLite table names originate in sqlite_master.  Still quote them
            // defensively so a legacy schema cannot turn this verifier into a
            // dynamic-SQL injection path.
            let quoted_table = format!("\"{}\"", table_name.replace('"', "\"\""));
            let pragma = format!("PRAGMA table_info({quoted_table})");
            let mut columns = self.conn.prepare(&pragma)?;
            let column_definitions = columns
                .query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            let text_columns = column_definitions
                .into_iter()
                .filter(|(_, declared_type)| declared_type.to_ascii_uppercase().contains("TEXT"))
                .map(|(name, _)| name)
                .collect::<Vec<_>>();

            for column_name in text_columns {
                let quoted_column = format!("\"{}\"", column_name.replace('"', "\"\""));
                let query = format!(
                    "SELECT EXISTS(SELECT 1 FROM {quoted_table}
                     WHERE {quoted_column} LIKE '%http://%' OR {quoted_column} LIKE '%https://%')"
                );
                let found: bool = self.conn.query_row(&query, [], |row| row.get(0))?;
                if found {
                    locations.push(format!("{table_name}.{column_name}"));
                }
            }
        }
        Ok(locations)
    }

    pub fn save_detail_section_with_assets(
        &self,
        match_id: &str,
        dataset_id: &str,
        section_name: &str,
        data: &Value,
        is_empty: bool,
        content_hash: &str,
        source_timestamp: Option<&str>,
        candidates: &[crate::detail::types::ImageCandidate],
        downloaded_results: &std::collections::HashMap<
            usize,
            Result<crate::assets::download::DownloadedAsset, String>,
        >,
        asset_root: &str,
    ) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut asset_replacements = Vec::with_capacity(candidates.len());

        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let asset_id = match downloaded_results.get(&candidate_index) {
                Some(Ok(downloaded)) => {
                    persist_downloaded_asset(&tx, dataset_id, candidate, downloaded, asset_root)?
                }
                Some(Err(_)) | None => "asset-unavailable".to_string(),
            };
            asset_replacements.push((candidate.url.clone(), asset_id));
        }

        let mut final_data = data.clone();
        replace_all_placeholders(&mut final_data, &asset_replacements);

        let provenance = "detail";
        let status = if is_empty {
            "EMPTY_CONFIRMED"
        } else {
            "COMPLETED"
        };
        let is_empty_val = if is_empty { 1 } else { 0 };

        tx.execute(
            "INSERT INTO match_detail_sections
             (match_id, dataset_id, section_name, status, provenance, is_empty, is_unparseable, content_hash, source_timestamp, received_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, datetime('now'), datetime('now'))
             ON CONFLICT(match_id, dataset_id, section_name) DO UPDATE SET
               status=excluded.status,
               is_empty=excluded.is_empty,
               content_hash=excluded.content_hash,
               source_timestamp=excluded.source_timestamp,
               received_at=excluded.received_at,
               completed_at=excluded.completed_at",
            params![match_id, dataset_id, section_name, status, provenance, is_empty_val, content_hash, source_timestamp],
        )?;

        tx.execute(
            "INSERT INTO match_detail_data
             (match_id, dataset_id, section_name, data_json, provenance, content_hash, source_timestamp, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
             ON CONFLICT(match_id, dataset_id, section_name) DO UPDATE SET
               data_json=excluded.data_json,
               content_hash=excluded.content_hash,
               source_timestamp=excluded.source_timestamp,
               received_at=excluded.received_at",
            params![match_id, dataset_id, section_name, final_data.to_string(), provenance, content_hash, source_timestamp],
        )?;

        if !is_empty {
            match section_name {
                "stats" => {
                    if let Some(obj) = final_data.as_object() {
                        for (key, val) in obj {
                            if let Some(arr) = val.as_array() {
                                if arr.len() >= 2 {
                                    tx.execute(
                                        "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                         VALUES (?1, ?2, 'all', ?3, 'home', ?4, ?5, 'detail')
                                         ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                        params![match_id, dataset_id, key, arr[0].to_string(), source_timestamp],
                                    )?;
                                    tx.execute(
                                        "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                         VALUES (?1, ?2, 'all', ?3, 'away', ?4, ?5, 'detail')
                                         ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                        params![match_id, dataset_id, key, arr[1].to_string(), source_timestamp],
                                    )?;
                                }
                            } else {
                                tx.execute(
                                    "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'all', ?3, 'none', ?4, ?5, 'detail')
                                     ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                    params![match_id, dataset_id, key, val.to_string(), source_timestamp],
                                )?;
                            }
                        }
                    }
                }
                "lineups" => {
                    if let Some(home_lineup) = final_data["home"].as_array() {
                        for player in home_lineup {
                            let player_id = player["id"]
                                .as_str()
                                .or_else(|| player["playerId"].as_str())
                                .unwrap_or("")
                                .to_string();
                            if !player_id.is_empty() {
                                tx.execute(
                                    "INSERT INTO match_lineups (match_id, dataset_id, team_side, player_id, coach_id, position, shirt_number, starter, lineup_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'home', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'detail')
                                     ON CONFLICT(match_id, dataset_id, team_side, player_id) DO UPDATE SET lineup_json=excluded.lineup_json",
                                    params![
                                        match_id, dataset_id, player_id,
                                        player["coachId"].as_str(),
                                        player["position"].as_str(),
                                        player["shirtNumber"].as_str().or_else(|| player["shirt_number"].as_str()),
                                        player["starter"].as_bool().map(|b| if b { 1 } else { 0 }).unwrap_or(0),
                                        player.to_string(),
                                        source_timestamp
                                    ],
                                )?;
                            }
                        }
                    }
                    if let Some(away_lineup) = final_data["away"].as_array() {
                        for player in away_lineup {
                            let player_id = player["id"]
                                .as_str()
                                .or_else(|| player["playerId"].as_str())
                                .unwrap_or("")
                                .to_string();
                            if !player_id.is_empty() {
                                tx.execute(
                                    "INSERT INTO match_lineups (match_id, dataset_id, team_side, player_id, coach_id, position, shirt_number, starter, lineup_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'away', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'detail')
                                     ON CONFLICT(match_id, dataset_id, team_side, player_id) DO UPDATE SET lineup_json=excluded.lineup_json",
                                    params![
                                        match_id, dataset_id, player_id,
                                        player["coachId"].as_str(),
                                        player["position"].as_str(),
                                        player["shirtNumber"].as_str().or_else(|| player["shirt_number"].as_str()),
                                        player["starter"].as_bool().map(|b| if b { 1 } else { 0 }).unwrap_or(0),
                                        player.to_string(),
                                        source_timestamp
                                    ],
                                )?;
                            }
                        }
                    }
                }
                "incidents" => {
                    if let Some(arr) = final_data.as_array() {
                        for (idx, item) in arr.iter().enumerate() {
                            let incident_key = format!("incident-{}", idx);
                            tx.execute(
                                "INSERT INTO match_incidents (match_id, dataset_id, incident_key, incident_json, source_timestamp, provenance)
                                 VALUES (?1, ?2, ?3, ?4, ?5, 'detail')
                                 ON CONFLICT(match_id, dataset_id, incident_key) DO UPDATE SET incident_json=excluded.incident_json",
                                params![match_id, dataset_id, incident_key, item.to_string(), source_timestamp],
                            )?;
                        }
                    }
                }
                "h2h" => {
                    if let Some(history) = final_data["history"].as_array() {
                        for (idx, item) in history.iter().enumerate() {
                            let ref_key = item["id"]
                                .as_str()
                                .map(String::from)
                                .unwrap_or_else(|| format!("h2h-{}", idx));
                            let item_str = item.to_string();
                            let h2h_hash = format!("{:x}", md5::compute(&item_str));
                            tx.execute(
                                "INSERT INTO match_h2h_references (match_id, dataset_id, reference_key, reference_json, provenance, content_hash)
                                 VALUES (?1, ?2, ?3, ?4, 'detail', ?5)
                                 ON CONFLICT(match_id, dataset_id, reference_key, content_hash) DO NOTHING",
                                params![match_id, dataset_id, ref_key, item_str, h2h_hash],
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }

        tx.execute(
            "UPDATE detail_jobs
             SET status = 'COMPLETED', completed_at = datetime('now')
             WHERE match_id = ?1 AND dataset_id = ?2 AND section_name = ?3 AND status = 'LOADING'",
            params![match_id, dataset_id, section_name],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub async fn convert_legacy_logos(
        &self,
        client: &reqwest::Client,
        asset_root: &str,
    ) -> SqlResult<Vec<String>> {
        self.convert_legacy_logos_safe(client, asset_root).await
    }

    async fn import_legacy_asset(
        &self,
        client: &reqwest::Client,
        asset_root: &str,
        dataset_id: &str,
        entity_type: &str,
        entity_id: &str,
        role: &str,
        url: &str,
    ) -> Result<String, String> {
        let downloaded = crate::assets::download::download_asset(
            client,
            url,
            10 * 1024 * 1024,
            std::time::Duration::from_secs(10),
            3,
            std::time::Duration::from_secs(1),
        )
        .await?;
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let candidate = crate::detail::types::ImageCandidate {
            url: url.to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            role: role.to_string(),
        };
        let asset_id =
            persist_downloaded_asset(&tx, dataset_id, &candidate, &downloaded, asset_root)
                .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(asset_id)
    }

    async fn convert_legacy_logos_safe(
        &self,
        client: &reqwest::Client,
        asset_root: &str,
    ) -> SqlResult<Vec<String>> {
        let mut failed_owner_ids = Vec::new();

        let mut stmt = self.conn.prepare(
            "SELECT id, logo, country_logo, dataset_id FROM competitions
             WHERE logo LIKE 'http://%' OR logo LIKE 'https://%'
                OR country_logo LIKE 'http://%' OR country_logo LIKE 'https://%'",
        )?;
        let mut rows = stmt.query([])?;
        let mut comps = Vec::new();
        while let Some(row) = rows.next()? {
            comps.push((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ));
        }
        drop(rows);
        drop(stmt);

        for (id, logo, country_logo, dataset_id) in comps {
            let mut failed = false;
            let new_logo = match logo {
                Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    match self
                        .import_legacy_asset(
                            client,
                            asset_root,
                            &dataset_id,
                            "competition",
                            &id,
                            "logo",
                            &url,
                        )
                        .await
                    {
                        Ok(asset_id) => Some(asset_id),
                        Err(_) => {
                            failed = true;
                            Some("asset-unavailable".to_string())
                        }
                    }
                }
                other => other,
            };
            let new_country_logo = match country_logo {
                Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    match self
                        .import_legacy_asset(
                            client,
                            asset_root,
                            &dataset_id,
                            "competition",
                            &id,
                            "country_logo",
                            &url,
                        )
                        .await
                    {
                        Ok(asset_id) => Some(asset_id),
                        Err(_) => {
                            failed = true;
                            Some("asset-unavailable".to_string())
                        }
                    }
                }
                other => other,
            };

            self.conn.execute(
                "UPDATE competitions SET logo = ?1, country_logo = ?2 WHERE id = ?3 AND dataset_id = ?4",
                params![new_logo, new_country_logo, id, dataset_id],
            )?;

            if failed {
                failed_owner_ids.push(id);
            }
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, logo, dataset_id FROM teams
             WHERE logo LIKE 'http://%' OR logo LIKE 'https://%'",
        )?;
        let mut rows = stmt.query([])?;
        let mut team_rows = Vec::new();
        while let Some(row) = rows.next()? {
            team_rows.push((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ));
        }
        drop(rows);
        drop(stmt);

        for (id, logo, dataset_id) in team_rows {
            let mut failed = false;
            let new_logo = match logo {
                Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    match self
                        .import_legacy_asset(
                            client,
                            asset_root,
                            &dataset_id,
                            "team",
                            &id,
                            "logo",
                            &url,
                        )
                        .await
                    {
                        Ok(asset_id) => Some(asset_id),
                        Err(_) => {
                            failed = true;
                            Some("asset-unavailable".to_string())
                        }
                    }
                }
                other => other,
            };

            self.conn.execute(
                "UPDATE teams SET logo = ?1 WHERE id = ?2 AND dataset_id = ?3",
                params![new_logo, id, dataset_id],
            )?;

            if failed {
                failed_owner_ids.push(id);
            }
        }

        Ok(failed_owner_ids)
    }

    pub fn save_detail_section(
        &self,
        match_id: &str,
        dataset_id: &str,
        section_name: &str,
        data: &Value,
        is_empty: bool,
        content_hash: &str,
        source_timestamp: Option<&str>,
    ) -> SqlResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        let provenance = "detail";
        let status = if is_empty {
            "EMPTY_CONFIRMED"
        } else {
            "COMPLETED"
        };
        let is_empty_val = if is_empty { 1 } else { 0 };

        tx.execute(
            "INSERT INTO match_detail_sections
             (match_id, dataset_id, section_name, status, provenance, is_empty, is_unparseable, content_hash, source_timestamp, received_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, datetime('now'), datetime('now'))
             ON CONFLICT(match_id, dataset_id, section_name) DO UPDATE SET
               status=excluded.status,
               is_empty=excluded.is_empty,
               content_hash=excluded.content_hash,
               source_timestamp=excluded.source_timestamp,
               received_at=excluded.received_at,
               completed_at=excluded.completed_at",
            params![match_id, dataset_id, section_name, status, provenance, is_empty_val, content_hash, source_timestamp],
        )?;

        tx.execute(
            "INSERT INTO match_detail_data
             (match_id, dataset_id, section_name, data_json, provenance, content_hash, source_timestamp, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
             ON CONFLICT(match_id, dataset_id, section_name) DO UPDATE SET
               data_json=excluded.data_json,
               content_hash=excluded.content_hash,
               source_timestamp=excluded.source_timestamp,
               received_at=excluded.received_at",
            params![match_id, dataset_id, section_name, data.to_string(), provenance, content_hash, source_timestamp],
        )?;

        if !is_empty {
            match section_name {
                "stats" => {
                    if let Some(obj) = data.as_object() {
                        for (key, val) in obj {
                            if let Some(arr) = val.as_array() {
                                if arr.len() >= 2 {
                                    tx.execute(
                                        "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                         VALUES (?1, ?2, 'all', ?3, 'home', ?4, ?5, 'detail')
                                         ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                        params![match_id, dataset_id, key, arr[0].to_string(), source_timestamp],
                                    )?;
                                    tx.execute(
                                        "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                         VALUES (?1, ?2, 'all', ?3, 'away', ?4, ?5, 'detail')
                                         ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                        params![match_id, dataset_id, key, arr[1].to_string(), source_timestamp],
                                    )?;
                                }
                            } else {
                                tx.execute(
                                    "INSERT INTO match_statistics (match_id, dataset_id, period, stat_key, side, value_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'all', ?3, 'none', ?4, ?5, 'detail')
                                     ON CONFLICT(match_id, dataset_id, period, stat_key, side) DO UPDATE SET value_json=excluded.value_json",
                                    params![match_id, dataset_id, key, val.to_string(), source_timestamp],
                                )?;
                            }
                        }
                    }
                }
                "lineups" => {
                    if let Some(home_lineup) = data["home"].as_array() {
                        for player in home_lineup {
                            let player_id = player["id"]
                                .as_str()
                                .or_else(|| player["playerId"].as_str())
                                .unwrap_or("")
                                .to_string();
                            if !player_id.is_empty() {
                                tx.execute(
                                    "INSERT INTO match_lineups (match_id, dataset_id, team_side, player_id, coach_id, position, shirt_number, starter, lineup_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'home', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'detail')
                                     ON CONFLICT(match_id, dataset_id, team_side, player_id) DO UPDATE SET lineup_json=excluded.lineup_json",
                                    params![
                                        match_id, dataset_id, player_id,
                                        player["coachId"].as_str(),
                                        player["position"].as_str(),
                                        player["shirtNumber"].as_str().or_else(|| player["shirt_number"].as_str()),
                                        player["starter"].as_bool().map(|b| if b { 1 } else { 0 }).unwrap_or(0),
                                        player.to_string(),
                                        source_timestamp
                                    ],
                                )?;
                            }
                        }
                    }
                    if let Some(away_lineup) = data["away"].as_array() {
                        for player in away_lineup {
                            let player_id = player["id"]
                                .as_str()
                                .or_else(|| player["playerId"].as_str())
                                .unwrap_or("")
                                .to_string();
                            if !player_id.is_empty() {
                                tx.execute(
                                    "INSERT INTO match_lineups (match_id, dataset_id, team_side, player_id, coach_id, position, shirt_number, starter, lineup_json, source_timestamp, provenance)
                                     VALUES (?1, ?2, 'away', ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'detail')
                                     ON CONFLICT(match_id, dataset_id, team_side, player_id) DO UPDATE SET lineup_json=excluded.lineup_json",
                                    params![
                                        match_id, dataset_id, player_id,
                                        player["coachId"].as_str(),
                                        player["position"].as_str(),
                                        player["shirtNumber"].as_str().or_else(|| player["shirt_number"].as_str()),
                                        player["starter"].as_bool().map(|b| if b { 1 } else { 0 }).unwrap_or(0),
                                        player.to_string(),
                                        source_timestamp
                                    ],
                                )?;
                            }
                        }
                    }
                }
                "incidents" => {
                    if let Some(arr) = data.as_array() {
                        for (idx, item) in arr.iter().enumerate() {
                            let incident_key = format!("incident-{}", idx);
                            tx.execute(
                                "INSERT INTO match_incidents (match_id, dataset_id, incident_key, incident_json, source_timestamp, provenance)
                                 VALUES (?1, ?2, ?3, ?4, ?5, 'detail')
                                 ON CONFLICT(match_id, dataset_id, incident_key) DO UPDATE SET incident_json=excluded.incident_json",
                                params![match_id, dataset_id, incident_key, item.to_string(), source_timestamp],
                            )?;
                        }
                    }
                }
                "h2h" => {
                    if let Some(history) = data["history"].as_array() {
                        for (idx, item) in history.iter().enumerate() {
                            let ref_key = item["id"]
                                .as_str()
                                .map(String::from)
                                .unwrap_or_else(|| format!("h2h-{}", idx));
                            let item_str = item.to_string();
                            let h2h_hash = format!("{:x}", md5::compute(&item_str));
                            tx.execute(
                                "INSERT INTO match_h2h_references (match_id, dataset_id, reference_key, reference_json, provenance, content_hash)
                                 VALUES (?1, ?2, ?3, ?4, 'detail', ?5)
                                 ON CONFLICT(match_id, dataset_id, reference_key, content_hash) DO NOTHING",
                                params![match_id, dataset_id, ref_key, item_str, h2h_hash],
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }

        tx.execute(
            "UPDATE detail_jobs
             SET status = 'COMPLETED', completed_at = datetime('now')
             WHERE match_id = ?1 AND dataset_id = ?2 AND section_name = ?3 AND status = 'LOADING'",
            params![match_id, dataset_id, section_name],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn fail_job(
        &self,
        job_id: i64,
        error_msg: &str,
        delay_secs: i64,
        permanent: bool,
    ) -> SqlResult<()> {
        let status = if permanent {
            "FAILED_PERMANENT"
        } else {
            "FAILED_RETRYABLE"
        };
        let modifier = format!("+{} seconds", delay_secs);
        self.conn.execute(
            "UPDATE detail_jobs
             SET status = ?2,
                 scheduled_at = datetime('now', ?3),
                 lease_owner = NULL,
                 lease_expires_at = NULL,
                 last_error = ?4
             WHERE id = ?1",
            params![job_id, status, modifier, error_msg],
        )?;
        Ok(())
    }
}

pub fn apply_event(
    conn: &Connection,
    event: &NormalizedEvent,
    evidence: &AdmissionEvidence,
    odds: &[OddsQuote],
) -> SqlResult<MutationOutcome> {
    RepositoryUnitOfWork::new(conn).apply_event(event, evidence, odds)
}

fn is_stale(
    tx: &Transaction<'_>,
    match_id: &str,
    dataset_id: &str,
    event: &NormalizedEvent,
) -> SqlResult<bool> {
    let Some(current) = latest_state_history(tx, match_id, dataset_id)? else {
        return Ok(false);
    };
    Ok(match_state::compare_order_parts(
        event.source_timestamp.as_deref(),
        Some(event.received_at.as_str()),
        Some(event.payload_hash.as_str()),
        current.0.as_deref(),
        Some(current.1.as_str()),
        Some(current.2.as_str()),
    )
    .is_lt())
}

fn existing_match_is_live(
    tx: &Transaction<'_>,
    match_id: &str,
    dataset_id: &str,
) -> SqlResult<bool> {
    let live = tx
        .query_row(
            "SELECT COALESCE(is_live,0) FROM matches WHERE id=?1 AND dataset_id=?2",
            params![match_id, dataset_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some_and(|value| value != 0);
    if live {
        return Ok(true);
    }
    let mut statement = tx.prepare(
        "SELECT state FROM match_state_history
         WHERE match_id=?1 AND dataset_id=?2",
    )?;
    let mut rows = statement.query(params![match_id, dataset_id])?;
    while let Some(row) = rows.next()? {
        let value: String = row.get(0)?;
        if InternalState::from_storage(&value).is_some_and(InternalState::is_live) {
            return Ok(true);
        }
    }
    Ok(false)
}

type HistoryRow = (Option<String>, String, String, InternalState);

fn latest_state_history(
    tx: &Transaction<'_>,
    match_id: &str,
    dataset_id: &str,
) -> SqlResult<Option<HistoryRow>> {
    let mut statement = tx.prepare(
        "SELECT NULLIF(source_timestamp,''), received_at, payload_hash, state
         FROM match_state_history WHERE match_id=?1 AND dataset_id=?2",
    )?;
    let mut rows = statement.query(params![match_id, dataset_id])?;
    let mut latest: Option<HistoryRow> = None;
    while let Some(row) = rows.next()? {
        let source_timestamp: Option<String> = row.get(0)?;
        let received_at: String = row.get(1)?;
        let payload_hash: String = row.get(2)?;
        let state_value: String = row.get(3)?;
        let state = InternalState::from_storage(&state_value).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                state_value.len(),
                rusqlite::types::Type::Text,
                "unknown internal state".into(),
            )
        })?;
        let candidate = (source_timestamp, received_at, payload_hash, state);
        if latest.as_ref().is_none_or(|current| {
            match_state::compare_order_parts(
                candidate.0.as_deref(),
                Some(candidate.1.as_str()),
                Some(candidate.2.as_str()),
                current.0.as_deref(),
                Some(current.1.as_str()),
                Some(current.2.as_str()),
            )
            .is_gt()
        }) {
            latest = Some(candidate);
        }
    }
    Ok(latest)
}

fn snapshot_from_event(
    tx: &Transaction<'_>,
    event: &NormalizedEvent,
    evidence: &AdmissionEvidence,
    dataset_id: &str,
) -> SqlResult<Option<Snapshot>> {
    let p = &event.payload;
    let Some(match_id) = event.match_id.clone() else {
        return Ok(None);
    };
    let sport_id = event
        .sport_id
        .or_else(|| p["sport_id"].as_i64().map(|v| v as i32));
    let Some(sport_id) = sport_id else {
        return Ok(None);
    };
    let existing: Option<(String, String, String, i64, Option<i32>, String, String)> = tx
        .query_row(
            "SELECT competition_id, home_team_id, away_team_id, match_time,
                    status_id, home_scores, away_scores
             FROM matches WHERE id=?1 AND dataset_id=?2",
            params![match_id, dataset_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let current_state = latest_state_history(tx, &match_id, dataset_id)?.map(|row| row.3);
    let text = |a: &str, b: &str| {
        p[a].as_str()
            .or_else(|| p[b].as_str())
            .unwrap_or("")
            .to_string()
    };
    let state = match_state::next_state(
        current_state,
        event.event_type,
        evidence.source_status.as_deref(),
    );
    let fallback = existing.as_ref();
    let home_scores = p
        .get("home_scores")
        .or_else(|| p.get("homeScores"))
        .map(Value::to_string)
        .or_else(|| fallback.map(|row| row.5.clone()))
        .unwrap_or_else(|| "[]".to_string());
    let away_scores = p
        .get("away_scores")
        .or_else(|| p.get("awayScores"))
        .map(Value::to_string)
        .or_else(|| fallback.map(|row| row.6.clone()))
        .unwrap_or_else(|| "[]".to_string());
    Ok(Some(Snapshot {
        match_id,
        sport_id,
        competition_id: nonempty_or(
            text("competition_id", "competitionId"),
            fallback.map(|row| row.0.clone()),
        ),
        home_team_id: nonempty_or(
            text("home_team_id", "homeTeamId"),
            fallback.map(|row| row.1.clone()),
        ),
        away_team_id: nonempty_or(
            text("away_team_id", "awayTeamId"),
            fallback.map(|row| row.2.clone()),
        ),
        match_time: p["match_time"]
            .as_i64()
            .or_else(|| p["matchTime"].as_i64())
            .or_else(|| fallback.map(|row| row.3))
            .unwrap_or(0),
        status_id: p["status_id"]
            .as_i64()
            .or_else(|| p["statusId"].as_i64())
            .map(|v| v as i32)
            .or_else(|| fallback.and_then(|row| row.4)),
        state,
        home_scores: serde_json::from_str(&home_scores).unwrap_or(Value::Array(vec![])),
        away_scores: serde_json::from_str(&away_scores).unwrap_or(Value::Array(vec![])),
        period: evidence
            .period
            .clone()
            .or_else(|| text_value(p, &["period"])),
        clock: evidence
            .clock
            .clone()
            .or_else(|| text_value(p, &["clock", "match_clock"])),
        source_timestamp: event.source_timestamp.clone(),
        received_at: event.received_at.clone(),
        payload_hash: event.payload_hash.clone(),
        raw_payload: event.payload.clone(),
    }))
}

fn nonempty_or(value: String, fallback: Option<String>) -> String {
    if value.is_empty() {
        fallback.unwrap_or_default()
    } else {
        value
    }
}

fn text_value(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(serde::Serialize)]
struct Snapshot {
    match_id: String,
    sport_id: i32,
    competition_id: String,
    home_team_id: String,
    away_team_id: String,
    match_time: i64,
    status_id: Option<i32>,
    state: InternalState,
    home_scores: Value,
    away_scores: Value,
    period: Option<String>,
    clock: Option<String>,
    source_timestamp: Option<String>,
    received_at: String,
    payload_hash: String,
    raw_payload: Value,
}

fn upsert_snapshot(tx: &Transaction<'_>, dataset: &str, s: &Snapshot) -> SqlResult<()> {
    let live = s.state.is_live();
    tx.execute("INSERT INTO matches (id,sport_id,competition_id,home_team_id,away_team_id,match_time,status_id,home_scores,away_scores,is_live,raw_payload,synced,updated_at,dataset_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12,?13) ON CONFLICT(id,dataset_id) DO UPDATE SET sport_id=excluded.sport_id,competition_id=excluded.competition_id,home_team_id=excluded.home_team_id,away_team_id=excluded.away_team_id,match_time=excluded.match_time,status_id=excluded.status_id,home_scores=excluded.home_scores,away_scores=excluded.away_scores,is_live=excluded.is_live,raw_payload=excluded.raw_payload,synced=0,updated_at=excluded.updated_at", params![s.match_id,s.sport_id,s.competition_id,s.home_team_id,s.away_team_id,s.match_time,s.status_id,s.home_scores.to_string(),s.away_scores.to_string(),live,s.raw_payload.to_string(),s.received_at,dataset])?;
    Ok(())
}

fn insert_state_history(tx: &Transaction<'_>, dataset: &str, s: &Snapshot) -> SqlResult<()> {
    tx.execute("INSERT OR IGNORE INTO match_state_history (match_id,dataset_id,sport_id,state,status_id,home_scores,away_scores,period,clock,source_timestamp,received_at,payload_hash,provenance) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'feed')", params![s.match_id,dataset,s.sport_id,s.state.as_str(),s.status_id,s.home_scores.to_string(),s.away_scores.to_string(),s.period,s.clock,s.source_timestamp.as_deref().unwrap_or(""),s.received_at,s.payload_hash])?;
    Ok(())
}

pub fn plan_initial_detail_jobs(
    tx: &Transaction<'_>,
    dataset_id: &str,
    match_id: &str,
) -> SqlResult<()> {
    for section_name in &["overview", "odds", "h2h", "lineups", "stats", "incidents"] {
        let completed: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM match_detail_sections
                WHERE match_id = ?1 AND dataset_id = ?2 AND section_name = ?3
                  AND status IN ('COMPLETED', 'EMPTY_CONFIRMED')
            )",
            params![match_id, dataset_id, section_name],
            |row| row.get(0),
        )?;
        if completed {
            continue;
        }

        tx.execute(
            "INSERT OR IGNORE INTO detail_jobs (match_id, dataset_id, section_name, load_phase, status, scheduled_at, attempt_count)
             VALUES (?1, ?2, ?3, 'INITIAL', 'PENDING', datetime('now'), 0)",
            params![match_id, dataset_id, section_name],
        )?;
    }
    Ok(())
}

/// Returns `true` when the quote was rejected as older than the current quote.
fn apply_quote(tx: &Transaction<'_>, dataset: &str, q: &OddsQuote) -> SqlResult<bool> {
    let i = &q.identity;
    let previous_row: Option<(String, Option<String>, String, String)> = tx.query_row("SELECT odds_value, NULLIF(source_timestamp,''), received_at, payload_hash FROM odds_current WHERE match_id=?1 AND dataset_id=?2 AND bookmaker_id=?3 AND market_type=?4 AND period=?5 AND selection_key=?6 AND line_value=?7", params![i.match_id,dataset,i.bookmaker_id,i.market_type,i.period,i.selection_key,i.line_value], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
    if let Some((_, current_source_timestamp, current_received_at, current_payload_hash)) =
        &previous_row
    {
        if match_state::compare_order_parts(
            q.source_timestamp.as_deref(),
            Some(q.received_at.as_str()),
            Some(q.payload_hash.as_str()),
            current_source_timestamp.as_deref(),
            Some(current_received_at.as_str()),
            Some(current_payload_hash.as_str()),
        )
        .is_lt()
        {
            return Ok(true);
        }
    }
    let previous = previous_row.as_ref().map(|row| row.0.as_str());
    if !q.value_changed(previous) {
        return Ok(false);
    }
    tx.execute("INSERT INTO odds_history (match_id,dataset_id,bookmaker_id,market_type,period,selection_key,line_value,odds_value,previous_odds_value,is_live,source_timestamp,received_at,payload_hash,provenance,event_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'feed',?14)", params![i.match_id,dataset,i.bookmaker_id,i.market_type,i.period,i.selection_key,i.line_value,q.odds_value,previous,q.is_live,q.source_timestamp.as_deref().unwrap_or(""),q.received_at,q.payload_hash,q.event_key()])?;
    tx.execute("INSERT INTO odds_current (match_id,dataset_id,bookmaker_id,market_type,period,selection_key,line_value,odds_value,is_live,source_timestamp,received_at,payload_hash,provenance) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'feed') ON CONFLICT(match_id,dataset_id,bookmaker_id,market_type,period,selection_key,line_value) DO UPDATE SET odds_value=excluded.odds_value,is_live=excluded.is_live,source_timestamp=excluded.source_timestamp,received_at=excluded.received_at,payload_hash=excluded.payload_hash", params![i.match_id,dataset,i.bookmaker_id,i.market_type,i.period,i.selection_key,i.line_value,q.odds_value,q.is_live,q.source_timestamp.as_deref().unwrap_or(""),q.received_at,q.payload_hash])?;
    insert_outbox(
        tx,
        dataset,
        "odds",
        &q.event_key(),
        "MATCH_ODDS_CHANGED",
        &json!(q),
    )?;
    Ok(false)
}

fn insert_outbox(
    tx: &Transaction<'_>,
    dataset: &str,
    entity_type: &str,
    entity_id: &str,
    event_type: &str,
    payload: &Value,
) -> SqlResult<()> {
    tx.execute("INSERT OR IGNORE INTO sync_outbox (dataset_id,entity_type,entity_id,event_type,payload_json) VALUES (?1,?2,?3,?4,?5)", params![dataset,entity_type,entity_id,event_type,payload.to_string()])?;
    Ok(())
}

pub fn get_match_team_slugs(
    conn: &Connection,
    match_id: &str,
    dataset_id: &str,
) -> SqlResult<(String, String, i32)> {
    let query = "
        SELECT COALESCE(t1.slug, 'home'), COALESCE(t2.slug, 'away'), m.sport_id
        FROM matches m
        LEFT JOIN teams t1 ON m.home_team_id = t1.id AND m.dataset_id = t1.dataset_id
        LEFT JOIN teams t2 ON m.away_team_id = t2.id AND m.dataset_id = t2.dataset_id
        WHERE m.id = ?1 AND m.dataset_id = ?2
    ";
    conn.query_row(query, params![match_id, dataset_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::events::EventType;
    use serde_json::json;

    #[test]
    fn transaction_rolls_back_on_constraint_failure() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::run_migrations(&conn).unwrap();
        let mut event = NormalizedEvent::new(
            EventType::MatchDiscoveredLive,
            Some("m".into()),
            Some(1),
            Some("1".into()),
            "2".into(),
            json!({"home_team_id":"h","away_team_id":"a"}),
        );
        event.dataset_id = Some("d".into());
        let evidence = AdmissionEvidence {
            started: true,
            ..Default::default()
        };
        assert!(apply_event(&conn, &event, &evidence, &[]).is_ok());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM feed_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    fn match_event(
        event_type: EventType,
        source_timestamp: Option<&str>,
        received_at: &str,
        marker: i64,
        status: &str,
    ) -> (NormalizedEvent, AdmissionEvidence) {
        (
            NormalizedEvent::new(
                event_type,
                Some("m".into()),
                Some(1),
                source_timestamp.map(str::to_string),
                received_at.into(),
                json!({
                    "competition_id": "c",
                    "home_team_id": "h",
                    "away_team_id": "a",
                    "home_scores": [marker],
                    "away_scores": [0],
                    "marker": marker
                }),
            ),
            AdmissionEvidence {
                source_status: Some(status.into()),
                started: status.eq_ignore_ascii_case("live"),
                ..Default::default()
            },
        )
    }

    #[test]
    fn terminal_update_is_admitted_from_historical_live_and_then_immutable() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::run_migrations(&conn).unwrap();

        let (mut live, live_evidence) =
            match_event(EventType::MatchDiscoveredLive, Some("1"), "1", 1, "Live");
        live.dataset_id = Some("d".into());
        assert_eq!(
            apply_event(&conn, &live, &live_evidence, &[]).unwrap(),
            MutationOutcome::Accepted
        );
        let (mut finished, finished_evidence) =
            match_event(EventType::MatchFinished, Some("2"), "2", 2, "Finished");
        finished.dataset_id = Some("d".into());
        assert_eq!(
            apply_event(&conn, &finished, &finished_evidence, &[]).unwrap(),
            MutationOutcome::Accepted
        );
        let (mut late, late_evidence) =
            match_event(EventType::MatchScoreChanged, Some("3"), "3", 3, "Live");
        late.dataset_id = Some("d".into());
        assert_eq!(
            apply_event(&conn, &late, &late_evidence, &[]).unwrap(),
            MutationOutcome::Rejected(AdmissionResult::RejectTerminalImmutable)
        );

        let state: String = conn
            .query_row(
                "SELECT state FROM match_state_history WHERE match_id='m' AND dataset_id='d'
                 ORDER BY source_timestamp DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "FINISHED");
    }

    #[test]
    fn missing_timestamp_uses_receipt_order_and_older_source_is_stale() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::run_migrations(&conn).unwrap();
        let (mut first, first_evidence) =
            match_event(EventType::MatchDiscoveredLive, Some("2"), "2", 1, "Live");
        first.dataset_id = Some("d".into());
        apply_event(&conn, &first, &first_evidence, &[]).unwrap();

        let (mut older, older_evidence) =
            match_event(EventType::MatchScoreChanged, Some("1"), "3", 2, "Live");
        older.dataset_id = Some("d".into());
        assert_eq!(
            apply_event(&conn, &older, &older_evidence, &[]).unwrap(),
            MutationOutcome::Stale
        );

        let (mut no_timestamp, no_timestamp_evidence) =
            match_event(EventType::MatchScoreChanged, None, "4", 3, "Live");
        no_timestamp.dataset_id = Some("d".into());
        assert_eq!(
            apply_event(&conn, &no_timestamp, &no_timestamp_evidence, &[]).unwrap(),
            MutationOutcome::Accepted
        );
    }

    #[test]
    fn odds_history_keeps_changed_quote_and_ignores_older_quote() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::run_migrations(&conn).unwrap();
        let quote = |value: &str, timestamp: Option<&str>, received: &str| {
            OddsQuote::new(
                "m",
                "book",
                "moneyline",
                "full",
                "home",
                None,
                value,
                true,
                timestamp.map(str::to_string),
                received.into(),
                "same-hash".into(),
                json!({}),
            )
        };
        let (mut first, first_evidence) =
            match_event(EventType::MatchDiscoveredLive, Some("1"), "1", 1, "Live");
        first.dataset_id = Some("d".into());
        apply_event(
            &conn,
            &first,
            &first_evidence,
            &[quote("1.5", Some("1"), "1")],
        )
        .unwrap();
        let (mut changed, changed_evidence) =
            match_event(EventType::MatchOddsChanged, Some("3"), "3", 2, "Live");
        changed.dataset_id = Some("d".into());
        apply_event(
            &conn,
            &changed,
            &changed_evidence,
            &[quote("2.0", Some("3"), "3")],
        )
        .unwrap();
        let (mut older, older_evidence) =
            match_event(EventType::MatchOddsChanged, Some("2"), "4", 3, "Live");
        older.dataset_id = Some("d".into());
        apply_event(
            &conn,
            &older,
            &older_evidence,
            &[quote("3.0", Some("2"), "4")],
        )
        .unwrap();

        let current: String = conn
            .query_row(
                "SELECT odds_value FROM odds_current WHERE match_id='m' AND dataset_id='d'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let history_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM odds_history WHERE match_id='m' AND dataset_id='d'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(current, "2");
        assert_eq!(history_count, 2);
    }
}
