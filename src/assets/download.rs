use std::time::Duration;
use reqwest::Client;

/// The result of a successful bounded asset download.
pub struct DownloadedAsset {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Validate image bytes based on MIME type signatures.
pub fn validate_image_bytes(bytes: &[u8], mime_type: &str) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("Empty bytes".to_string());
    }
    match mime_type {
        "image/png" => {
            if bytes.len() >= 8 && bytes[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
                Ok(())
            } else {
                Err("Invalid PNG magic bytes".to_string())
            }
        }
        "image/jpeg" | "image/jpg" => {
            if bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF] {
                Ok(())
            } else {
                Err("Invalid JPEG magic bytes".to_string())
            }
        }
        "image/gif" => {
            if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
                Ok(())
            } else {
                Err("Invalid GIF magic bytes".to_string())
            }
        }
        "image/webp" => {
            if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
                Ok(())
            } else {
                Err("Invalid WEBP magic bytes".to_string())
            }
        }
        "image/svg+xml" => {
            let s = std::str::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 for SVG: {}", e))?;
            if s.contains("<svg") {
                Ok(())
            } else {
                Err("SVG does not contain '<svg' tag".to_string())
            }
        }
        _ => Err(format!("Unsupported MIME type: {}", mime_type)),
    }
}

/// Download an asset from a source URL with configured size limit, timeout, and retry.
pub async fn download_asset(
    client: &Client,
    url: &str,
    max_size_bytes: usize,
    timeout: Duration,
    max_retries: u32,
    retry_delay: Duration,
) -> Result<DownloadedAsset, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match download_asset_once(client, url, max_size_bytes, timeout).await {
            Ok(asset) => return Ok(asset),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(format!("Failed after {} attempts. Last error: {}", attempt, e));
                }
                tokio::time::sleep(retry_delay).await;
            }
        }
    }
}

async fn download_asset_once(
    client: &Client,
    url: &str,
    max_size_bytes: usize,
    timeout: Duration,
) -> Result<DownloadedAsset, String> {
    let res = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("HTTP status error: {}", res.status()));
    }

    let mime_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_lowercase())
        .ok_or_else(|| "Missing Content-Type header".to_string())?;

    if !matches!(
        mime_type.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/svg+xml"
    ) {
        return Err(format!("Unsupported or wrong Content-Type: {}", mime_type));
    }

    if let Some(len) = res.content_length() {
        if len > max_size_bytes as u64 {
            return Err(format!("Content-Length {} exceeds limit {}", len, max_size_bytes));
        }
    }

    let mut body = res;
    let mut bytes = Vec::new();
    while let Some(chunk) = body
        .chunk()
        .await
        .map_err(|e| format!("Error reading body chunk: {}", e))?
    {
        if bytes.len() + chunk.len() > max_size_bytes {
            return Err(format!("Response body size exceeded maximum limit of {}", max_size_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }

    validate_image_bytes(&bytes, &mime_type)?;

    let (width, height) = match imagesize::blob_size(&bytes) {
        Ok(size) => (Some(size.width as i32), Some(size.height as i32)),
        Err(_) => (None, None),
    };

    Ok(DownloadedAsset {
        bytes,
        mime_type,
        width,
        height,
    })
}
