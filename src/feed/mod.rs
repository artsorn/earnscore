//! Owned, source-neutral live feed runtime.
//!
//! The feed module deliberately owns only the browser process and targets it
//! creates.  It does not attach to, navigate, or close a user's browser tabs.

pub mod adapters;
pub mod browser;

use crate::domain::{EventType, NormalizedEvent};
use adapters::{FeedAdapter, FeedError, SourceEnvelope};
use browser::TargetRole;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use adapters::{BasketballAdapter, FootballAdapter, NormalizedMatch, Readiness};
pub use browser::{BrowserHealth, OwnedBrowser, OwnedBrowserConfig, OwnedTarget, TargetRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Ready,
    Connected,
    Stale,
    Disconnected,
    Stopped,
}

#[derive(Clone)]
pub struct FeedSessionConfig {
    pub browser: OwnedBrowserConfig,
    pub heartbeat_interval: Duration,
    pub stale_after: Duration,
    pub lifecycle_hook: Option<FeedLifecycleHook>,
}

pub type FeedLifecycleHook = Arc<dyn Fn(FeedLifecycleEvent) + Send + Sync>;

impl std::fmt::Debug for FeedSessionConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FeedSessionConfig")
            .field("browser", &self.browser)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("stale_after", &self.stale_after)
            .field(
                "lifecycle_hook",
                &self.lifecycle_hook.as_ref().map(|_| "installed"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedLifecycleEvent {
    Disconnected { session_id: String, sport_id: i32 },
    Reconnected { session_id: String, sport_id: i32 },
}

impl Default for FeedSessionConfig {
    fn default() -> Self {
        Self {
            browser: OwnedBrowserConfig::default(),
            heartbeat_interval: Duration::from_secs(5),
            stale_after: Duration::from_secs(20),
            lifecycle_hook: None,
        }
    }
}

pub struct FeedSession<A: FeedAdapter> {
    adapter: A,
    browser: std::sync::Arc<OwnedBrowser>,
    target: OwnedTarget,
    state: SessionState,
    seen_live: HashSet<String>,
    seen_events: HashSet<String>,
    last_heartbeat: std::time::Instant,
    session_id: String,
    disconnect_notified: bool,
    config: FeedSessionConfig,
}

impl<A: FeedAdapter> FeedSession<A> {
    pub fn new(
        adapter: A,
        browser: std::sync::Arc<OwnedBrowser>,
        target: OwnedTarget,
        config: FeedSessionConfig,
    ) -> Self {
        Self {
            adapter,
            browser,
            target,
            state: SessionState::Ready,
            seen_live: HashSet::new(),
            seen_events: HashSet::new(),
            last_heartbeat: std::time::Instant::now(),
            session_id: format!("feed-{}", uuid::Uuid::new_v4()),
            disconnect_notified: false,
            config,
        }
    }

    pub async fn start(adapter: A, config: FeedSessionConfig) -> Result<Self, FeedError> {
        let browser = OwnedBrowser::launch(config.browser.clone())
            .await
            .map_err(|error| FeedError::Browser(error.to_string()))?;
        let target = browser
            .create_target(TargetRole::Live, adapter.sport_id(), adapter.target_url())
            .await
            .map_err(|error| FeedError::Browser(error.to_string()))?;
        Ok(Self::new(
            adapter,
            std::sync::Arc::new(browser),
            target,
            config,
        ))
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }
    pub fn target(&self) -> &OwnedTarget {
        &self.target
    }
    pub fn state(&self) -> SessionState {
        self.state
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn is_stale(&self) -> bool {
        self.last_heartbeat.elapsed() >= self.config.stale_after
    }

    pub fn heartbeat(&mut self) -> NormalizedEvent {
        self.last_heartbeat = std::time::Instant::now();
        self.state = SessionState::Connected;
        NormalizedEvent::new(
            EventType::FeedHeartbeat,
            None,
            Some(self.adapter.sport_id()),
            None,
            now(),
            serde_json::json!({"sport": self.adapter.name()}),
        )
    }

    /// Mark a transport disconnect exactly once.  Parser/source-contract
    /// failures intentionally do not call this method and therefore cannot
    /// turn a malformed payload into a match-finished signal.
    pub fn mark_disconnected(&mut self) -> NormalizedEvent {
        self.state = SessionState::Disconnected;
        if !self.disconnect_notified {
            self.disconnect_notified = true;
            if let Some(hook) = &self.config.lifecycle_hook {
                hook(FeedLifecycleEvent::Disconnected {
                    session_id: self.session_id.clone(),
                    sport_id: self.adapter.sport_id(),
                });
            }
        }
        NormalizedEvent::new(
            EventType::FeedDisconnected,
            None,
            Some(self.adapter.sport_id()),
            None,
            now(),
            serde_json::json!({"sport": self.adapter.name()}),
        )
    }

    /// Validate a source and emit only matches admitted by the live filter.
    /// Terminal rows are emitted only for identities seen live in this session.
    pub fn ingest(&mut self, envelope: SourceEnvelope) -> Result<Vec<NormalizedEvent>, FeedError> {
        let snapshot = self.adapter.extract(&envelope)?;
        self.last_heartbeat = std::time::Instant::now();
        self.state = SessionState::Connected;
        let mut events = Vec::new();
        for item in snapshot {
            let id = item.match_id.clone();
            if item.is_live {
                self.seen_live.insert(id.clone());
            }
            if !item.is_live && !self.seen_live.contains(&id) {
                continue;
            }
            let event_type = if !item.is_live {
                EventType::MatchFinished
            } else {
                EventType::MatchDiscoveredLive
            };
            let event = NormalizedEvent::new(
                event_type,
                Some(id),
                Some(self.adapter.sport_id()),
                item.source_timestamp.clone(),
                now(),
                item.payload,
            );
            if self.seen_events.insert(event.event_key()) {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Select the strongest available source before validating its contract.
    pub fn ingest_preferred(
        &mut self,
        mut envelopes: Vec<SourceEnvelope>,
    ) -> Result<Vec<NormalizedEvent>, FeedError> {
        envelopes.sort_by_key(|envelope| envelope.layer.priority());
        if envelopes.is_empty() {
            return Err(FeedError::SourceChanged("no feed source available".into()));
        }
        envelopes.sort_by_key(|envelope| envelope.layer.priority());
        let mut last_error = None;
        for envelope in envelopes {
            match self.ingest(envelope) {
                Ok(events) => return Ok(events),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| FeedError::SourceChanged("no valid feed source".into())))
    }

    /// Run one automatic liveness check. The caller can invoke this from its
    /// event loop; no external heartbeat bookkeeping is required.
    pub async fn watchdog_tick(&mut self) -> Result<Option<NormalizedEvent>, FeedError> {
        if !matches!(self.browser.health().await, BrowserHealth::Healthy) {
            self.mark_disconnected();
            self.reconnect().await?;
            return Ok(Some(self.heartbeat()));
        }
        if self.last_heartbeat.elapsed() >= self.config.stale_after {
            self.state = SessionState::Stale;
            self.mark_disconnected();
            self.reconnect().await?;
            return Ok(Some(self.heartbeat()));
        }
        if self.last_heartbeat.elapsed() >= self.config.heartbeat_interval {
            return Ok(Some(self.heartbeat()));
        }
        Ok(None)
    }

    pub async fn reconnect(&mut self) -> Result<(), FeedError> {
        let old = self.target.clone();
        self.target = self
            .browser
            .recreate_target(&old, self.adapter.target_url())
            .await
            .map_err(|error| FeedError::Browser(error.to_string()))?;
        self.state = SessionState::Connected;
        self.last_heartbeat = std::time::Instant::now();
        if self.disconnect_notified {
            self.disconnect_notified = false;
            if let Some(hook) = &self.config.lifecycle_hook {
                hook(FeedLifecycleEvent::Reconnected {
                    session_id: self.session_id.clone(),
                    sport_id: self.adapter.sport_id(),
                });
            }
        }
        Ok(())
    }

    /// Capture one source cycle. Network is attempted first; a lost socket or
    /// malformed response intentionally falls through to store and DOM
    /// snapshots. The returned events are still admitted by the adapter's
    /// live-filter and identity guards.
    pub async fn poll_once(&mut self) -> Result<Vec<NormalizedEvent>, FeedError> {
        let mut sources = Vec::new();
        let connected = if let Ok(mut cdp) = self.browser.connect_target(&self.target).await {
            if let Ok(Some(network)) = cdp.next_network_source(Duration::from_millis(250)).await {
                sources.push(network);
            }
            match cdp.capture_fallbacks().await {
                Ok(fallbacks) => sources.extend(fallbacks),
                Err(_) if sources.is_empty() => {
                    self.mark_disconnected();
                    return Err(FeedError::Disconnected);
                }
                Err(_) => {}
            }
            true
        } else {
            false
        };
        if !connected {
            self.mark_disconnected();
            return Err(FeedError::Disconnected);
        }
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        self.ingest_preferred(sources)
    }

    /// Drive capture and liveness from one runtime loop. The bounded duration
    /// makes this useful for supervisors and deterministic tests alike.
    pub async fn run_for(&mut self, runtime: Duration) -> Result<Vec<NormalizedEvent>, FeedError> {
        let deadline = tokio::time::Instant::now() + runtime;
        let mut ticker =
            tokio::time::interval(self.config.heartbeat_interval.min(Duration::from_secs(1)));
        let mut events = Vec::new();
        while tokio::time::Instant::now() < deadline {
            ticker.tick().await;
            if let Some(heartbeat) = self.watchdog_tick().await? {
                events.push(heartbeat);
            }
            match self.poll_once().await {
                Ok(mut captured) => events.append(&mut captured),
                Err(FeedError::Disconnected) => {
                    self.reconnect().await?;
                }
                Err(FeedError::SourceChanged(reason)) => {
                    self.state = SessionState::Disconnected;
                    return Err(FeedError::SourceChanged(reason));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(events)
    }

    pub async fn shutdown(mut self) -> Result<(), FeedError> {
        self.state = SessionState::Stopped;
        self.browser
            .close_target(&self.target)
            .await
            .map_err(|error| FeedError::Browser(error.to_string()))?;
        if std::sync::Arc::strong_count(&self.browser) == 1 {
            self.browser
                .shutdown()
                .await
                .map_err(|error| FeedError::Browser(error.to_string()))?;
        }
        Ok(())
    }
}

/// One owned browser process containing exactly one isolated live target per sport.
pub struct FeedCoordinator {
    browser: std::sync::Arc<OwnedBrowser>,
    pub football_session: FeedSession<FootballAdapter>,
    pub basketball_session: FeedSession<BasketballAdapter>,
}

impl FeedCoordinator {
    pub async fn start(config: FeedSessionConfig) -> Result<Self, FeedError> {
        let browser = std::sync::Arc::new(
            OwnedBrowser::launch(config.browser.clone())
                .await
                .map_err(|error| FeedError::Browser(error.to_string()))?,
        );
        let football = match browser
            .create_target(
                TargetRole::Live,
                FootballAdapter.sport_id(),
                FootballAdapter.target_url(),
            )
            .await
        {
            Ok(target) => target,
            Err(error) => {
                let _ = browser.shutdown().await;
                return Err(FeedError::Browser(error.to_string()));
            }
        };
        let basketball = match browser
            .create_target(
                TargetRole::Live,
                BasketballAdapter.sport_id(),
                BasketballAdapter.target_url(),
            )
            .await
        {
            Ok(target) => target,
            Err(error) => {
                let _ = browser.close_target(&football).await;
                let _ = browser.shutdown().await;
                return Err(FeedError::Browser(error.to_string()));
            }
        };
        let football_session =
            FeedSession::new(FootballAdapter, browser.clone(), football, config.clone());
        let basketball_session =
            FeedSession::new(BasketballAdapter, browser.clone(), basketball, config);
        Ok(Self {
            browser,
            football_session,
            basketball_session,
        })
    }

    pub fn targets(&self) -> [&OwnedTarget; 2] {
        [
            self.football_session.target(),
            self.basketball_session.target(),
        ]
    }

    pub fn endpoint(&self) -> &str {
        &self.browser.endpoint
    }

    pub async fn run(
        mut self,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<(), FeedError> {
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                _ = ticker.tick() => {
                    if let Some(event) = self.football_session.watchdog_tick().await? {
                        println!("[Live Event] {}", serde_json::to_string(&event).unwrap_or_default());
                    }
                    match self.football_session.poll_once().await {
                        Ok(events) => {
                            for event in events {
                                println!("[Live Event] {}", serde_json::to_string(&event).unwrap_or_default());
                            }
                        }
                        Err(FeedError::Disconnected) => {
                            let _ = self.football_session.reconnect().await;
                        }
                        Err(FeedError::SourceChanged(reason)) => {
                            println!("[Live Error] Football Source Changed: {}", reason);
                        }
                        Err(e) => {
                            println!("[Live Error] Football Error: {:?}", e);
                        }
                    }

                    if let Some(event) = self.basketball_session.watchdog_tick().await? {
                        println!("[Live Event] {}", serde_json::to_string(&event).unwrap_or_default());
                    }
                    match self.basketball_session.poll_once().await {
                        Ok(events) => {
                            for event in events {
                                println!("[Live Event] {}", serde_json::to_string(&event).unwrap_or_default());
                            }
                        }
                        Err(FeedError::Disconnected) => {
                            let _ = self.basketball_session.reconnect().await;
                        }
                        Err(FeedError::SourceChanged(reason)) => {
                            println!("[Live Error] Basketball Source Changed: {}", reason);
                        }
                        Err(e) => {
                            println!("[Live Error] Basketball Error: {:?}", e);
                        }
                    }
                }
            }
        }
        println!("[Feed] Shutting down live targets and browser...");
        self.shutdown().await
    }

    pub async fn shutdown(self) -> Result<(), FeedError> {
        let _ = self.football_session.shutdown().await;
        let _ = self.basketball_session.shutdown().await;
        self.browser
            .shutdown()
            .await
            .map_err(|error| FeedError::Browser(error.to_string()))
    }
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::adapters::SourceLayer;
    use serde_json::json;

    #[test]
    fn source_changed_is_fail_closed() {
        let adapter = FootballAdapter;
        let err = adapter
            .extract(&SourceEnvelope::new(
                SourceLayer::Network,
                json!({"activeFilter":"live", "sportId": 1, "matches":{}}),
            ))
            .unwrap_err();
        assert!(matches!(err, FeedError::SourceChanged(_)));
    }

    #[tokio::test]
    async fn test_live_coordinator_startup_and_run() {
        let config = FeedSessionConfig::default();
        let coordinator = FeedCoordinator::start(config)
            .await
            .expect("failed to start coordinator");
        assert_eq!(coordinator.targets().len(), 2);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move { coordinator.run(shutdown_rx).await });

        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = shutdown_tx.send(());
        let res = handle.await.expect("task panicked");
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_dom_fallback_extraction() {
        let config = OwnedBrowserConfig::default();
        let browser = OwnedBrowser::launch(config)
            .await
            .expect("failed to launch browser");
        let target = browser
            .create_target(TargetRole::Live, 1, "about:blank")
            .await
            .expect("failed to create target");
        let mut cdp = browser
            .connect_target(&target)
            .await
            .expect("failed to connect target");

        let setup_script = r#"
            document.body.innerHTML = `
                <div class="match-list">
                    <div class="match-item"
                         data-match-id="dom-match-1"
                         data-status-id="2"
                         data-competition-id="comp-1"
                         data-home-team-id="team-1"
                         data-away-team-id="team-2"
                         data-home-scores="[2, 0]"
                         data-away-scores="[1, 0]"
                         data-period="1st"
                         data-clock="15:30">
                    </div>
                </div>
            `;
            document.body.dataset.activeFilter = "live";
            document.body.dataset.sportId = "1";
        "#;
        cdp.evaluate(setup_script)
            .await
            .expect("failed to setup mock DOM");

        let fallbacks = cdp
            .capture_fallbacks()
            .await
            .expect("failed to capture fallbacks");
        let dom_envelope = fallbacks
            .into_iter()
            .find(|env| env.layer == SourceLayer::Dom)
            .expect("no DOM fallback found");

        let adapter = FootballAdapter;
        let matches = adapter
            .extract(&dom_envelope)
            .expect("failed to extract matches from DOM fallback");

        assert_eq!(matches.len(), 1);
        let m = &matches[0];
        assert_eq!(m.match_id, "dom-match-1");
        assert_eq!(m.status_id, 2);
        assert_eq!(m.home_score, serde_json::json!([2, 0]));
        assert_eq!(m.away_score, serde_json::json!([1, 0]));
        assert_eq!(m.period.as_deref(), Some("1st"));
        assert_eq!(m.clock.as_deref(), Some("15:30"));
        assert!(m.is_live);

        browser
            .shutdown()
            .await
            .expect("failed to shutdown browser");
    }

    #[tokio::test]
    async fn test_dom_fallback_fails_closed_on_invalid_shape() {
        let config = OwnedBrowserConfig::default();
        let browser = OwnedBrowser::launch(config)
            .await
            .expect("failed to launch browser");
        let target = browser
            .create_target(TargetRole::Live, 1, "about:blank")
            .await
            .expect("failed to create target");
        let mut cdp = browser
            .connect_target(&target)
            .await
            .expect("failed to connect target");

        // Setup invalid mock DOM (missing data-status-id)
        let setup_script = r#"
            document.body.innerHTML = `
                <div class="match-list">
                    <div class="match-item"
                         data-match-id="dom-match-1"
                         data-competition-id="comp-1"
                         data-home-team-id="team-1"
                         data-away-team-id="team-2">
                    </div>
                </div>
            `;
            document.body.dataset.activeFilter = "live";
            document.body.dataset.sportId = "1";
        "#;
        cdp.evaluate(setup_script)
            .await
            .expect("failed to setup mock DOM");

        let fallbacks = cdp
            .capture_fallbacks()
            .await
            .expect("failed to capture fallbacks");
        let dom_envelope = fallbacks
            .into_iter()
            .find(|env| env.layer == SourceLayer::Dom)
            .expect("no DOM fallback found");

        let adapter = FootballAdapter;
        let err = adapter.extract(&dom_envelope).unwrap_err();
        assert!(matches!(err, FeedError::SourceChanged(_)));

        browser
            .shutdown()
            .await
            .expect("failed to shutdown browser");
    }

    #[tokio::test]
    async fn test_live_coordinator_drives_both_sessions_deterministically() {
        let config = FeedSessionConfig::default();
        let browser = OwnedBrowser::launch(config.browser.clone())
            .await
            .expect("failed to launch browser");
        let browser_arc = std::sync::Arc::new(browser);

        // Create football and basketball blank targets
        let football_target = browser_arc
            .create_target(TargetRole::Live, 1, "about:blank")
            .await
            .expect("failed to create football target");
        let basketball_target = browser_arc
            .create_target(TargetRole::Live, 2, "about:blank")
            .await
            .expect("failed to create basketball target");

        // Set up DOM mock in both targets
        let mut football_cdp = browser_arc
            .connect_target(&football_target)
            .await
            .expect("failed to connect football target");
        let setup_football = r#"
            document.body.innerHTML = `
                <div class="match-list">
                    <div class="match-item"
                         data-match-id="foot-match-1"
                         data-status-id="2"
                         data-competition-id="comp-1"
                         data-home-team-id="team-1"
                         data-away-team-id="team-2"
                         data-home-scores="[1]"
                         data-away-scores="[0]"
                         data-period="1st"
                         data-clock="10:00">
                    </div>
                </div>
            `;
            document.body.dataset.activeFilter = "live";
            document.body.dataset.sportId = "1";
        "#;
        football_cdp
            .evaluate(setup_football)
            .await
            .expect("failed mock football DOM");

        let mut basketball_cdp = browser_arc
            .connect_target(&basketball_target)
            .await
            .expect("failed to connect basketball target");
        let setup_basketball = r#"
            document.body.innerHTML = `
                <div class="match-list">
                    <div class="match-item"
                         data-match-id="bask-match-1"
                         data-status-id="2"
                         data-competition-id="comp-2"
                         data-home-team-id="team-3"
                         data-away-team-id="team-4"
                         data-home-scores="[20]"
                         data-away-scores="[18]"
                         data-period="Q1"
                         data-clock="08:00">
                    </div>
                </div>
            `;
            document.body.dataset.activeFilter = "live";
            document.body.dataset.sportId = "2";
        "#;
        basketball_cdp
            .evaluate(setup_basketball)
            .await
            .expect("failed mock basketball DOM");

        // Create coordinator manually
        let football_session = FeedSession::new(
            FootballAdapter,
            browser_arc.clone(),
            football_target,
            config.clone(),
        );
        let basketball_session = FeedSession::new(
            BasketballAdapter,
            browser_arc.clone(),
            basketball_target,
            config.clone(),
        );
        let mut coordinator = FeedCoordinator {
            browser: browser_arc.clone(),
            football_session,
            basketball_session,
        };

        // Assert that both targets are stored correctly
        assert_eq!(coordinator.targets().len(), 2);

        // Poll both sessions once directly and verify they return the mock matches
        let football_events = coordinator
            .football_session
            .poll_once()
            .await
            .expect("failed football poll");
        let basketball_events = coordinator
            .basketball_session
            .poll_once()
            .await
            .expect("failed basketball poll");

        // Prove that they normalized the matches from the DOM fallback correctly (and DOM fallback isn't returning empty matches)
        assert_eq!(football_events.len(), 1);
        assert_eq!(football_events[0].match_id.as_deref(), Some("foot-match-1"));

        assert_eq!(basketball_events.len(), 1);
        assert_eq!(
            basketball_events[0].match_id.as_deref(),
            Some("bask-match-1")
        );

        // Run coordinator for a short time using run with channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move { coordinator.run(shutdown_rx).await });

        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = shutdown_tx.send(());
        let res = handle.await.expect("task panicked");
        assert!(res.is_ok());
    }
}
