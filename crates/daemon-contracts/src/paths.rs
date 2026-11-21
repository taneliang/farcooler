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

/// Stable per-install id, generated once.
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
