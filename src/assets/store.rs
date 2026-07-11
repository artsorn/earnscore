use std::path::{Path, PathBuf};
use std::fs;
use sha2::{Sha256, Digest};

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
    let root = Path::new(asset_root);
    let tmp_dir = root.join("tmp");
    fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("Failed to create tmp dir: {}", e))?;

    // Create a unique temporary filename
    let tmp_filename = format!("tmp_{}_{}_{}", entity_type, entity_id, uuid::Uuid::new_v4());
    let tmp_file_path = tmp_dir.join(tmp_filename);

    fs::write(&tmp_file_path, bytes)
        .map_err(|e| format!("Failed to write temporary file: {}", e))?;

    // Construct destination path: {asset_root}/{entity_type}/{entity_id}/{hash}.{ext}
    let target_dir = root.join(entity_type).join(entity_id);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create target dir: {}", e))?;

    let target_file_path = target_dir.join(format!("{}.{}", hash, ext));

    // Atomically rename
    if let Err(e) = fs::rename(&tmp_file_path, &target_file_path) {
        let _ = fs::remove_file(&tmp_file_path);
        return Err(format!("Failed to atomically rename temporary file to target path: {}", e));
    }

    Ok(target_file_path)
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
