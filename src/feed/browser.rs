use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone)]
pub struct OwnedBrowserConfig {
    pub executable: Option<PathBuf>,
    pub startup_timeout: Duration,
    pub profile_dir: Option<PathBuf>,
}
impl Default for OwnedBrowserConfig {
    fn default() -> Self {
        Self {
            executable: None,
            startup_timeout: Duration::from_secs(15),
            profile_dir: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserHealth {
    Starting,
    Healthy,
    Stale,
    Disconnected,
    Stopped,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRole {
    Live,
    Detail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedTarget {
    pub target_id: String,
    pub websocket_url: String,
    pub role: TargetRole,
    pub sport_id: i32,
    pub session_id: String,
}
#[derive(Debug, Clone, Deserialize)]
struct DevtoolsTarget {
    id: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
    #[serde(rename = "type")]
    target_type: Option<String>,
}
#[derive(Default, Clone)]
pub struct TargetRegistry {
    inner: Arc<Mutex<HashMap<String, OwnedTarget>>>,
}
impl TargetRegistry {
    pub async fn owned(&self) -> Vec<OwnedTarget> {
        self.inner.lock().await.values().cloned().collect()
    }
    pub async fn insert(&self, target: OwnedTarget) {
        self.inner
            .lock()
            .await
            .insert(target.target_id.clone(), target);
    }
    pub async fn remove(&self, id: &str) {
        self.inner.lock().await.remove(id);
    }
}

pub struct OwnedBrowser {
    child: Arc<Mutex<Child>>,
    pub endpoint: String,
    pub profile_dir: PathBuf,
    pub session_id: String,
    pub pid: u32,
    pub registry: TargetRegistry,
    client: Client,
}
impl OwnedBrowser {
    pub async fn launch(
        config: OwnedBrowserConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let executable = config
            .executable
            .or_else(discover_executable)
            .ok_or("Chrome/Chromium executable not found")?;
        let profile = config.profile_dir.unwrap_or_else(|| {
            std::env::temp_dir().join(format!("earnscore-chrome-{}", uuid::Uuid::new_v4()))
        });
        std::fs::create_dir_all(&profile)?;
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))?
            .local_addr()?
            .port();
        let mut command = Command::new(executable);
        let stdout_file = std::fs::File::create(profile.join("chrome-stdout.log"))?;
        let stderr_file = std::fs::File::create(profile.join("chrome-stderr.log"))?;
        command
            .args([
                "--headless=new",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-gpu",
                "--no-sandbox",
                "--remote-debugging-address=127.0.0.1",
            ])
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(stdout_file)
            .stderr(stderr_file);
        let child = command.spawn()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let client = Client::new();
        let ready = timeout(config.startup_timeout, async {
            loop {
                if client
                    .get(format!("{endpoint}/json/version"))
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if ready.is_err() {
            let mut child = child;
            let _ = child.kill().await;
            let stderr_content = std::fs::read_to_string(profile.join("chrome-stderr.log"))
                .unwrap_or_else(|_| "could not read stderr".to_string());
            let stdout_content = std::fs::read_to_string(profile.join("chrome-stdout.log"))
                .unwrap_or_else(|_| "could not read stdout".to_string());
            let _ = std::fs::remove_dir_all(&profile);
            return Err(format!(
                "owned browser startup timeout. stdout: {}. stderr: {}",
                stdout_content, stderr_content
            )
            .into());
        }
        Ok(Self {
            pid: child.id().unwrap_or_default(),
            child: Arc::new(Mutex::new(child)),
            endpoint,
            profile_dir: profile,
            session_id: uuid::Uuid::new_v4().to_string(),
            registry: TargetRegistry::default(),
            client,
        })
    }
    pub async fn health(&self) -> BrowserHealth {
        if self.child.lock().await.try_wait().ok().flatten().is_some() {
            return BrowserHealth::Disconnected;
        }
        if self
            .client
            .get(format!("{}/json/version", self.endpoint))
            .send()
            .await
            .is_ok()
        {
            BrowserHealth::Healthy
        } else {
            BrowserHealth::Disconnected
        }
    }

    pub async fn recreate_target(
        &self,
        old: &OwnedTarget,
        url: &str,
    ) -> Result<OwnedTarget, Box<dyn std::error::Error + Send + Sync>> {
        let role = old.role;
        let sport_id = old.sport_id;
        let _ = self.close_target(old).await;
        self.create_target(role, sport_id, url).await
    }

    pub async fn connect_target(
        &self,
        target: &OwnedTarget,
    ) -> Result<CdpTarget, FeedBrowserError> {
        if !self
            .registry
            .owned()
            .await
            .iter()
            .any(|item| item.target_id == target.target_id)
        {
            return Err(FeedBrowserError::NotOwned);
        }
        CdpTarget::connect(&target.websocket_url).await
    }
    pub async fn create_target(
        &self,
        role: TargetRole,
        sport_id: i32,
        url: &str,
    ) -> Result<OwnedTarget, Box<dyn std::error::Error + Send + Sync>> {
        let response = self
            .client
            .put(format!("{}/json/new?{}", self.endpoint, url))
            .send()
            .await?;
        let target: DevtoolsTarget = response.json().await?;
        let ws = target
            .websocket_url
            .ok_or("owned target has no websocket URL")?;
        let owned = OwnedTarget {
            target_id: target.id,
            websocket_url: ws,
            role,
            sport_id,
            session_id: self.session_id.clone(),
        };
        self.registry.insert(owned.clone()).await;
        Ok(owned)
    }
    pub async fn close_target(
        &self,
        target: &OwnedTarget,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self
            .registry
            .owned()
            .await
            .iter()
            .any(|item| item.target_id == target.target_id)
        {
            self.client
                .get(format!("{}/json/close/{}", self.endpoint, target.target_id))
                .send()
                .await?;
            self.registry.remove(&target.target_id).await;
        }
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for target in self.registry.owned().await {
            let _ = self.close_target(&target).await;
        }
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        let _ = std::fs::remove_dir_all(&self.profile_dir);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedBrowserError {
    NotOwned,
    Connect(String),
    Protocol(String),
}
impl std::fmt::Display for FeedBrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FeedBrowserError {}

/// Minimal CDP transport used only for an owned feed target. It enables
/// Network first, while keeping store/DOM evaluation as explicit fallbacks.
pub struct CdpTarget {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    network_enabled: bool,
}

impl CdpTarget {
    pub async fn connect(url: &str) -> Result<Self, FeedBrowserError> {
        let (socket, _) = connect_async(url)
            .await
            .map_err(|error| FeedBrowserError::Connect(error.to_string()))?;
        let mut target = Self {
            socket,
            next_id: 1,
            network_enabled: false,
        };
        target
            .command("Network.enable", serde_json::json!({}))
            .await?;
        target
            .command("Runtime.enable", serde_json::json!({}))
            .await?;
        target.network_enabled = true;
        Ok(target)
    }

    async fn command(&mut self, method: &str, params: Value) -> Result<Value, FeedBrowserError> {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::Text(
                serde_json::json!({"id": id, "method": method, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
        while let Some(message) = self.socket.next().await {
            let message = message.map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
            if value["id"].as_u64() == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(FeedBrowserError::Protocol(error.to_string()));
                }
                return Ok(value["result"].clone());
            }
        }
        Err(FeedBrowserError::Protocol("CDP socket closed".into()))
    }

    pub async fn evaluate(&mut self, expression: &str) -> Result<Value, FeedBrowserError> {
        let result = self
            .command(
                "Runtime.evaluate",
                serde_json::json!({"expression": expression, "returnByValue": true}),
            )
            .await?;
        Ok(result["result"]["value"].clone())
    }

    pub async fn capture_fallbacks(
        &mut self,
    ) -> Result<Vec<crate::feed::adapters::SourceEnvelope>, FeedBrowserError> {
        let mut sources = Vec::new();
        let mut last_error = None;
        for (layer, expression) in [
            (
                crate::feed::adapters::SourceLayer::Store,
                "window.__EARN_SCORE_FEED_STORE__ || null",
            ),
            (
                crate::feed::adapters::SourceLayer::Dom,
                r#"(function() {
  const body = document.body;
  if (!body) return null;
  const activeFilter = body.dataset.activeFilter;
  const sportId = Number(body.dataset.sportId);
  if (!activeFilter || isNaN(sportId)) {
    return { activeFilter: null, sportId: null, matches: null };
  }

  const matchElements = Array.from(document.querySelectorAll('.match-item, .match-list-item, [data-match-id]'));
  if (matchElements.length === 0) {
    if (!document.querySelector('.match-list, .match-container, #match-list')) {
      return { activeFilter, sportId, matches: null };
    }
  }

  const matches = [];
  for (const el of matchElements) {
    const id = el.dataset.matchId || el.getAttribute('data-match-id') || el.id;
    const statusIdStr = el.dataset.statusId || el.getAttribute('data-status-id');
    const compId = el.dataset.competitionId || el.getAttribute('data-competition-id');
    const homeTeamId = el.dataset.homeTeamId || el.getAttribute('data-home-team-id');
    const awayTeamId = el.dataset.awayTeamId || el.getAttribute('data-away-team-id');

    if (!id || !statusIdStr || !compId || !homeTeamId || !awayTeamId) {
      return { activeFilter, sportId, matches: null };
    }

    const statusId = Number(statusIdStr);
    if (isNaN(statusId)) {
      return { activeFilter, sportId, matches: null };
    }

    let homeScores = [];
    let awayScores = [];
    try {
      if (el.dataset.homeScores) homeScores = JSON.parse(el.dataset.homeScores);
      if (el.dataset.awayScores) awayScores = JSON.parse(el.dataset.awayScores);
    } catch (_) {}

    let odds = null;
    try {
      if (el.dataset.odds) odds = JSON.parse(el.dataset.odds);
    } catch (_) {}

    matches.push({
      id: id,
      statusId: statusId,
      competition: { id: compId },
      homeTeam: { id: homeTeamId },
      awayTeam: { id: awayTeamId },
      homeScores: homeScores,
      awayScores: awayScores,
      odds: odds,
      period: el.dataset.period || el.getAttribute('data-period') || null,
      clock: el.dataset.clock || el.getAttribute('data-clock') || null
    });
  }

  return {
    activeFilter,
    sportId,
    matches
  };
})()"#,
            ),
        ] {
            let value = match self.evaluate(expression).await {
                Ok(value) => value,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            if value.is_object() && !value.is_null() {
                sources.push(crate::feed::adapters::SourceEnvelope::new(layer, value));
            }
        }
        if sources.is_empty() {
            return Err(last_error.unwrap_or_else(|| {
                FeedBrowserError::Protocol("no fallback source available".into())
            }));
        }
        Ok(sources)
    }

    pub async fn next_network_source(
        &mut self,
        wait: Duration,
    ) -> Result<Option<crate::feed::adapters::SourceEnvelope>, FeedBrowserError> {
        if !self.network_enabled {
            return Ok(None);
        }
        let message = timeout(wait, self.socket.next())
            .await
            .map_err(|_| FeedBrowserError::Protocol("network event timeout".into()))?
            .transpose()
            .map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
        let Some(Message::Text(text)) = message else {
            return Ok(None);
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
        if value["method"].as_str() != Some("Network.responseReceived") {
            return Ok(None);
        }
        let request_id = value["params"]["requestId"].as_str().unwrap_or_default();
        let body = self
            .command(
                "Network.getResponseBody",
                serde_json::json!({"requestId": request_id}),
            )
            .await?;
        let body = body["body"].as_str().unwrap_or_default();
        let payload: Value = serde_json::from_str(body)
            .map_err(|error| FeedBrowserError::Protocol(error.to_string()))?;
        Ok(Some(crate::feed::adapters::SourceEnvelope::new(
            crate::feed::adapters::SourceLayer::Network,
            payload,
        )))
    }
}
impl Drop for OwnedBrowser {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

fn discover_executable() -> Option<PathBuf> {
    let candidates = ["google-chrome", "chromium", "chromium-browser", "chrome"];
    candidates.iter().find_map(|name| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {name}"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.exists())
    })
}
