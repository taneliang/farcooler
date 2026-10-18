//! Each member's agents run on their own Claude login (ov-583, phase 1 of
//! `reports/ov-581/design.md`).
//!
//! **Whose login a pane runs on** is decided once, at its first launch:
//!
//! - a member's rep: that member's;
//! - a workspace's orchestrator: its lead's (the runner's own while the owner
//!   leads);
//! - a pane opened from inside another pane, through the local socket (an
//!   orchestrator dispatching a lane, an agent opening a helper): the
//!   orchestrator's lead's, or the opening pane's own login, **whatever
//!   device the connection names** (review 1, H1: a session preamble's
//!   device id isn't a secret, and any process on the runner can send one);
//! - a pane opened by a caller in no pane, on a member's device: theirs;
//! - anything else the owner opens: the runner's own, as before.
//!
//! A member's login is recorded on the pane (`Store::set_terminal_login`) and
//! every relaunch reads it back. The runner's own is recorded as nothing.
//!
//! **An orchestrator stays on its lead's login.** When the lead is handed
//! on, the orchestrator is started afresh on the new lead's (`lead_change`,
//! ov-585), and a relaunch that finds an old lead's login does the same. What
//! can't be started afresh is refused `lead_changed`: a pane on another login
//! made the orchestrator (`set_role`), and a lane opened by an orchestrator
//! still on an old lead's login.
//!
//! **Where a login lives:** `<runtime dir>/claude/<login id>/`, 0700, passed
//! to claude as `CLAUDE_CONFIG_DIR`. The login id is the member's generation
//! (`Store::ensure_member_login`), never their handle: a handle removed and
//! given to someone new gets a new directory and a new Keychain login, and
//! the old person's panes are refused (`login_gone`). claude keys its Keychain
//! entry on the directory string exactly as passed (ov-582), so the launch
//! passes one canonical absolute path and a relaunch passes back the very
//! string it recorded.
//!
//! **The runner's own login never gets the variable** (ov-582):
//! `CLAUDE_CONFIG_DIR=~/.claude` is a different Keychain entry from leaving it
//! unset, so setting it for the owner's panes would sign the owner out.
//!
//! **Never the owner's login for a member.** A member's launch that can't be
//! made as theirs (a removed member, a directory that's a link or isn't where
//! it should be, a claude too old to key its login by directory) is refused,
//! never started on the runner's login.
//!
//! **What decides it:** the caller's process ancestry from the kernel's
//! process table (`people::pane_of`), then the connection's device binding
//! (`people::person_of`), the store's rep bindings and workspace leads, and
//! the pane's own recorded login. Never a request field, and never a pane's
//! environment (`FARCOOLER_ACTOR`, `FARCOOLER_REP`, `CLAUDE_CONFIG_DIR`),
//! which anything in the pane can change.
//!
//! **A guardrail, not a boundary**, as `people` says of the rest: every pane
//! runs as the same Unix user, so the owner, any agent and anyone with
//! Control can read a member's directory and run claude on it. The ancestry
//! walk fails open the way `rep_pane_of` does (review 1, M3, accepted): a
//! process that leaves its pane's tree (`( … & )`, `setsid`, `tmux
//! new-window`) reads as in no pane, and then as whoever its connection says:
//! the owner for a plain local socket, or, with a session preamble naming a
//! member's device (device ids aren't secret), that member, so a detached
//! process can get another member's login as well as the owner's. A same-user process can always do the
//! equivalent by hand (`CLAUDE_CONFIG_DIR=~/.claude claude`), so this keeps
//! the shape the owner chose and stops mistakes, not someone determined.
//!
//! Hooks, the mailbox and the plugin come with every launch (`--settings`,
//! `--plugin-dir`, the runtime directory), not from the config directory, so
//! a member's directory needs nothing installed in it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::{self, TerminalRole};
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::people::Person;
use crate::reps::Seat;
use crate::service::Service;

/// The directory under the runtime directory that holds every member's.
pub const MEMBERS_DIR: &str = "claude";

/// The oldest claude that keys its Keychain login by config directory:
/// 1.0.8, measured by ov-582 from the 1.0.7 to 1.0.8 npm diff, where the
/// service becomes `Claude Code-credentials-<first 8 hex of the sha256 of
/// CLAUDE_CONFIG_DIR>`. An older one writes every directory's login to the
/// one shared entry, which is the owner's, so a member's sign-in would
/// overwrite it.
pub const MIN_MEMBER_CLAUDE: Version = Version(1, 0, 8);

/// Whose Claude login a launch runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// The runner's own: no `CLAUDE_CONFIG_DIR`, as every pane had before.
    Runner,
    /// A member's, by handle, with what the pane recorded of it; `None`
    /// before its first launch (or an old rep's first relaunch) records it.
    Member { handle: String, recorded: Option<TerminalLogin> },
}

impl Login {
    pub(crate) fn member(handle: String) -> Login {
        Login::Member { handle, recorded: None }
    }

}

/// What a launch on a login carries: the directory it passes, and the row
/// to record when the pane has none yet.
#[derive(Debug, Default)]
pub(crate) struct LaunchLogin {
    pub dir: Option<String>,
    record: Option<TerminalLogin>,
}

/// A claude version, `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version in `claude --version`'s output: a line that is exactly
/// `N.N.N (Claude Code)`. Anything else a login shell printed (an rc file's
/// banner, a version manager's notice, a pre-release) is not it, and an
/// output with no such line has no version (review 1, M1).
pub fn version_line(stdout: &str) -> Option<Version> {
    stdout.lines().find_map(|line| {
        let number = line.strip_suffix(" (Claude Code)")?;
        let mut parts = number.split('.').map(|p| {
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse::<u32>().ok()).flatten()
        });
        let (a, b, c) = (parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(Version(a, b, c))
    })
}

/// Where every member's directory sits under `runtime`.
pub fn member_root(runtime: &Path) -> PathBuf {
    runtime.join(MEMBERS_DIR)
}

/// Whether `name` is a login id as the store makes them: a uuid's 32 hex
/// digits, so it never names a path but its own.
pub(crate) fn is_login_id(name: &str) -> bool {
    name.len() == 32 && Uuid::try_parse(name).is_ok()
}

/// The directory of login `login_id` under `runtime`.
pub fn login_dir(runtime: &Path, login_id: &str) -> Option<PathBuf> {
    is_login_id(login_id).then(|| member_root(runtime).join(login_id))
}

/// Every member directory there is now: a real directory, never a link,
/// named by a login id.
pub fn member_dirs(runtime: &Path) -> Vec<PathBuf> {
    let root = member_root(runtime);
    if !is_real_dir(&root) {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_login_id))
        .map(|e| e.path())
        .filter(|p| is_real_dir(p))
        .collect();
    dirs.sort();
    dirs
}

/// The runner's own config directory, which lookups read and no launch
/// names: `$CLAUDE_CONFIG_DIR` when it's set and not empty, else
/// `<home>/.claude`.
pub fn runner_dir(config_env: Option<std::ffi::OsString>, home: Option<&Path>) -> Option<PathBuf> {
    config_env.filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".claude")))
}

/// Every config directory a session may be under: the runner's, then each
/// member's.
pub fn every_dir(runner: Option<&Path>, runtime: &Path) -> Vec<PathBuf> {
    runner.map(Path::to_path_buf).into_iter().chain(member_dirs(runtime)).collect()
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// Make `path` a directory only this user can enter, or check that it is
/// one, setting it back to 0700 if it was opened up. A link, or anything
/// else that isn't a directory, is refused.
pub(crate) fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => {
            if m.permissions().mode() & 0o777 != 0o700 {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Ok(_) => Err(std::io::Error::other("not a directory")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::DirBuilder::new().mode(0o700).create(path),
        Err(e) => Err(e),
    }
}

/// Whether `dir` is login `login_id`'s directory and nothing else: a real
/// directory, named by the id, directly under the runtime's member root as
/// that resolves now (review 1, L1).
fn is_login_dir_of(runtime: &Path, dir: &Path, login_id: &str) -> bool {
    let Ok(root) = std::fs::canonicalize(member_root(runtime)) else { return false };
    dir.is_absolute()
        && dir.parent() == Some(root.as_path())
        && dir.file_name().and_then(|n| n.to_str()) == Some(login_id)
        && is_real_dir(dir)
}

/// Login `login_id`'s directory, made 0700 if it's missing, as the one
/// canonical string claude is handed: absolute, every link above it
/// resolved, no trailing slash. Refused for anything but a login id, and for
/// a root or member directory that's a link or resolves anywhere else.
pub fn ensure_login_dir(runtime: &Path, login_id: &str) -> Result<String> {
    let dir = login_dir(runtime, login_id).ok_or(DomainError::InvalidArgument { what: "login" })?;
    let made = private_dir(&member_root(runtime)).and_then(|()| private_dir(&dir)).and_then(|()| std::fs::canonicalize(&dir));
    let path = match made {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!(kind = ?e.kind(), "couldn't make a member's Claude directory; the launch is refused");
            return Err(DomainError::OperationFailed);
        }
    };
    if !is_login_dir_of(runtime, &path, login_id) {
        tracing::warn!("a member's Claude directory resolved somewhere else; the launch is refused");
        return Err(DomainError::OperationFailed);
    }
    path.into_os_string().into_string().map_err(|_| DomainError::OperationFailed)
}

/// Whether `recorded` can still be handed to claude: its directory is its
/// login's, under this runtime's root, and is set back to 0700. A row only
/// the runner writes, checked anyway, so a directory swapped for a link to
/// someone else's is refused rather than followed.
fn recorded_dir_holds(runtime: &Path, recorded: &TerminalLogin) -> bool {
    let dir = Path::new(&recorded.config_dir);
    is_login_dir_of(runtime, dir, &recorded.login_id) && private_dir(dir).is_ok()
}

/// Who opened the request now being served: set around every request's
/// dispatch (`launching`), read only when a pane is launched.
#[derive(Clone)]
struct Caller {
    peer: Peer,
    pid: Option<i32>,
}

tokio::task_local! {
    static CALLER: Caller;
}

/// Run `request` as `peer`'s, its local process `pid`: a pane it opens
/// takes its login from them. A pane opened outside any request (the
/// runner's own work, a test) is the runner's.
pub async fn launching<F: Future>(peer: Peer, pid: Option<i32>, request: F) -> F::Output {
    CALLER.scope(Caller { peer, pid }, request).await
}

/// What the version check knows.
pub struct State {
    /// The runner's own config directory (`runner_dir`): `open` reads the
    /// environment; a test's service, one under its root.
    pub(crate) runner: Option<PathBuf>,
    /// The version a harness's claude claims, in place of asking a real one
    /// (`Service::claiming_claude_version`): `Some(None)` claims it can't be
    /// read. `None`, outside the daemon's own tests, asks. The asking itself
    /// is `ask_claude_version`, tested on its own.
    pub(crate) claimed: Mutex<Option<Option<Version>>>,
    /// Where a removed member's Keychain entry is deleted (ov-588).
    pub(crate) keychain: crate::member_removal::Keychain,
    /// Members signing in to Claude now (ov-584).
    pub(crate) sign_ins: crate::claude_sign_in::SignIns,
    /// Run once, inside the next version check: a test's removal of the
    /// member mid-launch (ov-588 review 2).
    #[cfg(test)]
    pub(crate) at_version_check: Hook,
    /// Run once, at the next check just before a spawn (`still_on_its_login`).
    #[cfg(test)]
    pub(crate) before_spawn: Hook,
    /// Each directory a test's version check was asked in.
    #[cfg(test)]
    pub(crate) asked_in: Mutex<Vec<PathBuf>>,
}

impl State {
    pub fn new(runner: Option<PathBuf>) -> State {
        State {
            runner,
            // The daemon's own tests never ask a real claude.
            claimed: Mutex::new(cfg!(test).then_some(Some(MIN_MEMBER_CLAUDE))),
            keychain: crate::member_removal::Keychain::default(),
            sign_ins: crate::claude_sign_in::SignIns::default(),
            #[cfg(test)]
            at_version_check: Mutex::new(None),
            #[cfg(test)]
            before_spawn: Mutex::new(None),
            #[cfg(test)]
            asked_in: Mutex::new(Vec::new()),
        }
    }
}

/// A test's hook, run once where it's set.
#[cfg(test)]
pub(crate) type Hook = Mutex<Option<Box<dyn FnOnce() + Send>>>;

#[cfg(test)]
fn run_hook(hook: &Hook) {
    let hook = hook.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(hook) = hook {
        hook();
    }
}

/// How long `claude --version` may take, login shell and all.
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);

/// `claude --version` as a pane in `cwd` would run it, through `shell` as a
/// login shell, so it finds the claude that pane finds (a version manager
/// picks per directory). No `CLAUDE_CONFIG_DIR`, so it touches no member's
/// directory. **Fails closed:** a non-zero exit, a timeout, or no line that
/// is exactly a version (`version_line`) is no version.
pub(crate) async fn ask_claude_version(shell: &str, cwd: &Path) -> Option<Version> {
    let mut command = tokio::process::Command::new(shell);
    command
        .args(["-ilc", "claude --version"])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output()).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    version_line(&String::from_utf8_lossy(&output.stdout))
}

impl Service {
    /// The claude a pane in `cwd` runs, asked afresh on every member launch
    /// (review 2: a kept answer outlives a version manager's switch). A stub
    /// or a stand-in (a test's daemon) has no Keychain to protect, so it
    /// passes.
    pub(crate) async fn claude_version(&self, cwd: &Path) -> Option<Version> {
        #[cfg(test)]
        self.claude_logins.asked_in.lock().unwrap_or_else(|e| e.into_inner()).push(cwd.to_path_buf());
        #[cfg(test)]
        run_hook(&self.claude_logins.at_version_check);
        if let Some(claimed) = *self.claude_logins.claimed.lock().unwrap_or_else(|e| e.into_inner()) {
            return claimed;
        }
        if cfg!(test) || crate::service::test_stub_agents() || crate::service::stand_in_agent().is_some() {
            return Some(MIN_MEMBER_CLAUDE);
        }
        ask_claude_version(&farcooler_core::shell::login_shell(), cwd).await
    }

    /// A service whose claude claims `version` (`None`: one that can't be
    /// read), for a harness in another crate that runs this library in its
    /// own process, where `cfg(test)` is off and no claude it launches is
    /// real (ov-583). The version check runs on what it claims, so the
    /// harness still meets the floor and every refusal: never a probe of the
    /// claude on whoever's machine runs the suite.
    pub fn claiming_claude_version(self, version: Option<Version>) -> Self {
        *self.claude_logins.claimed.lock().unwrap_or_else(|e| e.into_inner()) = Some(version);
        self
    }

    /// Refuse a member's launch on a claude older than `MIN_MEMBER_CLAUDE`
    /// (`claude_too_old`). A claude pane on a claude whose version can't be
    /// read is refused too. Another pane (a shell) on a runner with no claude
    /// to read passes: nothing in it can sign in until someone installs one.
    async fn check_claude_version(&self, preset: &str, cwd: &Path) -> Result<()> {
        match self.claude_version(cwd).await {
            Some(v) if v >= MIN_MEMBER_CLAUDE => Ok(()),
            None if !preset.starts_with("claude") => Ok(()),
            seen => {
                tracing::info!(seen = ?seen.map(|v| v.to_string()), min = %MIN_MEMBER_CLAUDE, "refused a member's launch on an older claude");
                Err(DomainError::Conflict { what: "claude_too_old" })
            }
        }
    }

    /// Whose login a new pane in `seat` runs on (the module's list).
    pub(crate) fn login_for_new_pane(&self, seat: Option<&Seat>) -> Result<Login> {
        match seat {
            Some(Seat::Rep(_, handle)) => return Ok(Login::member(handle.clone())),
            Some(Seat::Orchestrator(workspace)) => return self.lead_login(*workspace),
            None => {}
        }
        let Ok(caller) = CALLER.try_with(Caller::clone) else { return Ok(Login::Runner) };
        // A caller in a pane is that pane's, whatever device it names.
        if let Some(pane) = caller.pid.and_then(|pid| crate::people::pane_of(self, pid)) {
            return self.login_of_opener(pane);
        }
        match crate::people::person_of(&self.store, &caller.peer)? {
            Person::Member(handle) => Ok(Login::member(handle)),
            Person::Released => Err(DomainError::NotYours { what: "not_yours" }),
            Person::Owner => Ok(Login::Runner),
        }
    }

    /// The login a pane opened from inside `pane` runs on: its lead's for an
    /// orchestrator, else `pane`'s own.
    ///
    /// Only while the opener is on a login that's still current (review 2):
    /// an orchestrator still running on an old lead's login is refused
    /// `lead_changed` rather than handing its lanes the new lead's (the
    /// owner's, once a member lead is removed), and a pane on a removed
    /// member's login is refused `login_gone` rather than opening on whoever
    /// has their handle now.
    fn login_of_opener(&self, pane: Uuid) -> Result<Login> {
        let term = self.store.get_terminal(pane)?;
        let opener = self.recorded_login(&term)?;
        self.refuse_a_gone_generation(&opener)?;
        match (term.role, term.workspace_id) {
            (TerminalRole::Orchestrator, Some(workspace)) => {
                self.refuse_another_leads_login(&term, workspace)?;
                self.lead_login(workspace)
            }
            _ => Ok(match opener {
                Login::Member { handle, .. } => Login::member(handle),
                Login::Runner => Login::Runner,
            }),
        }
    }

    /// Refuse `login_gone` when `login` records a generation of its member
    /// that isn't theirs now: they were removed, and maybe someone else has
    /// the handle.
    fn refuse_a_gone_generation(&self, login: &Login) -> Result<()> {
        if let Login::Member { handle, recorded: Some(recorded) } = login
            && self.store.member_login(handle)?.as_deref() != Some(recorded.login_id.as_str())
        {
            return Err(DomainError::Conflict { what: "login_gone" });
        }
        Ok(())
    }

    pub(crate) fn lead_login(&self, workspace: Uuid) -> Result<Login> {
        Ok(self.store.workspace_lead(workspace)?.map_or(Login::Runner, Login::member))
    }

    /// The login `term` was launched on: the one its row records, else its
    /// rep's member (a rep from before logins were recorded), else the
    /// runner's.
    pub(crate) fn recorded_login(&self, term: &models::Terminal) -> Result<Login> {
        if let Some(recorded) = self.store.terminal_login(term.id)? {
            return Ok(Login::Member { handle: recorded.handle.clone(), recorded: Some(recorded) });
        }
        Ok(match self.store.rep_of_terminal(term.id)? {
            Some(rep) => Login::member(rep.handle),
            None => Login::Runner,
        })
    }

    /// Refuse `lead_changed` when `term`, an orchestrator of `workspace`, is
    /// on a login that isn't its lead's now.
    pub(crate) fn refuse_another_leads_login(&self, term: &models::Terminal, workspace: Uuid) -> Result<()> {
        if self.same_login(&self.recorded_login(term)?, &self.lead_login(workspace)?)? {
            return Ok(());
        }
        tracing::info!(terminal = %term.id, "refused: an orchestrator on a login that isn't its lead's");
        Err(DomainError::Conflict { what: "lead_changed" })
    }

    /// Whether `a` and `b` are one login: the runner's both, or one
    /// generation of one member's (review 1, L5: a handle removed and given
    /// to someone new is another login). A login not yet recorded is the
    /// member's generation now.
    pub(crate) fn same_login(&self, a: &Login, b: &Login) -> Result<bool> {
        Ok(self.generation(a)? == self.generation(b)?)
    }

    fn generation(&self, login: &Login) -> Result<Option<(String, Option<String>)>> {
        Ok(match login {
            Login::Runner => None,
            Login::Member { handle, recorded: Some(recorded) } => Some((handle.clone(), Some(recorded.login_id.clone()))),
            Login::Member { handle, recorded: None } => Some((handle.clone(), self.store.member_login(handle)?)),
        })
    }

    /// `refuse_another_leads_login`, before `terminal` is made its
    /// workspace's orchestrator (`set_terminal_role`).
    pub(crate) fn refuse_orchestrator_on_another_login(&self, terminal: Uuid) -> Result<()> {
        let term = self.store.get_terminal(terminal)?;
        match term.workspace_id {
            Some(workspace) => self.refuse_another_leads_login(&term, workspace),
            None => Ok(()),
        }
    }

    /// `cwd` is the pane's worktree, where the version is asked; an
    /// orchestrator starts in its workspace home, and is asked in its
    /// repository's main checkout instead (review 2, L3, left as is).
    ///
    /// What a launch on `login` of `preset` in `cwd` passes: no directory for
    /// the runner's; for a member, the string recorded on the pane, or a new
    /// directory's, with the row to record. Refused, never the runner's
    /// instead, for a member who's gone or whose handle is someone else's now
    /// (`login_gone`), a claude too old (`claude_too_old`), or a directory
    /// that can't be made or isn't what was recorded.
    pub(crate) async fn launch_dir(&self, login: &Login, preset: &str, cwd: &Path) -> Result<LaunchLogin> {
        let Login::Member { handle, recorded } = login else { return Ok(LaunchLogin::default()) };
        let gone = DomainError::Conflict { what: "login_gone" };
        match self.store.member(handle) {
            Ok(_) => {}
            Err(DomainError::NotFound) => return Err(gone),
            Err(e) => return Err(e),
        }
        if let Some(recorded) = recorded {
            self.refuse_a_gone_generation(login)?;
            self.check_claude_version(preset, cwd).await?;
            // Again after the wait: a removal in between (ov-588 review 2)
            // has wiped this login. A new pane's `ensure_member_login` below
            // is its own check.
            self.refuse_a_gone_generation(login)?;
            if !recorded_dir_holds(self.root_dir(), recorded) {
                tracing::warn!("a pane's recorded Claude directory isn't its login's any more; the launch is refused");
                return Err(DomainError::OperationFailed);
            }
            return Ok(LaunchLogin { dir: Some(recorded.config_dir.clone()), record: None });
        }
        self.check_claude_version(preset, cwd).await?;
        let login_id = self.store.ensure_member_login(handle).map_err(|e| match e {
            DomainError::NotFound => DomainError::Conflict { what: "login_gone" },
            other => other,
        })?;
        let dir = ensure_login_dir(self.root_dir(), &login_id)?;
        let record = TerminalLogin { handle: handle.clone(), login_id, config_dir: dir.clone() };
        Ok(LaunchLogin { dir: Some(dir), record: Some(record) })
    }

    /// Refuse `login_gone`, just before `term`'s pane is spawned, when the
    /// login it's recorded on was removed since `launch_dir` passed it: a
    /// launch awaits between the two, and a removal in that time has wiped
    /// the login (ov-588 review 2).
    pub(crate) fn still_on_its_login(&self, term: &models::Terminal) -> Result<()> {
        #[cfg(test)]
        run_hook(&self.claude_logins.before_spawn);
        self.refuse_a_gone_generation(&self.recorded_login(term)?)
    }

    /// Record a pane's member login, the first time it launches on one.
    pub(crate) fn record_login(&self, terminal: Uuid, launch: &LaunchLogin) -> Result<()> {
        match &launch.record {
            Some(login) => self.store.set_terminal_login(terminal, login),
            None => Ok(()),
        }
    }

    /// The directory a relaunch of `term` passes: on the login it started
    /// on, as recorded, never the runner's instead. An orchestrator whose
    /// lead has changed is refused (`lead_changed`). Not reached from
    /// `relaunch_held` today, which starts such an orchestrator afresh first
    /// under its workspace's lock (`lead_change`): kept so no other caller
    /// can relaunch one on an old lead's login (tested on its own). An old
    /// rep's login is recorded now (review 1, L2).
    pub(crate) async fn relaunch_dir(&self, term: &models::Terminal, cwd: &Path) -> Result<Option<String>> {
        if let (TerminalRole::Orchestrator, Some(workspace)) = (term.role, term.workspace_id) {
            self.refuse_another_leads_login(term, workspace)?;
        }
        let launch = self.launch_dir(&self.recorded_login(term)?, &term.command_preset, cwd).await?;
        self.record_login(term.id, &launch)?;
        Ok(launch.dir)
    }

    /// The config directory `term`'s claude reads and writes: the recorded
    /// member's, else the runner's own. For finding its transcript.
    pub fn claude_dir_of(&self, term: &models::Terminal) -> Option<PathBuf> {
        match self.recorded_login(term).ok()? {
            Login::Member { recorded: Some(recorded), .. } => Some(PathBuf::from(recorded.config_dir)),
            Login::Member { handle, recorded: None } => self.member_claude_dir(&handle),
            Login::Runner => self.claude_logins.runner.clone(),
        }
    }

    /// `handle`'s Claude directory now, if they have a login yet.
    pub fn member_claude_dir(&self, handle: &str) -> Option<PathBuf> {
        login_dir(self.root_dir(), &self.store.member_login(handle).ok()??)
    }

    /// Every config directory a session on this runner may be under.
    pub fn every_claude_dir(&self) -> Vec<PathBuf> {
        every_dir(self.claude_logins.runner.as_deref(), self.root_dir())
    }
}

#[cfg(test)]
#[path = "claude_logins_tests.rs"]
pub(crate) mod tests;
