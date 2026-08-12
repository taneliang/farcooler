//! Runtime locations.
//!
//! Runtime data lives under the user's application-support directory with
//! user-only permissions. The socket and identity/database files are created in
//! user-only subdirectories and never inside a replaceable app bundle.

use std::path::PathBuf;

use farcooler_core::{DomainError, Result};

/// `~/Library/Application Support/com.farcooler.FarCooler` on macOS, for a
/// release build.
pub fn runtime_dir() -> Result<PathBuf> {
    runtime_dir_for(farcooler_protocol::CHANNEL)
}

/// Where a given channel's install lives.
///
/// This is the seam the whole channel design turns on. Everything else the
/// daemon owns is a path under this one — the socket, the database, the
/// install-id and therefore the tmux server (`tmux -L farcooler-<install-id>`),
/// the managed worktrees, the pastes — so choosing a directory per channel
/// separates all of it at once.
///
/// Three application names rather than one name with a channel subdirectory,
/// so the three are siblings: nesting a beta inside the release directory
/// would mean deleting a release install takes the beta's database with it.
///
/// Release's name is what it was before channels existed and must stay that
/// way. An existing user's entire fleet lives under it.
///
/// `FARCOOLER_HOME` still overrides everything and is checked first: it is how
/// tests and scratch daemons get an isolated home, and the channel is only
/// consulted when nobody has named a directory outright.
pub fn runtime_dir_for(channel: farcooler_protocol::Channel) -> Result<PathBuf> {
    use farcooler_protocol::Channel;
    if let Ok(over) = std::env::var("FARCOOLER_HOME") {
        return Ok(PathBuf::from(over));
    }
    let app = match channel {
        Channel::Release => "FarCooler",
        Channel::Beta => "FarCoolerBeta",
        Channel::Dev => "FarCoolerDev",
    };
    let dirs = directories::ProjectDirs::from("com", "farcooler", app)
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

/// Where images pasted into a terminal are written, one file per paste.
///
/// Under the runtime directory rather than the worktree, so a paste never
/// appears in anyone's `git status`, and so one sweep owns every one of them.
///
/// Takes the root rather than reading `FARCOOLER_HOME`, for the reason
/// `Service.root` is held rather than re-derived: the environment is
/// process-global, and two tests running in parallel would otherwise write into
/// whichever directory was set last — or, in production, into the real one.
pub fn pastes_dir_in(root: &std::path::Path) -> Result<PathBuf> {
    let d = root.join("pastes");
    std::fs::create_dir_all(&d).map_err(|_| DomainError::OperationFailed)?;
    Ok(d)
}

/// Where a paste's bytes accumulate before it is complete.
///
/// A dotted subdirectory of `pastes_dir_in`, so the sweep can find partials
/// without walking anywhere else, and so nothing reading the paste directory
/// for finished images ever sees one that is still arriving.
pub fn pastes_incoming_dir_in(root: &std::path::Path) -> Result<PathBuf> {
    let d = pastes_dir_in(root)?.join(".incoming");
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
    use farcooler_protocol::Channel;

    /// The isolation everything else rests on.
    ///
    /// These read no environment and touch no disk, so they are safe to run in
    /// parallel with every other test in this crate — which matters, because
    /// `FARCOOLER_HOME` is process-global and a test that set it would move the
    /// ground under the ones running beside it.
    #[test]
    fn each_channel_gets_its_own_runtime_directory() {
        let dev = runtime_dir_for(Channel::Dev).unwrap();
        let beta = runtime_dir_for(Channel::Beta).unwrap();
        let release = runtime_dir_for(Channel::Release).unwrap();
        assert_ne!(dev, beta);
        assert_ne!(beta, release);
        assert_ne!(dev, release);
    }

    #[test]
    fn a_channel_directory_is_a_sibling_never_a_child() {
        // Nesting beta inside release would mean deleting a release install
        // deletes the beta's database with it.
        let beta = runtime_dir_for(Channel::Beta).unwrap();
        let release = runtime_dir_for(Channel::Release).unwrap();
        assert!(!beta.starts_with(&release), "{beta:?} must not sit inside {release:?}");
        assert!(!release.starts_with(&beta), "{release:?} must not sit inside {beta:?}");
    }

    #[test]
    fn release_keeps_the_directory_it_has_always_had() {
        // An existing install must not move. If this ever changes, every
        // workspace and terminal a user already has disappears on upgrade.
        let release = runtime_dir_for(Channel::Release).unwrap();
        let historic = directories::ProjectDirs::from("com", "farcooler", "FarCooler")
            .unwrap()
            .data_dir()
            .to_path_buf();
        assert_eq!(release, historic);
    }

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
