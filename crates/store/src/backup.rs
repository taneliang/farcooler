//! Pre-migration backups.
//!
//! A migration rewrites schema in place. Before it runs against a database
//! that might already hold real data, this writes an untouched copy next to
//! it plus a checksum, so a bad migration is recoverable instead of silently
//! destructive. Fresh, never-migrated databases skip this: there is nothing
//! yet that migrating could destroy.

use std::fs;
use std::path::{Path, PathBuf};

use farcooler_core::{DomainError, Result};

/// FNV-1a 64-bit. Not cryptographic; this only has to catch a truncated or
/// corrupted copy, not an adversary, and needs no dependency beyond the
/// standard library.
fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    data.iter().fold(OFFSET, |hash, &b| (hash ^ b as u64).wrapping_mul(PRIME))
}

fn checksum_sidecar(backup_path: &Path) -> PathBuf {
    let mut p = backup_path.as_os_str().to_owned();
    p.push(".checksum");
    PathBuf::from(p)
}

/// Copies `path` to `<path>.bak-v<from_schema_version>` and writes a
/// `<...>.checksum` sidecar holding the hex FNV-1a digest, so a restore can be
/// verified byte for byte before anyone trusts it.
pub(crate) fn write_checksummed_backup(path: &Path, from_schema_version: u32) -> Result<PathBuf> {
    let bytes = fs::read(path).map_err(|_| DomainError::OperationFailed)?;
    let checksum = fnv1a64(&bytes);

    let mut backup_path = path.as_os_str().to_owned();
    backup_path.push(format!(".bak-v{from_schema_version}"));
    let backup_path = PathBuf::from(backup_path);

    fs::write(&backup_path, &bytes).map_err(|_| DomainError::OperationFailed)?;
    fs::write(checksum_sidecar(&backup_path), format!("{checksum:016x}"))
        .map_err(|_| DomainError::OperationFailed)?;

    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farcooler-store-backup-{}", Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn backup_is_byte_identical_and_checksum_verifies() {
        let dir = scratch_dir();
        let db_path = dir.join("db.sqlite3");
        fs::write(&db_path, b"pretend sqlite file contents").unwrap();

        let backup_path = write_checksummed_backup(&db_path, 1).unwrap();
        let original = fs::read(&db_path).unwrap();
        let copy = fs::read(&backup_path).unwrap();
        assert_eq!(original, copy, "backup must be byte identical to the source");

        let recorded = fs::read_to_string(checksum_sidecar(&backup_path)).unwrap();
        assert_eq!(recorded, format!("{:016x}", fnv1a64(&original)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backup_of_missing_file_fails_cleanly() {
        let dir = scratch_dir();
        let missing = dir.join("does-not-exist.sqlite3");
        let err = write_checksummed_backup(&missing, 1).unwrap_err();
        assert!(matches!(err, DomainError::OperationFailed));
        fs::remove_dir_all(&dir).ok();
    }
}
