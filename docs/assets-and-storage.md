# Ephemeral Asset Pipeline and Local Storage Layout

This document describes the design, storage layout, legacy conversion process, and troubleshooting instructions for the EarnScore local asset pipeline.

## 1. Storage Layout

All downloaded and validated image assets are stored locally under the configured asset root directory (default: `data/assets`).

- **Base Directory**: `data/assets`
- **Temporary Downloads**: `data/assets/tmp/`
  - Temporary files are named uniquely using UUIDs: `tmp_{entity_type}_{entity_id}_{uuid}`.
  - Temporary files are deleted recursively on application startup to ensure they never leak or consume disk space permanently.
- **Published Assets**: `data/assets/{entity_type}/{entity_id}/{sha256_hash}.{extension}`
  - `entity_type` corresponds to the detail section name or domain type (e.g. `team`, `competition`, `lineups`).
  - `entity_id` is the ID of the entity that owns the asset.
  - `sha256_hash` is the SHA-256 hash of the verified image bytes.
  - `extension` is mapped from the verified MIME type (e.g. `.png`, `.jpg`, `.gif`, `.webp`, `.svg`).

## 2. Permissions and Path Management

- The application must have read and write permissions to the configured asset root directory.
- Parent directories (including temporary and sub-entity folders) are created dynamically and recursively with appropriate permissions before file publication.
- Path normalization is enforced using absolute normalized paths to prevent directory traversal vulnerabilities.

## 3. Deduplication and Owner Links

- Deduplication is content-addressed:
  1. Image bytes are downloaded and checked.
  2. The SHA-256 hash is computed.
  3. If the hash already exists in the `assets` table, the existing physical file is reused.
  4. Only the new link is created in the `asset_links` table mapping the asset to the requesting entity, role, and dataset.
- This ensures that duplicate images (such as identical team logos or player avatars) are stored only once on disk.

## 4. Legacy Asset Conversion

During database migrations (triggered via the `migrate` command), a one-time conversion process is executed:
1. Logo URLs starting with `http://` or `https://` are read from the active `competitions` and `teams` tables.
2. The images are downloaded, validated, and stored in the asset pipeline.
3. The active remote URLs are cleared and replaced with the new internal reference (e.g., `asset-xxxx`) or `"asset-unavailable"` if download failed.
4. The original remote URLs are preserved in the verified backup created prior to the migration.

## 5. Troubleshooting & Recovery from Partial Files

- **Orphaned / Partial Files**:
  - In the event of a crash or interruption during download, files in the `tmp/` folder might remain.
  - These are automatically cleaned up on next restart via `clean_temp_assets`.
- **Atomic Renames**:
  - Assets are written to the `tmp/` folder first and then moved to the final path using `std::fs::rename`.
  - On POSIX filesystems, this rename is atomic, preventing partial or corrupted files from being visible as final assets.
