//! Private tmux server lifecycle.
//!
//! Far Cooler never mixes managed windows into the user's default tmux server and
//! never depends on the user's tmux configuration. It runs its own server on a
//! dedicated socket with minimal config:
//!
//! ```text
//! tmux -L farcooler-<install-id> -f <farcooler-managed.conf>
//! ```
//!
//! A worktree is a daemon grouping of TAGGED WINDOWS, not a tmux session. There
//! is one runner-wide session.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use farcooler_core::{DomainError, Result, SCHEMA_VERSION, tags};
use tokio::process::Command;
use uuid::Uuid;

/// Display name of the single runner-wide session. Addressed internally by its
/// stable tmux session id, never by this name.
pub const SESSION_NAME: &str = "farcooler";

#[derive(Debug, Clone)]
pub struct TmuxServer {
    socket: String,
    daemon_id: Uuid,
    config_path: PathBuf,
    /// Dead panes whose exit status never arrived within the settle window.
    ///
    /// Remembered so `list_tagged_panes` waits for each such pane once, not on
    /// every read for as long as it stays that way. See `EXIT_SETTLE`.
    pub(crate) unsettled_exits: Arc<Mutex<HashSet<String>>>,
    /// The tmux binary, named outright. `None` everywhere but a test, which is
    /// how one puts a deliberately slow tmux in front of a real server without
    /// touching the process-wide lookup every other test shares.
    program: Option<PathBuf>,
}

/// Far Cooler's own minimal tmux configuration.
///
/// This is NOT the user's config: the server starts with `-f` pointing here, so
/// nothing in `~/.tmux.conf` can change managed behavior, and these two options
/// are in force from the very first window rather than being applied afterwards.
///
/// `remain-on-exit` is the load-bearing one. Applied post-hoc it would race a
/// command that exits immediately, and that terminal would derive `lost` when it
/// actually exited cleanly.
///
/// `default-shell` is here rather than left to tmux because tmux's own default
/// is `$SHELL` — and the process starting this server is a daemon launched by
/// launchd or sshd, so its `$SHELL` is whatever that inherited rather than
/// anything the user chose. Left alone it produced a server whose
/// `default-shell` was `/bin/zsh` for a user whose login shell is fish, which
/// showed up twice: every pane command ran through a zsh wrapper it had no
/// reason to, and `$SHELL` inside a pane named a shell nobody was typing into.
fn managed_config() -> String {
    format!(
        "\
set -g remain-on-exit on
set -g window-size latest
set -g status off
set -g default-shell {}
",
        farcooler_core::shell::login_shell()
    )
}

/// Raw result of one tmux invocation.
#[derive(Debug)]
pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

/// How long an ordinary tmux command may take before it is abandoned. See
/// `run` and `deadline_for`.
const TMUX_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// How long a command that opens, tags or closes a pane may take. See
/// `deadline_for`.
const TMUX_LIFECYCLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The commands that get `TMUX_LIFECYCLE_TIMEOUT` rather than a second.
const LIFECYCLE_COMMANDS: &[&str] = &[
    // Whether the session exists, which decides `new-session` or `new-window`:
    // a `has-session` abandoned on a session that does exist reads as "not
    // running", and the `new-session` that follows fails as a duplicate.
    "has-session",
    "new-session",
    "new-window",
    "split-window",
    "respawn-pane",
    "break-pane",
    "join-pane",
    // Forks a `sh -c` inside the server, as `new-window` does, and runs on
    // attach, just after the open.
    "pipe-pane",
    // The tags. A pane whose tags never landed is a process the inventory
    // cannot name.
    "set-option",
    "kill-window",
    "kill-pane",
    // What a failed first open takes back. See `abandon` in windows.rs.
    "kill-session",
];

/// The deadline for one tmux command, by its verb.
///
/// A second is right for the commands that run all the time, and wrong for
/// the ones that bring a pane into being. The second exists for `send-keys`
/// into a pane nobody reads (see `run`), and it is short so that a wedged pane
/// costs one request rather than the session. Opening a terminal is a
/// different command: it happens once, it is what the person asked for, and
/// the first one starts the tmux server, which is the slowest thing tmux does.
///
/// A loaded machine takes more than a second over it. Under disk writeback
/// on a four-core Linux VM, a cold `new-session` took up to 4.2 s and a
/// `set-option` 1.1 s, against 5 to 15 ms at rest. Cut off at a second, the
/// open failed as "tmux is unavailable" with tmux working (ov-176, from a CI
/// run whose rerun was green), and worse than failed: in the reproduction it
/// was the session's tag that ran out, after `new-session` had made the
/// window, so the pane was left running untagged while its terminal record
/// was marked failed.
///
/// So the commands that open, tag or close a pane get ten seconds, which is
/// still a bound on a server that really is wedged, and everything else
/// keeps its second.
fn deadline_for(args: &[&str]) -> std::time::Duration {
    match args.first() {
        Some(verb) if LIFECYCLE_COMMANDS.contains(verb) => TMUX_LIFECYCLE_TIMEOUT,
        _ => TMUX_COMMAND_TIMEOUT,
    }
}

impl TmuxServer {
    pub fn new(install_id: &str, daemon_id: Uuid) -> Self {
        let config_path = std::env::temp_dir().join(format!("farcooler-{install_id}.tmux.conf"));
        Self::with_config(install_id, daemon_id, config_path)
    }

    pub fn with_config(install_id: &str, daemon_id: Uuid, config_path: PathBuf) -> Self {
        Self {
            socket: format!("farcooler-{install_id}"),
            daemon_id,
            config_path,
            unsettled_exits: Arc::default(),
            program: None,
        }
    }

    /// This server, run through `program` instead of the tmux `find_tmux`
    /// finds. For a test that needs a tmux slower than the real one.
    #[cfg(test)]
    pub(crate) fn with_program(mut self, program: PathBuf) -> Self {
        self.program = Some(program);
        self
    }

    /// Write the managed config if what is on disk is not what we want.
    ///
    /// Compared rather than merely checked for existence. The file is keyed on
    /// the install id and lives in the temp directory, so it long outlives any
    /// one daemon — and "it exists, leave it" meant an upgrade that changed
    /// these options never reached a runner that had already run the previous
    /// build. `default-shell` is the option that made that visible: the fix
    /// for it shipped and did nothing, because the stale file was still there.
    fn ensure_config(&self) -> Result<()> {
        let wanted = managed_config();
        if std::fs::read_to_string(&self.config_path).is_ok_and(|on_disk| on_disk == wanted) {
            return Ok(());
        }
        std::fs::write(&self.config_path, &wanted).map_err(|e| {
            tracing::warn!(error = %e, "could not write managed tmux config");
            DomainError::TmuxUnavailable
        })
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    pub fn daemon_id(&self) -> Uuid {
        self.daemon_id
    }

    /// The raw recovery command shown to users for transparency. It reaches the
    /// same live session and is documented as bypassing writer-lease enforcement.
    pub fn recovery_command(&self) -> String {
        format!(
            "tmux -L {} -f {} attach -t {}",
            self.socket,
            self.config_path.display(),
            SESSION_NAME
        )
    }

    /// Run a tmux command against the private server.
    pub async fn run(&self, args: &[&str]) -> Result<Output> {
        self.ensure_config()?;

        // Resolved rather than spawned by name.
        //
        // A Dock-launched Mac app inherits launchd's default `PATH` —
        // `/usr/bin:/bin:/usr/sbin:/sbin` — which has no Homebrew prefix in it,
        // so `Command::new("tmux")` failed with `ENOENT`. That is not a degraded
        // app: the inventory becomes unusable, `derive_terminal` reports every
        // terminal as `Lost`, and the whole product looks broken because of a
        // missing directory. See `farcooler_core::programs`.
        let tmux = match &self.program {
            Some(program) => program.clone(),
            None => find_tmux().await.ok_or_else(|| {
                tracing::warn!("tmux is not installed anywhere this daemon can find");
                DomainError::TmuxUnavailable
            })?,
        };

        let mut cmd = Command::new(&tmux);
        // Give tmux a UTF-8 locale when the daemon inherited none.
        //
        // The other half of the same launchd problem `programs::find` solves
        // above: a Dock-launched app inherits no `LANG` either, and a tmux
        // running in the C locale SANITIZES control characters out of format
        // output — every `-F` and `display-message` string here delimits fields
        // with a TAB, and tmux turns each one into `_`.
        //
        // Every parser then splits on `\t`, gets one field, and returns `None`.
        // The inventory comes back empty, `derive_terminal` reports every
        // terminal `Lost`, pane modes and cursor position stop parsing too, and
        // nothing anywhere says why. Verified against tmux 3.7b: the same
        // binary on the same socket emits `\t` with `LANG=en_US.UTF-8` and `_`
        // without it.
        if let Some((key, value)) = utf8_locale() {
            cmd.env(key, value);
        }
        cmd.arg("-L").arg(&self.socket).arg("-f").arg(&self.config_path);
        cmd.args(args);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Killed if it outlives its welcome, which is what makes the timeout
        // below a real bound rather than a way of losing track of a process.
        cmd.kill_on_drop(true);

        let child = cmd.spawn().map_err(|e| {
            tracing::warn!(error = %e, "failed to spawn tmux");
            DomainError::TmuxUnavailable
        })?;

        // Every tmux command gets a deadline, because one of them can block
        // forever and take everything with it.
        //
        // `send-keys` writes to a pane's pty. A program that never reads its
        // input — `sleep` is the honest example, and any pane sitting at a
        // prompt nobody is typing at is the common one — eventually lets that
        // buffer fill, and then the write blocks. tmux blocks with it, this
        // call blocks with tmux, and because a client connection answers one
        // request at a time, every other terminal's requests queue behind a
        // pane nobody is even looking at. Scrolling one terminal could stop
        // scrolling in all of them.
        //
        // Local commands answer in milliseconds, so a second is already far
        // outside normal and still short enough that a wedged pane costs one
        // request rather than the session. Opening a pane is the exception, and
        // gets longer: see `deadline_for`.
        let deadline = deadline_for(args);
        let out = match tokio::time::timeout(deadline, child.wait_with_output()).await {
            Ok(result) => result.map_err(|e| {
                tracing::warn!(error = %e, "tmux failed");
                DomainError::TmuxUnavailable
            })?,
            Err(_) => {
                tracing::warn!(command = ?args, ?deadline, "tmux did not answer in time");
                // Not `TmuxUnavailable`: tmux is there, and the advice that
                // goes with that word, install tmux, would be wrong.
                return Err(DomainError::TmuxTimedOut);
            }
        };

        Ok(Output {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// True when the private server is currently running.
    pub async fn is_running(&self) -> bool {
        self.run(&["has-session", "-t", SESSION_NAME]).await.map(|o| o.ok()).unwrap_or(false)
    }

    /// Tag the session and set its size policy once it exists.
    ///
    /// The session contains managed terminal windows only and keeps NO fake
    /// sentinel shell. Creating a placeholder window would squat the session's
    /// base index and make the first real terminal fail with "index 0 in use",
    /// so the first terminal creates the session instead. See
    /// `create_terminal_window`.
    pub(crate) async fn tag_session(&self) -> Result<()> {
        self.set_session_option(tags::DAEMON_ID, &self.daemon_id.to_string()).await?;
        self.set_session_option(tags::SCHEMA_VERSION, &SCHEMA_VERSION.to_string()).await?;

        // `remain-on-exit`, `window-size latest` and `default-shell` come from
        // `managed_config()` so they are in force from the first window, not
        // applied afterwards.
        Ok(())
    }

    async fn set_session_option(&self, key: &str, value: &str) -> Result<()> {
        let out = self.run(&["set-option", "-t", SESSION_NAME, key, value]).await?;
        if !out.ok() {
            tracing::warn!(key, stderr = %out.stderr, "failed to set session tag");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Kill the private server entirely. Test and uninstall use only; ordinary
    /// worktree removal never uses `kill-session`.
    pub async fn kill_server(&self) -> Result<()> {
        let _ = self.run(&["kill-server"]).await;
        Ok(())
    }
}

/// The locale variable to set for tmux, or `None` when the inherited one is
/// already fine.
///
/// Only `LC_CTYPE`, and deliberately: it is the category that decides character
/// classification, which is the only thing tmux's sanitizing depends on. Setting
/// `LC_ALL` would also override collation and number formatting the user may
/// have chosen on purpose, to fix a problem that has nothing to do with either.
fn utf8_locale() -> Option<(&'static str, &'static str)> {
    let read = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    ctype_override(
        read("LC_ALL").as_deref(),
        read("LC_CTYPE").as_deref(),
        read("LANG").as_deref(),
    )
    .map(|value| ("LC_CTYPE", value))
}

/// Where tmux is, found without blocking the async runtime.
///
/// `programs::find` can wait on the user's login shell — up to its timeout,
/// and every concurrent caller waits on the same shell. Called straight from
/// async code, each of those waits parks a runtime worker, so a burst of tmux
/// commands behind a slow profile could stall every worker at once. A cached
/// answer, which is every call after the first, is taken on the spot; anything
/// else is resolved on a blocking thread.
pub async fn find_tmux() -> Option<PathBuf> {
    off_runtime(farcooler_core::programs::known("tmux"), || {
        farcooler_core::programs::find("tmux")
    })
    .await
}

/// `known` if there is one, otherwise `find` on a blocking thread. Split out
/// so a test can hand it a slow `find`.
async fn off_runtime(
    known: Option<PathBuf>,
    find: impl FnOnce() -> Option<PathBuf> + Send + 'static,
) -> Option<PathBuf> {
    if known.is_some() {
        return known;
    }
    tokio::task::spawn_blocking(find).await.ok().flatten()
}

/// Which locale to impose, given what was inherited.
///
/// Pure so it can be tested: the real thing reads process-global environment,
/// and these tests run in parallel with everything else in the crate.
///
/// The precedence is libc's own — `LC_ALL` beats `LC_CTYPE` beats `LANG` — so if
/// the winning one already names UTF-8 there is nothing to do, and a user who
/// deliberately runs a non-UTF-8 locale is left alone rather than overridden.
fn ctype_override(
    lc_all: Option<&str>,
    lc_ctype: Option<&str>,
    lang: Option<&str>,
) -> Option<&'static str> {
    let effective = lc_all.or(lc_ctype).or(lang);
    match effective {
        // Something is set, and it is the user's business what.
        Some(_) => None,
        // Nothing at all, which is what launchd hands a Dock-launched app.
        None => Some(DEFAULT_UTF8_LOCALE),
    }
}

/// A UTF-8 locale that exists on the platform the daemon runs on.
///
/// macOS ships `en_US.UTF-8` always. Linux gets `C.UTF-8`, which glibc has had
/// since 2.35 and musl always has, and which does not impose an American
/// English anything on a machine that never asked for one.
///
/// Naming one that does not exist costs nothing: libc falls back to `C`, which
/// is exactly where this started, so the change either helps or is inert.
#[cfg(target_os = "macos")]
const DEFAULT_UTF8_LOCALE: &str = "en_US.UTF-8";
#[cfg(not(target_os = "macos"))]
const DEFAULT_UTF8_LOCALE: &str = "C.UTF-8";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_inherited_means_impose_a_utf8_locale() {
        // What launchd hands a Dock-launched app. Without this, tmux runs in the
        // C locale and sanitizes the tab delimiter out of every format string.
        assert_eq!(ctype_override(None, None, None), Some(DEFAULT_UTF8_LOCALE));
    }

    #[test]
    fn an_inherited_locale_is_left_alone_whichever_variable_carries_it() {
        // Including a non-UTF-8 one. A user who deliberately runs a C locale in
        // their shell is not someone to override — and if they do, tmux behaves
        // for Far Cooler exactly as it does for them in a terminal, which is the
        // property worth preserving.
        assert_eq!(ctype_override(Some("en_US.UTF-8"), None, None), None);
        assert_eq!(ctype_override(None, Some("en_GB.UTF-8"), None), None);
        assert_eq!(ctype_override(None, None, Some("ja_JP.UTF-8")), None);
        assert_eq!(ctype_override(Some("C"), None, None), None, "their choice");
    }

    #[test]
    fn precedence_follows_libcs_own() {
        // LC_ALL beats LC_CTYPE beats LANG, so a set LC_ALL means there is
        // nothing to decide however empty the others are.
        assert_eq!(ctype_override(Some("C"), Some("en_US.UTF-8"), Some("en_US.UTF-8")), None);
    }

    #[test]
    fn the_imposed_locale_actually_says_utf8() {
        // The whole point of the value. A default that was not UTF-8 would set a
        // variable and change nothing.
        assert!(
            DEFAULT_UTF8_LOCALE.to_ascii_uppercase().contains("UTF-8"),
            "{DEFAULT_UTF8_LOCALE}"
        );
    }

    #[test]
    fn only_lc_ctype_is_imposed() {
        // Never LC_ALL: that would also override collation and number formatting
        // somebody may have chosen on purpose, to fix a character-classification
        // problem that has nothing to do with either.
        let (key, _) = utf8_locale().unwrap_or(("LC_CTYPE", DEFAULT_UTF8_LOCALE));
        assert_eq!(key, "LC_CTYPE");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_slow_lookup_does_not_stall_the_runtime() {
        // One worker, so a lookup that blocks it stops everything else: here,
        // a ticker that should keep counting while a login shell takes its
        // time. On a blocking thread the ticker runs throughout.
        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let ticker = {
            let ticks = ticks.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
        };
        tokio::task::yield_now().await;

        let found = off_runtime(None, || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            Some(PathBuf::from("/slow/tmux"))
        })
        .await;
        ticker.abort();

        assert_eq!(found, Some(PathBuf::from("/slow/tmux")));
        let ticks = ticks.load(std::sync::atomic::Ordering::Relaxed);
        assert!(ticks >= 20, "the runtime ticked {ticks} times in 500 ms; the lookup blocked it");
    }

    /// A private server whose every command goes through a wrapper that
    /// sleeps 1.5 s before the verbs in `slow` (a `case` pattern such as
    /// `new-session|send-keys`) and then runs the real tmux: a machine too
    /// loaded to answer in a second, made to order. Its server, its wrapper
    /// and its config go with it.
    struct SlowTmux {
        server: TmuxServer,
        real: PathBuf,
        wrapper: PathBuf,
        /// One of these at a time. Their timings are the point, and a dozen
        /// servers starting at once in one process made a `list-panes` with
        /// no server behind it miss its second.
        _turn: tokio::sync::MutexGuard<'static, ()>,
    }

    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    impl SlowTmux {
        /// `None` off CI when there is no tmux to wrap; on CI, which installs
        /// tmux for this job, a missing one is a failure, never a skip.
        async fn start(test: &str, slow: &str) -> Option<SlowTmux> {
            SlowTmux::wrapping(test, &format!("{slow}) sleep 1.5 ;;")).await
        }

        /// A wrapper whose `case` on the verb has `arms` in it, before the real
        /// tmux runs. `@FAIL@` in `arms` is a file `fail_from_now` creates.
        async fn wrapping(test: &str, arms: &str) -> Option<SlowTmux> {
            let turn = ONE_AT_A_TIME.lock().await;
            use std::os::unix::fs::PermissionsExt;
            let Some(real) = farcooler_core::programs::find("tmux") else {
                assert!(std::env::var_os("CI").is_none(), "tmux is not installed, and CI must run {test}");
                eprintln!("skipping {test}: no tmux");
                return None;
            };
            let install = format!("test-slow-{}", Uuid::now_v7().simple());
            let wrapper = std::env::temp_dir().join(format!("farcooler-{install}-tmux"));
            // `-L <socket> -f <config> <verb> …`, so the verb is the fifth.
            let script = format!(
                "#!/bin/sh\ncase \"$5\" in {} esac\nexec '{}' \"$@\"\n",
                arms.replace("@FAIL@", &format!("{}.fail", wrapper.display()))
                    .replace("@REAL@", &real.display().to_string()),
                real.display()
            );
            std::fs::write(&wrapper, script).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            let server = TmuxServer::new(&install, Uuid::now_v7()).with_program(wrapper.clone());
            Some(SlowTmux { server, real, wrapper, _turn: turn })
        }
    }

    impl SlowTmux {
        fn fail_marker(&self) -> PathBuf {
            PathBuf::from(format!("{}.fail", self.wrapper.display()))
        }

        /// Each pane with the command tmux started it with.
        fn every_start(&self) -> Vec<(String, String)> {
            let out = std::process::Command::new(&self.real)
                .args(["-L", self.server.socket(), "list-panes", "-a", "-F", "#{pane_id}\t#{pane_start_command}"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.split_once('\t').map(|(p, c)| (p.to_string(), c.to_string())))
                .collect()
        }

        /// A pane added the way a person or an agent adds one: `tmux` typed
        /// in a Far Cooler terminal, which reaches our session through `TMUX`.
        fn by_hand(&self, args: &[&str]) -> String {
            let out = std::process::Command::new(&self.real)
                .args(["-L", self.server.socket()])
                .args(args)
                .args(["-d", "-P", "-F", "#{pane_id}"])
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        /// Arms an `@FAIL@` test in the wrapper from here on.
        fn fail_from_now(&self) {
            std::fs::write(self.fail_marker(), "").unwrap();
        }

        /// Every pane on the server, read with the real tmux, so what the
        /// wrapper does cannot hide one.
        fn every_pane(&self) -> Vec<String> {
            let out = std::process::Command::new(&self.real)
                .args(["-L", self.server.socket(), "list-panes", "-a", "-F", "#{pane_id}"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
        }

        async fn open(&self) -> crate::windows::ManagedWindow {
            let dir = std::env::temp_dir();
            self.server
                .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "t", &dir.to_string_lossy(), "sleep 30")
                .await
                .expect("open a pane")
        }
    }

    /// A tag that fails once the marker exists.
    const TAGS_FAIL: &str = "set-option) [ -e '@FAIL@' ] && exit 1 ;;";

    #[tokio::test(flavor = "current_thread")]
    async fn a_first_open_whose_tags_fail_leaves_no_pane_behind() {
        // ov-176: the window was made, the tag failed, and the pane ran on
        // with no id while its record said the open had failed.
        let Some(tmux) = SlowTmux::wrapping("a_first_open_whose_tags_fail_leaves_no_pane_behind", TAGS_FAIL).await else {
            return;
        };
        tmux.fail_from_now();
        let dir = std::env::temp_dir();
        let opened = tmux
            .server
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "t", &dir.to_string_lossy(), "sleep 30")
            .await;
        assert!(opened.is_err(), "{opened:?}");
        assert_eq!(tmux.every_pane(), Vec::<String>::new(), "the failed open left a pane running");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_first_open_that_fails_takes_only_its_own_window_not_the_session() {
        // While the first open is still tagging, something else adds a window
        // to the session it just made: another open, or a person. Its failure
        // must take its own window and nothing else.
        let arms = "set-option) [ -e '@FAIL@' ] && { '@REAL@' -L \"$2\" new-window -d -t farcooler: 'sleep 30'; exit 1; } ;;";
        let Some(tmux) =
            SlowTmux::wrapping("a_first_open_that_fails_takes_only_its_own_window_not_the_session", arms).await
        else {
            return;
        };
        tmux.fail_from_now();
        let terminal = Uuid::now_v7();
        let dir = std::env::temp_dir();
        let opened = tmux
            .server
            .create_terminal_window(Uuid::now_v7(), terminal, "t", &dir.to_string_lossy(), "sleep 30")
            .await;
        assert!(opened.is_err(), "{opened:?}");
        let left = tmux.every_start();
        assert!(!left.is_empty(), "the windows added meanwhile went with the failed open's");
        assert!(
            left.iter().all(|(_, started)| !started.contains(&terminal.to_string())),
            "the failed open's own pane is still there: {left:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_later_open_whose_tags_fail_takes_back_only_its_own_window() {
        let Some(tmux) = SlowTmux::wrapping("a_later_open_whose_tags_fail_takes_back_only_its_own_window", TAGS_FAIL).await
        else {
            return;
        };
        let first = tmux.open().await;
        tmux.fail_from_now();
        let dir = std::env::temp_dir();
        let opened = tmux
            .server
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "t", &dir.to_string_lossy(), "sleep 30")
            .await;
        assert!(opened.is_err(), "{opened:?}");
        assert_eq!(tmux.every_pane(), vec![first.pane_id], "the earlier pane stays, the failed one goes");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_split_whose_tag_fails_takes_back_its_pane() {
        let Some(tmux) = SlowTmux::wrapping("a_split_whose_tag_fails_takes_back_its_pane", TAGS_FAIL).await else {
            return;
        };
        let first = tmux.open().await;
        tmux.fail_from_now();
        let dir = std::env::temp_dir();
        let split = tmux
            .server
            .split_pane(&first.pane_id, crate::windows::Axis::Horizontal, Uuid::now_v7(), &dir.to_string_lossy(), "sleep 30", false)
            .await;
        assert!(split.is_err(), "{split:?}");
        assert_eq!(tmux.every_pane(), vec![first.pane_id]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_a_marked_untagged_pane_is_an_unfinished_open() {
        let Some(tmux) = SlowTmux::wrapping("only_a_marked_untagged_pane_is_an_unfinished_open", "").await else {
            return;
        };
        let tagged = tmux.open().await;
        // What an open cut off before its tags leaves: a window started with
        // the opening mark, and no id on it.
        let unfinished = crate::windows::marked("sleep 30", Uuid::now_v7());
        let out = tmux
            .server
            .run(&["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", "farcooler:", &unfinished])
            .await
            .unwrap();
        let unfinished = out.stdout.trim().to_string();
        // And what a person or an agent adds through `TMUX`: untagged too,
        // and not ours to touch.
        let window = tmux.by_hand(&["new-window", "-t", "farcooler:"]);
        let split = tmux.by_hand(&["split-window", "-t", &tagged.pane_id]);
        let listed: Vec<String> =
            tmux.server.unfinished_opens().await.expect("list").into_iter().map(|p| p.pane_id).collect();
        assert_eq!(
            listed,
            vec![unfinished],
            "only the marked pane: not the tagged {}, the hand window {window} or the hand split {split}",
            tagged.pane_id
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_server_means_no_unfinished_opens() {
        let Some(tmux) = SlowTmux::wrapping("no_server_means_no_unfinished_opens", "").await else { return };
        assert_eq!(tmux.server.unfinished_opens().await.expect("no server is not an error"), Vec::new());
    }

    impl Drop for SlowTmux {
        fn drop(&mut self) {
            let _ = std::process::Command::new(&self.real)
                .args(["-L", self.server.socket(), "kill-server"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = std::fs::remove_file(&self.wrapper);
            let _ = std::fs::remove_file(self.fail_marker());
            let _ = std::fs::remove_file(&self.server.config_path);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_server_slow_to_start_still_opens_the_first_pane() {
        // ov-176: the first pane starts the server, and a loaded machine takes
        // more than a second over that. This open used to be cut off at one
        // second and reported as "tmux is unavailable".
        let Some(slow) = SlowTmux::start("a_server_slow_to_start_still_opens_the_first_pane", "new-session").await else { return };
        let terminal = Uuid::now_v7();
        let dir = std::env::temp_dir();
        let opened = slow
            .server
            .create_terminal_window(Uuid::now_v7(), terminal, "slow", &dir.to_string_lossy(), "sleep 30")
            .await;
        let window = opened.expect("a server that takes 1.5 s to start is slow, not unavailable");

        let panes = slow.server.list_tagged_panes().await.expect("list the panes");
        let pane = panes.iter().find(|p| p.pane_id == window.pane_id).expect("the pane is there");
        assert_eq!(pane.terminal_id, terminal, "and tagged, so the inventory can name it");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_keystroke_tmux_will_not_take_still_gives_up_in_a_second() {
        // The second the opening commands no longer get is still the bound on
        // everything else: a `send-keys` into a pane that never reads must not
        // hold the connection any longer than it did.
        let Some(slow) = SlowTmux::start("a_keystroke_tmux_will_not_take_still_gives_up_in_a_second", "send-keys").await else {
            return;
        };
        let dir = std::env::temp_dir();
        let window = slow
            .server
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "slow", &dir.to_string_lossy(), "sleep 30")
            .await
            .expect("open a pane");

        let started = std::time::Instant::now();
        let sent = slow.server.send_keys(&window.pane_id, "x").await;
        let took = started.elapsed();
        assert!(matches!(sent, Err(DomainError::TmuxTimedOut)), "a timeout says so: {sent:?}");
        assert!(took < std::time::Duration::from_millis(1400), "send-keys waited {took:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn attaching_to_a_pane_outlasts_a_slow_fork() {
        // `pipe-pane` forks a shell inside the server, the way opening a pane
        // does, and it is what attaching runs straight after the open.
        let Some(slow) = SlowTmux::start("attaching_to_a_pane_outlasts_a_slow_fork", "pipe-pane").await else {
            return;
        };
        let dir = std::env::temp_dir();
        let window = slow
            .server
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "slow", &dir.to_string_lossy(), "sleep 30")
            .await
            .expect("open a pane");
        let piped = slow.server.pipe_pane_start(&window.pane_id, "cat > /dev/null").await;
        assert!(piped.is_ok(), "a pipe-pane that takes 1.5 s is slow, not unavailable: {piped:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_cached_answer_is_taken_without_a_lookup() {
        let found = off_runtime(Some(PathBuf::from("/cached/tmux")), || {
            panic!("a cached answer must not be looked up again")
        })
        .await;
        assert_eq!(found, Some(PathBuf::from("/cached/tmux")));
    }
}
