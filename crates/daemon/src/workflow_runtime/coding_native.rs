//! Retained managed tmux PTY + normalized hook ownership, without Service activation.
use super::*;
use super::coding::FileValidation;
use super::native_messages::{ClaudeStop, Followup, IdleDirection, send_native_reply};
use farcooler_agent_core::event::{AgentEvent, EndReason};
use farcooler_agent_hooks::{Agent, assemble::MessageAssembler, facts::facts, wire::HookLine};
use farcooler_tmux::TmuxServer;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[path = "coding_native_receipts.rs"]
mod receipts;
use receipts::{NativeOwner, NativeSettlement};

#[path = "coding_native_quiesce.rs"]
mod quiesce;

#[cfg(test)]
thread_local! { pub(super) static TEST_ARGV: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) }; }
use crate::native_hold::{self, Answer};

/// How long a reviewer that has recorded its verdict may stay quiet before its turn is taken as over (ov-1082): a
/// Stop lost on the way, under load, must not hold a decided review for the turn's three hours. Measured from the
/// verdict or the reviewer's last hook, whichever is later, so one that is still writing its summary finishes it.
#[cfg(not(test))]
const VERDICT_QUIET: Duration = Duration::from_secs(60);
#[cfg(test)]
const VERDICT_QUIET: Duration = Duration::from_secs(1);
/// How long a build that ended its turn without committing is kept, quiet, for its next turn (ov-1090). An agent that
/// backgrounds a wait (until the load is down; for a test run) ends its turn and is woken when the wait is over:
/// ending it at its first turn's end left ov-1076's and ov-1084's edits uncommitted. Measured from the turn's end or
/// the agent's last hook, whichever is later. Past it, the build has given up: its step fails and runs again in the
/// same tree, which still holds the work.
#[cfg(not(test))]
const UNFINISHED_QUIET: Duration = Duration::from_secs(30 * 60);
#[cfg(test)]
const UNFINISHED_QUIET: Duration = Duration::from_secs(2);

/// A startup refusal ends the turn even if the harness leaves its interactive prompt alive. This is
/// negative evidence only; screen text can never supply a successful result.
fn startup_refusal(agent: Agent, output: &str) -> Option<String> {
    if agent != Agent::Codex { return None; }
    // tmux wraps the error at the pane width. Only the harness's error line starts the match.
    let line = output.lines().position(|line| line.trim_start().starts_with("■ The '"))?;
    let tail = output.lines().skip(line).collect::<Vec<_>>().join("\n");
    let tail = tail.trim_start().strip_prefix("■ The '")?;
    let words = tail.split_whitespace().collect::<Vec<_>>().join(" ");
    let (model, rest) = words.split_once("' model is not supported when using Codex with a ChatGPT account")?;
    if model.is_empty() || model.len() > 100 || !model.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !rest.is_empty() && !rest.starts_with(['.', '\n', ' ']) { return None; }
    Some(format!("The '{model}' model is not supported when using Codex with a ChatGPT account. Choose another model."))
}

/// What the item says of a turn the runner stopped reading: a sentence the orchestrator can act on.
fn turn_ended_reason(error: &Error) -> String {
    let why = match error { Error::Refused(why) => why.to_string(), other => other.to_string() };
    format!("The agent's turn ended before it finished: {why}. Its work stays in the worktree for the step's next \
        attempt, a retry or a send-back.")
}
/// What the item says of a turn its step's time limit ended. Time a landing gate or a hand pause held it is not
/// counted, so this is the agent's own time.
fn out_of_time_reason(limit: Duration) -> String {
    let minutes = limit.as_secs().div_ceil(60);
    format!("The agent's turn ran past the step's {minutes}-minute time limit, so it was ended. Its work stays in the \
        worktree for the step's next attempt, a retry or a send-back.")
}
/// How long the runner may fail to read a live agent's pane before it gives the step up (ov-1301). A tmux that
/// doesn't answer in a second is a busy runner, not a lost session: the Canary ended four builds in a minute on that
/// misreading, each Failed with no reason. Only a pane tmux answers for as closed or dead ends the turn at once.
#[cfg(not(test))]
const UNREADABLE_PANE_FOR: Duration = Duration::from_secs(120);
#[cfg(test)]
const UNREADABLE_PANE_FOR: Duration = Duration::from_secs(2);
#[cfg(test)]
thread_local! {
    /// Test hook: how many pane reads to fail as a tmux that didn't answer in time.
    pub(super) static UNREADABLE_PANE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
/// When the pane reads started failing, and how many in a row were closed: a closed pane is read twice, 250 ms
/// apart, before the turn is taken as lost.
#[derive(Default)]
struct PaneUnread { since: Option<Duration>, closed: u8 }
enum PaneLook { Live, Unread, Lost(&'static str) }
/// What one look at the agent's pane says of its turn, at `now` on the step's active clock.
fn pane_look(life: farcooler_core::Result<farcooler_tmux::PaneLife>, unread: &mut PaneUnread, now: Duration) -> PaneLook {
    match life {
        Ok(farcooler_tmux::PaneLife::Live) => { *unread = PaneUnread::default(); PaneLook::Live }
        Ok(_) => {
            unread.closed += 1;
            if unread.closed >= 2 { PaneLook::Lost("the native session was lost without a receipt") } else { PaneLook::Unread }
        }
        Err(_) => {
            unread.closed = 0;
            let since = *unread.since.get_or_insert(now);
            if now.saturating_sub(since) >= UNREADABLE_PANE_FOR {
                PaneLook::Lost("the runner could not read the agent's pane for two minutes")
            } else { PaneLook::Unread }
        }
    }
}

// Each effect has its own retained server, never the installed daemon's socket.
// A cancelled future tears down its server but cannot publish quiescence/success.
struct Host(TmuxServer);
impl Drop for Host {
    fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); }
}
/// Close an ended owner's window. A window tmux has already closed is closed.
async fn close_native(tmux: &TmuxServer, owner: &NativeOwner) -> Result<()> {
    if tmux.kill_terminal_window(owner.terminal).await.map_err(|_| Error::Refused("native terminal close failed"))?
        || tmux.pane_life(&owner.pane, owner.terminal).await.is_ok_and(|life| life == farcooler_tmux::PaneLife::Gone) {
        return Ok(());
    }
    Err(Error::Refused("native terminal close unproved"))
}
pub(super) fn terminal(binding: &Binding) -> Uuid {
    let pin = digest(&("native-terminal/v1", binding));
    Uuid::parse_str(&pin[4..36]).expect("digest UUID")
}
fn server(root: &Path, binding: &Binding) -> TmuxServer {
    let id = terminal(binding);
    TmuxServer::in_dir(&format!("workflow-{id}"), id, root)
}
pub(super) async fn snapshot() -> Result<crate::foreground::Foreground> {
    let raw = crate::proc_table::topology_async().await.ok_or(Error::Refused("native process table unavailable"))?;
    // Unlike display/label reads, an empty/partial fallback is never quiescence.
    if !raw.lines().any(|line| line.split_whitespace().next() == Some(std::process::id().to_string().as_str())) {
        return Err(Error::Refused("native process table incomplete"));
    }
    Ok(crate::foreground::parse(&raw))
}

/// The user on the other end of a hook's connection. A hook that isn't gating has hung up by the time it is read
/// (crates/cli/src/hook.rs, `first_contact`). On macOS tokio's `peer_cred` also asks for the peer's pid, which the
/// kernel refuses with ENOTCONN once the peer has closed, and that ended every real build at its first hook
/// (ov-1075). `getpeereid` keeps the uid the socket was connected with.
#[cfg(target_os = "macos")]
fn peer_uid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    (unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0).then_some(uid)
}
/// Linux's `SO_PEERCRED` is kept from connect, so it answers after the peer has closed.
#[cfg(not(target_os = "macos"))]
fn peer_uid(stream: &tokio::net::UnixStream) -> Option<u32> { stream.peer_cred().ok().map(|cred| cred.uid()) }

/// How long the hold may take to end an agent: its grace, then however long the kernel takes to kill the group.
const END_WAIT: Duration = Duration::from_secs(30);

/// The script the hold runs for `command` (ov-1330): `pane_command` puts it under the hold.
fn native_launch(command: &CodingCommand, quarantine_receipt: &Path) -> String {
    let mut launch = String::from("env -i");
    for (key, value) in &command.environment {
        launch.push(' '); launch.push_str(&farcooler_core::shell::shell_quote(&format!("{key}={value}")));
    }
    if cfg!(target_os = "macos") {
        launch.push(' ');
        launch.push_str(&farcooler_core::shell::shell_quote(&format!(
            "FARCOOLER_NATIVE_QUARANTINE_RECEIPT={}", quarantine_receipt.display(),
        )));
    }
    for arg in &command.argv { launch.push(' '); launch.push_str(&farcooler_core::shell::shell_quote(arg)); }
    // Retained PTY stdin, not null: read gates launch until tags and owner are durable. The hold is the pane's
    // leader (tmux continues a stopped leader); this shell leads the agent's own group and becomes the agent.
    format!("read -r workflow_start; exec {launch}")
}

impl Exec {
    /// A step that starts a native agent: a build, or an agent's review gate (ov-969).
    pub(super) fn is_native_agent(&self, binding: &Binding) -> bool {
        matches!(&binding.effect.kind, EffectKind::Execute(kind) if starts_agent(kind))
            && matches!(self.coding.as_ref().map(|c| &c.harness), Some(CodingHarness::Native { .. }))
    }
    /// Each native build listens on a socket of its own, so builds run side by side. The pinned
    /// hooks file still names `h.sock`; the build's pane tells the hook CLI where to report instead.
    pub(super) fn native_socket(&self, binding: &Binding) -> PathBuf {
        self.root.join(format!("n-{}.sock", &terminal(binding).simple().to_string()[..12]))
    }
    /// The hold listens beside the hook socket, with the same path length budget on Darwin.
    pub(super) fn native_hold_socket(&self, binding: &Binding) -> PathBuf {
        self.root.join(format!("h-{}.sock", &terminal(binding).simple().to_string()[..12]))
    }
    /// The pane command that runs `script` under this binding's hold.
    pub(super) fn held(&self, binding: &Binding, script: &str) -> Result<String> {
        let hold = crate::runtime::fanout_binary().ok_or(Error::Refused("this runner's farcoolerd can't be found to hold the agent"))?;
        Ok(native_hold::pane_command(&hold, &self.native_hold_socket(binding), script))
    }
    /// Whether the binding's hold answers. Until it does nothing it runs has been let out of the launch barrier.
    pub(super) async fn hold_ready(&self, binding: &Binding) -> bool {
        let socket = self.native_hold_socket(binding);
        for _ in 0..100 {
            if native_hold::ask(&socket, "ready", Duration::from_secs(1)).await == Answer::Said("ready".into()) { return true; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }
    fn native_path(&self, binding: &Binding, suffix: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.native-{suffix}.json", digest(&binding.effect)))
    }
    fn quarantine_receipt(&self, binding: &Binding) -> PathBuf {
        self.native_path(binding, "quarantine")
    }
    fn quarantine_reason(&self, binding: &Binding) -> Option<String> {
        if !cfg!(target_os = "macos") { return None; }
        let path = std::fs::read(self.quarantine_receipt(binding)).ok()?;
        // The whole path (Darwin's limit is 1024 bytes): a shortened one would break the command.
        let path: String = String::from_utf8_lossy(&path).chars().filter(|c| !c.is_control()).take(1024).collect();
        if path.is_empty() { return None; }
        // The command comes last, with nothing after it, so it can be copied as it stands. Only a person runs it.
        Some(format!("{} macOS marked {path} with com.apple.quarantine. Verify the installation before retrying. \
            To clear the mark, run: /usr/bin/xattr -d com.apple.quarantine {}",
            crate::engine_stuck::QUARANTINE_PREFIX, farcooler_core::shell::shell_quote(&path)))
    }
    /// The agent a build runs: the model it picked at Start, or the configured preset (ov-966).
    pub(super) fn native_agent(&self, binding: &Binding) -> Result<Agent> {
        let Some(CodingConfig { harness: CodingHarness::Native { preset, .. }, .. }) = &self.coding else {
            return Err(Error::Refused("not a native harness"));
        };
        let model = self.run_model(binding);
        let preset = model.as_deref().unwrap_or(preset);
        preset.split(':').next().unwrap_or("").parse().map_err(|_| Error::Refused("unknown native agent"))
    }
    /// Copy the runner's pinned Codex hooks into a Codex run's worktree, where Codex reads them (ov-1088): the engine
    /// supplies them, as it hands Claude `--settings`, so no repository commits a file naming this machine's paths.
    /// Written only when the worktree's copy differs, and never through a link or over a file the repository tracks,
    /// which is somebody's own. Untracked, it is left out of the candidate's changes like the app's (`tree_changes`).
    /// Nothing for any other agent, or for a configuration that pins the repository's own file instead.
    pub(super) async fn install_codex_hooks(&self, binding: &Binding, tree: &Path) -> Result<()> {
        let Some(CodingConfig { harness: CodingHarness::Native { codex_hooks: Some(hooks), .. }, .. }) = &self.coding else { return Ok(()) };
        if self.native_agent(binding)? != Agent::Codex { return Ok(()); }
        hooks.verify()?;
        let bytes = self.bytes(&hooks.path)?;
        let text = String::from_utf8(bytes).map_err(|_| Error::Refused("invalid hook text"))?;
        let dest = tree.join(crate::hook_install::CODEX_HOOKS);
        let dir = dest.parent().ok_or(Error::Refused("foreign codex hooks path"))?;
        if std::fs::symlink_metadata(dir).is_ok_and(|m| !m.is_dir()) || std::fs::symlink_metadata(&dest).is_ok_and(|m| !m.is_file()) {
            return Err(Error::Refused("the worktree's .codex is not a plain directory and file"));
        }
        if std::fs::read(&dest).ok().as_deref() == Some(text.as_bytes()) { return Ok(()); }
        if !self.git_recorded(binding, tree, &["ls-files", "--", crate::hook_install::CODEX_HOOKS]).await?.is_empty() {
            return Err(Error::Refused("the repository commits its own codex hooks"));
        }
        crate::skill_install::write_atomically(&dest, &text)?;
        Ok(())
    }
    /// Codex may skip a project hook layer in a fresh linked worktree. Supply the pinned hooks
    /// through its per-invocation config too, without changing the owner's Codex home (ov-1205).
    pub(super) fn codex_hook_override(&self, binding: &Binding) -> Result<Option<String>> {
        if self.native_agent(binding)? != Agent::Codex { return Ok(None); }
        // Reconstruct only the engine-owned registrations that native_hooks validates. Recovery
        // reconstructs argv even after launch files disappear, so it must never read those files.
        let value = crate::hook_install::native_codex_hooks(&self.root.join("h.sock"));
        let hooks = toml::Value::try_from(&value["hooks"]).map_err(|_| Error::Refused("native Codex hooks cannot be passed as config"))?;
        Ok(Some(format!("hooks={hooks}")))
    }
    /// Pin the already-installed endpoint as well as its bytes. No hook writes.
    pub(super) fn validate_native_host(&self) -> Result<()> {
        if let Some(config) = &self.coding { self.validate_native_configuration(config)?; }
        Ok(())
    }
    pub(super) fn validate_native_configuration(&self, config: &CodingConfig) -> Result<()> {
        if matches!(config.harness, CodingHarness::Native { .. }) {
            if self.root.join("n-000000000000.sock").as_os_str().len() > 103 {
                return Err(Error::Refused("native runtime needs a short private socket path"));
            }
            self.native_config_hooks(config)?;
        }
        Ok(())
    }
    /// The hooks of the agent this build runs, which its turn is read from.
    fn native_hooks(&self, binding: &Binding) -> Result<()> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing native config"))?;
        self.native_agent_hooks(config, self.native_agent(binding)?)
    }
    /// Every hooks file the configuration pins targets this host, and the configured preset's is one of them:
    /// a run may pick any harness whose hooks are pinned (ov-966).
    fn native_config_hooks(&self, config: &CodingConfig) -> Result<()> {
        let CodingHarness::Native { preset, settings, project_hooks, codex_hooks, .. } = &config.harness else { return Ok(()) };
        let agent: Agent = preset.split(':').next().unwrap_or("").parse().map_err(|_| Error::Refused("unknown native agent"))?;
        self.native_agent_hooks(config, agent)?;
        if settings.is_some() { self.native_agent_hooks(config, Agent::Claude)?; }
        if codex_hooks.is_some() { self.native_agent_hooks(config, Agent::Codex)?; }
        if let Some(hooks) = project_hooks {
            let other = [Agent::Codex, Agent::Cursor].into_iter()
                .find(|a| super::coding::project_hooks_path(a.as_str()).is_some_and(|r| hooks.path == config.repository.join(r)))
                .ok_or(Error::Refused("foreign project hook path"))?;
            self.native_agent_hooks(config, other)?;
        }
        Ok(())
    }
    fn native_agent_hooks(&self, config: &CodingConfig, agent: Agent) -> Result<()> {
        let CodingHarness::Native { settings, .. } = &config.harness else { return Ok(()) };
        let file = if agent == Agent::Claude { settings.as_ref() } else { super::coding::worktree_hooks(config, agent.as_str()) }
            .ok_or(Error::Refused("missing native hooks"))?;
        file.verify()?;
        let text = String::from_utf8(self.bytes(&file.path)?).map_err(|_| Error::Refused("invalid hook text"))?;
        let socket = self.root.join("h.sock");
        if !crate::hook_install::native_completion_targets(&text, agent.as_str(), &socket) {
            if agent == Agent::Claude {
                return Err(Error::Refused("native Claude hooks need prompt, stop, and synchronous gating PreToolUse for this private host"));
            }
            return Err(Error::Refused("native hooks must target this private host and include required active steering hooks"));
        }
        Ok(())
    }
    /// An agent review whose verdict for this very step is recorded (ov-1082). The verdict is the review: what the
    /// reviewer's turn does after it decides nothing (`reviewing`).
    pub(super) fn recorded_verdict(&self, binding: &Binding) -> bool {
        matches!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent)))
            && self.bytes(&self.provider_path(binding)).ok()
                .and_then(|raw| serde_json::from_slice::<CodingReviewReceipt>(&raw).ok())
                .is_some_and(|r| r.binding == digest(binding) && r.identity == AGENT_REVIEWER
                    && binding.effect.candidate.as_ref() == Some(&r.candidate))
    }
    async fn native_turn(&self, listener: &tokio::net::UnixListener, binding: &Binding,
        heard_from: &std::sync::atomic::AtomicBool, clock: &super::native_control::ActiveClock) -> Result<(bool, Vec<HookLine>, Option<String>)> {
        let agent = self.native_agent(binding)?;
        let tree = self.tree(binding);
        let prompt = self.native_prompt(binding)?;
        // A prompt hook echoes the entire pinned brief. JSON can escape each byte as six bytes
        // (\uXXXX); keep the ordinary 64 KiB allowance for the envelope and other payload fields.
        // Bound the read by what we launched, rather than dropping long tickets or reading without a limit.
        let hook_limit = 65_536u64.saturating_add((prompt.len() as u64).saturating_mul(6));
        let mut assembler = MessageAssembler::new();
        let mut session = (agent == Agent::Claude).then(|| terminal(binding).to_string());
        let mut prompted = false;
        let mut startup_check = tokio::time::Instant::now();
        let mut hooks = Vec::new();
        let mut continued_stop: Option<(serde_json::Value, Vec<u8>)> = None;
        // A reviewer's recorded verdict ends its turn once it has gone quiet, if its Stop never arrives (ov-1082).
        let mut look = tokio::time::interval(Duration::from_millis(250));
        let hold = self.native_hold_socket(binding);
        let mut heard = clock.now();
        let mut verdict = None;
        let building = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
        let mut unfinished: Option<Duration> = None;
        let mut claude_stop = None;
        let mut unread = PaneUnread::default();
        let mut exited_cleanly = false;
        loop {
            let stream = tokio::select! {
                biased;
                accepted = listener.accept() => accepted?.0,
                _ = std::future::ready(()), if exited_cleanly => return Ok((true, hooks, None)),
                _ = look.tick() => {
                    if let Some(reason) = self.quarantine_reason(binding) { return Ok((false, hooks, Some(reason))); }
                    if building && let Some(native_hold::HoldStatus::Exited { code: 0, natural: true, .. }) = native_hold::status(&hold).await {
                        // A final hook may have connected while the hold status
                        // was in flight. Read queued hooks before taking the exit.
                        exited_cleanly = true;
                        continue;
                    }
                    let owner: NativeOwner = serde_json::from_slice(&self.bytes(&self.native_path(binding, "owner"))?)?;
                    let tmux = server(&self.root, binding);
                    let life = tmux.pane_life_with_exit_evidence(&owner.pane, owner.terminal).await;
                    #[cfg(test)]
                    let life = if UNREADABLE_PANE.with(|n| n.get()) > 0 {
                        UNREADABLE_PANE.with(|n| n.set(n.get() - 1));
                        Err(farcooler_core::DomainError::TmuxTimedOut)
                    } else { life };
                    match pane_look(life, &mut unread, clock.now()) {
                        PaneLook::Live => {}
                        PaneLook::Unread => {
                            if let Some(stop) = &mut claude_stop { *stop = ClaudeStop::default(); }
                            continue;
                        }
                        // The pane closed after a valid Stop, so Claude cannot start another turn.
                        PaneLook::Lost(_) if claude_stop.is_some() && unread.closed >= 2 => return Ok((true, hooks, None)),
                        PaneLook::Lost(why) => {
                            let _ = super::execution::ACTIVITY.try_with(|a| {
                                a.session.store(false, std::sync::atomic::Ordering::SeqCst);
                                a.lost.store(true, std::sync::atomic::Ordering::SeqCst);
                            });
                            return Err(Error::Refused(why));
                        }
                    }
                    if agent == Agent::Codex && startup_check.elapsed() >= Duration::from_secs(1) {
                        startup_check = tokio::time::Instant::now();
                        let output = tmux.capture_pane(&owner.pane, 40).await
                            .map_err(|_| Error::Refused("native startup output unavailable"))?;
                        if let Some(reason) = startup_refusal(agent, &output) { return Ok((false, hooks, Some(reason))); }
                    }
                    if verdict.is_none() && self.recorded_verdict(binding) { verdict = Some(clock.now()); }
                    if let Some(stop) = &mut claude_stop {
                        if verdict.is_some() { return Ok((true, hooks, None)); }
                        let now = clock.now();
                        if !stop.idle(&tmux, &owner.pane, now, heard).await { continue; }
                        if unfinished.is_none() && building && self.unfinished_build(binding, &tree).await.ok().flatten().is_some() {
                            tracing::info!(effect = %binding.effect.id, "the build ended its turn without committing; waiting for its next turn");
                            unfinished = Some(now);
                        }
                        let closing = unfinished.is_none_or(|at| now.saturating_sub(at.max(heard)) >= UNFINISHED_QUIET);
                        match self.idle_claude_direction(binding, &tmux, &owner.pane, &owner.tty, closing).await? {
                            IdleDirection::Empty if closing => return Ok((true, hooks, None)),
                            IdleDirection::Submitted => {
                                claude_stop = None;
                                unfinished = None;
                            }
                            IdleDirection::Empty | IdleDirection::Waiting => {}
                        }
                        continue;
                    }
                    if verdict.is_some_and(|at: Duration| clock.now().saturating_sub(at.max(heard)) >= VERDICT_QUIET) {
                        tracing::warn!(effect = %binding.effect.id, "the reviewer recorded its verdict and went quiet without its turn's end being heard; the verdict ends it");
                        return Ok((true, hooks, None));
                    }
                    if unfinished.is_some_and(|at| clock.now().saturating_sub(at.max(heard)) >= UNFINISHED_QUIET) {
                        tracing::warn!(effect = %binding.effect.id, "the build ended its turn without committing and was quiet after; its turn is over");
                        return Ok((true, hooks, None));
                    }
                    continue;
                }
            };
            if peer_uid(&stream) != Some(unsafe { libc::getuid() }) { continue; }
            let mut stream = tokio::io::BufReader::new(stream);
            let mut raw = Vec::new();
            // Bounded by the overall turn deadline as well as this connection.
            use tokio::io::AsyncReadExt;
            let n = tokio::time::timeout(Duration::from_secs(1), (&mut stream).take(hook_limit + 1).read_until(b'\n', &mut raw)).await;
            if !matches!(n, Ok(Ok(_))) || raw.len() as u64 > hook_limit || raw.last() != Some(&b'\n') { continue; }
            let Ok(line) = serde_json::from_slice::<HookLine>(&raw) else { continue };
            // Never grant permissions or tool-fence bypass from this host. A hook that isn't gating has hung up
            // before this answer (crates/cli/src/hook.rs, `first_contact`), so a failed write is its news, not ours:
            // failing the turn on it ended every real build at its first hook, with none recorded (ov-1075).
            let fact = facts(line.agent, &line.payload);
            // An agent's hooks name the directory it is in now, which is wherever it last moved to: ov-1064's reviewer
            // ended its turn in `apps/android`, and a hook that had to name the worktree itself was never heard (ov-1082).
            if line.agent != agent || !fact.cwd.as_deref().is_some_and(|cwd| cwd.starts_with(&tree)) {
                let _ = stream.get_mut().write_all(b"{}\n").await;
                continue;
            }
            let Some(sid) = &fact.session_id else { continue };
            if session.is_none() && matches!(line.event.as_str(), "SessionStart" | "sessionStart") {
                session = Some(sid.clone());
            }
            if session.as_ref() != Some(sid) {
                let _ = stream.get_mut().write_all(b"{}\n").await;
                continue;
            }
            heard = clock.now();
            heard_from.store(true, std::sync::atomic::Ordering::Relaxed);
            if matches!(line.event.as_str(), "UserPromptSubmit" | "beforeSubmitPrompt") {
                // What the agent was launched with is the briefed prompt, so that is what its hook reports.
                if line.payload.get("prompt").and_then(|p| p.as_str()) == Some(prompt.as_str()) { prompted = true; }
            }
            // Codex loads hooks from every active config source. A project copy and the engine
            // override may therefore report the same Stop concurrently. Replay its continuation;
            // a duplicate empty queue check must not settle the step before that continuation runs.
            let keyed_stop = agent == Agent::Codex && line.event == "Stop" && line.payload["turn_id"].is_string();
            if keyed_stop && let Some((payload, reply)) = &continued_stop && payload == &line.payload {
                send_native_reply(binding, &line, stream.get_mut(), reply).await?;
                continue;
            }
            let events = assembler.accept(line.agent, &line.event, &line.payload);
            let ended = prompted && events.iter().any(|event| matches!(event,
                AgentEvent::TurnEnded { reason: EndReason::EndTurn }));
            // Claude can issue Stop before drawing its idle prompt beside running shells.
            // Reply to the hook first so it can redraw and wake for their completion.
            let check_claude = ended && agent == Agent::Claude && !self.recorded_verdict(binding);
            // Keep accepting direction while a deferred Stop or an unfinished build can resume.
            let waiting = check_claude || (ended && building && self.unfinished_build(binding, &tree).await.ok().flatten().is_some());
            let followup = if prompted && (ended || line.event == "PreToolUse") {
                self.native_followup(binding, &line, stream.get_mut(), ended && !waiting).await?
            } else { Followup::NoMessage };
            let continued = matches!(followup, Followup::Sent(_));
            if matches!(followup, Followup::NoMessage)
                && !crate::watch::answer_wake::engine::context_reply(terminal(binding), &line, stream.get_mut()).await {
                let _ = stream.get_mut().write_all(b"{}\n").await;
            }
            if keyed_stop && let Followup::Sent(reply) = followup { continued_stop = Some((line.payload.clone(), reply)); }
            hooks.push(line);
            if hooks.len() > 4096 { return Err(Error::Refused("too many native hooks")); }
            if continued || check_claude {
                claude_stop = (!continued).then(ClaudeStop::default);
                unfinished = None;
                continue;
            }
            if prompted {
                for event in events {
                    if let AgentEvent::TurnEnded { reason } = event {
                        // A build that ends its turn with nothing committed may be waiting on something it backgrounded:
                        // it stays, and its next turn's end is read the same way (ov-1090). A tree that can't be read
                        // ends the turn as before.
                        if waiting && reason == EndReason::EndTurn {
                            tracing::info!(effect = %binding.effect.id, "the build ended its turn without committing; waiting for its next turn");
                            unfinished = Some(clock.now());
                            continue;
                        }
                        return Ok((reason == EndReason::EndTurn, hooks, None));
                    }
                }
            }
        }
    }
    pub(super) async fn run_native(&self, binding: &Binding, command: &CodingCommand,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<crate::effect_runner::ProcessExit> {
        use super::native_control::Turn;
        self.native_hooks(binding)?;
        let socket = self.native_socket(binding);
        // No unlink-before-bind: a surviving owner is evidence, never a restart.
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let host = Host(server(&self.root, binding));
        let tmux = &host.0;
        let id = terminal(binding);
        // A hold socket already here is a surviving owner's, evidence like the hook socket above.
        if self.native_hold_socket(binding).try_exists()? {
            drop(listener);
            std::fs::remove_file(socket)?;
            return Err(Error::Refused("a native hold for this step is still listening"));
        }
        let mut diagnostic = String::new();
        let opened = match self.held(binding, &native_launch(command, &self.quarantine_receipt(binding))) {
            Err(error) => Err(error.to_string()),
            Ok(gated) => match tmux.create_terminal_window_diagnostic(id, id, "workflow coding",
                &self.tree(binding).to_string_lossy(), &gated, &mut diagnostic).await {
                // Nothing is let out of the barrier, and no owner is written, until the hold answers (ov-1330).
                Ok(window) if self.hold_ready(binding).await => Ok(window),
                Ok(_) => Err("the agent's hold did not start".to_owned()),
                Err(error) => Err(if diagnostic.is_empty() { error.to_string() } else { diagnostic }),
            },
        };
        let window = match opened {
            Ok(window) => window,
            Err(detail) => {
                // The launch barrier has never been released; reaping this effect's private server
                // cannot interrupt an agent (a hold that started ends its waiting group on the hangup).
                // Retain the refusal even when no output stream existed.
                drop(host);
                let why = format!("native managed PTY could not open: {}", detail.chars().filter(|c| !c.is_control()).take(1000).collect::<String>());
                self.refuse_unstarted(binding, why.clone())?;
                std::fs::write(self.process_log(binding), why)?;
                drop(listener);
                std::fs::remove_file(socket)?;
                return Ok(crate::effect_runner::ProcessExit { code: None, cancelled: false });
            }
        };
        let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native PTY inventory unavailable"))?;
        let pane = panes.iter().find(|p| p.terminal_id == id && p.pane_id == window.pane_id)
            .ok_or(Error::Refused("native PTY tags missing"))?;
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), hold: true, supervisor: None };
        durable(&self.native_path(binding, "owner"), &serde_json::to_vec(&owner)?)?;
        // Subscribe before releasing the launch barrier. Share the watcher's fanout, so watching
        // cannot replace our pipe and screen clears cannot erase the process's earlier output.
        let runtime = crate::runtime::Runtime { inventory: farcooler_tmux::LiveInventory::new(tmux.clone()),
            tmux: tmux.clone(), marks: None };
        let stream = runtime.attach_to_fanout(&owner.pane).await
            .map_err(|_| Error::Refused("native output stream unavailable"))?;
        let mut output = tokio::spawn(crate::effect_runner::capture(stream, self.process_log(binding)));
        // A native builder pauses while a landing gate runs; a review never does (ov-1214).
        let _builder = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build))
            .then(|| self.yielding.builder(crate::yielding::Target::Hold(self.native_hold_socket(binding))));
        // Every ticket this run owns can watch and message it (ov-1171). Declared after `host`, so
        // all tickets are unlisted before the server is reaped, on success, failure or cancellation.
        let _watchable = crate::watch_view::publish_effect(&binding.workspace, &binding.effect, tmux.clone(), pane.clone());
        // Before the barrier, so a tmux older than 3.7 still learns the agent's first bracketed-paste request (ov-1263).
        crate::watch::answer_wake::engine::follow_bracketing(id, runtime.clone(), owner.pane.clone()).await;
        let startup = crate::engine_stuck::Startup::new(&binding.workspace, &binding.effect.item, self.native_agent(binding)?.as_str());
        tmux.send_keys(&window.pane_id, "\n").await.map_err(|_| Error::Refused("native launch delivery uncertain"))?;
        // A build stopped on a question, or never heard from, is told in Needs You (ov-1077).
        let heard_from = std::sync::atomic::AtomicBool::new(false);
        let stuck = crate::engine_stuck::watch(startup, tmux, &window.pane_id, &heard_from, _builder.as_ref());
        let (send, orders) = mpsc::channel(8);
        let review = !matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
        let _steerable = crate::engine_steering::publish(&binding.workspace, &binding.effect.item, id,
            self.native_agent(binding)?.as_str(), self.run_model(binding), review, self.steering_tickets(binding)?, send);
        let _inbox = super::native_messages::Inbox { store:&self.store, id };
        // Its quiet periods, like its time limit, leave out time a landing gate held it stopped (ov-1301).
        let clock = super::native_control::ActiveClock::held_by(_builder.as_ref());
        let turn = self.native_turn(&listener, binding, &heard_from, &clock);
        let cancel_signal = cancel.clone();
        let mut stop_reply = None;
        let mut frozen = None;
        let hold = self.native_hold_socket(binding);
        let controlled = super::native_control::controlled(turn, tmux, &window.pane_id, &hold, id,
            &clock, Duration::from_millis(command.timeout_ms), orders, cancel, &mut frozen, _builder.as_ref(), &mut stop_reply);
        let _ = super::execution::ACTIVITY.try_with(|a| a.session.store(true, std::sync::atomic::Ordering::SeqCst));
        let turn = tokio::select! {
            never = stuck => match never {},
            result = controlled => result,
        };
        // When Stop was deferred, admission stayed open for the resumed turn's hooks.
        crate::engine_steering::inbox(id, |open| { *open = false; Ok(()) }).map_err(Error::Domain)?;
        let _ = super::execution::ACTIVITY.try_with(|a| {
            a.settle();
            a.session.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let cancelled = matches!(turn, Ok(Turn::Cancelled));
        let (completed, hooks, failure) = match turn {
            Ok(Turn::Completed((passed, hooks, failure))) => (Some(passed), hooks, failure),
            Ok(Turn::TimedOut) => (Some(false), Vec::new(), Some(out_of_time_reason(Duration::from_millis(command.timeout_ms)))),
            Ok(Turn::Cancelled | Turn::Stopped) => (None, Vec::new(), None),
            Err(error) => {
                tracing::warn!(effect = %binding.effect.id, %error, "the native turn could not be read");
                (Some(false), Vec::new(), Some(turn_ended_reason(&error)))
            }
        };
        let turn = NativeSettlement::Launched { owner: owner.clone(), hooks, completed, failure };
        self.save_native_turn(binding, &turn)?;
        self.quiesce_native_turn(&owner, &cancel_signal).await?;
        if let Some(stopped) = frozen.take() { stopped.quiescent(); }
        // Closing the pane ends its fanout. Drain it before hashing the log into the receipt.
        match tokio::time::timeout(Duration::from_secs(5), &mut output).await {
            Ok(result) => result.map_err(|_| Error::Refused("native output capture failed"))??,
            Err(_) => { output.abort(); return Err(Error::Refused("native output did not drain")); }
        }
        let completed = self.save_native_settlement(binding, turn)?;
        drop(listener);
        std::fs::remove_file(socket)?;
        if let Some(reply) = stop_reply { let _ = reply.send(Ok(())); }
        Ok(crate::effect_runner::ProcessExit { code: completed.map(|passed| if passed { 0 } else { 1 }), cancelled })
    }
    /// End an interrupted native owner after a Stop, recorded verdict, or recovery of a pending turn.
    /// Recovery never launches or sends input to the agent. `completed` is what
    /// the settlement says of the turn; a settlement already written (the daemon died after it) is kept, and what it
    /// says is returned.
    pub(super) async fn cancel_interrupted_native(&self, binding: &Binding, completed: Option<bool>) -> Result<Option<bool>> {
        let settled = self.native_path(binding, "settled");
        let completed = if settled.try_exists()? {
            let settlement: NativeSettlement = serde_json::from_slice(&self.bytes(&settled)?)?;
            settlement.completion(binding)?
        } else if self.native_review_without_owner(binding)? {
            // Launch waits for a durable owner before releasing the PTY's stdin (ov-1130). No owner and no
            // panes proves this review never ran. Read every pane, including an untagged, unfinished open.
            if !self.native_server_empty(binding).await? {
                return Err(Error::Refused("native review without owner still has an unproved PTY"));
            }
            // Verification may have refused before run_coding made the log. Keep any output already saved.
            if !self.process_log(binding).try_exists()? { durable(&self.process_log(binding), b"")?; }
            durable(&settled, &serde_json::to_vec(&NativeSettlement::NeverLaunched { binding: binding.clone() })?)?;
            None
        } else {
            let owner: NativeOwner = serde_json::from_slice(&self.bytes(&self.native_path(binding, "owner"))?)?;
            if owner.binding != *binding || owner.terminal != terminal(binding) || owner.tty.is_empty() {
                return Err(Error::Refused("foreign interrupted native owner"));
            }
            let tmux = server(&self.root, binding);
            let life = tmux.pane_life(&owner.pane, owner.terminal).await
                .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
            if life == farcooler_tmux::PaneLife::Gone {
                self.quiesce_native(&owner).await?;
                // The pane cannot be captured or closed again. Its saved output and verdict survive the install. A
                // log that is gone (a daemon that died before its first write) is no doubt about the process: nothing
                // is on its tty, so an empty log stands for it, and a cancel ends instead of staying Uncertain (ov-1301).
                match self.bytes(&self.process_log(binding)) {
                    Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => durable(&self.process_log(binding), b"")?,
                    other => { other?; }
                }
            } else {
                self.capture_native_output(binding, &tmux, &owner.pane).await?;
                self.quiesce_native(&owner).await?;
            }
            // A refusal written before the runner stopped is a failed launch, so keep it across recovery.
            let failure = if completed.is_none() { self.quarantine_reason(binding) } else { None };
            let completed = if failure.is_some() { Some(false) } else { completed };
            let turn = self.saved_native_turn(binding)?.unwrap_or(NativeSettlement::Launched {
                owner, hooks: Vec::new(), completed, failure,
            });
            self.save_native_settlement(binding, turn)?
        };
        self.remove_native_hook_socket(binding)?;
        Ok(completed)
    }
    async fn capture_native_output(&self, binding: &Binding, tmux: &TmuxServer, pane: &str) -> Result<()> {
        // Recovery has no surviving subscription. Keep the stream already published before the
        // crash, followed by whatever the retained pane can still tell us about its last moments.
        // Scrollback alone excludes the visible screen, where the final error usually is.
        // Request enough lines to cover the byte cap, then keep the same bounded tail as commands.
        let output = tmux.capture_pane(pane, crate::effect_runner::LOG_LIMIT as u32).await
            .map_err(|_| Error::Refused("native PTY output unavailable"))?;
        let log = self.process_log(binding);
        let mut bytes = match self.bytes(&log) {
            Ok(bytes) => bytes,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        bytes.extend_from_slice(output.as_bytes());
        crate::effect_runner::save_tail(&log, &bytes)?;
        Ok(())
    }
    /// Read prose only from the retained, matched native settlement, never as a success verdict.
    pub(super) fn native_last_message(&self, binding: &Binding) -> Result<Option<String>> {
        let raw = match self.bytes(&self.native_path(binding, "settled")) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let settlement: NativeSettlement = serde_json::from_slice(&raw)?;
        settlement.completion(binding)?;
        let mut last = None;
        let mut assembler = MessageAssembler::new();
        if let NativeSettlement::Launched { hooks, .. } = settlement {
            // Cursor's stop hook carries no prose; it names the transcript the prose is in (ov-1235).
            let cursor_transcript = hooks.iter().rev().filter(|h| h.agent == Agent::Cursor)
                .find_map(|h| facts(h.agent, &h.payload).transcript_path);
            if let Some(path) = cursor_transcript { last = cursor_last_message(&path); }
            for hook in hooks {
                for event in assembler.accept(hook.agent, &hook.event, &hook.payload) {
                    if let AgentEvent::Message { role: farcooler_agent_core::event::Role::Agent, text, parent: None } = event {
                        if !text.is_empty() { last = Some(text); }
                    }
                }
            }
        }
        Ok(last)
    }
    /// What a no-commit build's report says of its last message: the message, or for an agent whose message cannot
    /// be read, the words that it is unavailable (ov-1235).
    pub(super) fn last_message_note(&self, binding: &Binding) -> Result<Option<String>> {
        Ok(match self.native_last_message(binding)? {
            Some(message) => Some(format!("The agent's last message:\n{message}")),
            None if self.native_agent(binding)? == Agent::Cursor => Some("The agent's last message: unavailable for this agent.".to_string()),
            None => None,
        })
    }
    pub(super) fn verify_native_settlement(&self, binding: &Binding, exit: Option<i32>) -> Result<Option<String>> {
        let settlement: NativeSettlement = serde_json::from_slice(&self.bytes(&self.native_path(binding, "settled"))?)?;
        if settlement.completion(binding)?.map(|p| if p { 0 } else { 1 }) != exit {
            return Err(Error::Refused("foreign native settlement"));
        }
        let failure = match settlement { NativeSettlement::Launched { failure, .. } => failure, NativeSettlement::NeverLaunched { .. } => None };
        if failure.is_some() && exit != Some(1) {
            return Err(Error::Refused("native startup failure has no failed settlement"));
        }
        Ok(failure)
    }
    /// A native review or build can recover a missing owner as never launched (ov-1130; builds since ov-1301): launch
    /// holds the agent at a read until its owner is durable. A present but unreadable or malformed owner is still
    /// uncertain; absence alone is not enough until the private server is checked.
    pub(super) fn native_review_without_owner(&self, binding: &Binding) -> Result<bool> {
        Ok(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent) | StageKind::Build))
            && !self.native_path(binding, "owner").try_exists()?)
    }
    pub(super) fn native_session(binding: &Binding) -> String { terminal(binding).to_string() }
    /// Whether this step's private tmux server holds no pane at all, or isn't running.
    async fn native_server_empty(&self, binding: &Binding) -> Result<bool> {
        let out = server(&self.root, binding).run(&["list-panes", "-a", "-F", "#{pane_id}"]).await
            .map_err(|_| Error::Refused("native unstarted inventory unavailable"))?;
        let absent = out.stderr.contains("no server running") || out.stderr.contains("no current session")
            || (out.stderr.contains("error connecting") && (out.stderr.contains("No such file or directory")
                || out.stderr.contains("Connection refused")));
        Ok(out.ok() && out.stdout.trim().is_empty() || !out.ok() && absent)
    }
    fn retained_native_owner(&self, binding: &Binding) -> Result<Option<NativeOwner>> {
        let owner: NativeOwner = match self.bytes(&self.native_path(binding, "owner")) {
            Ok(raw) => serde_json::from_slice(&raw)?,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if owner.binding != *binding || owner.terminal != terminal(binding) || owner.tty.is_empty() {
            return Err(Error::Refused("foreign interrupted native owner"));
        }
        Ok(Some(owner))
    }

    /// For the orchestrator's resolve (ov-1301): `None` when nothing of this native agent runs (its pane is gone or
    /// dead with nothing on its terminal, or it was never let out and its server is empty), else what still runs.
    pub(super) async fn native_ended(&self, binding: &Binding) -> Result<Option<&'static str>> {
        let Some(owner) = self.retained_native_owner(binding)? else {
            return Ok((!self.native_server_empty(binding).await?).then_some("its agent's terminal is still open"));
        };
        let life = server(&self.root, binding).pane_life(&owner.pane, owner.terminal).await
            .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
        if life == farcooler_tmux::PaneLife::Live { return Ok(Some("its agent is still running")); }
        if owner.hold {
            return Ok(self.native_hold_socket(binding).try_exists()?.then_some("its agent's hold has not confirmed exit"));
        }
        let table = snapshot().await?;
        Ok((!owner.processes(&table)?.is_empty()).then_some("processes are still running on its agent's terminal"))
    }

}

/// The last thing Cursor said in its latest turn, from the tail of its transcript. A transcript that is missing,
/// too large to be one, or has no prose gives `None`; it never fails the step (ov-1235).
fn cursor_last_message(path: &Path) -> Option<String> {
    use farcooler_core::session_log::{TurnEvent, cursor::parse_line};
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 1 << 20;
    let mut file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() { return None; }
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut raw = Vec::new();
    file.take(TAIL).read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let mut last = None;
    for line in text.lines().filter(|l| l.starts_with('{')) {
        for event in parse_line(line) {
            match event {
                TurnEvent::Started { .. } => last = None,
                TurnEvent::Said { text, .. } if !text.is_empty() => last = Some(text),
                _ => {}
            }
        }
    }
    last
}

#[cfg(test)]
#[path = "coding_native_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "coding_native_unfinished_tests.rs"]
mod unfinished_tests;

#[cfg(test)]
#[path = "coding_native_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "coding_native_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "coding_native_quiesce_tests.rs"]
mod quiesce_order_tests;

#[cfg(test)]
#[path = "coding_native_unread_tests.rs"]
mod unread_tests;
