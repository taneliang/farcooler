//! A member signs in to Claude on the runner from their own device (ov-584,
//! phase 2 of `reports/ov-581/design.md`; design in `reports/ov-584`).
//!
//! **No pane.** The sign-in is `claude auth login` in the member's own config
//! directory, a child of the daemon with piped standard streams, never a
//! tmux pane and never a terminal row. So it's in no `terminal.list`, layout,
//! Needs You item, agent row or inventory, and no screen of it exists for
//! anyone to read. An interactive `claude /login` was the other shape, and it
//! stays a live claude session on the member's account once it's signed in:
//! anyone who could type into that pane could use their account. `auth login`
//! exits when it's done.
//!
//! **The steps** (measured on claude 2.1.301 by ov-584's spike 0):
//!
//! 1. `claude.sign_in` checks the claude is new enough to take a pasted code
//!    and to report its config directory (`MIN_SIGN_IN_CLAUDE`), makes the
//!    member's directory exactly as a launch does (`ensure_login_dir`, so the
//!    Keychain entry claude keys on the string is the one their panes use),
//!    and runs `BROWSER=/usr/bin/false CLAUDE_CONFIG_DIR=<dir> claude auth
//!    login`. The browser variable stops claude opening a page on the
//!    runner's own screen. claude prints `If the browser didn't open, visit:
//!    <URL>`; the URL is checked (`sign_in_page`) and goes to the device that
//!    asked, and to nobody else.
//! 2. `claude.sign_in_finish` writes the pasted `code#state` and a newline to
//!    claude's standard input, so the code is never on a command line. claude
//!    exits 0 when it took it. The success text isn't measured yet, so it's
//!    not read: an exit of 0 is confirmed by `claude auth status --json`
//!    saying `loggedIn` in this very directory.
//! 3. `claude.sign_out`, and a removal (`member_removal`), run `claude auth
//!    logout` in the directory, which asks Anthropic to revoke the sign-in as
//!    well (ov-582 spike 2 read that in claude's code). It exits 0 even when
//!    nothing was signed in, so its status is confirmed the same way: that
//!    proves the login is gone from the runner, not that the revoke reached
//!    Anthropic, which isn't measured (review M3).
//!
//! **Who may** (`people::may_sign_in`): the member themself, from their own
//! device, in no pane. Never the owner, another member, a runner admin for
//! someone else, a released device, or any process in a pane, whatever
//! device its connection names. A guardrail, not a boundary, as
//! `claude_logins` says: every process on the runner is the same Unix user,
//! and one outside any pane can name a member's device.
//!
//! **What's never kept or logged:** the code, claude's output, and the URL.
//! A failure is logged by its kind alone. A sign-in lives in memory, ends
//! after `LIFETIME`, and its process is killed with its whole process group
//! when it ends, however it ends (`Live`'s drop).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, ClaudeLoginState};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use uuid::Uuid;

use crate::claude_logins::{Version, ensure_login_dir};
use crate::service::{Service, shell_quote};

/// The oldest claude whose `auth login` takes a pasted code (2.1.126) and
/// whose `auth status --json` names its config directory (2.1.268), which
/// the finish checks: ov-582 spike 1, ov-584 spike 0.
pub const MIN_SIGN_IN_CLAUDE: Version = Version(2, 1, 268);

/// What `$BROWSER` is for the sign-in: a program that opens nothing, so the
/// runner's own screen never shows a member's sign-in page (spike 0: claude
/// ignores a browser that fails, and prints the URL).
pub const BROWSER_OFF: &str = "/usr/bin/false";

/// How long a sign-in lasts from its start.
pub const LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The longest a pasted code may be.
pub const MAX_CODE: usize = 2048;

/// The longest sign-in page address taken from claude.
pub const MAX_URL: usize = 4096;

/// How many sign-ins one member may start in `START_WINDOW` (checklist 16):
/// each runs a claude, so a stuck button or a script can't run dozens.
pub const MAX_STARTS: usize = 5;

/// The window `MAX_STARTS` counts in.
pub const START_WINDOW: Duration = Duration::from_secs(60);

/// The POSIX `sh` script a sign-in runs under, whatever the login shell is
/// (review H1: fish refuses POSIX syntax): it starts a watchdog that kills
/// the whole process group at `$1` seconds, runs the login shell (`$2…`) on
/// the standard input it was given, waits for it, stops the watchdog, and
/// exits as the login shell did. `auth login` waits for its code forever
/// (spike 0), and a daemon that crashes or is killed never drops the
/// sign-in that would kill it.
///
/// A non-interactive `sh` has no job control, so both stay in its group. A
/// background job's input would be /dev/null, so the shell gets the
/// script's own by descriptor 3, which the watchdog closes (`3<&-`) along
/// with its standard streams: it holds none of claude's, so claude's end is
/// the end of its output, and nothing but claude reads the code. On TERM it
/// stops its `sleep` too, leaving nothing of the group behind.
const WATCHDOG: &str = "s=$1; shift; exec 3<&0; \
( trap 'kill $p 2>/dev/null; exit 0' TERM; sleep \"$s\" & p=$!; wait $p; kill -KILL 0 ) </dev/null >/dev/null 2>&1 3<&- & w=$!; \
\"$@\" <&3 3<&- & c=$!; wait $c; st=$?; kill -TERM $w 2>/dev/null; wait $w 2>/dev/null; exit $st";

/// The sign-in pages claude is measured to give, by host and path: a
/// claude.ai login, and the Console's (`--console`, never passed here, but a
/// claude that defaults to it gives that page).
const PAGES: &[(&str, &str)] = &[("claude.com", "/cai/oauth/authorize"), ("platform.claude.com", "/oauth/authorize")];

/// The waits, each injectable so a test runs them near zero.
#[derive(Debug, Clone, Copy)]
pub struct Waits {
    /// For claude to print the sign-in page.
    pub page: Duration,
    /// For claude to exit after the code.
    pub finish: Duration,
    /// For `auth status` or `auth logout`.
    pub command: Duration,
    /// A sign-in's whole life.
    pub lifetime: Duration,
    /// How long past its life a sign-in ends itself, with no daemon to end
    /// it (`SignIns::command`'s watchdog).
    pub margin: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Waits { page: Duration::from_secs(30), finish: Duration::from_secs(30), command: Duration::from_secs(20), lifetime: LIFETIME, margin: Duration::from_secs(60) }
    }
}

/// The claude a sign-in runs, in place of the one a login shell finds: a
/// test's stand-in (`tests/fixtures/fake-claude.pl`), run by `/bin/sh -c`.
#[derive(Debug, Clone)]
pub struct Program {
    pub shell: String,
    pub flag: &'static str,
    pub claude: String,
    /// Variables set on the shell itself: none for the real one; a test's
    /// empty `HOME`, so no rc file of whoever runs the suite reads in.
    pub env: Vec<(String, String)>,
}

impl Program {
    /// The claude a pane finds: through the login shell, as a pane runs it,
    /// after its rc files, so none of them can change the directory.
    fn real() -> Program {
        Program { shell: farcooler_core::shell::login_shell(), flag: "-ilc", claude: "claude".into(), env: Vec::new() }
    }
}

/// Every sign-in under way, one per member, and how they're run.
#[derive(Default)]
pub struct SignIns {
    live: Arc<Mutex<HashMap<String, Live>>>,
    /// A test's or a harness's claude; `None` is the real one, outside the
    /// daemon's own tests, which never run it.
    pub(crate) program: Mutex<Option<Program>>,
    pub(crate) waits: Mutex<Waits>,
    /// When each member last started sign-ins, within `START_WINDOW`.
    starts: Mutex<HashMap<String, Vec<Instant>>>,
    /// One gate per member, held by a finish under way, so a removal can wait
    /// for it (`end_and_wait`) and wipe what it signed in (review M2).
    finishing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Each command a sign-in, status or logout ran, as its shell's payload,
    /// for a test to read: never a code, which goes by standard input.
    #[cfg(test)]
    pub(crate) ran: Mutex<Vec<String>>,
}

/// One sign-in under way.
struct Live {
    id: Uuid,
    /// The device that started it, which alone may finish or cancel it.
    client_id: Option<String>,
    login_id: String,
    dir: String,
    /// Its process group, the shell's pid.
    group: Option<i32>,
    /// When its life is over: a finish that times out after it ends it.
    expires: Instant,
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
}

impl Drop for Live {
    /// Its whole process group, so the claude the login shell started goes
    /// with the shell (`kill_on_drop` reaches the shell alone). Only while
    /// its shell is still running and so unreaped: then no one else can
    /// have the group's id. Once it has ended and been reaped, the id may be
    /// someone else's, and it's left alone (review M1); the watchdog script
    /// leaves nothing behind when its shell ends.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            kill_group(self.group);
        }
    }
}

fn kill_group(group: Option<i32>) {
    if let Some(group) = group.and_then(rustix::process::Pid::from_raw) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SignIns {
    fn waits(&self) -> Waits {
        *lock(&self.waits)
    }

    /// The claude to run, or a refusal where none may run: the daemon's own
    /// tests without a stand-in, and a harness's daemon (stub agents, a
    /// stand-in agent), which must never touch a real login.
    fn program(&self) -> Result<Program> {
        if let Some(program) = lock(&self.program).clone() {
            return Ok(program);
        }
        if cfg!(test) || crate::service::test_stub_agents() || crate::service::stand_in_agent().is_some() {
            return Err(DomainError::OperationFailed);
        }
        Ok(Program::real())
    }

    /// End `handle`'s sign-in, if one is under way.
    pub(crate) fn end_for(&self, handle: &str) {
        let ended = lock(&self.live).remove(handle);
        drop(ended);
    }

    /// End `handle`'s sign-in, and wait (bounded) for a finish under way,
    /// whose sign-in is out of the map while claude takes the code: a
    /// removal does this once the member is gone, so a finish can't land
    /// after the removal's wipe (review M2).
    pub(crate) async fn end_and_wait(&self, handle: &str) {
        self.end_for(handle);
        let gate = self.gate(handle);
        let waits = self.waits();
        if tokio::time::timeout(waits.finish + waits.command * 3, gate.lock()).await.is_err() {
            tracing::warn!("a sign-in's finish was still under way when its member was removed");
        }
        // Again, once the finish has let go: one that timed out may have put
        // its sign-in back, and a start may have slipped in before the first
        // (re-review).
        self.end_for(handle);
    }

    fn gate(&self, handle: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.finishing).entry(handle.to_string()).or_default().clone()
    }

    /// Every sign-in ended, its processes with it: the daemon's shutdown
    /// (review L3).
    pub fn end_all(&self) {
        let ended: Vec<Live> = lock(&self.live).drain().map(|(_, l)| l).collect();
        drop(ended);
    }

    /// Count a start for `handle`, or refuse it past `MAX_STARTS` in
    /// `START_WINDOW`.
    fn count_start(&self, handle: &str) -> Result<()> {
        let now = Instant::now();
        let mut starts = lock(&self.starts);
        let mine = starts.entry(handle.to_string()).or_default();
        mine.retain(|t| now.duration_since(*t) < START_WINDOW);
        if mine.len() >= MAX_STARTS {
            tracing::info!("refused a sign-in: too many started this minute");
            return Err(DomainError::Conflict { what: "sign_in_too_often" });
        }
        mine.push(now);
        Ok(())
    }

    /// Whether `handle` has a sign-in under way.
    pub fn under_way(&self, handle: &str) -> bool {
        lock(&self.live).contains_key(handle)
    }

    fn take(&self, handle: &str, session: Uuid, client_id: Option<&str>) -> Result<Live> {
        let mut live = lock(&self.live);
        match live.get(handle) {
            Some(l) if l.id == session && l.client_id.as_deref() == client_id => Ok(live.remove(handle).expect("just seen")),
            // Theirs from another device, or someone else's.
            _ if live.values().any(|l| l.id == session) => Err(DomainError::NotYours { what: "not_yours" }),
            _ => Err(DomainError::NotFound),
        }
    }

    /// Put `live` back, unless another sign-in took its place meanwhile,
    /// its life is over (its expiry ran while it was out, and nothing would
    /// end it again: review M1), or its login isn't its member's any more
    /// (`still_theirs` false: they were removed meanwhile, re-review). Either
    /// way, it's ended.
    fn put_back(&self, handle: &str, live: Live, still_theirs: bool) {
        if !still_theirs || Instant::now() >= live.expires {
            return;
        }
        lock(&self.live).entry(handle.to_string()).or_insert(live);
    }

    /// Run `claude <args>` in `dir` through `program`, its output read but
    /// never logged: `(exit status, standard output)`, or `None` when it
    /// couldn't run or didn't end in `wait`.
    async fn run(&self, program: &Program, dir: &str, args: &str, wait: Duration) -> Option<(std::process::ExitStatus, String)> {
        let mut command = self.command(program, dir, args, None);
        // Nothing to read from (review L5); `auth login` alone needs its input.
        command.stdin(std::process::Stdio::null());
        command.stderr(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());
        let mut child = command.spawn().map_err(|e| tracing::warn!(kind = ?e.kind(), "couldn't run claude for a sign-in")).ok()?;
        let group = child.id().map(|p| p as i32);
        let mut stdout = child.stdout.take()?;
        let output = tokio::time::timeout(wait, async {
            let mut out = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut out).await?;
            Ok::<_, std::io::Error>((child.wait().await?, out))
        })
        .await;
        match output {
            Ok(Ok((status, out))) => Some((status, String::from_utf8_lossy(&out).into_owned())),
            Ok(Err(e)) => {
                tracing::warn!(kind = ?e.kind(), "claude for a sign-in failed");
                None
            }
            Err(_) => {
                // Its whole group, while the shell is unreaped (review L4).
                if matches!(child.try_wait(), Ok(None)) {
                    kill_group(group);
                }
                tracing::warn!("claude for a sign-in didn't finish in time");
                None
            }
        }
    }

    /// `claude <args>` with `CLAUDE_CONFIG_DIR` set to exactly `dir` inside
    /// the login shell's payload, and no browser, in its own process group.
    /// The payload is one simple command, which every login shell reads the
    /// same, fish's included (review H1). With `watchdog`, it runs under
    /// `WATCHDOG`, a POSIX `sh` that ends the group when that time is up.
    ///
    /// Never in `dir`, which a removal may have deleted (review M2): in the
    /// daemon's own directory.
    fn command(&self, program: &Program, dir: &str, args: &str, watchdog: Option<Duration>) -> tokio::process::Command {
        let payload = format!(
            "{}BROWSER={BROWSER_OFF} CLAUDE_CONFIG_DIR={} {} {args}",
            crate::claude_logins::env_without_owner_credentials(),
            shell_quote(dir),
            program.claude
        );
        #[cfg(test)]
        lock(&self.ran).push(payload.clone());
        let mut command = match watchdog {
            Some(after) => {
                let mut sh = tokio::process::Command::new("/bin/sh");
                let seconds = after.as_secs().max(1).to_string();
                sh.args(["-c", WATCHDOG, "sign-in", &seconds, &program.shell, program.flag, &payload]);
                sh
            }
            None => {
                let mut shell = tokio::process::Command::new(&program.shell);
                shell.args([program.flag, &payload]);
                shell
            }
        };
        command.envs(program.env.iter().map(|(k, v)| (k, v))).process_group(0).kill_on_drop(true);
        command
    }
}

/// The sign-in page in a line claude printed, when it's one this runner
/// trusts (`PAGES`): https, a known host and path, no user, port, space or
/// control character, at most `MAX_URL` long. Anything else, from a claude
/// that changed or a program that isn't claude, is no page: a member is
/// never sent to an address the runner can't vouch for.
pub fn sign_in_page(line: &str) -> Option<String> {
    let url = line.split_whitespace().find(|w| w.starts_with("https://"))?;
    if url.len() > MAX_URL || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return None;
    }
    let rest = &url["https://".len()..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, path_and_query) = rest.split_at(end);
    let path = path_and_query.split(['?', '#']).next().unwrap_or_default();
    PAGES
        .iter()
        .any(|(h, p)| host == *h && path == *p && path_and_query[path.len()..].starts_with('?'))
        .then(|| url.to_string())
}

/// A pasted code as claude takes it: `code#state`, both halves there, no
/// control character, at most `MAX_CODE` bytes, trimmed of the spaces and
/// line break a clipboard brings.
pub fn pasted_code(code: &str) -> Result<&str> {
    let code = code.trim();
    let bad = DomainError::InvalidArgument { what: "sign_in_code" };
    if code.len() > MAX_CODE || code.chars().any(char::is_control) {
        return Err(bad);
    }
    match code.split_once('#') {
        Some((a, b)) if !a.is_empty() && !b.is_empty() => Ok(code),
        _ => Err(bad),
    }
}

/// What `claude auth status --json` says of a directory: whether it's
/// signed in, and the directory it read. Tolerant of every other key, since
/// the signed-in answer isn't measured yet.
pub fn status_of(stdout: &str) -> Option<(bool, String)> {
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    Some((json.get("loggedIn")?.as_bool()?, json.get("configDirectory")?.as_str()?.to_string()))
}

impl Service {
    /// `claude.sign_in` for `handle`, from the device `client_id`; who may
    /// is checked before (`people::may_sign_in`).
    pub(crate) async fn start_sign_in(&self, handle: &str, client_id: Option<&str>) -> Result<pb::ClaudeSignIn> {
        let sign_ins = &self.claude_logins.sign_ins;
        self.store.member(handle)?;
        let program = sign_ins.program()?;
        match self.claude_version(self.root_dir()).await {
            Some(v) if v >= MIN_SIGN_IN_CLAUDE => {}
            seen => {
                tracing::info!(seen = ?seen.map(|v| v.to_string()), min = %MIN_SIGN_IN_CLAUDE, "refused a sign-in on an older claude");
                return Err(DomainError::Conflict { what: "sign_in_too_old" });
            }
        }
        let login_id = self.store.ensure_member_login(handle).map_err(|e| match e {
            DomainError::NotFound => DomainError::Conflict { what: "login_gone" },
            other => other,
        })?;
        let dir = ensure_login_dir(self.root_dir(), &login_id)?;
        sign_ins.count_start(handle)?;
        sign_ins.end_for(handle);

        let waits = sign_ins.waits();
        // Past its life, in case the daemon isn't there to end it.
        let mut command = sign_ins.command(&program, &dir, "auth login", Some(waits.lifetime + waits.margin));
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
        let mut child = command.spawn().map_err(|e| {
            tracing::warn!(kind = ?e.kind(), "couldn't start claude auth login");
            DomainError::OperationFailed
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or(DomainError::OperationFailed)?;
        let group = child.id().map(|p| p as i32);
        let expires = Instant::now() + waits.lifetime;
        let live = Live { id: Uuid::now_v7(), client_id: client_id.map(str::to_string), login_id, dir, group, expires, child, stdin };

        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let page = tokio::time::timeout(waits.page, async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(page) = sign_in_page(&line) {
                    return Some(page);
                }
            }
            None
        })
        .await;
        let Ok(Some(url)) = page else {
            tracing::warn!("claude auth login gave no sign-in page this runner knows");
            return Err(DomainError::Conflict { what: "sign_in_unreadable" });
        };
        // Its later output (the prompt, a success line) is drained, unread:
        // a closed pipe would end claude.
        let mut rest = lines.into_inner();
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut rest, &mut tokio::io::sink()).await;
        });

        // A removal while claude started: the login is gone.
        if self.store.member_login(handle)?.as_deref() != Some(live.login_id.as_str()) {
            return Err(DomainError::Conflict { what: "login_gone" });
        }
        let expires_at = std::time::SystemTime::now() + waits.lifetime;
        let session = live.id;
        lock(&sign_ins.live).insert(handle.to_string(), live);
        let all = sign_ins.live.clone();
        let (who, deadline) = (handle.to_string(), expires);
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline.into()).await;
            let mut all = lock(&all);
            if all.get(&who).is_some_and(|l| l.id == session) {
                all.remove(&who);
            }
        });
        Ok(pb::ClaudeSignIn {
            session_id: crate::wire::id_bytes(session),
            url,
            expires_at: expires_at.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default(),
        })
    }

    /// `claude.sign_in_finish`: `code` to `handle`'s sign-in `session`,
    /// started from `client_id`. A code that isn't whole is refused and the
    /// sign-in goes on; one claude refuses ends it.
    pub(crate) async fn finish_sign_in(&self, handle: &str, session: Uuid, client_id: Option<&str>, code: &str) -> Result<pb::MemberClaudeLogin> {
        let sign_ins = &self.claude_logins.sign_ins;
        let code = pasted_code(code)?;
        // Held until this finish is over, so a removal waits for it.
        let _finishing = sign_ins.gate(handle).lock_owned().await;
        let mut live = sign_ins.take(handle, session, client_id)?;
        let waits = sign_ins.waits();
        let Some(mut stdin) = live.stdin.take() else { return Err(DomainError::OperationFailed) };
        let wrote = async {
            stdin.write_all(code.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        }
        .await;
        if let Err(e) = wrote {
            tracing::warn!(kind = ?e.kind(), "couldn't give claude the sign-in code");
            return Err(DomainError::OperationFailed);
        }
        let exited = match tokio::time::timeout(waits.finish, live.child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                tracing::warn!(kind = ?e.kind(), "couldn't wait for claude auth login");
                return Err(DomainError::OperationFailed);
            }
            Err(_) => {
                // It's still waiting: give it back its input, and the
                // sign-in goes on until it ends or is cancelled.
                tracing::info!("claude auth login didn't take the code in time");
                live.stdin = Some(stdin);
                let still_theirs = matches!(self.store.member_login(handle), Ok(Some(id)) if id == live.login_id);
                sign_ins.put_back(handle, live, still_theirs);
                return Err(DomainError::Conflict { what: "code_refused" });
            }
        };
        if !exited.success() {
            tracing::info!(exit = ?exited.code(), "claude refused a sign-in code");
            return Err(DomainError::Conflict { what: "code_refused" });
        }
        // Removed while signing in: what claude just signed in is wiped as
        // a removal wipes, so nothing of theirs is left (review M2).
        if self.store.member_login(handle)?.as_deref() != Some(live.login_id.as_str()) {
            let failed = self.wipe_sign_in(&live.login_id).await;
            if !failed.is_empty() {
                tracing::warn!(?failed, "a removed member's sign-in couldn't be wiped whole");
            }
            return Err(DomainError::Conflict { what: "login_gone" });
        }
        match self.claude_status(&sign_ins.program()?, &live.dir).await {
            Some(true) => {
                // Signing in again ends the "stopped working" and the sign-in
                // ask, and with them the member's Needs You item.
                self.store.record_sign_in(handle, crate::review::now_millis())?;
                Ok(self.member_claude_login(handle, true))
            }
            _ => {
                tracing::warn!("claude auth login exited 0 but the directory isn't signed in");
                Err(DomainError::OperationFailed)
            }
        }
    }

    /// `claude.sign_in_cancel`.
    pub(crate) fn cancel_sign_in(&self, handle: &str, session: Uuid, client_id: Option<&str>) -> Result<pb::MemberClaudeLogin> {
        drop(self.claude_logins.sign_ins.take(handle, session, client_id)?);
        Ok(self.member_claude_login(handle, true))
    }

    /// `claude.sign_out`: `claude auth logout` in every string `handle`'s
    /// directory was handed to claude as, then confirmed signed out.
    pub(crate) async fn sign_out(&self, handle: &str) -> Result<pb::MemberClaudeLogin> {
        self.store.member(handle)?;
        let sign_ins = &self.claude_logins.sign_ins;
        sign_ins.end_for(handle);
        let Some(login_id) = self.store.member_login(handle)? else { return Ok(self.member_claude_login(handle, true)) };
        let program = sign_ins.program()?;
        let mut all_out = true;
        for dir in self.login_strings(&login_id) {
            all_out &= self.claude_logout(&program, &dir).await;
        }
        if !all_out {
            return Err(DomainError::OperationFailed);
        }
        // Deliberate: `none`, with nothing left to nag about.
        self.store.clear_login_flags(handle)?;
        Ok(self.member_claude_login(handle, true))
    }

    /// The directory strings login `login_id` is, as they exist now: the
    /// canonical one and each a pane recorded, each still a real directory.
    fn login_strings(&self, login_id: &str) -> Vec<String> {
        let canonical = crate::claude_logins::login_dir(self.root_dir(), login_id)
            .and_then(|d| std::fs::canonicalize(d).ok())
            .and_then(|d| d.into_os_string().into_string().ok());
        let recorded = self.store.terminal_logins().unwrap_or_default().into_iter().filter(|(_, l)| l.login_id == login_id).map(|(_, l)| l.config_dir);
        let mut dirs: Vec<String> = canonical.into_iter().chain(recorded).filter(|d| is_real_dir(Path::new(d))).collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// `claude auth logout` in `dir`, then `true` once `auth status` says
    /// it's signed out there. Never run for an empty directory or the
    /// runner's own: either would sign the owner out.
    pub(crate) async fn claude_logout(&self, program: &Program, dir: &str) -> bool {
        if dir.is_empty() || self.claude_logins.runner.as_deref().is_some_and(|r| r == Path::new(dir)) {
            tracing::warn!("refused to sign out the runner's own Claude login");
            return false;
        }
        let waits = self.claude_logins.sign_ins.waits();
        if self.claude_logins.sign_ins.run(program, dir, "auth logout", waits.command).await.is_none() {
            return false;
        }
        self.claude_status(program, dir).await == Some(false)
    }

    /// Whether `dir` is signed in, by `claude auth status --json`: `None`
    /// when claude didn't answer, or answered for another directory.
    async fn claude_status(&self, program: &Program, dir: &str) -> Option<bool> {
        let waits = self.claude_logins.sign_ins.waits();
        let (_, stdout) = self.claude_logins.sign_ins.run(program, dir, "auth status --json", waits.command).await?;
        let (signed_in, read) = status_of(&stdout)?;
        if read != dir {
            tracing::warn!("claude auth status read another directory");
            return None;
        }
        Some(signed_in)
    }

    /// `handle`'s Claude login as `member.list` shows it: signed in when
    /// their directory names an account. Its email only when `with_email`.
    pub(crate) fn member_claude_login(&self, handle: &str, with_email: bool) -> pb::MemberClaudeLogin {
        let email = self.member_claude_dir(handle).and_then(|d| crate::people::account_in(&d.join(".claude.json")));
        // A login that exists but whose agents fail to authenticate
        // (`claude_login_renewal`); only while it names an account, so a
        // Sign Out is `none` however it got there.
        let stopped = email.is_some()
            && with_email
            && self.store.login_flags_of(handle).ok().flatten().is_some_and(|f| f.stopped_at.is_some());
        let state = match (email.is_some(), stopped) {
            (false, _) => ClaudeLoginState::None,
            (true, false) => ClaudeLoginState::SignedIn,
            (true, true) => ClaudeLoginState::StoppedWorking,
        };
        pb::MemberClaudeLogin {
            handle: handle.to_string(),
            state: state as i32,
            email: email.filter(|_| with_email).unwrap_or_default(),
        }
    }

    /// End every sign-in under way, its processes with it: the daemon's
    /// shutdown (review L3).
    pub fn end_sign_ins(&self) {
        self.claude_logins.sign_ins.end_all();
    }

    /// The claude a sign-in or a sign-out runs, for a removal's sign-out.
    pub(crate) fn sign_in_program(&self) -> Result<Program> {
        self.claude_logins.sign_ins.program()
    }
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

#[cfg(test)]
#[path = "claude_sign_in_tests.rs"]
mod tests;
