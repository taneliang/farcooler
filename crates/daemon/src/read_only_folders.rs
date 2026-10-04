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
//! **The guard.** Whatever the config says, a folder is refused when it holds
//! the runner user's home or the daemon's runtime directory (so `/`, `/Users`
//! and `~` are never folders), or sits inside the runtime directory, `~/.ssh`
//! or `~/.gnupg`. Each refusal is a warning in the daemon's log, and the
//! daemon starts without that folder.

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

/// The directories a folder may neither hold nor sit in.
#[derive(Debug, Clone)]
pub struct Guarded {
    /// The runner user's home, `$HOME`.
    pub home: Option<PathBuf>,
    /// The daemon's runtime directory: its database, keys and sockets.
    pub runtime: PathBuf,
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
    /// It holds the home or the runtime directory.
    TooBroad,
    /// It sits inside the runtime directory, `~/.ssh` or `~/.gnupg`.
    Secret,
}

fn good_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.contains('/')
        && !name.chars().any(char::is_control)
        && !name.chars().all(|c| c == '.')
}

/// `path` resolved, or as given when it can't be (absent, unreadable).
fn real_or_given(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let runtime = real_or_given(&guarded.runtime);
    let home = guarded.home.as_deref().map(real_or_given);
    if runtime.starts_with(&real) || home.as_ref().is_some_and(|h| h.starts_with(&real)) {
        return Err(Refused::TooBroad);
    }
    let secret = |dir: &Path| real.starts_with(dir);
    if secret(&runtime) || home.as_ref().is_some_and(|h| secret(&h.join(".ssh")) || secret(&h.join(".gnupg"))) {
        return Err(Refused::Secret);
    }
    if real != path {
        tracing::info!(folder = %name, configured, real = %real.display(), "a read-only folder is resolved through a link, once, at startup");
    }
    Ok(Folder { name: name.to_string(), configured: configured.to_string(), real })
}

/// Every configured folder that passes `admit`, in name order. The rest are
/// each a warning.
pub fn resolve(configured: Vec<(String, String)>, guarded: &Guarded) -> Vec<Folder> {
    configured
        .into_iter()
        .filter_map(|(name, path)| match admit(&name, &path, guarded) {
            Ok(folder) => Some(folder),
            Err(why) => {
                tracing::warn!(folder = %name, path, ?why, "leaving out a read-only folder");
                None
            }
        })
        .collect()
}

/// The runner's folders, from its config file, guarded by its home and
/// `runtime`.
pub fn load(runtime: &Path) -> Vec<Folder> {
    let guarded = Guarded { home: std::env::var_os("HOME").map(PathBuf::from), runtime: runtime.to_path_buf() };
    resolve(farcooler_core::config::load_read_only_folders(), &guarded)
}

/// The folder a client named, by exact name and nothing else: a path, `..`
/// or a name that isn't configured is no folder.
pub fn find<'a>(folders: &'a [Folder], name: &str) -> Option<&'a Folder> {
    folders.iter().find(|f| f.name == name)
}

#[cfg(test)]
mod tests;
