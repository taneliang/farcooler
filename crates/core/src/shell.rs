//! Which shell is *the user's* shell.
//!
//! One answer, shared, because two of them would disagree: the daemon builds
//! the command a pane runs and `tmux` sets the `default-shell` that command is
//! launched through, and a pane whose wrapper and whose contents are different
//! shells is a pane where `$SHELL` lies about what you are typing into.

/// The user's login shell, as an absolute path.
///
/// From the password database rather than `$SHELL`, because the daemon does
/// not run in a shell and therefore does not inherit a meaningful one. It is
/// started by launchd as a login item on the Mac and by sshd on a remote host,
/// and what those hand down is whatever the session that spawned them happened
/// to carry. On a Mac whose login shell is fish, the daemon's environment
/// still reads `SHELL=/bin/zsh` — so every terminal Far Cooler opened was the
/// wrong shell, silently, with the user's own config unread and their aliases,
/// functions and prompt missing.
///
/// The passwd entry IS the definition of a login shell: it is what `chsh`
/// writes, what `login` reads, and what Terminal.app and iTerm2 both open. On
/// macOS the directory service backs `getpwuid`, so a user who comes from a
/// directory rather than `/etc/passwd` resolves the same way.
///
/// `$SHELL` remains the fallback for when the database cannot answer — a uid
/// with no passwd entry at all, which happens inside some containers — and
/// `/bin/sh` behind that, because a shell that exists everywhere beats a
/// terminal that will not open.
///
/// A test run names its own shell instead, in `TEST_SHELL`. See there.
pub fn login_shell() -> String {
    test_shell().unwrap_or_else(users_shell)
}

/// The variable a test run sets to the shell its panes start, in place of the
/// user's login shell (ov-547).
///
/// Every tmux pane a test opens starts through the login shell: as the
/// server's `default-shell`, and as the `-ilc` wrapper around an agent's
/// command. On the Mac this was built on that is fish, which reads the
/// owner's whole config and prints a greeting, about 0.25 s and a screenful
/// at rest and seconds under a parallel run. Tests typed into panes still
/// starting and read the greeting where a prompt or a dialog belonged. So the
/// workspace's `.cargo/config.toml` sets this to `/bin/sh` for everything
/// cargo runs, which is the tests (and nextest) and the daemons they spawn.
///
/// An absolute path or nothing: a relative one is ignored, as unset. Read
/// only by a debug build: a release build can't be steered by it at all. A
/// debug daemon a developer's `cargo run` leaves running would otherwise
/// keep it for its life, so the CLI drops it when it starts one
/// (`daemon_link::spawn_daemon`), and the log says so whenever it's used.
pub const TEST_SHELL: &str = "FARCOOLER_TEST_SHELL";

fn test_shell() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let shell = named(std::env::var(TEST_SHELL).ok())?;
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| tracing::warn!(%shell, "panes start this shell, not the login shell ({TEST_SHELL})"));
    Some(shell)
}

/// `TEST_SHELL`'s value, when it names a shell.
fn named(value: Option<String>) -> Option<String> {
    value.filter(|shell| shell.starts_with('/'))
}

/// The user's own login shell, whatever a test run asked for.
fn users_shell() -> String {
    from_passwd()
        .or_else(|| std::env::var("SHELL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// `pw_shell` for the uid this process runs as, or `None` when there is no
/// entry to read.
fn from_passwd() -> Option<String> {
    // The reentrant form rather than plain `getpwuid`, which hands back a
    // pointer into a shared static that another thread's call can overwrite
    // underneath the read. This is a multi-threaded daemon; the plain one is
    // a data race waiting for a second caller.
    //
    // SAFETY: every pointer handed to `getpwuid_r` is to storage owned here
    // and outliving the call. It reports its answer through `found`, which is
    // left null when the uid has no entry, so the result is checked rather
    // than assumed. `pw_shell` is only dereferenced after both the return code
    // and `found` say there is a record to read.
    unsafe {
        let mut record: libc::passwd = std::mem::zeroed();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // Comfortably above what any real entry needs. `sysconf(_SC_GETPW_R_SIZE_MAX)`
        // would be exact, but it is allowed to answer -1 and then this needs a
        // growth loop for a buffer that is never going to be too small — and
        // the cost of being wrong is one fallback to `$SHELL`, not a failure.
        let mut buffer = vec![0 as libc::c_char; 4096];
        let code = libc::getpwuid_r(
            libc::getuid(),
            &mut record,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        );
        if code != 0 || found.is_null() || record.pw_shell.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(record.pw_shell)
            .to_str()
            .ok()
            .map(str::to_string)
            .filter(|shell| !shell.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_shell_is_an_absolute_path_to_something_that_exists() {
        // Deliberately not asserting WHICH shell: this test runs on the
        // developer's machine and in CI, and those disagree by design. What
        // must hold either way is that the answer is a path a pane can
        // actually be launched with — the failure this replaced handed back
        // `/bin/zsh` on hosts with no zsh at all. The user's shell, not the
        // one a test run names (`TEST_SHELL`), which is always `/bin/sh` here.
        let shell = users_shell();
        assert!(shell.starts_with('/'), "not an absolute path: {shell}");
        assert!(std::path::Path::new(&shell).exists(), "no such shell: {shell}");
    }

    #[test]
    fn the_password_database_outranks_a_stale_environment() {
        // The whole point. `$SHELL` is inherited from whatever started this
        // process, and for a daemon that is launchd or sshd rather than a
        // shell the user chose — so it cannot be allowed to win.
        //
        // Scoped to a run where the database can actually answer; a container
        // with no passwd entry for its uid legitimately falls through to the
        // environment, and asserting otherwise there would be asserting the
        // fallback is broken.
        let Some(from_database) = from_passwd() else { return };
        unsafe { std::env::set_var("SHELL", "/definitely/not/a/real/shell") };
        assert_eq!(users_shell(), from_database);
        unsafe { std::env::remove_var("SHELL") };
    }

    #[test]
    fn a_test_run_names_its_own_shell_and_nothing_else_does() {
        // This test binary runs under the workspace's `[env]`, so the
        // variable is set here exactly as it is for every pane a test opens.
        // Asserted rather than assumed: if cargo stopped passing it, every
        // tmux test would quietly go back to the owner's fish.
        // Any absolute path a developer exports wins over the config's.
        let named_here = std::env::var(TEST_SHELL).expect("set by .cargo/config.toml");
        assert!(named_here.starts_with('/'), "{named_here}");
        assert_eq!(login_shell(), named_here);
        assert_eq!(test_shell(), Some(named_here));
        // Unset, empty or relative, it names nothing, and the user's shell
        // stands. Asked of the filter rather than by unsetting a variable
        // other tests in this process read.
        for nothing in [None, Some(""), Some("sh")] {
            assert_eq!(named(nothing.map(str::to_string)), None, "{nothing:?}");
        }
        assert_eq!(named(Some("/bin/zsh".into())).as_deref(), Some("/bin/zsh"));
    }
}
