//! Trusting a repository for codex, the way codex's own trust screen does.
//!
//! codex stops in a directory it has not been told to trust and asks "Do you
//! trust the contents of this directory?". Only one table in codex's config
//! answers it: `-c projects."<path>".trust_level=…` on the command line does
//! not skip the screen. Measured on this runner with a throwaway `CODEX_HOME`
//! (codex-cli 0.153.4):
//!
//! - **A worktree is trusted through its repository's main checkout.**
//!   Accepting the screen inside a linked worktree writes
//!   `[projects."<main checkout>"] trust_level = "trusted"`, not the
//!   worktree's path, and every other worktree of that repository then opens
//!   with no screen. A trusted ancestor directory does nothing.
//! - **The path is the resolved one.** Opened as `/tmp/…/wt1`, codex wrote
//!   `/private/tmp/…/repo`, and an entry spelled `/tmp/…/repo` did not skip
//!   the screen. So the key is the canonical parent of
//!   `git rev-parse --path-format=absolute --git-common-dir`, which is what
//!   codex names on its screen as "the repository root".
//!
//! So Far Cooler writes that same entry, once per repository, when codex
//! launches in a worktree Far Cooler forked for a new task. That is the line
//! cursor's `--trust` is drawn on (`Service::forked_this_worktree`).
//!
//! **What the entry reaches.** It is the repository's entry, not the
//! worktree's, so it has the same effect as one Enter on codex's screen:
//! codex then opens without asking in the main checkout too, and in any
//! worktree of that repository, an adopted branch included. cursor's
//! `--trust` is per launch and per directory; this is not.
//!
//! **Only for the install an app runs.** A daemon with `FARCOOLER_HOME` set
//! (a scratch daemon, `scripts/demo-host.sh`), one whose home isn't its
//! channel's default (every test), or one with `FARCOOLER_NO_CODEX_TRUST` set
//! writes nothing. See `skip_reason`. Otherwise a test or a live check on a
//! scratch repository would leave its entry in the owner's real config.
//!
//! One scratch recipe still passes that gate: a daemon started with only
//! `HOME` overridden (`HOME=/tmp/x farcoolerd`, no `FARCOOLER_HOME`). Its
//! home is `/tmp/x/Library/Application Support/…`, which is exactly its
//! channel's default under that `HOME`, so it writes. That is safe only while
//! `CODEX_HOME` is unset: codex's home is then `/tmp/x/.codex`, as scratch as
//! the rest. With `CODEX_HOME` exported, such a daemon would write the real
//! config. So a scratch daemon sets `FARCOOLER_HOME` (or
//! `FARCOOLER_NO_CODEX_TRUST=1`), as the documented recipe and
//! `scripts/demo-host.sh` do.
//!
//! The config is the owner's file, so the write is as small and as careful as
//! it can be:
//!
//! - **An entry already there is final**, in any spelling of the same
//!   directory and whatever it says. An owner who answered "No" has
//!   `trust_level = "untrusted"`, and that stays. Only keys ending in the
//!   repository's own name are ever resolved to find out; see
//!   `same_directory`.
//! - **A config that won't parse is left alone** and logged, and codex asks.
//!   So is one `toml_edit` wouldn't give back byte for byte (CRLF line
//!   endings, a byte-order mark), one that is read-only, a symbolic link, or
//!   a hard link with more than one name.
//! - **`toml_edit`**, so comments, order and spacing everywhere else survive.
//!   Before renaming, the new file is checked to be the old one with one run
//!   of bytes inserted.
//! - **A unique temporary file and a rename**, with the file's own mode.
//! - **One writer per daemon at a time** (`WRITING`, taken with `try_lock`:
//!   a launch that finds another write running skips its own rather than
//!   wait behind it). Between processes there
//!   is no lock: codex takes none we could share. The file is read again just
//!   before the rename, and a change seen there cancels the write. A change
//!   landing in the microseconds between that read and the rename would still
//!   be lost. That is the residual, and the one place it matters is codex
//!   saving an answer to its own screen for this same repository at that
//!   instant.
//! - **Never fails a launch.** Every refusal and failure costs exactly what
//!   doing nothing would: codex shows its screen.
//! - **Nothing removes it**, and it comes back. The entry is the
//!   repository's, and accepting codex's screen leaves it behind in the same
//!   way. Deleting it by hand lasts until the next codex launch in a forked
//!   worktree of that repository. To say no for good, set
//!   `trust_level = "untrusted"`.
//!
//! A repository made with `git init --separate-git-dir` gets the git
//! directory's parent as its key, as codex derives it too. That entry trusts
//! a directory nobody opens codex in, and does nothing else.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// What `trust_repository` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trusted {
    /// The entry was added.
    Wrote,
    /// The config already had an entry for this directory, trusted or not.
    AlreadyDecided,
    /// Nothing was written, for the reason given.
    LeftAlone(&'static str),
}

/// How long `git rev-parse` may take before the launch goes on without it.
const GIT_DEADLINE: Duration = Duration::from_secs(5);

/// How long the whole step may hold up a launch.
const STEP_DEADLINE: Duration = Duration::from_secs(10);

/// Why this daemon should write nothing into codex's config, or `None` if it
/// may.
///
/// - `opt_out` is `FARCOOLER_NO_CODEX_TRUST`. Any value but empty or `0`
///   turns it off.
/// - `farcooler_home` is `FARCOOLER_HOME`. Set at all, it names a home that
///   isn't an app's. The Mac app never sets it (`CLI.swift`, "Deliberately
///   does NOT set FARCOOLER_HOME"). The daemon the CLI spawns inherits the
///   app's environment, and a remote runner's forced command sets none.
/// - `root` must be `default_root`, this channel's default home, so a daemon
///   opened on any other directory (every test's `Service::open_in`) writes
///   nothing, whatever its environment says.
pub fn skip_reason(
    root: &Path,
    farcooler_home: Option<&OsStr>,
    opt_out: Option<&OsStr>,
    default_root: Option<&Path>,
) -> Option<&'static str> {
    if opt_out.is_some_and(|v| !v.is_empty() && v != "0") {
        return Some("FARCOOLER_NO_CODEX_TRUST is set");
    }
    if farcooler_home.is_some_and(|v| !v.is_empty()) {
        return Some("FARCOOLER_HOME names this daemon's home");
    }
    let Some(default_root) = default_root else { return Some("this channel has no default home") };
    let resolved = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if resolved(root) != resolved(default_root) {
        return Some("this daemon's home is not its channel's default");
    }
    None
}

/// What `skip_reason` is asked with: `FARCOOLER_HOME`,
/// `FARCOOLER_NO_CODEX_TRUST` and this channel's default home.
///
/// Under `cfg(test)`, the environment is ignored and the default home is only
/// ever the one a test set with `test_home::set`, so no unit test passes the
/// gate by accident.
fn gate_inputs() -> (Option<OsString>, Option<OsString>, Option<PathBuf>) {
    #[cfg(test)]
    {
        (None, None, test_home::DEFAULT_ROOT.with(|r| r.borrow().clone()))
    }
    #[cfg(not(test))]
    {
        (
            std::env::var_os("FARCOOLER_HOME"),
            std::env::var_os("FARCOOLER_NO_CODEX_TRUST"),
            crate::paths::default_runtime_dir_for(farcooler_protocol::CHANNEL).ok(),
        )
    }
}

/// codex's home directory for this runner's user: `$CODEX_HOME`, or
/// `~/.codex`, the same order codex itself reads them in.
///
/// Under `cfg(test)` this is only ever the directory a test set with
/// `test_home::set`, and `None` otherwise.
pub fn codex_home() -> Option<PathBuf> {
    #[cfg(test)]
    {
        test_home::HOME.with(|h| h.borrow().clone())
    }
    #[cfg(not(test))]
    {
        match std::env::var_os("CODEX_HOME") {
            Some(home) if !home.is_empty() => Some(PathBuf::from(home)),
            _ => user_home().map(|h| h.join(".codex")),
        }
    }
}

/// The runner user's home, as the rest of the daemon finds it.
fn user_home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

#[cfg(test)]
pub(crate) mod test_home {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        pub(crate) static HOME: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
        pub(crate) static DEFAULT_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Point `codex_home` at `home`, and treat `default_root` as this
    /// channel's default install home, on this thread until the guard drops.
    pub(crate) fn set(home: &Path, default_root: &Path) -> Guard {
        HOME.with(|h| *h.borrow_mut() = Some(home.to_path_buf()));
        DEFAULT_ROOT.with(|r| *r.borrow_mut() = Some(default_root.to_path_buf()));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            HOME.with(|h| *h.borrow_mut() = None);
            DEFAULT_ROOT.with(|r| *r.borrow_mut() = None);
        }
    }
}

/// Trust the repository `worktree` belongs to in codex's config, for the
/// daemon whose home is `root`, and log what happened. Never fails: see the
/// module doc.
///
/// The gate and codex's home are read here, on the caller's thread, and the
/// `git` call and the file work run on the blocking pool, bounded by
/// `STEP_DEADLINE`. A step that runs past it is logged and the launch goes
/// on. If it finishes later, it has the same effect as the owner answering
/// the screen later.
pub async fn trust_for_worktree(root: &Path, worktree: &Path) {
    let (farcooler_home, opt_out, default_root) = gate_inputs();
    if let Some(why) = skip_reason(root, farcooler_home.as_deref(), opt_out.as_deref(), default_root.as_deref()) {
        tracing::debug!(why, "not writing codex's trust entry");
        return;
    }
    let Some(home) = codex_home() else { return };
    let worktree = worktree.to_path_buf();
    let user = user_home();
    let step = tokio::task::spawn_blocking(move || {
        let Some(repository) = main_checkout(&worktree) else {
            tracing::info!(worktree = %worktree.display(), "could not find this worktree's repository; codex will ask to trust it");
            return;
        };
        let config = home.join("config.toml");
        match trust_repository(&home, &repository, user.as_deref()) {
            Trusted::Wrote => tracing::info!(
                repository = %repository.display(),
                config = %config.display(),
                "told codex to trust this repository, as its own trust screen would"
            ),
            Trusted::AlreadyDecided => {}
            Trusted::LeftAlone(why) => tracing::warn!(
                repository = %repository.display(),
                config = %config.display(),
                why,
                "left codex's config alone; codex will ask to trust this repository"
            ),
        }
    });
    if tokio::time::timeout(STEP_DEADLINE, step).await.is_err() {
        tracing::warn!("codex's trust entry took too long; launching without waiting for it");
    }
}

/// The path codex trusts a worktree through: the parent of the repository's
/// common git directory, resolved the way codex resolves it.
///
/// Blocking, and bounded: `git` is killed after `GIT_DEADLINE`. `GIT_DIR`
/// and `GIT_COMMON_DIR` are removed so the daemon's own environment can't
/// point the question at another repository.
pub fn main_checkout(worktree: &Path) -> Option<PathBuf> {
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < GIT_DEADLINE => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    use std::os::unix::ffi::OsStrExt;
    let common = OsStr::from_bytes(out.strip_suffix(b"\n").unwrap_or(&out));
    repository_of(Path::new(common))
}

/// The directory codex names as the repository root for a common git
/// directory: its parent, with every symbolic link resolved. git already
/// answers in that form when asked with `-C`, because it `chdir`s and then
/// asks the kernel where it is; this resolves again so the key doesn't rest
/// on that.
fn repository_of(common: &Path) -> Option<PathBuf> {
    common.canonicalize().ok()?.parent().map(Path::to_path_buf)
}

/// One trust write at a time in this process: two launches in two
/// repositories at once would otherwise both read the same file and the
/// second rename would drop the first one's entry.
///
/// Taken with `try_lock`, never waited on. A write still running (one stuck
/// in the kernel on a path that won't answer, say) then costs a later launch
/// nothing: that launch skips its write, and codex asks, as it always did.
static WRITING: Mutex<()> = Mutex::new(());

/// Add `[projects."<repository>"] trust_level = "trusted"` to
/// `<codex_home>/config.toml`, unless an entry for that directory, in any
/// spelling, is already there.
///
/// `repository` is written exactly as given; `main_checkout` is what gives it
/// the form codex reads. `user_home` expands a `~` in an existing key.
pub fn trust_repository(codex_home: &Path, repository: &Path, user_home: Option<&Path>) -> Trusted {
    trust_repository_holding(&WRITING, codex_home, repository, user_home)
}

/// `trust_repository` under `lock` rather than `WRITING`, so a test can hold
/// one without racing every other test that writes.
fn trust_repository_holding(
    lock: &Mutex<()>,
    codex_home: &Path,
    repository: &Path,
    user_home: Option<&Path>,
) -> Trusted {
    let _one_at_a_time = match lock.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            return Trusted::LeftAlone("another trust write is still running");
        }
    };
    trust_repository_between(codex_home, repository, user_home, || {})
}

/// `trust_repository`, running `between` after the temporary file is written
/// and before the config is read again, which is where a test puts somebody
/// else's save.
fn trust_repository_between(
    codex_home: &Path,
    repository: &Path,
    user_home: Option<&Path>,
    between: impl FnOnce(),
) -> Trusted {
    let Some(key) = repository.to_str() else { return Trusted::LeftAlone("the path is not UTF-8") };
    // codex makes its home on first run. One that isn't there is a codex that
    // has never run, and this is not the place to make it.
    if !codex_home.is_dir() {
        return Trusted::LeftAlone("codex's home directory does not exist");
    }
    let config = codex_home.join("config.toml");
    let (before, mode) = match read_no_follow(&config) {
        Ok(Some(read)) => read,
        Ok(None) => (Vec::new(), 0o600),
        Err(why) => return Trusted::LeftAlone(why),
    };
    let Ok(text) = std::str::from_utf8(&before) else { return Trusted::LeftAlone("the config is not UTF-8") };
    let Ok(mut document) = text.parse::<toml_edit::DocumentMut>() else {
        return Trusted::LeftAlone("the config is not valid TOML");
    };
    // `toml_edit` gives most files back byte for byte, but not all of them:
    // it writes LF after every header and key it prints, and drops a
    // byte-order mark. A file it wouldn't reproduce is left as it is.
    if document.to_string() != text {
        return Trusted::LeftAlone("the config would not keep its exact bytes");
    }
    let projects = document.entry("projects").or_insert_with(|| {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        toml_edit::Item::Table(table)
    });
    let inline = projects.is_inline_table();
    let Some(projects) = projects.as_table_like_mut() else {
        return Trusted::LeftAlone("its `projects` is not a table");
    };
    // Asked at most once, and only if some key needs it.
    let case = std::cell::OnceCell::new();
    let mut ignores = || *case.get_or_init(|| ignores_case(repository));
    let mut resolve = |path: &Path| path.canonicalize().ok();
    if projects
        .iter()
        .any(|(existing, _)| same_directory(existing, repository, user_home, &mut resolve, &mut ignores))
    {
        return Trusted::AlreadyDecided;
    }
    if inline {
        let mut entry = toml_edit::InlineTable::new();
        entry.insert("trust_level", "trusted".into());
        projects.insert(key, toml_edit::Item::Value(entry.into()));
    } else {
        let mut entry = toml_edit::Table::new();
        entry.insert("trust_level", toml_edit::value("trusted"));
        projects.insert(key, toml_edit::Item::Table(entry));
    }
    let after = document.to_string();
    if !one_insertion(text, &after) {
        return Trusted::LeftAlone("the edit would have changed more than the new entry");
    }
    match replace(&config, after.as_bytes(), mode, &before, between) {
        Ok(true) => Trusted::Wrote,
        Ok(false) => Trusted::LeftAlone("the config changed while it was being written"),
        Err(e) => {
            tracing::warn!(error = %e, config = %config.display(), "could not write codex's config");
            Trusted::LeftAlone("the write failed")
        }
    }
}

/// Whether `key`, an existing `projects` key, names `repository`, however it
/// is spelled.
///
/// A trailing `/` is dropped and a leading `~` is expanded, with no system
/// call. A key spelled exactly as `repository` matches there. Otherwise only
/// a key whose last component is the repository's own name, ignoring case,
/// is looked at any further; every other key is never resolved, so an
/// unrelated project on a mount that doesn't answer, or in a folder macOS
/// guards (`~/Documents`), is never touched. A key of the same name is
/// resolved with `resolve` (`canonicalize`), which catches `/tmp` for
/// `/private/tmp`, a linked parent, `..`, and, on APFS, another case, since
/// the resolved path comes back in the case on disk.
///
/// A key of the same name that can't be resolved (gone, or in a folder we
/// may not read) is compared as text, ignoring case when `ignores_case` says
/// the repository's volume does. That is the one case the case check
/// decides; a key that resolves never needs it.
///
/// What this gives up: a key naming the repository through a link whose own
/// name differs from the repository's. It reads as another directory, and
/// the entry is added beside it.
fn same_directory(
    key: &str,
    repository: &Path,
    user_home: Option<&Path>,
    resolve: &mut dyn FnMut(&Path) -> Option<PathBuf>,
    ignores_case: &mut dyn FnMut() -> bool,
) -> bool {
    let trimmed = match key.trim_end_matches('/') {
        "" if key.starts_with('/') => "/",
        trimmed => trimmed,
    };
    let expanded = match (trimmed.strip_prefix('~'), user_home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(trimmed),
    };
    if expanded == repository {
        return true;
    }
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    if name(repository).is_none() || name(&expanded) != name(repository) {
        return false;
    }
    match resolve(&expanded) {
        Some(resolved) => resolved == repository,
        None => {
            ignores_case()
                && expanded.to_string_lossy().to_lowercase() == repository.to_string_lossy().to_lowercase()
        }
    }
}

/// Whether the volume `path` is on ignores case: the same path with every
/// ASCII letter's case flipped names the very same file.
fn ignores_case(path: &Path) -> bool {
    let Some(text) = path.to_str() else { return false };
    let flipped: String = text
        .chars()
        .map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
        .collect();
    if flipped == text {
        return false;
    }
    match (std::fs::metadata(path), std::fs::metadata(&flipped)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `after` is `before` with one run of bytes inserted and nothing
/// else changed.
fn one_insertion(before: &str, after: &str) -> bool {
    let (before, after) = (before.as_bytes(), after.as_bytes());
    if after.len() <= before.len() {
        return false;
    }
    let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let room = before.len() - prefix;
    let suffix = before.iter().rev().zip(after.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    prefix + suffix == before.len()
}

/// The file's bytes and permission bits, `None` if there is no file, or why
/// it won't be edited. `O_NOFOLLOW` makes a symbolic link an error, not a
/// read of whatever it points at.
fn read_no_follow(path: &Path) -> Result<Option<(Vec<u8>, u32)>, &'static str> {
    let mut file = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err("the config is a symbolic link"),
        Err(_) => return Err("the config can't be read"),
    };
    let meta = file.metadata().map_err(|_| "the config can't be read")?;
    if !meta.is_file() {
        return Err("the config is not a file");
    }
    // A rename would leave the other name holding the old file.
    if meta.nlink() > 1 {
        return Err("the config has more than one name");
    }
    // The owner took write permission away; that's a no.
    if meta.permissions().mode() & 0o200 == 0 {
        return Err("the config is read-only");
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| "the config can't be read")?;
    Ok(Some((bytes, meta.permissions().mode() & 0o7777)))
}

/// Tells one write's temporary file from another's in the same process.
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A temporary name beside `path` no other write shares: the file's own
/// name, the process id, a count of writes in this process, and the time,
/// so a file left by a crash in an earlier process with the same id doesn't
/// block this one.
fn temporary_beside(path: &Path) -> Option<PathBuf> {
    let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = path.file_name()?.to_string_lossy();
    Some(path.parent()?.join(format!(".{name}.farcooler-{}-{write}-{nanos}.tmp", std::process::id())))
}

/// Write `contents` to a new temporary file beside `path` with `mode`, then
/// rename it over `path` if `path` still holds `before` (or is still absent
/// when `before` is empty), and say whether it did.
///
/// Also how `service::replace_hooks_file` writes a hooks file. The read that
/// decides refuses the same things for both (`read_no_follow`): a symbolic
/// link, a file with more than one name, a read-only file.
pub(crate) fn replace(path: &Path, contents: &[u8], mode: u32, before: &[u8], between: impl FnOnce()) -> std::io::Result<bool> {
    let temp = temporary_beside(path).ok_or(std::io::ErrorKind::InvalidInput)?;
    replace_through(path, &temp, contents, mode, before, between)
}

/// `replace`, through a temporary name the caller chose.
///
/// `create_new` with `O_NOFOLLOW` refuses anything already at `temp`, a
/// planted link included. The read just before the rename is what cancels
/// the write when somebody saved the file meanwhile; see the module doc for
/// what it doesn't cover.
fn replace_through(
    path: &Path,
    temp: &Path,
    contents: &[u8],
    mode: u32,
    before: &[u8],
    between: impl FnOnce(),
) -> std::io::Result<bool> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(temp)?;
    let written = (|| {
        file.write_all(contents)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.sync_all()
    })();
    drop(file);
    between();
    let unmoved = match read_no_follow(path) {
        Ok(Some((now, _))) => now == before,
        Ok(None) => before.is_empty(),
        Err(_) => false,
    };
    let renamed = match written {
        Ok(()) if unmoved => std::fs::rename(temp, path).map(|()| true),
        Ok(()) => Ok(false),
        Err(e) => Err(e),
    };
    if matches!(renamed, Ok(true)) {
        // The rename itself, on disk. Best effort: without it a crash can
        // only bring the old file back, and codex asks.
        if let Some(dir) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
            let _ = dir.sync_all();
        }
    } else {
        let _ = std::fs::remove_file(temp);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNERS: &str = "# my codex settings\nmodel = \"gpt-5.6\"   # keep\n\n[projects.\"/elsewhere\"]\ntrust_level = \"trusted\"\n";

    fn home_with(config: Option<&str>) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        if let Some(text) = config {
            std::fs::write(home.path().join("config.toml"), text).unwrap();
        }
        home
    }

    fn read(home: &tempfile::TempDir) -> String {
        std::fs::read_to_string(home.path().join("config.toml")).unwrap()
    }

    fn trust(home: &Path, repository: &str) -> Trusted {
        trust_repository_between(home, Path::new(repository), None, || {})
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    /// The table codex writes when its screen is accepted, byte for byte, with
    /// everything of the owner's above it untouched.
    #[test]
    fn the_entry_is_the_one_codex_writes() {
        let home = home_with(Some(OWNERS));
        assert_eq!(trust(home.path(), "/private/tmp/r/repo"), Trusted::Wrote);
        assert_eq!(
            read(&home),
            format!("{OWNERS}\n[projects.\"/private/tmp/r/repo\"]\ntrust_level = \"trusted\"\n")
        );
        // Once per repository: the second launch finds it and writes nothing.
        let after = read(&home);
        assert_eq!(trust(home.path(), "/private/tmp/r/repo"), Trusted::AlreadyDecided);
        assert_eq!(read(&home), after);
    }

    /// No config yet: the file is just the entry, owner-only.
    #[test]
    fn a_missing_config_becomes_the_entry() {
        let home = home_with(None);
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::Wrote);
        assert_eq!(read(&home), "[projects.\"/r/repo\"]\ntrust_level = \"trusted\"\n");
        assert_eq!(mode_of(&home.path().join("config.toml")), 0o600);
    }

    /// An existing file keeps its own mode, whatever it was.
    #[test]
    fn the_config_keeps_its_mode() {
        for mode in [0o644, 0o640, 0o600] {
            let home = home_with(Some(OWNERS));
            let config = home.path().join("config.toml");
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(trust(home.path(), "/r/repo"), Trusted::Wrote, "{mode:o}");
            assert_eq!(mode_of(&config), mode);
        }
    }

    /// An owner who told codex "No" has decided. That entry, in every shape
    /// TOML can spell one, is left byte for byte.
    #[test]
    fn an_untrusted_entry_is_left_byte_identical() {
        for config in [
            "[projects.\"/r/repo\"]\ntrust_level = \"untrusted\"\n",
            "[projects.'/r/repo']\ntrust_level = \"untrusted\"\n",
            "projects = { \"/r/repo\" = { trust_level = \"untrusted\" } }\n",
            "[projects]\n\"/r/repo\" = { trust_level = \"untrusted\" }\n",
            "projects.\"/r/repo\".trust_level = \"untrusted\"\n",
        ] {
            let home = home_with(Some(config));
            assert_eq!(trust(home.path(), "/r/repo"), Trusted::AlreadyDecided, "{config}");
            assert_eq!(read(&home), config);
        }
    }

    /// The same directory spelled another way is the same decision: a
    /// trailing slash, `~`, a path through a linked parent, and, on a volume
    /// that ignores case, another case. `untrusted` first, because that is
    /// the one an extra entry would overrule; then `trusted`, which needs no
    /// second entry either.
    #[test]
    fn an_entry_in_another_spelling_is_left_byte_identical() {
        let user = tempfile::tempdir().unwrap();
        let user_home = user.path().canonicalize().unwrap();
        let repository = user_home.join("Dev").join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        std::os::unix::fs::symlink(user_home.join("Dev"), user_home.join("linked")).unwrap();
        let real = repository.display().to_string();
        let mut spellings = vec![
            format!("{real}/"),
            format!("{real}//"),
            "~/Dev/repo".to_string(),
            "~/Dev/repo/".to_string(),
            format!("{}/linked/repo", user_home.display()),
            format!("{}/Dev/../Dev/repo", user_home.display()),
        ];
        // `/tmp` and `/private/tmp` where the temporary directory has both.
        let unresolved = user.path().join("Dev").join("repo");
        if unresolved != repository {
            spellings.push(unresolved.display().to_string());
        }
        let case = real.to_uppercase();
        if ignores_case(&repository) {
            spellings.push(case.clone());
        } else {
            // A volume that keeps case: another case is another directory.
            let home = home_with(Some(&format!("[projects.\"{case}\"]\ntrust_level = \"untrusted\"\n")));
            assert_eq!(trust_repository_between(home.path(), &repository, Some(&user_home), || {}), Trusted::Wrote);
        }
        for level in ["untrusted", "trusted"] {
            for spelling in &spellings {
                let config = format!("[projects.\"{spelling}\"]\ntrust_level = \"{level}\"\n");
                let home = home_with(Some(&config));
                assert_eq!(
                    trust_repository_between(home.path(), &repository, Some(&user_home), || {}),
                    Trusted::AlreadyDecided,
                    "{config}"
                );
                assert_eq!(read(&home), config);
            }
        }
        // And a different directory is not the same one.
        let home = home_with(Some(&format!("[projects.\"{real}-other\"]\ntrust_level = \"untrusted\"\n")));
        assert_eq!(trust_repository_between(home.path(), &repository, Some(&user_home), || {}), Trusted::Wrote);
    }

    /// A config that won't parse is the owner's to fix. Nothing is written,
    /// and codex asks, as it would have anyway.
    #[test]
    fn an_unparseable_config_is_left_byte_identical() {
        let broken = "model = \"gpt-5.6\n[projects.\"/elsewhere\"\ntrust_level = trusted\n";
        let home = home_with(Some(broken));
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::LeftAlone("the config is not valid TOML"));
        assert_eq!(read(&home), broken);
        // `projects` that isn't a table is left alone too, not replaced.
        let home = home_with(Some("projects = 3\n"));
        assert!(matches!(trust(home.path(), "/r/repo"), Trusted::LeftAlone(_)));
        assert_eq!(read(&home), "projects = 3\n");
    }

    /// A file `toml_edit` wouldn't give back byte for byte is left alone:
    /// CRLF line endings and a byte-order mark.
    #[test]
    fn a_config_that_would_not_round_trip_is_left_byte_identical() {
        for config in [
            "model = \"gpt-5.6\"\r\n\r\n[projects.\"/elsewhere\"]\r\ntrust_level = \"trusted\"\r\n",
            "\u{feff}model = \"gpt-5.6\"\n",
        ] {
            let home = home_with(Some(config));
            assert_eq!(
                trust(home.path(), "/r/repo"),
                Trusted::LeftAlone("the config would not keep its exact bytes"),
                "{config:?}"
            );
            assert_eq!(read(&home), config);
        }
    }

    /// `one_insertion` is the check before the rename: the new file is the
    /// old one with one run of bytes added.
    #[test]
    fn one_insertion_means_one() {
        assert!(one_insertion("a\nb\n", "a\nNEW\nb\n"));
        assert!(one_insertion("", "x"));
        assert!(one_insertion("ab", "abc"));
        assert!(!one_insertion("a\nb\n", "A\nNEW\nb\n"), "a change and an insertion");
        assert!(!one_insertion("a\nb\n", "aX\nbY\n"), "two insertions");
        assert!(!one_insertion("a\nb\n", "a\nb\n"), "nothing added");
        assert!(!one_insertion("abc", "xabcx"), "two runs");
    }

    /// A config the owner made read-only is a no.
    #[test]
    fn a_read_only_config_is_left_byte_identical() {
        let home = home_with(Some(OWNERS));
        let config = home.path().join("config.toml");
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::LeftAlone("the config is read-only"));
        assert_eq!(read(&home), OWNERS);
        assert_eq!(mode_of(&config), 0o400);
    }

    /// A hard-linked config is left alone: a rename would split its names.
    #[test]
    fn a_hard_linked_config_is_left_alone() {
        let home = home_with(Some(OWNERS));
        std::fs::hard_link(home.path().join("config.toml"), home.path().join("dotfiles-copy")).unwrap();
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::LeftAlone("the config has more than one name"));
        assert_eq!(read(&home), OWNERS);
    }

    /// An inline `projects` table gets an inline entry, so the file stays in
    /// the shape its owner chose.
    #[test]
    fn an_inline_projects_table_stays_inline() {
        let home = home_with(Some("projects = { \"/a\" = { trust_level = \"trusted\" } }\n"));
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::Wrote);
        let text = read(&home);
        assert!(text.starts_with("projects = {"), "{text}");
        let parsed: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(parsed["projects"]["/r/repo"]["trust_level"].as_str(), Some("trusted"));
        assert_eq!(parsed["projects"]["/a"]["trust_level"].as_str(), Some("trusted"));
    }

    /// A `config.toml` that is a symbolic link is not read through or
    /// replaced, and what it points at is untouched.
    #[test]
    fn a_linked_config_is_not_followed() {
        let home = home_with(None);
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("config.toml");
        std::fs::write(&target, OWNERS).unwrap();
        std::os::unix::fs::symlink(&target, home.path().join("config.toml")).unwrap();
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::LeftAlone("the config is a symbolic link"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), OWNERS);
        assert!(std::fs::symlink_metadata(home.path().join("config.toml")).unwrap().file_type().is_symlink());
    }

    /// A codex that has never run has no home, and none is made for it.
    #[test]
    fn a_missing_home_is_not_made() {
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join(".codex");
        assert!(matches!(trust(&home, "/r/repo"), Trusted::LeftAlone(_)));
        assert!(!home.exists());
    }

    /// A config shaped like a real one: codex's own project tables first,
    /// then the owner's other tables, two comments, LF, a final newline.
    /// Synthetic throughout. The entry lands right after the last project
    /// table, as one run of bytes, and nothing else moves.
    #[test]
    fn an_owner_shaped_config_takes_the_entry_after_its_projects() {
        let mut projects = String::from("# settings, by hand\nmodel = \"gpt-5.6\"\n");
        for i in 0..30 {
            projects.push_str(&format!("\n[projects.\"/Users/someone/code/p{i}\"]\ntrust_level = \"trusted\"\n"));
        }
        let mut rest = String::new();
        for i in 0..30 {
            if i == 15 {
                rest.push_str("\n# the tools I use");
            }
            let table = if i < 15 { format!("mcp_servers.s{i}") } else { format!("profiles.p{i}") };
            rest.push_str(&format!("\n[{table}]\ncommand = \"run-{i}\"\n"));
        }
        let config = format!("{projects}{rest}");
        let home = home_with(Some(&config));
        assert_eq!(trust(home.path(), "/r/repo"), Trusted::Wrote);
        assert_eq!(read(&home), format!("{projects}\n[projects.\"/r/repo\"]\ntrust_level = \"trusted\"\n{rest}"));
    }

    /// Only a key ending in the repository's own name is ever looked up.
    /// Every other key, on a mount that might not answer or in a folder
    /// macOS guards, is decided without a single call.
    #[test]
    fn only_a_key_of_the_same_name_is_ever_resolved() {
        let repository = Path::new("/Users/someone/Dev/repo");
        let user_home = Some(Path::new("/Users/someone"));
        let asked = std::cell::RefCell::new(Vec::<PathBuf>::new());
        let case_asked = std::cell::Cell::new(0);
        let mut resolve = |p: &Path| -> Option<PathBuf> {
            asked.borrow_mut().push(p.to_path_buf());
            None
        };
        let mut ignores = || {
            case_asked.set(case_asked.get() + 1);
            true
        };
        for key in [
            "/Volumes/nas/share/other",
            "/net/unreachable/project",
            "~/Documents/notes",
            "/Users/someone/Dev/repo-2",
            "/Users/someone/Dev",
            "/",
        ] {
            assert!(!same_directory(key, repository, user_home, &mut resolve, &mut ignores), "{key}");
        }
        assert_eq!(*asked.borrow(), Vec::<PathBuf>::new(), "an unrelated key was resolved");
        assert_eq!(case_asked.get(), 0, "the volume was asked about for an unrelated key");
        // The exact spelling, and `~` for it, need no lookup either.
        assert!(same_directory("/Users/someone/Dev/repo/", repository, user_home, &mut resolve, &mut ignores));
        assert!(same_directory("~/Dev/repo", repository, user_home, &mut resolve, &mut ignores));
        assert_eq!(*asked.borrow(), Vec::<PathBuf>::new());
        // A key of the same name is looked up.
        assert!(!same_directory("/elsewhere/Repo", repository, user_home, &mut resolve, &mut ignores));
        assert_eq!(*asked.borrow(), vec![PathBuf::from("/elsewhere/Repo")]);
    }

    /// A key of the repository's name that can't be resolved is compared as
    /// text, ignoring case only where the volume does.
    #[test]
    fn an_unresolvable_key_matches_by_case_only_where_case_is_ignored() {
        let repository = Path::new("/Users/someone/Dev/Repo");
        let mut unresolvable = |_: &Path| -> Option<PathBuf> { None };
        let key = "/users/someone/dev/REPO";
        assert!(same_directory(key, repository, None, &mut unresolvable, &mut || true));
        assert!(!same_directory(key, repository, None, &mut unresolvable, &mut || false));
        assert!(!same_directory("/users/someone/other/repo", repository, None, &mut unresolvable, &mut || true));
    }

    /// A trust write still running (stuck, say) is not waited on: this
    /// launch skips its own, and the file is untouched.
    #[test]
    fn a_write_already_running_is_not_waited_for() {
        let home = home_with(Some(OWNERS));
        let lock = Mutex::new(());
        let held = lock.lock().unwrap();
        assert_eq!(
            trust_repository_holding(&lock, home.path(), Path::new("/r/repo"), None),
            Trusted::LeftAlone("another trust write is still running")
        );
        assert_eq!(read(&home), OWNERS);
        drop(held);
        assert_eq!(trust_repository_holding(&lock, home.path(), Path::new("/r/repo"), None), Trusted::Wrote);
    }

    /// Somebody saves the config between our read and our rename: their
    /// save stands, ours is dropped, and no temporary file is left.
    #[test]
    fn a_save_while_writing_cancels_the_write() {
        let home = home_with(Some(OWNERS));
        let config = home.path().join("config.toml");
        let theirs = "[projects.\"/r/repo\"]\ntrust_level = \"untrusted\"\n";
        let outcome =
            trust_repository_between(home.path(), Path::new("/r/repo"), None, || std::fs::write(&config, theirs).unwrap());
        assert_eq!(outcome, Trusted::LeftAlone("the config changed while it was being written"));
        assert_eq!(read(&home), theirs);
        let names: Vec<_> = std::fs::read_dir(home.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("config.toml")]);
    }

    /// Something already at the temporary name, a link included, is never
    /// written through or replaced, and the config is untouched.
    #[test]
    fn a_planted_temporary_file_is_refused() {
        let home = home_with(Some(OWNERS));
        let config = home.path().join("config.toml");
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("victim");
        std::fs::write(&victim, "keep\n").unwrap();
        let temp = home.path().join(".config.toml.planted.tmp");
        std::os::unix::fs::symlink(&victim, &temp).unwrap();
        let outcome = replace_through(&config, &temp, b"new\n", 0o600, OWNERS.as_bytes(), || {});
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep\n");
        assert_eq!(read(&home), OWNERS);
        assert!(std::fs::symlink_metadata(&temp).unwrap().file_type().is_symlink());
        // A plain file there is refused the same way.
        let plain = home.path().join(".config.toml.plain.tmp");
        std::fs::write(&plain, "somebody's\n").unwrap();
        assert!(replace_through(&config, &plain, b"new\n", 0o600, OWNERS.as_bytes(), || {}).is_err());
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "somebody's\n");
        assert_eq!(read(&home), OWNERS);
    }

    /// No temporary file is left beside the config, written or not.
    #[test]
    fn no_temporary_file_is_left_behind() {
        let home = home_with(Some(OWNERS));
        trust(home.path(), "/r/repo");
        trust(home.path(), "/r/repo");
        let names: Vec<_> = std::fs::read_dir(home.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("config.toml")]);
    }

    /// The gate, both sides: only a daemon on its channel's default home,
    /// with neither variable set, writes.
    #[test]
    fn only_the_default_install_writes() {
        let default = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = default.path();
        assert_eq!(skip_reason(root, None, None, Some(root)), None);
        // Through a link, the default home is still the default home.
        let link = other.path().join("link");
        std::os::unix::fs::symlink(root, &link).unwrap();
        assert_eq!(skip_reason(&link, None, None, Some(root)), None);
        // `FARCOOLER_NO_CODEX_TRUST=0`, or empty, is not an opt-out.
        assert_eq!(skip_reason(root, None, Some(OsStr::new("0")), Some(root)), None);
        assert_eq!(skip_reason(root, Some(OsStr::new("")), Some(OsStr::new("")), Some(root)), None);

        assert!(skip_reason(root, None, Some(OsStr::new("1")), Some(root)).is_some(), "opted out");
        assert!(skip_reason(root, Some(root.as_os_str()), None, Some(root)).is_some(), "FARCOOLER_HOME, even the default");
        assert!(skip_reason(other.path(), None, None, Some(root)).is_some(), "another home");
        assert!(skip_reason(root, None, None, None).is_some(), "no default home");
    }

    /// The path is the resolved main checkout, the form codex reads: reached
    /// through a symbolic link and from a linked worktree, it is still the
    /// repository's real directory.
    #[test]
    fn the_key_is_the_resolved_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        let git = |args: &[&str], at: &Path| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(at)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        };
        std::fs::create_dir(real.join("repo")).unwrap();
        git(&["init", "-q"], &real.join("repo"));
        git(&["commit", "-q", "--allow-empty", "-m", "init"], &real.join("repo"));
        git(&["worktree", "add", "-q", "../wt", "-b", "wt"], &real.join("repo"));
        std::os::unix::fs::symlink(&real, real.join("link")).unwrap();
        let through_link = real.join("link").join("wt");
        assert_eq!(main_checkout(&through_link), Some(real.join("repo")));
        assert_eq!(main_checkout(&real.join("nowhere")), None);
        // And a common directory named through a link resolves too, which
        // is the half that doesn't lean on git.
        assert_eq!(repository_of(&real.join("link").join("repo").join(".git")), Some(real.join("repo")));
    }
}
