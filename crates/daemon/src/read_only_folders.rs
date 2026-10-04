//! Extra folders the file viewer may read, beside worktrees (ov-232).
//!
//! Set only in the runner's own config file, `[files.read_only]`
//! (`farcooler_core::config::read_only_folders_from`), as a name and an
//! absolute path, and read once, when the daemon starts (`Service::open`).
//! A client sees each by name (`Host.read_only_folders`) and asks for one by
//! name (`folder` on `worktree.list_dir` and `worktree.read_file`): it never
//! sends a path for the folder itself, and no method adds, changes or removes
//! one. The list lives on the service with no setter.
//!
//! Inside a folder, `worktree_files` applies every check a worktree gets: only
//! plain relative names, `O_NOFOLLOW` at every component, a link answered as a
//! link and never followed, only regular files read, the 512 KiB cap.
//!
//! **A configured path that is, or passes through, a symbolic link** is
//! resolved once, at startup, and the folder is then the real directory it
//! named then. Every request walks that real path from `/` with `O_NOFOLLOW`
//! on every component, so a link swapped in later, at the folder or anywhere
//! above it, is refused rather than followed. Resolving rather than refusing,
//! because on macOS `/var` itself is a link to `/private/var`, and the owner's
//! example is `/var/log`. The resolved path is what the guard below checks,
//! so a link that already pointed at `~/.ssh` when the daemon started is
//! caught there, and the log says when a path was resolved through a link.
//!
//! **The guard** (`guard`). Whatever the config says, a folder is refused
//! when it holds the runner user's home (so `/`, `/Users` and `~` are never
//! folders), or holds or sits inside a protected directory: credentials,
//! keychains, browser profiles, every channel's Far Cooler home, the config's
//! directory, and `/proc` on Linux. Compared by `(st_dev, st_ino)` as well as
//! by path, so an APFS firmlink or a bind mount can't name one under another
//! path. With `$HOME` unset, every folder is refused. Each refusal is a
//! warning in the daemon's log, and the daemon starts without that folder.

use std::path::{Component, Path, PathBuf};

/// One folder, admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// What a client asks for it by.
    pub name: String,
    /// The path as the config file wrote it, for a client to show.
    pub configured: String,
    /// The real directory it named at startup: absolute, with no link in it.
    real: PathBuf,
}

impl Folder {
    /// The real directory, absolute and with no link in it when the daemon
    /// started. `worktree_files` walks it from `/` without following links.
    pub fn real(&self) -> &Path {
        &self.real
    }
}

/// What a folder is checked against (`guard`).
#[derive(Debug, Clone)]
pub struct Guarded {
    /// The runner user's home: a folder may sit in it, never hold it.
    home: PathBuf,
    /// What a folder may neither hold nor sit in (`guard::protected`).
    protected: Vec<PathBuf>,
}

impl Guarded {
    /// The guard for a runner whose home is `home`, with `runtimes` (every
    /// channel's Far Cooler home) and its `config_dir`. `None` without a
    /// home: no folder is admitted when the home can't be named, since every
    /// protected directory but the runtimes is found from it.
    pub fn new(home: Option<PathBuf>, runtimes: &[PathBuf], config_dir: Option<&Path>) -> Option<Self> {
        let home = home.filter(|h| h.is_absolute())?;
        let protected = guard::protected(&home, runtimes, config_dir);
        Some(Guarded { home, protected })
    }
}

/// Why a configured folder was left out.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Empty, over 64 characters, or with a `/`, a control character, or
    /// only dots.
    BadName,
    /// Not an absolute path.
    NotAbsolute,
    /// Nothing there when the daemon started, or not a directory.
    NotADirectory,
    /// It holds the home or a protected directory (and isn't one).
    TooBroad,
    /// It is a protected directory or sits inside one.
    Secret,
    /// The runner's home is unknown (`$HOME` unset), so nothing can be
    /// checked against it: every folder is refused.
    NoHome,
}

fn good_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.contains('/')
        && !name.chars().any(char::is_control)
        && !name.chars().all(|c| c == '.')
}

/// One configured folder, checked and resolved.
pub fn admit(name: &str, configured: &str, guarded: &Guarded) -> Result<Folder, Refused> {
    if !good_name(name) {
        return Err(Refused::BadName);
    }
    let path = Path::new(configured);
    if !path.is_absolute() {
        return Err(Refused::NotAbsolute);
    }
    let real = std::fs::canonicalize(path).map_err(|_| Refused::NotADirectory)?;
    if !real.is_dir() || !real.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_))) {
        return Err(Refused::NotADirectory);
    }
    // Inside first, so a protected directory itself reads as a secret.
    if guarded.protected.iter().any(|p| guard::inside(&real, p)) {
        return Err(Refused::Secret);
    }
    if guard::holds(&real, &guarded.home) || guarded.protected.iter().any(|p| guard::holds(&real, p)) {
        return Err(Refused::TooBroad);
    }
    if real != path {
        tracing::info!(folder = %name, configured, real = %real.display(), "a read-only folder is resolved through a link, once, at startup");
    }
    Ok(Folder { name: name.to_string(), configured: configured.to_string(), real })
}

/// Every configured folder that passes `admit`, in name order. The rest are
/// each a warning; without a guard (no home), all of them.
pub fn resolve(configured: Vec<(String, String)>, guarded: Option<&Guarded>) -> Vec<Folder> {
    configured
        .into_iter()
        .filter_map(|(name, path)| match guarded.map_or(Err(Refused::NoHome), |g| admit(&name, &path, g)) {
            Ok(folder) => Some(folder),
            Err(why) => {
                tracing::warn!(folder = %name, path, ?why, "leaving out a read-only folder");
                None
            }
        })
        .collect()
}

/// Every Far Cooler home on this machine: this daemon's `runtime`, and each
/// channel's default, since `config.toml` is shared by all of them.
pub fn runtimes(runtime: &Path) -> Vec<PathBuf> {
    use farcooler_protocol::Channel;
    let mut all = vec![runtime.to_path_buf()];
    for channel in [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable] {
        all.extend(crate::paths::default_runtime_dir_for(channel).ok());
    }
    all
}

/// The runner's folders, from its config file, guarded by its home, every
/// channel's runtime directory and the config's own directory.
pub fn load(runtime: &Path) -> Vec<Folder> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let config = farcooler_core::config::config_path();
    let guarded = Guarded::new(home, &runtimes(runtime), config.as_deref().and_then(Path::parent));
    resolve(farcooler_core::config::load_read_only_folders(), guarded.as_ref())
}

/// The folder a client named, by exact name and nothing else: a path, `..`
/// or a name that isn't configured is no folder.
pub fn find<'a>(folders: &'a [Folder], name: &str) -> Option<&'a Folder> {
    folders.iter().find(|f| f.name == name)
}

mod guard;

#[cfg(test)]
mod tests;
