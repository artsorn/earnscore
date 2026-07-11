use rusqlite::Connection;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum BackupError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Invalid(String),
    Verification(String),
}

impl Display for BackupError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "backup I/O error: {error}"),
            Self::Sql(error) => write!(formatter, "backup SQLite error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid backup request: {message}"),
            Self::Verification(message) => {
                write!(formatter, "backup verification failed: {message}")
            }
        }
    }
}

impl Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub type BackupResult<T> = std::result::Result<T, BackupError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupManifest {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size_bytes: u64,
    pub checksum: String,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupVerification {
    pub size_bytes: u64,
    pub checksum: String,
    pub integrity_ok: bool,
}

fn fnv1a_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn file_checksum(path: &Path) -> BackupResult<(u64, String)> {
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    let mut hash = 0xcbf29ce484222325_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hash = fnv1a_update(hash, &buffer[..read]);
    }
    Ok((size, format!("{hash:016x}")))
}

fn absolute(path: &Path) -> BackupResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn paths_are_same(first: &Path, second: &Path) -> BackupResult<bool> {
    let first_absolute = absolute(first)?;
    let second_absolute = absolute(second)?;
    if first_absolute == second_absolute {
        return Ok(true);
    }
    if first_absolute.exists() && second_absolute.exists() {
        let first_metadata = fs::metadata(first_absolute)?;
        let second_metadata = fs::metadata(second_absolute)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            return Ok(first_metadata.dev() == second_metadata.dev()
                && first_metadata.ino() == second_metadata.ino());
        }
        #[cfg(not(unix))]
        {
            return Ok(first_metadata.len() == second_metadata.len()
                && first_metadata.modified().ok() == second_metadata.modified().ok());
        }
    }
    Ok(false)
}

fn ensure_distinct(source: &Path, destination: &Path) -> BackupResult<()> {
    if paths_are_same(source, destination)? {
        return Err(BackupError::Invalid(
            "source and destination must be different files".to_string(),
        ));
    }
    Ok(())
}

fn verify_sqlite_file(path: &Path) -> BackupResult<()> {
    let conn = Connection::open(path)?;
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if !integrity.eq_ignore_ascii_case("ok") {
        return Err(BackupError::Verification(format!(
            "SQLite integrity_check returned {integrity}"
        )));
    }
    let mut foreign_keys = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = foreign_keys.query([])?;
    if rows.next()?.is_some() {
        return Err(BackupError::Verification(
            "SQLite foreign_key_check returned violations".to_string(),
        ));
    }
    Ok(())
}

fn temporary_path(destination: &Path) -> BackupResult<PathBuf> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| BackupError::Invalid("destination must have a file name".to_string()))?;
    Ok(parent.join(format!(".{file_name}.tmp-{}", uuid::Uuid::now_v7())))
}

fn sync_file(path: &Path) -> BackupResult<()> {
    let file = File::open(path)?;
    file.sync_all()?;
    Ok(())
}

fn replace_from_temporary(temporary: &Path, destination: &Path) -> BackupResult<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn verify_pair(source: &Path, copy: &Path) -> BackupResult<BackupVerification> {
    let (source_size, source_checksum) = file_checksum(source)?;
    let (copy_size, copy_checksum) = file_checksum(copy)?;
    if source_size != copy_size || source_checksum != copy_checksum {
        return Err(BackupError::Verification(format!(
            "size/checksum mismatch: source={source_size}/{source_checksum}, copy={copy_size}/{copy_checksum}"
        )));
    }
    verify_sqlite_file(copy)?;
    Ok(BackupVerification {
        size_bytes: copy_size,
        checksum: copy_checksum,
        integrity_ok: true,
    })
}

/// Create an atomic, verified copy of a SQLite database. The source is held
/// under an immediate transaction while it is copied so a concurrent writer
/// cannot produce a checksum that does not describe one consistent file.
pub fn create_verified_backup(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> BackupResult<BackupManifest> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    if !source.is_file() {
        return Err(BackupError::Invalid(format!(
            "source database does not exist: {}",
            source.display()
        )));
    }
    ensure_distinct(source, destination)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    let source_conn = Connection::open(source)?;
    source_conn.execute_batch("PRAGMA wal_checkpoint(FULL); BEGIN IMMEDIATE;")?;
    let temporary = temporary_path(destination)?;
    let result = (|| {
        fs::copy(source, &temporary)?;
        sync_file(&temporary)?;
        let verification = verify_pair(source, &temporary)?;
        source_conn.execute_batch("COMMIT;")?;
        replace_from_temporary(&temporary, destination)?;
        verify_sqlite_file(destination)?;
        Ok(BackupManifest {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            size_bytes: verification.size_bytes,
            checksum: verification.checksum,
            verified: verification.integrity_ok,
        })
    })();

    if result.is_err() {
        let _ = source_conn.execute_batch("ROLLBACK;");
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Verify a copy against its source, including byte size, checksum and SQLite
/// integrity. A truncated or corrupt destination fails closed.
pub fn verify_backup(
    source: impl AsRef<Path>,
    backup: impl AsRef<Path>,
) -> BackupResult<BackupVerification> {
    let source = source.as_ref();
    let backup = backup.as_ref();
    if !source.is_file() || !backup.is_file() {
        return Err(BackupError::Invalid(
            "both source and backup must be regular files".to_string(),
        ));
    }
    ensure_distinct(source, backup)?;
    verify_pair(source, backup)
}

/// Verify a standalone backup when the original source is unavailable.
pub fn verify_backup_file(backup: impl AsRef<Path>) -> BackupResult<BackupVerification> {
    let backup = backup.as_ref();
    if !backup.is_file() {
        return Err(BackupError::Invalid(format!(
            "backup does not exist: {}",
            backup.display()
        )));
    }
    let (size_bytes, checksum) = file_checksum(backup)?;
    verify_sqlite_file(backup)?;
    Ok(BackupVerification {
        size_bytes,
        checksum,
        integrity_ok: true,
    })
}

/// Restore only after validating a temporary copy. The destination is not
/// replaced when checksum or SQLite integrity verification fails.
pub fn restore_verified_backup(
    backup: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> BackupResult<BackupManifest> {
    let backup = backup.as_ref();
    let destination = destination.as_ref();
    if !backup.is_file() {
        return Err(BackupError::Invalid(format!(
            "backup does not exist: {}",
            backup.display()
        )));
    }
    ensure_distinct(backup, destination)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary = temporary_path(destination)?;
    let result = (|| {
        fs::copy(backup, &temporary)?;
        sync_file(&temporary)?;
        let verification = verify_backup_file(&temporary)?;
        replace_from_temporary(&temporary, destination)?;
        verify_sqlite_file(destination)?;
        Ok(BackupManifest {
            source: backup.to_path_buf(),
            destination: destination.to_path_buf(),
            size_bytes: verification.size_bytes,
            checksum: verification.checksum,
            verified: verification.integrity_ok,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn create_backup(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> BackupResult<BackupManifest> {
    create_verified_backup(source, destination)
}

pub fn restore_backup(
    backup: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> BackupResult<BackupManifest> {
    restore_verified_backup(backup, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Write;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{label}-{}.db", uuid::Uuid::now_v7()))
    }

    fn database(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO sample (value) VALUES ('kept');",
        )
        .unwrap();
    }

    #[test]
    fn backup_verification_round_trip() {
        let source = temp_path("backup-source");
        let backup = temp_path("backup-copy");
        database(&source);
        let manifest = create_verified_backup(&source, &backup).unwrap();
        assert!(manifest.verified);
        assert!(verify_backup(&source, &backup).unwrap().integrity_ok);
        let restored = temp_path("backup-restored");
        restore_verified_backup(&backup, &restored).unwrap();
        let conn = Connection::open(&restored).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "kept");
        let _ = fs::remove_file(source);
        let _ = fs::remove_file(backup);
        let _ = fs::remove_file(restored);
    }

    #[test]
    fn truncated_backup_fails_closed() {
        let source = temp_path("backup-truncate-source");
        let backup = temp_path("backup-truncate-copy");
        database(&source);
        create_verified_backup(&source, &backup).unwrap();
        let original = fs::read(&backup).unwrap();
        let mut file = File::create(&backup).unwrap();
        file.write_all(&original[..original.len() / 2]).unwrap();
        file.sync_all().unwrap();
        assert!(verify_backup(&source, &backup).is_err());
        assert!(verify_backup_file(&backup).is_err());
        let _ = fs::remove_file(source);
        let _ = fs::remove_file(backup);
    }

    #[test]
    fn backup_rejects_same_destination() {
        let source = temp_path("backup-same-source");
        database(&source);
        assert!(create_verified_backup(&source, &source).is_err());
        let _ = fs::remove_file(source);
    }
}
