pub mod download;
pub mod store;
pub mod types;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use reqwest::Client;
use crate::detail::types::ImageCandidate;

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
}

impl AssetCoordinator {
    pub fn new(db_path: String, config: AssetWorkerConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        Self {
            db_path,
            config,
            client: Client::builder()
                .user_agent("EarnScoreCrawler/3.0")
                .build()
                .unwrap_or_default(),
            semaphore,
        }
    }

    /// Process a batch of image candidates for a detail section.
    /// Downloads them concurrently using the semaphore, obeying delay settings,
    /// and returns the download results mapped by their URL.
    pub async fn process_candidates(
        &self,
        candidates: &[ImageCandidate],
    ) -> std::collections::HashMap<String, Result<download::DownloadedAsset, String>> {
        let mut results = std::collections::HashMap::new();
        if candidates.is_empty() {
            return results;
        }

        let mut tasks = tokio::task::JoinSet::new();
        for candidate in candidates {
            let client = self.client.clone();
            let url = candidate.url.clone();
            let max_size = self.config.max_size_bytes;
            let timeout = self.config.timeout;
            let max_retries = self.config.max_retries;
            let retry_delay = self.config.retry_delay;
            let download_delay = self.config.download_delay;
            let sem = self.semaphore.clone();

            tasks.spawn(async move {
                // Enforce download concurrency limit via semaphore
                let _permit = sem.acquire().await.ok();
                
                // Enforce configured delay between requests
                if !download_delay.is_zero() {
                    sleep(download_delay).await;
                }

                let res = download::download_asset(&client, &url, max_size, timeout, max_retries, retry_delay).await;
                (url, res)
            });
        }

        while let Some(res) = tasks.join_next().await {
            if let Ok((url, download_res)) = res {
                results.insert(url, download_res);
            }
        }

        results
    }
}
