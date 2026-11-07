//! Runtime locations.
//!
//! Runtime data lives under the user's application-support directory with
//! user-only permissions. The socket and identity/database files are created in
//! user-only subdirectories and never inside a replaceable app bundle.

use std::path::PathBuf;

use farcooler_core::{DomainError, Result};

/// The longest path `bind` will accept for a Unix socket, minus a byte for
/// the NUL: `sun_path` is 104 bytes on macOS. The default runtime directory
/// is already 66 of them, so every socket the daemon binds keeps its own name
/// short.
pub const MAX_SOCKET_PATH: usize = 103;

/// `~/Library/Application Support/com.farcooler.FarCooler` on macOS, for a
/// release build.
pub fn runtime_dir() -> Result<PathBuf> {
    Ok(selection()?.root.clone())
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
    Ok(RuntimeSelection::from_override(channel, std::env::var_os("FARCOOLER_HOME"))?.root)
}

/// A validated runtime root and its provenance, captured before startup effects.
#[derive(Clone, Debug)]
pub struct RuntimeSelection {
    root: PathBuf,
    explicit: bool,
}

impl RuntimeSelection {
    /// Pure explicit-input validation; default resolution only computes a path.
    /// Ownership, symlinks and a fresh private root remain launcher preconditions.
    pub fn from_override(channel: farcooler_protocol::Channel, value: Option<std::ffi::OsString>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self { root: default_runtime_dir_for(channel)?, explicit: false });
        };
        let text = value.to_str().ok_or(DomainError::OperationFailed)?;
        let root = PathBuf::from(text);
        let normalized: PathBuf = root.components().collect();
        if text.is_empty() || text.chars().any(char::is_control) || !root.is_absolute()
            || root.parent().is_none() || normalized.as_os_str() != value
            || root.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir))
            || root.join("farcoolerd.sock").as_os_str().len() > MAX_SOCKET_PATH
        {
            return Err(DomainError::OperationFailed);
        }
        Ok(Self { root, explicit: true })
    }

    pub fn root(&self) -> &std::path::Path { &self.root }
    pub fn is_explicit(&self) -> bool { self.explicit }

    pub fn ensure_dir(&self) -> Result<PathBuf> { ensure_dir_in(self.root.clone()) }

    pub fn socket_path(&self) -> Result<PathBuf> {
        Ok(self.ensure_dir()?.join("farcoolerd.sock"))
    }
}

/// The process edge: both CLI and daemon hold the same validated selection.
/// Failed explicit input is also retained, so it can never fall back later.
pub fn selection() -> Result<&'static RuntimeSelection> {
    static SELECTED: std::sync::OnceLock<std::result::Result<RuntimeSelection, ()>> = std::sync::OnceLock::new();
    SELECTED.get_or_init(|| RuntimeSelection::from_override(
        farcooler_protocol::CHANNEL, std::env::var_os("FARCOOLER_HOME"),
    ).map_err(|_| ())).as_ref().map_err(|_| DomainError::OperationFailed)
}

/// Where a channel's install lives when nobody has named a directory: the
/// same answer as `runtime_dir_for` with `FARCOOLER_HOME` ignored.
///
/// For telling the install an app runs from apart from a scratch daemon,
/// a demo host or a test, which all name their own home. See `codex_trust`.
pub fn default_runtime_dir_for(channel: farcooler_protocol::Channel) -> Result<PathBuf> {
    use farcooler_protocol::Channel;
    let app = match channel {
        Channel::Stable => "FarCooler",
        Channel::Preview => "FarCoolerPreview",
        Channel::Canary => "FarCoolerCanary",
        Channel::Local => "FarCoolerLocal",
    };
    let dirs = directories::ProjectDirs::from("com", "farcooler", app)
        .ok_or(DomainError::OperationFailed)?;
    Ok(dirs.data_dir().to_path_buf())
}

/// Create the runtime directory with user-only permissions (0700).
pub fn ensure_runtime_dir() -> Result<PathBuf> {
    selection()?.ensure_dir()
}

fn ensure_dir_in(dir: PathBuf) -> Result<PathBuf> {
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
