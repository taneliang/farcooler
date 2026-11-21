//! Runtime locations.
//!
//! Runtime data lives under the user's application-support directory with
//! user-only permissions. The socket and identity/database files are created in
//! user-only subdirectories and never inside a replaceable app bundle.

use std::path::PathBuf;

use farcooler_core::{DomainError, Result};

pub use farcooler_daemon_contracts::paths::{MAX_SOCKET_PATH, RuntimeSelection, database_path, default_runtime_dir_for,
    ensure_runtime_dir, load_or_create_install_id_in, runtime_dir, runtime_dir_for, selection, socket_path};

/// Where managed worktrees are created, one directory per worktree.
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

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::Channel;

    const ALL: [Channel; 4] =
        [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable];

    #[test]
    fn runtime_selection_explicit_default_and_pre_effect_refusal() {
        let channel = Channel::Canary;
        let default = RuntimeSelection::from_override(channel, None).unwrap();
        assert!(!default.is_explicit());
        assert_eq!(default.root(), default_runtime_dir_for(channel).unwrap());
        let fixture = tempfile::Builder::new().prefix("ov937-").tempdir_in("/tmp").unwrap();
        let root = fixture.path().canonicalize().unwrap().join("uncreated");
        let selected = RuntimeSelection::from_override(channel, Some(root.clone().into_os_string())).unwrap();
        assert!(selected.is_explicit());
        assert_eq!(selected.root(), root);
        assert_eq!(crate::daemon_log::log_dir_for(&selected), Some(root.join("logs")));
        assert!(!root.exists(), "selection must have no filesystem effects");
        for value in ["", "relative", "/", "/tmp/../bad", "/tmp/./bad", "/tmp//bad", "/tmp/bad/", "/tmp/bad\0", "/tmp/bad\n"] {
            assert!(RuntimeSelection::from_override(channel, Some(value.into())).is_err(), "{value:?}");
        }
        assert!(RuntimeSelection::from_override(channel, Some(format!("/{}", "x".repeat(MAX_SOCKET_PATH)).into())).is_err());
        #[cfg(unix)] {
            use std::os::unix::ffi::OsStringExt;
            assert!(RuntimeSelection::from_override(channel, Some(std::ffi::OsString::from_vec(vec![b'/', 255]))).is_err());
        }
        assert!(!root.exists());
    }

    #[test]
    fn runtime_selection_refuses_before_socket_directory_effects() {
        let parent = scratch("rs-effects");
        let invalid = parent.join("new/../escape");
        let result = RuntimeSelection::from_override(Channel::Canary, Some(invalid.into_os_string()))
            .and_then(|selected| selected.socket_path());
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0, "no root, log, socket, lock or Store effects");
        std::fs::remove_dir(parent).unwrap();
    }

    /// The isolation everything else rests on.
    ///
    /// These read no environment and touch no disk, so they are safe to run in
    /// parallel with every other test in this crate — which matters, because
    /// `FARCOOLER_HOME` is process-global and a test that set it would move the
    /// ground under the ones running beside it.
    #[test]
    fn each_channel_gets_its_own_runtime_directory() {
        let dirs: Vec<_> = ALL.iter().map(|c| default_runtime_dir_for(*c).unwrap()).collect();
        let unique: std::collections::BTreeSet<_> = dirs.iter().collect();
        assert_eq!(unique.len(), ALL.len(), "two channels would share a database: {dirs:?}");
    }

    #[test]
    fn a_channel_directory_is_a_sibling_never_a_child() {
        // Nesting one inside another would mean deleting the outer install
        // takes the inner one's database with it.
        for a in ALL {
            for b in ALL {
                if a == b {
                    continue;
                }
                let (x, y) = (default_runtime_dir_for(a).unwrap(), default_runtime_dir_for(b).unwrap());
                assert!(!x.starts_with(&y), "{x:?} must not sit inside {y:?}");
            }
        }
    }

    #[test]
    fn stable_keeps_the_directory_it_has_always_had() {
        // An existing install must not move. If this ever changes, every
        // worktree and terminal a user already has disappears on upgrade.
        let release = default_runtime_dir_for(Channel::Stable).unwrap();
        let historic = directories::ProjectDirs::from("com", "farcooler", "FarCooler")
            .unwrap()
            .data_dir()
            .to_path_buf();
        assert_eq!(release, historic);
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        farcooler_core::scratch::dir(&format!("paths-{tag}"))
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
