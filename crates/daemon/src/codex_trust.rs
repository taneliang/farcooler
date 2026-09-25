//! Trusting a repository for codex, the way codex's own trust screen does.
//!
//! codex stops in a directory it has not been told to trust and asks "Do you
//! trust the contents of this directory?". What answers it is one table in
//! codex's config, and nothing else: `-c projects."<path>".trust_level=…` on
//! the command line does not skip the screen. Measured on this runner with a
//! throwaway `CODEX_HOME` (codex-cli 0.153.4):
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
//! launches in a worktree Far Cooler forked for a new task — the line cursor's
//! `--trust` is drawn on (`Service::forked_this_worktree`). The config is the
//! owner's file, so the write is as small and as careful as it can be:
//!
//! - **An entry already there is final**, whatever it says. An owner who
//!   answered "No" has `trust_level = "untrusted"`, and that stays.
//! - **A config that won't parse is left alone** and logged, and codex asks.
//! - **`toml_edit`**, so comments, order and spacing everywhere else survive.
//! - **A unique temporary file and a rename**, with the file's own mode, and
//!   no rename at all if the file changed while we were writing.
//! - **No symbolic link is followed** at the config file: a `config.toml`
//!   that is a link (a dotfiles checkout, say) is left alone.
//! - **Never fails a launch.** Every refusal and failure costs exactly what
//!   doing nothing would: codex shows its screen.
//! - **Nothing removes it.** The entry is the repository's, not a worktree's,
//!   and accepting codex's screen leaves it behind in the same way.

use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// What `trust_repository` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trusted {
    /// The entry was added.
    Wrote,
    /// The config already had an entry for this path, trusted or not.
    AlreadyDecided,
    /// Nothing was written, for the reason given.
    LeftAlone(&'static str),
}

/// codex's home directory for this runner's user: `$CODEX_HOME`, or
/// `~/.codex`, the same order codex itself reads them in.
///
/// Under `cfg(test)` this is only ever the directory a test set with
/// `test_home`, and `None` otherwise, so no unit test can reach the owner's
/// real config through a launch.
pub fn codex_home() -> Option<PathBuf> {
    #[cfg(test)]
    {
        test_home::HOME.with(|h| h.borrow().clone())
    }
    #[cfg(not(test))]
    {
        match std::env::var_os("CODEX_HOME") {
            Some(home) if !home.is_empty() => Some(PathBuf::from(home)),
            _ => std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| Path::new(&h).join(".codex")),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_home {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        pub(crate) static HOME: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Point `codex_home` at `home` on this thread until the guard drops.
    pub(crate) fn set(home: &Path) -> Guard {
        HOME.with(|h| *h.borrow_mut() = Some(home.to_path_buf()));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            HOME.with(|h| *h.borrow_mut() = None);
        }
    }
}

/// The path codex trusts a worktree through: the parent of the repository's
/// common git directory, resolved the way codex resolves it.
///
/// A plain `git` process rather than `git::git`, for the same reason
/// `git_tracks` is one: the launch path that calls this is synchronous.
pub fn main_checkout(worktree: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    use std::os::unix::ffi::OsStrExt;
    let common = std::ffi::OsStr::from_bytes(out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout));
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

/// Trust the repository `worktree` belongs to in codex's config, logging what
/// happened. Never fails: see the module doc.
pub fn trust_for_worktree(worktree: &Path) {
    let Some(home) = codex_home() else { return };
    let Some(repository) = main_checkout(worktree) else {
        tracing::info!(worktree = %worktree.display(), "could not find this worktree's repository; codex will ask to trust it");
        return;
    };
    match trust_repository(&home, &repository) {
        Trusted::Wrote => {
            tracing::info!(repository = %repository.display(), "told codex to trust this repository, as its own trust screen would")
        }
        Trusted::AlreadyDecided => {}
        Trusted::LeftAlone(why) => tracing::warn!(
            repository = %repository.display(),
            config = %home.join("config.toml").display(),
            why,
            "left codex's config alone; codex will ask to trust this repository"
        ),
    }
}

/// Add `[projects."<repository>"] trust_level = "trusted"` to
/// `<codex_home>/config.toml`, unless an entry for that path is already there.
///
/// `repository` is written exactly as given; `main_checkout` is what gives it
/// the form codex reads.
pub fn trust_repository(codex_home: &Path, repository: &Path) -> Trusted {
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
    let projects = document.entry("projects").or_insert_with(|| {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        toml_edit::Item::Table(table)
    });
    let inline = projects.is_inline_table();
    let Some(projects) = projects.as_table_like_mut() else {
        return Trusted::LeftAlone("its `projects` is not a table");
    };
    if projects.contains_key(key) {
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
    match replace(&config, document.to_string().as_bytes(), mode, &before) {
        Ok(true) => Trusted::Wrote,
        Ok(false) => Trusted::LeftAlone("the config changed while it was being written"),
        Err(e) => {
            tracing::warn!(error = %e, config = %config.display(), "could not write codex's config");
            Trusted::LeftAlone("the write failed")
        }
    }
}

/// The file's bytes and permission bits, `None` if there is no file, or why
/// it won't be read. `O_NOFOLLOW` makes a symbolic link an error, not a read
/// of whatever it points at.
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
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| "the config can't be read")?;
    Ok(Some((bytes, meta.permissions().mode() & 0o7777)))
}

/// Tells one write's temporary file from another's in the same process.
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write `contents` to a new temporary file beside `path` with `mode`, then
/// rename it over `path` if `path` still holds `before` (or is still absent
/// when `before` is empty), and say whether it did.
///
/// `create_new` with `O_NOFOLLOW` refuses anything already at the temporary
/// name, a planted link included. The check just before the rename is what
/// keeps codex accepting its own screen at the same moment from being undone.
fn replace(path: &Path, contents: &[u8], mode: u32, before: &[u8]) -> std::io::Result<bool> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = dir.join(format!(".config.toml.farcooler-{}-{write}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(contents)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.sync_all()
    })();
    let unmoved = match read_no_follow(path) {
        Ok(Some((now, _))) => now == before,
        Ok(None) => before.is_empty(),
        Err(_) => false,
    };
    let renamed = match written {
        Ok(()) if unmoved => std::fs::rename(&temp, path).map(|()| true),
        Ok(()) => Ok(false),
        Err(e) => Err(e),
    };
    if !matches!(renamed, Ok(true)) {
        let _ = std::fs::remove_file(&temp);
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

    /// The table codex writes when its screen is accepted, byte for byte, with
    /// everything of the owner's above it untouched.
    #[test]
    fn the_entry_is_the_one_codex_writes() {
        let home = home_with(Some(OWNERS));
        assert_eq!(trust_repository(home.path(), Path::new("/private/tmp/r/repo")), Trusted::Wrote);
        assert_eq!(
            read(&home),
            format!("{OWNERS}\n[projects.\"/private/tmp/r/repo\"]\ntrust_level = \"trusted\"\n")
        );
        // Once per repository: the second launch finds it and writes nothing.
        let after = read(&home);
        assert_eq!(trust_repository(home.path(), Path::new("/private/tmp/r/repo")), Trusted::AlreadyDecided);
        assert_eq!(read(&home), after);
    }

    /// No config yet: the file is just the entry, owner-only.
    #[test]
    fn a_missing_config_becomes_the_entry() {
        let home = home_with(None);
        assert_eq!(trust_repository(home.path(), Path::new("/r/repo")), Trusted::Wrote);
        assert_eq!(read(&home), "[projects.\"/r/repo\"]\ntrust_level = \"trusted\"\n");
        let mode = std::fs::metadata(home.path().join("config.toml")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// An owner who told codex "No" has decided. That entry, and one written
    /// by hand in any other shape, is left byte for byte.
    #[test]
    fn an_untrusted_entry_is_left_byte_identical() {
        for config in [
            "[projects.\"/r/repo\"]\ntrust_level = \"untrusted\"\n",
            "projects = { \"/r/repo\" = { trust_level = \"untrusted\" } }\n",
            "[projects]\n\"/r/repo\" = { trust_level = \"untrusted\" }\n",
        ] {
            let home = home_with(Some(config));
            assert_eq!(trust_repository(home.path(), Path::new("/r/repo")), Trusted::AlreadyDecided, "{config}");
            assert_eq!(read(&home), config);
        }
    }

    /// A config that won't parse is the owner's to fix. Nothing is written,
    /// and codex asks, as it would have anyway.
    #[test]
    fn an_unparseable_config_is_left_byte_identical() {
        let broken = "model = \"gpt-5.6\n[projects.\"/elsewhere\"\ntrust_level = trusted\n";
        let home = home_with(Some(broken));
        assert_eq!(
            trust_repository(home.path(), Path::new("/r/repo")),
            Trusted::LeftAlone("the config is not valid TOML")
        );
        assert_eq!(read(&home), broken);
        // `projects` that isn't a table is left alone too, not replaced.
        let home = home_with(Some("projects = 3\n"));
        assert!(matches!(trust_repository(home.path(), Path::new("/r/repo")), Trusted::LeftAlone(_)));
        assert_eq!(read(&home), "projects = 3\n");
    }

    /// An inline `projects` table gets an inline entry, so the file stays in
    /// the shape its owner chose.
    #[test]
    fn an_inline_projects_table_stays_inline() {
        let home = home_with(Some("projects = { \"/a\" = { trust_level = \"trusted\" } }\n"));
        assert_eq!(trust_repository(home.path(), Path::new("/r/repo")), Trusted::Wrote);
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
        assert_eq!(
            trust_repository(home.path(), Path::new("/r/repo")),
            Trusted::LeftAlone("the config is a symbolic link")
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), OWNERS);
        assert!(std::fs::symlink_metadata(home.path().join("config.toml")).unwrap().file_type().is_symlink());
    }

    /// A codex that has never run has no home, and none is made for it.
    #[test]
    fn a_missing_home_is_not_made() {
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join(".codex");
        assert!(matches!(trust_repository(&home, Path::new("/r/repo")), Trusted::LeftAlone(_)));
        assert!(!home.exists());
    }

    /// No temporary file is left beside the config, written or not.
    #[test]
    fn no_temporary_file_is_left_behind() {
        let home = home_with(Some(OWNERS));
        trust_repository(home.path(), Path::new("/r/repo"));
        trust_repository(home.path(), Path::new("/r/repo"));
        let names: Vec<_> = std::fs::read_dir(home.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
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
