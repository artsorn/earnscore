use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Calculate the SHA-256 hash of a byte slice and return it as a hex string.
pub fn calculate_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Map a MIME type to a file extension.
pub fn mime_to_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// Atomically write bytes under `{asset_root}/tmp` and publish to final destination.
pub fn publish_asset_file(
    asset_root: &str,
    entity_type: &str,
    entity_id: &str,
    hash: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    validate_component(entity_type, "entity type")?;
    validate_component(entity_id, "entity id")?;
    validate_component(hash, "content hash")?;
    validate_component(ext, "extension")?;

    let root = Path::new(asset_root);
    let tmp_dir = root.join("tmp");
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("Failed to create tmp dir: {}", e))?;

    // Create a unique temporary filename
    let tmp_filename = format!("tmp_{}_{}_{}", entity_type, entity_id, uuid::Uuid::new_v4());
    let tmp_file_path = tmp_dir.join(tmp_filename);

    let mut tmp_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_file_path)
        .map_err(|e| format!("Failed to create temporary file: {}", e))?;
    tmp_file
        .write_all(bytes)
        .and_then(|_| tmp_file.sync_all())
        .map_err(|e| format!("Failed to write temporary file: {}", e))?;

    // Construct destination path: {asset_root}/{entity_type}/{entity_id}/{hash}.{ext}
    let target_dir = root.join(entity_type).join(entity_id);
    fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create target dir: {}", e))?;

    let target_file_path = target_dir.join(format!("{}.{}", hash, ext));

    // Never replace a published content-addressed file.  A same-content race
    // is safe to reuse; a different-content collision is corruption.
    if target_file_path.exists() {
        let existing = fs::read(&target_file_path)
            .map_err(|e| format!("Failed to read existing asset: {}", e))?;
        let _ = fs::remove_file(&tmp_file_path);
        if calculate_sha256(&existing) == hash && existing == bytes {
            return Ok(target_file_path);
        }
        return Err("Existing asset path contains different bytes".to_string());
    }

    if let Err(error) = fs::rename(&tmp_file_path, &target_file_path) {
        if target_file_path.exists() {
            let existing = fs::read(&target_file_path)
                .map_err(|e| format!("Failed to read raced asset: {}", e))?;
            let _ = fs::remove_file(&tmp_file_path);
            if calculate_sha256(&existing) == hash && existing == bytes {
                return Ok(target_file_path);
            }
        } else {
            let _ = fs::remove_file(&tmp_file_path);
        }
        return Err(format!(
            "Failed to atomically rename temporary file to target path: {}",
            error
        ));
    }

    Ok(target_file_path)
}

fn validate_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("Invalid {} path component", label));
    }
    Ok(())
}

/// Recursively clean up the temporary folder `{asset_root}/tmp` under the asset root.
pub fn clean_temp_assets(asset_root: &str) -> std::io::Result<()> {
    let tmp_dir = Path::new(asset_root).join("tmp");
    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir)?;
        fs::create_dir_all(&tmp_dir)?;
    }
    Ok(())
}
