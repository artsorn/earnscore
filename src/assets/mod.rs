pub mod download;
pub mod store;
pub mod types;

use crate::detail::types::ImageCandidate;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AssetWorkerConfig {
    pub concurrency_limit: usize,
    pub download_delay: Duration,
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub asset_root: String,
    pub max_size_bytes: usize,
}

impl Default for AssetWorkerConfig {
    fn default() -> Self {
        Self {
            concurrency_limit: 3,
            download_delay: Duration::from_millis(500),
            timeout: Duration::from_secs(10),
            max_retries: 3,
            retry_delay: Duration::from_secs(2),
            asset_root: "data/assets".to_string(),
            max_size_bytes: 10 * 1024 * 1024, // 10MB
        }
    }
}

pub struct AssetCoordinator {
    pub db_path: String,
    pub config: AssetWorkerConfig,
    client: Client,
    semaphore: Arc<Semaphore>,
    request_gate: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

impl AssetCoordinator {
    pub fn new(db_path: String, config: AssetWorkerConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit.max(1)));
        Self {
            db_path,
            config,
            client: Client::builder()
                .user_agent("EarnScoreCrawler/3.0")
                .build()
                .unwrap_or_default(),
            semaphore,
            request_gate: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Process a batch of image candidates for a detail section.
    /// Downloads them concurrently using the semaphore, obeying delay settings,
    /// and returns the download results keyed by candidate index.
    pub async fn process_candidates(
        &self,
        candidates: &[ImageCandidate],
    ) -> std::collections::HashMap<usize, Result<download::DownloadedAsset, String>> {
        if candidates.is_empty() {
            return std::collections::HashMap::new();
        }

        let mut tasks = tokio::task::JoinSet::new();
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let client = self.client.clone();
            let url = candidate.url.clone();
            let max_size = self.config.max_size_bytes;
            let timeout = self.config.timeout;
            let max_retries = self.config.max_retries;
            let retry_delay = self.config.retry_delay;
            let download_delay = self.config.download_delay;
            let sem = self.semaphore.clone();
            let request_gate = self.request_gate.clone();

            tasks.spawn(async move {
                // Enforce download concurrency limit via semaphore
                let _permit = sem.acquire().await.ok();

                // Space request starts globally while still allowing bounded
                // request concurrency.
                let mut last_request = request_gate.lock().await;
                if let Some(previous) = *last_request {
                    let next = previous + download_delay;
                    tokio::time::sleep_until(next.into()).await;
                }
                *last_request = Some(std::time::Instant::now());
                drop(last_request);

                let res = download::download_asset(
                    &client,
                    &url,
                    max_size,
                    timeout,
                    max_retries,
                    retry_delay,
                )
                .await;
                (candidate_index, res)
            });
        }

        let mut results = std::collections::HashMap::with_capacity(candidates.len());
        while let Some(res) = tasks.join_next().await {
            if let Ok((candidate_index, download_res)) = res {
                results.insert(candidate_index, download_res);
            }
        }

        results
    }
}
