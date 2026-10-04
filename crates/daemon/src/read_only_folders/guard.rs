//! What a read-only folder may neither hold nor sit inside (ov-232).
//!
//! Compared by identity, `(st_dev, st_ino)`, as well as by path. A path alone
//! is not enough: `canonicalize` resolves symbolic links but not APFS
//! firmlinks, so `/System/Volumes/Data/Users/<me>/.ssh` is `~/.ssh` under a
//! name no string comparison matches, and a Linux bind mount is the same.
//!
//! - **Inside** a protected directory: the protected directory's identity is
//!   the folder's own, or one of its ancestors', walked both by the path's
//!   own parents and by `..` from the folder.
//! - **Holding** one: the folder's identity is the protected directory's own
//!   or one of its ancestors', walked the same two ways; or the folder joined
//!   with any tail of the protected path is the protected directory (which is
//!   what catches `/System/Volumes/Data`, whose `..` chain doesn't run
//!   through `/Users`). A protected directory that doesn't exist yet is
//!   compared by its nearest existing ancestor's identity instead.
//! - And by path, both ways.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{Mode, OFlags};

type Id = (u64, u64);

fn id_of(path: &Path) -> Option<Id> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// `path` resolved as far as it exists: a missing tail is kept as written,
/// on its resolved parent.
pub(super) fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => resolved(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// The identities of `path` and every directory above it, by its parents as
/// written and by `..` from the directory itself.
// `st_dev` and `st_ino` are different widths on macOS and Linux.
#[allow(clippy::unnecessary_cast)]
fn chain(path: &Path) -> HashSet<Id> {
    let mut ids: HashSet<Id> = path.ancestors().filter_map(id_of).collect();
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let Ok(mut at) = rustix::fs::open(path, flags, Mode::empty()) else { return ids };
    // Bounded, against a filesystem whose `..` never repeats.
    for _ in 0..256 {
        let Ok(stat) = rustix::fs::fstat(&at) else { break };
        ids.insert((stat.st_dev as u64, stat.st_ino as u64));
        let Ok(up) = rustix::fs::openat(&at, "..", flags, Mode::empty()) else { break };
        let Ok(up_stat) = rustix::fs::fstat(&up) else { break };
        if (up_stat.st_dev, up_stat.st_ino) == (stat.st_dev, stat.st_ino) {
            break;
        }
        at = up;
    }
    ids
}

/// Whether `folder`, resolved, sits inside `protected` or is it.
pub(super) fn inside(folder: &Path, protected: &Path) -> bool {
    if folder.starts_with(resolved(protected)) {
        return true;
    }
    id_of(protected).is_some_and(|p| chain(folder).contains(&p))
}

/// Whether `folder`, resolved, holds `protected` or is it.
pub(super) fn holds(folder: &Path, protected: &Path) -> bool {
    let real = resolved(protected);
    if real.starts_with(folder) {
        return true;
    }
    // Not there yet (`~/.config/gcloud` before a first login): its nearest
    // existing ancestor stands in, so a folder that is or holds `~/.config`
    // by another path can't hold what is created there later.
    let Some((real, target)) = real.ancestors().find_map(|a| id_of(a).map(|id| (a.to_path_buf(), id))) else {
        return false;
    };
    if id_of(folder).is_some_and(|f| chain(&real).contains(&f)) {
        return true;
    }
    let names: Vec<_> = real.components().filter(|c| matches!(c, Component::Normal(_))).collect();
    (0..names.len()).any(|k| {
        let tail: PathBuf = names[k..].iter().collect();
        id_of(&folder.join(tail)) == Some(target)
    })
}

/// Under the home: credentials, keychains, cloud and cluster logins, browser
/// profiles, and Far Cooler's own config. A folder may sit in the home beside
/// these (`~/Library/Logs`), but never in or around one of them.
const IN_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".docker",
    ".config/gh",
    ".config/farcooler",
    ".password-store",
    ".local/share/keyrings",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Containers/com.apple.Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Firefox",
];

/// Every protected directory: `IN_HOME` under `home`, each of `runtimes`
/// (every channel's Far Cooler home), `config_dir`, and `/proc` on Linux.
pub(super) fn protected(home: &Path, runtimes: &[PathBuf], config_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = IN_HOME.iter().map(|p| home.join(p)).collect();
    all.extend(runtimes.iter().cloned());
    all.extend(config_dir.map(Path::to_path_buf));
    if cfg!(target_os = "linux") {
        all.push(PathBuf::from("/proc"));
    }
    all
}
