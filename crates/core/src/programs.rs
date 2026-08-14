//! Finding a program *the user* installed.
//!
//! The daemon does not run in a shell, so it does not inherit a usable `PATH`.
//! This is the same fact `shell::login_shell` exists for, hitting a different
//! thing: launchd starts the daemon as a login item on a Mac and sshd starts it
//! on a remote host, and what those hand down is whatever the session that
//! spawned them happened to carry.
//!
//! For a Dock-launched Mac app that is launchd's own default —
//! `/usr/bin:/bin:/usr/sbin:/sbin` — which contains no Homebrew prefix, no
//! MacPorts, no Nix, and no nvm. So `Command::new("tmux")` fails with
//! `ENOENT`, the inventory is unusable, and `derive_terminal` reports every
//! terminal as `Lost`. The app is not degraded, it is dead, and it looks like
//! a bug in Far Cooler rather than a missing directory.
//!
//! **A fixed list of prefixes cannot fix this**, which is worth stating because
//! it is the obvious fix and it is not enough: `npx` under nvm lives at
//! `~/.nvm/versions/node/v20.12.2/bin`, with the version *in the path*, and an
//! agent installed by a tool like that is not in any predictable directory. The
//! only thing that reliably knows where the user's programs are is the user's
//! own shell, so that is what gets asked.
//!
//! Order, cheapest first:
//!
//! 1. A name that is already a path — used as given, so a config file can name
//!    a program outright.
//! 2. The inherited `PATH`. Correct whenever the daemon was started from a
//!    shell, which is every developer run and every `farcooler daemon ensure`
//!    from a terminal, and it keeps that case exactly as fast as it was.
//! 3. The user's **login shell**, asked once for where the program is. This is
//!    the authoritative answer — it is the same program they would get by
//!    typing the name in their terminal.
//! 4. Known install prefixes, for when even the shell cannot answer.
//!
//! Finding the program is only half of it. A program found this way still has
//! to RUN, and most of the ones this exists for cannot run without the same
//! directories on their own `PATH`: `npx` is a `#!/usr/bin/env node` script, so
//! an `npx` resolved out of `~/.nvm/…/bin` and then spawned with launchd's or
//! systemd's `PATH` fails to find `node` and dies — with the program itself
//! having resolved perfectly. So `search_path` hands the whole list to the
//! child, and callers that spawn a user's program are expected to use it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where package managers put things, for when the login shell cannot answer.
///
/// A backstop rather than the mechanism: step 3 above covers these and more.
/// This exists for a shell whose profile fails to load, or a `sh` with no
/// login-mode profile at all, which is what some minimal containers ship.
const KNOWN_PREFIXES: &[&str] = &[
    // Homebrew, Apple Silicon then Intel.
    "/opt/homebrew/bin",
    "/usr/local/bin",
    // MacPorts.
    "/opt/local/bin",
    // Nix, system profile then per-user.
    "/run/current-system/sw/bin",
    // Linuxbrew.
    "/home/linuxbrew/.linuxbrew/bin",
    // Where a distribution's own packages land.
    "/usr/bin",
    "/bin",
];

/// The absolute path to `name`, or `None` if nothing can find it.
///
/// Cached: `tmux` is resolved on the way into every tmux command, and asking a
/// login shell each time would put a shell spawn in front of every keystroke.
pub fn find(name: &str) -> Option<PathBuf> {
    // A path, not a name. Used as given so a config file can point at a program
    // in a directory nothing here would guess.
    if name.contains('/') {
        let path = PathBuf::from(name);
        return is_executable(&path).then_some(path);
    }

    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = cache.get(name) {
        return found.clone();
    }
    let found = resolve(name);
    if found.is_none() {
        tracing::warn!(
            program = %name,
            "could not find it on PATH, through the login shell, or in any known \
             install prefix"
        );
    }
    cache.insert(name.to_string(), found.clone());
    found
}

/// The `PATH` to give a program *the user* installed, when spawning it.
///
/// Every directory `find` would look in, in the order it looks: what this
/// process inherited, then what the login shell reports, then the known
/// prefixes. Resolving the program and then handing the child a stripped `PATH`
/// only moves the failure one process along — see the note at the top of this
/// module — and moves it somewhere much harder to read, because the child dies
/// with its own words rather than ours.
///
/// Unlike `find`, this always asks the login shell rather than stopping at the
/// first answer: the point is the whole list, not the first hit. That is one
/// shell spawn, cached for the life of the process, and it is paid on the path
/// that starts an agent rather than on the path that resolves `tmux` in front
/// of a keystroke.
pub fn search_path() -> OsString {
    static PATH: std::sync::OnceLock<OsString> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs = inherited_path().unwrap_or_default();
        dirs.extend(login_shell_path().unwrap_or_default());
        dirs.extend(prefixes());
        join_unique(dirs)
    })
    .clone()
}

/// `dirs` as a `PATH`, first occurrence winning and duplicates dropped.
///
/// Duplicates are the normal case, not an edge one: the inherited `PATH` and
/// the login shell's overlap almost entirely, and `KNOWN_PREFIXES` repeats
/// `/usr/bin` and `/bin` on top of both. Left in, a Linux daemon's `PATH` would
/// arrive at the child three times over.
///
/// A directory whose name contains the separator cannot be expressed in a
/// `PATH` at all, so it is dropped rather than allowed to fail the join and
/// take every other directory with it.
fn join_unique(dirs: Vec<PathBuf>) -> OsString {
    let mut seen = std::collections::HashSet::new();
    let kept = dirs
        .into_iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| !dir.to_string_lossy().contains(SEPARATOR))
        .filter(|dir| seen.insert(dir.clone()));
    std::env::join_paths(kept).unwrap_or_default()
}

#[cfg(unix)]
const SEPARATOR: char = ':';
#[cfg(not(unix))]
const SEPARATOR: char = ';';

type Cache = std::sync::Mutex<std::collections::HashMap<String, Option<PathBuf>>>;

fn cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn resolve(name: &str) -> Option<PathBuf> {
    inherited_path()
        .and_then(|dirs| find_in(name, &dirs))
        .or_else(|| login_shell_path().and_then(|dirs| find_in(name, &dirs)))
        .or_else(|| find_in(name, &prefixes()))
}

/// `PATH` as it was inherited, split into directories.
fn inherited_path() -> Option<Vec<PathBuf>> {
    let raw = std::env::var_os("PATH")?;
    Some(std::env::split_paths(&raw).collect())
}

fn prefixes() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = KNOWN_PREFIXES.iter().map(PathBuf::from).collect();
    // `~/.nix-profile/bin` and `~/.local/bin` are per-user, so they cannot be
    // constants.
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".nix-profile/bin"));
        dirs.push(home.join(".local/bin"));
    }
    dirs
}

/// What the user's login shell says `PATH` is.
///
/// A **login** shell (`-l`), because that is what reads the profile where a
/// package manager puts its `PATH` line — `.zprofile`, `.bash_profile`,
/// `config.fish`. A non-login shell reads none of it and would answer with the
/// same stripped `PATH` this function exists to get around.
///
/// Failure is not an error worth reporting. A shell that cannot start, a
/// profile that exits non-zero, a machine with no passwd entry: each simply
/// means the next step gets a turn.
fn login_shell_path() -> Option<Vec<PathBuf>> {
    let shell = crate::shell::login_shell();
    let output = std::process::Command::new(&shell)
        // `-l` for the profile, `-c` for the one command. Printing `$PATH`
        // rather than `command -v <name>` so ONE shell spawn answers for every
        // program this is ever asked about, however many that turns out to be.
        .args(["-lc", "printf %s \"$PATH\""])
        // No stdin, and stderr discarded: a profile that prints a banner or a
        // warning is extremely common and none of it is this function's news.
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(std::env::split_paths(raw).collect())
}

/// The first `dir/name` in `dirs` that is an executable file.
///
/// Split out from `resolve` so it can be tested against a temp directory: the
/// real search reads process-global environment and the real prefixes are not
/// something a test can create.
fn find_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// Whether this is a file something can actually be spawned from.
///
/// The executable bit as well as existence, because a directory named `tmux` on
/// the search path would otherwise be returned and then fail at spawn — the
/// same `ENOENT`-shaped confusion this module exists to remove.
fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "farcooler-programs-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn a_program_is_found_in_the_first_directory_that_has_it() {
        let first = scratch("first");
        let second = scratch("second");
        executable(&second, "thing");
        let expected = executable(&first, "thing");

        assert_eq!(
            find_in("thing", &[first.clone(), second.clone()]),
            Some(expected),
            "earlier directories win, the way PATH does"
        );
    }

    #[test]
    fn a_directory_with_the_right_name_is_not_a_program() {
        // The failure this guards is shaped exactly like the bug this module
        // exists for: something is "found", the spawn fails with ENOENT anyway,
        // and the message says nothing about why.
        let dir = scratch("dir-named-like-program");
        std::fs::create_dir_all(dir.join("tmux")).unwrap();
        assert_eq!(find_in("tmux", &[dir]), None);
    }

    #[test]
    fn a_file_without_the_executable_bit_is_not_a_program() {
        let dir = scratch("not-executable");
        std::fs::write(dir.join("tmux"), "not a program").unwrap();
        assert_eq!(find_in("tmux", &[dir]), None);
    }

    #[test]
    fn nothing_anywhere_is_none_rather_than_a_guess() {
        let dir = scratch("empty");
        assert_eq!(find_in("definitely-not-installed", &[dir]), None);
    }

    #[test]
    fn a_name_that_is_already_a_path_is_used_as_given() {
        // So a config file can name a program in a directory nothing here would
        // ever guess, which is the escape hatch for an exotic install.
        let dir = scratch("explicit-path");
        let program = executable(&dir, "mine");
        assert_eq!(find(program.to_str().unwrap()), Some(program.clone()));

        let missing = dir.join("nope");
        assert_eq!(find(missing.to_str().unwrap()), None, "a path that is not there is not found");
    }

    #[test]
    fn the_known_prefixes_cover_both_homebrew_layouts() {
        // The two that matter on a Mac, and the reason this list exists at all:
        // a Dock-launched app gets neither of them from launchd.
        assert!(KNOWN_PREFIXES.contains(&"/opt/homebrew/bin"), "Apple Silicon Homebrew");
        assert!(KNOWN_PREFIXES.contains(&"/usr/local/bin"), "Intel Homebrew");
    }

    #[test]
    fn a_program_every_unix_has_is_found_end_to_end() {
        // Exercises the real `find`, cache included, against something that is
        // at `/bin/sh` on every machine this runs on.
        let found = find("sh").expect("sh exists on every unix");
        assert!(found.is_absolute());
        assert!(is_executable(&found));
        // Twice, so the cached path is returned rather than re-resolved into
        // something different.
        assert_eq!(find("sh"), Some(found));
    }

    #[test]
    fn the_search_path_keeps_the_first_of_each_directory_and_drops_the_rest() {
        // Duplicates are the normal case here, not an edge one: the inherited
        // PATH and the login shell's overlap almost entirely, and the known
        // prefixes repeat /usr/bin and /bin on top of both.
        let joined = join_unique(vec![
            PathBuf::from("/first"),
            PathBuf::from("/second"),
            PathBuf::from("/first"),
        ]);
        assert_eq!(joined.to_string_lossy(), "/first:/second");
    }

    #[test]
    fn what_the_login_shell_knows_survives_a_daemons_stripped_path() {
        // The failure, in the shape it actually had. A `systemd --user` daemon
        // on a Linux machine has this PATH and no other; the tmux server it
        // starts hands exactly that down, and the agent adapter spawned under
        // it could not find the node its own shebang asks for. The login
        // shell's answer is the whole difference between a working flip and a
        // pane that goes quiet.
        let systemd = ["/usr/local/bin", "/usr/bin", "/bin", "/usr/games", "/snap/bin"];
        let login = ["/home/e/.nvm/versions/node/v20.12.2/bin", "/home/e/.local/bin", "/usr/bin"];
        let joined = join_unique(
            systemd.iter().chain(login.iter()).map(PathBuf::from).collect(),
        );
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        for dir in login {
            assert!(dirs.contains(&PathBuf::from(dir)), "{dir} must survive");
        }
        assert_eq!(dirs.iter().filter(|d| d.as_os_str() == "/usr/bin").count(), 1);
    }

    #[test]
    fn a_directory_that_cannot_be_written_in_a_path_does_not_take_the_others_with_it() {
        // `join_paths` refuses the whole list when one entry contains the
        // separator. Refusing it would hand the child an EMPTY PATH — strictly
        // worse than the stripped one this exists to replace.
        let joined = join_unique(vec![
            PathBuf::from("/fine"),
            PathBuf::from("/impossible:name"),
            PathBuf::from("/also-fine"),
        ]);
        assert_eq!(joined.to_string_lossy(), "/fine:/also-fine");
    }

    #[test]
    fn the_search_path_contains_everything_this_process_already_had() {
        // The property a spawned adapter depends on. Anything narrower than
        // "a superset of what we inherited" would be a regression for the
        // developer run, where the inherited PATH is already the right answer.
        let search: Vec<PathBuf> = std::env::split_paths(&search_path()).collect();
        for dir in inherited_path().expect("this test process has a PATH") {
            assert!(search.contains(&dir), "{} is missing from the search path", dir.display());
        }
    }

    #[test]
    fn the_search_path_can_find_the_programs_it_resolves() {
        // The bug this module grew to cover: `npx` is a `#!/usr/bin/env node`
        // script, so resolving it and then spawning it with a PATH that has no
        // `node` fails in the CHILD, with the child's words rather than ours.
        // Asserted against `sh`, which every unix has and `find` locates the
        // same way.
        let found = find("sh").expect("sh exists on every unix");
        let parent = found.parent().expect("an absolute path has a parent");
        let search: Vec<PathBuf> = std::env::split_paths(&search_path()).collect();
        assert!(
            search.contains(&parent.to_path_buf()),
            "the search path must contain {}, where `find` says sh lives",
            parent.display()
        );
    }

    #[test]
    fn a_program_that_does_not_exist_is_none_and_stays_none() {
        // The negative is cached too. Without that, every tmux command on a
        // machine with no tmux would spawn a login shell to be told so again.
        assert_eq!(find("farcooler-no-such-program-anywhere"), None);
        assert_eq!(find("farcooler-no-such-program-anywhere"), None);
    }
}
