# Ephemeral Asset Pipeline and Local Storage Layout

This document describes the design, storage layout, legacy conversion process, and troubleshooting instructions for the EarnScore local asset pipeline.

## 1. Storage Layout

All downloaded and validated image assets are stored locally under the configured asset root directory (default: `data/assets`).

- **Base Directory**: `data/assets`
- **Temporary Downloads**: `data/assets/tmp/`
  - Temporary files are created with exclusive names and flushed before publication.
  - Temporary files are deleted recursively on application startup to ensure they never leak or consume disk space permanently.
- **Published Assets**: `data/assets/{entity_type}/{entity_id}/{sha256_hash}.{extension}`
  - `entity_type` corresponds to the detail section name or domain type (e.g. `team`, `competition`, `lineups`).
  - `entity_id` is the ID of the entity that owns the asset.
  - `sha256_hash` is the SHA-256 hash of the verified image bytes.
  - `extension` is mapped from the verified MIME type (e.g. `.png`, `.jpg`, `.gif`, `.webp`, `.svg`).

## 2. Permissions and Path Management

- The application must have read and write permissions to the configured asset root directory.
- Parent directories (including temporary and sub-entity folders) are created dynamically and recursively before file publication.
- Each path component is restricted to ASCII letters, digits, `.`, `_`, and `-`; empty, `.`/`..`, and separator-containing components are rejected.
- A temporary file is flushed and atomically renamed. An existing content-addressed path is reused only when its bytes match the requested SHA-256 hash.

## 3. Deduplication and Owner Links

- Deduplication is content-addressed:
  1. Image bytes are downloaded and checked.
  2. The SHA-256 hash is computed.
  3. If the hash already exists in the `assets` table, the existing physical file is reused.
  4. Only the new link is created in the `asset_links` table mapping the asset to the requesting entity, role, and dataset.
- This ensures that duplicate images (such as identical team logos or player avatars) are stored only once on disk.
- The database stores metadata, links, and an `ASSET_UPLOAD_INTENT` outbox record; source URLs are never included in those persisted records.
- The migration verifier scans every text column in the active SQLite database after conversion. It reports only `table.column` locations containing `http://` or `https://`, never the URL itself, so diagnostics cannot reintroduce source URLs into logs.

## 4. Legacy Asset Conversion

During database migrations (triggered via the `migrate` command), a one-time conversion process is executed:
1. Logo URLs starting with `http://` or `https://` are read from the active `competitions` and `teams` tables.
2. The images are downloaded, validated, and stored in the asset pipeline.
3. The active remote URLs are replaced only after verified local publication with the internal reference (e.g., `asset-<sha256>`), or with `"asset-unavailable"` after a verified failure.
4. The original remote URLs are preserved in the verified backup created prior to the migration.
5. After conversion, run the source-URL verifier. Any reported location must be cleared, replaced with an `asset-<sha256>` reference, or explicitly set to `asset-unavailable` before treating the active database as converted.

## 5. Troubleshooting & Recovery from Partial Files

- **Orphaned / Partial Files**:
  - In the event of a crash or interruption during download, files in the `tmp/` folder might remain.
  - These are automatically cleaned up on next restart via `clean_temp_assets`.
- **Atomic Renames**:
  - Assets are written to the `tmp/` folder first and then moved to the final path using `std::fs::rename`.
  - On POSIX filesystems, this rename is atomic, preventing partial or corrupted files from being visible as final assets.
- **Request safety**:
  - Downloads accept only successful responses with an allowlisted image MIME type, bounded body size, and matching image bytes.
  - Concurrency, request-start delay, timeout, and retry limits are configurable through the asset worker settings.
