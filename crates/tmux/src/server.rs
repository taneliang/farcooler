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
//! A workspace is a daemon grouping of TAGGED WINDOWS, not a tmux session. There
//! is one runner-wide session.

use std::path::PathBuf;
use std::process::Stdio;

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

/// How long any one tmux command may take before it is abandoned. See `run`.
const TMUX_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

impl TmuxServer {
    pub fn new(install_id: &str, daemon_id: Uuid) -> Self {
        let config_path = std::env::temp_dir().join(format!("farcooler-{install_id}.tmux.conf"));
        Self::with_config(install_id, daemon_id, config_path)
    }

    pub fn with_config(install_id: &str, daemon_id: Uuid, config_path: PathBuf) -> Self {
        Self { socket: format!("farcooler-{install_id}"), daemon_id, config_path }
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
        let tmux = farcooler_core::programs::find("tmux").ok_or_else(|| {
            tracing::warn!("tmux is not installed anywhere this daemon can find");
            DomainError::TmuxUnavailable
        })?;

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
        // request rather than the session.
        let out = match tokio::time::timeout(TMUX_COMMAND_TIMEOUT, child.wait_with_output()).await {
            Ok(result) => result.map_err(|e| {
                tracing::warn!(error = %e, "tmux failed");
                DomainError::TmuxUnavailable
            })?,
            Err(_) => {
                tracing::warn!(command = ?args, "tmux did not answer in time");
                return Err(DomainError::TmuxUnavailable);
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
    /// workspace removal never uses `kill-session`.
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
}
