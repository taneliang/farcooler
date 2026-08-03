//! Runtime locations.
//!
//! Runtime data lives under the user's application-support directory with
//! user-only permissions. The socket and identity/database files are created in
//! user-only subdirectories and never inside a replaceable app bundle.

use std::path::PathBuf;

use farcooler_core::{DomainError, Result};

/// `~/Library/Application Support/Far Cooler` on macOS.
pub fn runtime_dir() -> Result<PathBuf> {
    if let Ok(over) = std::env::var("FARCOOLER_HOME") {
        return Ok(PathBuf::from(over));
    }
    let dirs = directories::ProjectDirs::from("com", "farcooler", "FarCooler")
        .ok_or(DomainError::OperationFailed)?;
    Ok(dirs.data_dir().to_path_buf())
}

/// Create the runtime directory with user-only permissions (0700).
pub fn ensure_runtime_dir() -> Result<PathBuf> {
    let dir = runtime_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        tracing::warn!(error = %e, "could not create runtime dir");
        DomainError::OperationFailed
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&dir)
            .map_err(|_| DomainError::OperationFailed)?
            .permissions();
        p.set_mode(0o700);
        let _ = std::fs::set_permissions(&dir, p);
    }

    Ok(dir)
}

pub fn database_path() -> Result<PathBuf> {
    Ok(ensure_runtime_dir()?.join("farcooler.db"))
}

/// The daemon-owned socket. Mode 0600 under a user-only directory.
pub fn socket_path() -> Result<PathBuf> {
    Ok(ensure_runtime_dir()?.join("farcoolerd.sock"))
}

pub fn install_id_path() -> Result<PathBuf> {
    Ok(ensure_runtime_dir()?.join("install-id"))
}

/// Where managed worktrees are created, one directory per workspace.
pub fn worktrees_dir() -> Result<PathBuf> {
    let d = ensure_runtime_dir()?.join("worktrees");
    std::fs::create_dir_all(&d).map_err(|_| DomainError::OperationFailed)?;
    Ok(d)
}

/// Stable per-install id, generated once.
pub fn load_or_create_install_id() -> Result<String> {
    load_or_create_install_id_in(&ensure_runtime_dir()?)
}

/// Explicit-directory form.
///
/// The environment is read only at the edge, so tests exercise this without
/// mutating a process-global variable that parallel tests would race on.
pub fn load_or_create_install_id_in(dir: &std::path::Path) -> Result<String> {
    let path = dir.join("install-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let id = uuid::Uuid::now_v7().simple().to_string();
    std::fs::create_dir_all(dir).map_err(|_| DomainError::OperationFailed)?;
    std::fs::write(&path, &id).map_err(|_| DomainError::OperationFailed)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir()
            .join(format!("farcooler-paths-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn install_id_is_stable_across_calls() {
        let dir = scratch("id");
        let a = load_or_create_install_id_in(&dir).unwrap();
        let b = load_or_create_install_id_in(&dir).unwrap();
        assert_eq!(a, b, "the install id must not change between runs");
        assert!(!a.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_different_install_gets_a_different_id() {
        let a = load_or_create_install_id_in(&scratch("id-a")).unwrap();
        let b = load_or_create_install_id_in(&scratch("id-b")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn an_empty_install_id_file_is_regenerated() {
        let dir = scratch("id-empty");
        std::fs::write(dir.join("install-id"), "   \n").unwrap();
        let id = load_or_create_install_id_in(&dir).unwrap();
        assert!(!id.trim().is_empty(), "a blank file must not yield a blank id");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
