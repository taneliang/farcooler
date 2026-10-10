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

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

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
/// A program that was found is cached for good; one that was not is asked
/// about again after `RETRY_AFTER`, so installing it, or a login shell that
/// failed once, does not need a daemon restart.
pub fn find(name: &str) -> Option<PathBuf> {
    finder().find(name)
}

/// `find`'s answer if it is already cached as found, without ever asking the
/// login shell.
///
/// For async callers: `find` can wait up to `LOGIN_SHELL_TIMEOUT` on a login
/// shell, which must not happen on a runtime worker. They take this answer
/// when there is one — every call after the first, for a program that was
/// found — and otherwise run `find` on a blocking thread.
pub fn known(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return None;
    }
    match lock(&finder().names).get(name) {
        Some(Known::Found(path)) => Some(path.clone()),
        _ => None,
    }
}

/// How long the login shell gets to print its `PATH`.
///
/// Generous for a real profile — nvm and oh-my-zsh together take a second or
/// two — and short enough that a profile waiting on the network, or on a
/// prompt nobody will answer, costs one bounded wait rather than every tmux
/// command for the life of the daemon.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a failure — a login shell that timed out or failed, or a program
/// nobody could find — is believed before it is asked about again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// `find`'s cache, with the login shell it asks.
///
/// A struct rather than bare statics so a test can build one around a fake
/// shell and a short clock; the process uses the one in `finder()`.
struct Finder {
    /// Never held across a resolve, only across a map read or write. The slow
    /// part — the login shell — has its own single-flight in `LoginPath`, so
    /// a lookup that is cheap (a program on the inherited `PATH`) never waits
    /// behind one that is not.
    names: Mutex<HashMap<String, Known>>,
    login: LoginPath,
}

enum Known {
    Found(PathBuf),
    Missing { since: Instant },
}

impl Finder {
    fn new(login: LoginPath) -> Self {
        Self { names: Mutex::default(), login }
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        // A path, not a name. Used as given so a config file can point at a
        // program in a directory nothing here would guess.
        if name.contains('/') {
            let path = PathBuf::from(name);
            return is_executable(&path).then_some(path);
        }

        match lock(&self.names).get(name) {
            Some(Known::Found(path)) => return Some(path.clone()),
            Some(Known::Missing { since }) if since.elapsed() < self.login.retry_after => {
                return None;
            }
            _ => {}
        }

        let found = self.resolve(name);
        let known = match &found {
            Some(path) => Known::Found(path.clone()),
            None => {
                tracing::warn!(
                    program = %name,
                    "could not find it on PATH, through the login shell, or in any known \
                     install prefix"
                );
                Known::Missing { since: Instant::now() }
            }
        };
        lock(&self.names).insert(name.to_string(), known);
        found
    }

    fn resolve(&self, name: &str) -> Option<PathBuf> {
        inherited_path()
            .and_then(|dirs| find_in(name, &dirs))
            .or_else(|| self.login.get().and_then(|dirs| find_in(name, &dirs)))
            .or_else(|| find_in(name, &prefixes()))
    }
}

fn finder() -> &'static Finder {
    static FINDER: OnceLock<Finder> = OnceLock::new();
    FINDER.get_or_init(|| {
        Finder::new(LoginPath::new(
            PathBuf::from(crate::shell::login_shell()),
            LOGIN_SHELL_TIMEOUT,
            RETRY_AFTER,
        ))
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
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
/// shell spawn, shared with `find`, and it is paid on the path that starts an
/// agent rather than on the path that resolves `tmux` in front of a keystroke.
///
/// Kept for the life of the process only once the login shell has answered;
/// without its answer the list is built again on the next call, so a shell
/// that failed once is asked again after `RETRY_AFTER`.
pub fn search_path() -> OsString {
    static PATH: OnceLock<OsString> = OnceLock::new();
    if let Some(path) = PATH.get() {
        return path.clone();
    }
    let mut dirs = inherited_path().unwrap_or_default();
    let login = finder().login.get();
    let complete = login.is_some();
    dirs.extend(login.unwrap_or_default());
    dirs.extend(prefixes());
    let joined = join_unique(dirs);
    if complete {
        let _ = PATH.set(joined.clone());
    }
    joined
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

/// What the user's login shell says `PATH` is, asked at most once at a time.
///
/// A **login** shell (`-l`), because that is what reads the profile where a
/// package manager puts its `PATH` line — `.zprofile`, `.bash_profile`,
/// `config.fish`. A non-login shell reads none of it and would answer with the
/// same stripped `PATH` this exists to get around.
///
/// Three properties, each the fix for a way one bad profile used to wedge every
/// tmux command in the daemon:
///
/// - **A deadline.** A profile can wait on the network or on a prompt forever.
///   Past `timeout` the shell's whole process group is killed and reaped, so a
///   `sleep` or `curl` it started goes too.
/// - **Single-flight, without a lock across the spawn.** One caller asks; the
///   others wait on a condition variable for its answer, and a `find` that
///   never needs the login shell never touches this at all.
/// - **A failure is believed for `retry_after`, not forever.** A profile that
///   failed once because the network was down gets another turn.
struct LoginPath {
    shell: PathBuf,
    timeout: Duration,
    retry_after: Duration,
    state: Mutex<Asked>,
    answered: Condvar,
}

enum Asked {
    Never,
    Asking,
    Answered(Vec<PathBuf>),
    Failed { at: Instant },
}

impl LoginPath {
    fn new(shell: PathBuf, timeout: Duration, retry_after: Duration) -> Self {
        Self { shell, timeout, retry_after, state: Mutex::new(Asked::Never), answered: Condvar::new() }
    }

    /// The login shell's `PATH`, or `None` while a recent failure stands.
    ///
    /// Failure is not an error worth surfacing. A shell that cannot start, a
    /// profile that exits non-zero or hangs, a host with no passwd entry: each
    /// is logged once with its reason and simply means the next step gets a
    /// turn.
    fn get(&self) -> Option<Vec<PathBuf>> {
        let mut state = lock(&self.state);
        loop {
            match &*state {
                Asked::Answered(dirs) => return Some(dirs.clone()),
                Asked::Failed { at } if at.elapsed() < self.retry_after => return None,
                // Bounded: whoever is asking gives up after `timeout`, and the
                // guard below settles the state even if that caller panics.
                Asked::Asking => {
                    state = self.answered.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                Asked::Never | Asked::Failed { .. } => break,
            }
        }
        *state = Asked::Asking;
        drop(state);

        let mut settle = Settle { login: self, outcome: None };
        let answer = self.ask();
        settle.outcome = Some(answer.clone());
        drop(settle);
        answer.ok()
    }

    fn ask(&self) -> Result<Vec<PathBuf>, String> {
        use std::io::Read;
        use std::os::unix::process::CommandExt;

        let mut child = std::process::Command::new(&self.shell)
            // `-l` for the profile, `-c` for the one command. Printing `$PATH`
            // rather than `command -v <name>` so ONE shell spawn answers for
            // every program this is ever asked about.
            .args(["-lc", "printf %s \"$PATH\""])
            // No stdin, and stderr discarded: a profile that prints a banner or
            // a warning is extremely common and none of it is this function's
            // news.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            // Its own process group, so a timeout can kill whatever the
            // profile started along with the shell itself.
            .process_group(0)
            .spawn()
            .map_err(|e| format!("could not start it: {e}"))?;

        // Read on a thread, so a full pipe cannot stall the shell and a
        // background job holding the pipe open cannot stall this.
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            let _ = tx.send(out);
        });

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    kill_group_and_reap(&mut child);
                    return Err(format!("timed out after {:?}", self.timeout));
                }
                Err(e) => {
                    kill_group_and_reap(&mut child);
                    return Err(format!("could not wait for it: {e}"));
                }
            }
        };
        // The shell is gone, but anything its profile started in the
        // background is still in its group, and may be holding stdout open —
        // an `ssh-agent`, a `sleep`, a stalled `curl`. None of it outlives this
        // throwaway shell's purpose, so the group goes too, which also closes
        // the pipe's last writers and lets the reader thread finish. The id is
        // safe to signal after the reap: a process group's id is not reused
        // while any member of the group is alive.
        kill_group(&child);
        if !status.success() {
            return Err(format!("it exited with {status}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let out = rx
            .recv_timeout(remaining.max(Duration::from_millis(100)))
            .map_err(|_| "it exited, but something outside its group kept its output open".to_string())?;
        let raw = String::from_utf8(out).map_err(|_| "its PATH was not UTF-8".to_string())?;
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("it printed an empty PATH".to_string());
        }
        Ok(std::env::split_paths(raw).collect())
    }
}

/// Settles `LoginPath::get`'s state on every way out, a panic included, so a
/// waiter is never left waiting on an `Asking` nobody will finish.
struct Settle<'a> {
    login: &'a LoginPath,
    outcome: Option<Result<Vec<PathBuf>, String>>,
}

impl Drop for Settle<'_> {
    fn drop(&mut self) {
        let next = match self.outcome.take() {
            Some(Ok(dirs)) => Asked::Answered(dirs),
            Some(Err(reason)) => {
                tracing::warn!(
                    shell = %self.login.shell.display(),
                    %reason,
                    retry_in = ?self.login.retry_after,
                    "the login shell did not say what PATH is"
                );
                Asked::Failed { at: Instant::now() }
            }
            None => Asked::Failed { at: Instant::now() },
        };
        *lock(&self.login.state) = next;
        self.login.answered.notify_all();
    }
}

/// SIGKILL the child's process group, then wait for the child so it is not
/// left a zombie.
fn kill_group_and_reap(child: &mut std::process::Child) {
    kill_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

/// SIGKILL every process in the group `child` leads.
fn kill_group(child: &std::process::Child) {
    // SAFETY: `kill` takes no pointers. The child was spawned with
    // `process_group(0)`, so its pid is the id of the group it leads.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
}

/// The first `dir/name` in `dirs` that is an executable file.
///
/// Split out from `resolve` so it can be tested against a temp directory: the
/// real search reads process-global environment and the real prefixes are not
/// something a test can create.
///
/// Absolute directories only. A relative one (`.`, or the empty entry a
/// stray `:` leaves in `PATH`) names a different place for every working
/// directory, and the daemon starts programs in worktrees an agent writes:
/// with `PATH=.:/usr/bin`, a `git` planted in the worktree ran in place of
/// `/usr/bin/git`.
fn find_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .filter(|dir| dir.is_absolute())
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
    fn a_relative_directory_is_never_searched() {
        // The same directory spelled relative to the working directory, which
        // is what `.` in `PATH` is: it finds the planted program only by
        // depending on where the search ran.
        let dir = scratch("relative");
        executable(&dir, "thing");
        let here = std::env::current_dir().unwrap();
        let up: PathBuf = here.components().skip(1).map(|_| "..").collect();
        let relative = up.join(dir.strip_prefix("/").unwrap());
        assert!(relative.is_relative() && relative.join("thing").exists(), "{relative:?} names the program");

        assert_eq!(find_in("thing", &[relative]), None);
        assert_eq!(find_in("thing", &[PathBuf::new()]), None, "the empty entry is the working directory too");
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
        // at `/bin/sh` on every host this runs on.
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
        // on a Linux host has this PATH and no other; the tmux server it
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
        // The negative is cached too, for `RETRY_AFTER`. Without that, every
        // tmux command on a host with no tmux would spawn a login shell to be
        // told so again.
        assert_eq!(find("farcooler-no-such-program-anywhere"), None);
        assert_eq!(find("farcooler-no-such-program-anywhere"), None);
    }

    /// A fake login shell: `body` as a `/bin/sh` script, ignoring the `-lc`
    /// it is handed.
    ///
    /// Not returned until it can be executed. On Linux a script just written
    /// cannot be exec'd (`ETXTBSY`) while any other thread's `fork` still holds
    /// a copy of the write descriptor, and the other tests in this binary fork
    /// all the time; on a loaded runner the window was wide enough for the
    /// shell to fail to start and the test to wait for a pid file that never
    /// came. Once the write descriptor is closed no new fork can inherit it,
    /// so one exec that succeeds proves the rest will. The probe is the script
    /// run with `FAKE_SHELL_PROBE` set, which it answers by exiting first.
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-shell");
        std::fs::write(
            &path,
            format!("#!/bin/sh\n[ -n \"$FAKE_SHELL_PROBE\" ] && exit 0\n{body}\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match std::process::Command::new(&path).env("FAKE_SHELL_PROBE", "1").status() {
                Ok(_) => return path,
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("cannot run the fake shell {}: {e}", path.display()),
            }
        }
    }

    fn wait_for_file(path: &Path) -> String {
        // Generous: the file is written by a process this test started, on a
        // runner that may be running a hundred other tests. A shell that
        // never runs fails the test just the same, thirty seconds later.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = std::fs::read_to_string(path)
                && !text.trim().is_empty()
            {
                return text.trim().to_string();
            }
            assert!(Instant::now() < deadline, "{} never appeared", path.display());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_login_shell_that_hangs_times_out_and_blocks_no_one_else() {
        // The wedge, as it happened: a profile waiting on the network never
        // returns. The lookup that asked must give up, kill and reap the
        // shell, and — while it is still waiting — a lookup that never needed
        // the login shell must not queue up behind it.
        let dir = scratch("hanging-shell");
        let pid_file = dir.join("pid");
        let sleeper_file = dir.join("sleeper");
        // A background job as well as the shell itself, the way a profile's
        // `curl` or `sleep` would be: killing only the shell would leave it.
        let shell = fake_shell(
            &dir,
            &format!(
                "sleep 1000 &\necho $! > '{}'\necho $$ > '{}'\nwait",
                sleeper_file.display(),
                pid_file.display()
            ),
        );
        // Long enough that a loaded runner has started the shell and let it
        // write its pids before the lookup gives up on it and kills it.
        let timeout = Duration::from_secs(10);
        let finder = std::sync::Arc::new(Finder::new(LoginPath::new(
            shell,
            timeout,
            Duration::from_secs(60),
        )));

        let stuck = {
            let finder = finder.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let found = finder.find("farcooler-not-on-any-path");
                (found, started.elapsed())
            })
        };
        let pid: libc::pid_t = wait_for_file(&pid_file).parse().unwrap();
        let sleeper: libc::pid_t = wait_for_file(&sleeper_file).parse().unwrap();

        // The other caller, while the shell is still hanging.
        let started = Instant::now();
        let sh = finder.find("sh");
        let waited = started.elapsed();
        assert!(sh.is_some(), "sh is on the inherited PATH");
        assert!(
            waited < timeout / 2,
            "a lookup that needs no login shell waited {waited:?} behind one that does"
        );
        assert!(!stuck.is_finished(), "the hanging lookup should still be waiting");

        let (found, took) = stuck.join().unwrap();
        assert_eq!(found, None);
        assert!(took >= timeout, "gave up after {took:?}, before the timeout");
        assert!(took < timeout + Duration::from_secs(2), "took {took:?}; the timeout is not a bound");

        // Killed and reaped: not running, and not a zombie either, because
        // `kill(pid, 0)` still succeeds on a zombie.
        // SAFETY: signal 0 checks for existence and delivers nothing.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "the timed-out shell {pid} is still around");
        assert!(gone_soon(sleeper), "the shell's background job {sleeper} outlived it");
    }

    /// Whether `pid` stops existing within a couple of seconds. A grandchild
    /// is reaped by init or launchd once it is reparented, not by this
    /// process, so it may linger as a zombie for a moment after the kill.
    fn gone_soon(pid: libc::pid_t) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            // SAFETY: signal 0 checks for existence and delivers nothing.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Not left running for the rest of the suite either way.
        // SAFETY: as above; `pid` is the sleeper this test's fake started.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        false
    }

    #[test]
    fn a_background_job_holding_the_output_is_killed_and_the_answer_kept() {
        // A profile that starts something in the background — an agent, a
        // `sleep`, a slow `curl` — hands it the shell's stdout. The shell
        // answers and exits, but the pipe stays open while the job lives.
        // The answer must still be read, promptly, and the job must not be
        // left running as an orphan.
        let dir = scratch("background-job-shell");
        let sleeper_file = dir.join("sleeper");
        let shell = fake_shell(
            &dir,
            &format!("sleep 1000 &\necho $! > '{}'\nprintf %s /answered", sleeper_file.display()),
        );
        let timeout = Duration::from_secs(3);
        let login = LoginPath::new(shell, timeout, Duration::from_secs(60));

        let started = Instant::now();
        let answer = login.get();
        let took = started.elapsed();
        // Checked before the asserts, so a failure does not leave it running.
        let sleeper: libc::pid_t = wait_for_file(&sleeper_file).parse().unwrap();
        let killed = gone_soon(sleeper);

        assert_eq!(answer, Some(vec![PathBuf::from("/answered")]));
        assert!(took < Duration::from_secs(1), "waited {took:?} on a pipe a background job held open");
        assert!(killed, "the profile's background job {sleeper} was left running");
    }

    #[test]
    fn a_failed_login_shell_is_asked_again_after_its_ttl() {
        // A profile that failed once — the network was down, a mount was
        // late — must get another turn, not be believed for the life of the
        // daemon. The fake fails on its first run and answers after that.
        let dir = scratch("retry-shell");
        let count = dir.join("count");
        let answer = dir.join("answer-dir");
        let shell = fake_shell(
            &dir,
            &format!(
                "echo run >> '{count}'\n\
                 [ \"$(wc -l < '{count}')\" -gt 1 ] || exit 1\n\
                 printf %s '{answer}'",
                count = count.display(),
                answer = answer.display(),
            ),
        );
        let retry_after = Duration::from_millis(300);
        let login = LoginPath::new(shell, Duration::from_secs(5), retry_after);
        let runs = || std::fs::read_to_string(&count).unwrap_or_default().lines().count();

        assert_eq!(login.get(), None, "the first run fails");
        assert_eq!(login.get(), None, "and is believed for a while");
        assert_eq!(runs(), 1, "without asking the shell again");

        std::thread::sleep(retry_after + Duration::from_millis(100));
        assert_eq!(login.get(), Some(vec![answer.clone()]), "asked again once the TTL is up");
        assert_eq!(login.get(), Some(vec![answer]), "and an answer is kept");
        assert_eq!(runs(), 2);
    }

    #[test]
    fn callers_that_arrive_together_share_one_login_shell() {
        // Single-flight: eight lookups at once is one shell, not eight.
        let dir = scratch("single-flight-shell");
        let count = dir.join("count");
        let shell = fake_shell(
            &dir,
            &format!("echo run >> '{}'\nsleep 0.3\nprintf %s /shared", count.display()),
        );
        let login = std::sync::Arc::new(LoginPath::new(
            shell,
            Duration::from_secs(5),
            Duration::from_secs(60),
        ));
        let callers: Vec<_> = (0..8)
            .map(|_| {
                let login = login.clone();
                std::thread::spawn(move || login.get())
            })
            .collect();
        for caller in callers {
            assert_eq!(caller.join().unwrap(), Some(vec![PathBuf::from("/shared")]));
        }
        let runs = std::fs::read_to_string(&count).unwrap().lines().count();
        assert_eq!(runs, 1, "{runs} shells for one question");
    }
}
