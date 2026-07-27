//! Runtime locations.
//!
//! Runtime data lives under the user's application-support directory with
//! user-only permissions. The socket and identity/database files are created in
//! user-only subdirectories and never inside a replaceable app bundle.

use std::path::PathBuf;

use overnight_core::{DomainError, Result};

/// `~/Library/Application Support/Overnight` on macOS.
pub fn runtime_dir() -> Result<PathBuf> {
    if let Ok(over) = std::env::var("OVERNIGHT_HOME") {
        return Ok(PathBuf::from(over));
    }
    let dirs = directories::ProjectDirs::from("com", "overnight", "Overnight")
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
    Ok(ensure_runtime_dir()?.join("overnight.db"))
}

/// The daemon-owned socket. Mode 0600 under a user-only directory.
pub fn socket_path() -> Result<PathBuf> {
    Ok(ensure_runtime_dir()?.join("overnightd.sock"))
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
    let path = install_id_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let id = uuid::Uuid::now_v7().simple().to_string();
    std::fs::write(&path, &id).map_err(|_| DomainError::OperationFailed)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overnight_home_overrides_the_default_location() {
        let tmp = std::env::temp_dir().join("overnight-paths-test");
        unsafe { std::env::set_var("OVERNIGHT_HOME", &tmp) };
        assert_eq!(runtime_dir().unwrap(), tmp);
        unsafe { std::env::remove_var("OVERNIGHT_HOME") };
    }

    #[test]
    fn install_id_is_stable_across_calls() {
        let tmp = std::env::temp_dir().join(format!("overnight-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        unsafe { std::env::set_var("OVERNIGHT_HOME", &tmp) };

        let a = load_or_create_install_id().unwrap();
        let b = load_or_create_install_id().unwrap();
        assert_eq!(a, b, "the install id must not change between runs");
        assert!(!a.is_empty());

        unsafe { std::env::remove_var("OVERNIGHT_HOME") };
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_dir_is_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("overnight-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        unsafe { std::env::set_var("OVERNIGHT_HOME", &tmp) };

        let dir = ensure_runtime_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "runtime dir must not be group or world readable");

        unsafe { std::env::remove_var("OVERNIGHT_HOME") };
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
