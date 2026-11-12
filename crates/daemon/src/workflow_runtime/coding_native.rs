//! Retained managed tmux PTY + normalized hook ownership, without Service activation.
use super::*;
use super::coding::FileValidation;
use farcooler_agent_core::event::{AgentEvent, EndReason};
use farcooler_agent_hooks::{Agent, assemble::MessageAssembler, facts::facts, wire::HookLine};
use farcooler_tmux::TmuxServer;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[cfg(test)]
thread_local! { pub(super) static TEST_ARGV: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) }; }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeOwner {
    binding: Binding,
    terminal: Uuid,
    pane: String,
    tty: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeSettlement {
    owner: NativeOwner,
    hooks: Vec<HookLine>,
    completed: Option<bool>,
}

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

// Each effect has its own retained server, never the installed daemon's socket.
// A cancelled future tears down its server but cannot publish quiescence/success.
struct Host(TmuxServer);
impl Drop for Host {
    fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); }
}
fn terminal(binding: &Binding) -> Uuid {
    let pin = digest(&("native-terminal/v1", binding));
    Uuid::parse_str(&pin[4..36]).expect("digest UUID")
}
fn server(root: &Path, binding: &Binding) -> TmuxServer {
    let id = terminal(binding);
    TmuxServer::in_dir(&format!("workflow-{id}"), id, root)
}
fn snapshot() -> Result<crate::foreground::Foreground> {
    let raw = crate::proc_table::snapshot().ok_or(Error::Refused("native process table unavailable"))?;
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

/// No such process, or one that has exited and is only waiting to be reaped.
fn gone(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 { return true; }
    // `/proc/<pid>/stat` is `pid (comm) S ...`; comm may hold parentheses, so read after the last one.
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
        .and_then(|stat| stat.rsplit_once(')').and_then(|(_, rest)| rest.trim_start().chars().next()))
        .is_some_and(|state| state == 'Z' || state == 'X')
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
    fn native_path(&self, binding: &Binding, suffix: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.native-{suffix}.json", digest(&binding.effect)))
    }
    /// The agent a build runs: the model it picked at Start, or the configured preset (ov-966).
    pub(super) fn native_agent(&self, binding: &Binding) -> Result<Agent> {
        let Some(CodingConfig { harness: CodingHarness::Native { preset, .. }, .. }) = &self.coding else {
            return Err(Error::Refused("not a native harness"));
        };
        let preset = binding.effect.model.as_deref().unwrap_or(preset);
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
            return Err(Error::Refused("native hooks must already target this private host"));
        }
        Ok(())
    }
    async fn quiesce_native(&self, owner: &NativeOwner) -> Result<()> {
        let tmux = server(&self.root, &owner.binding);
        if owner.terminal != terminal(&owner.binding) {
            return Err(Error::Refused("foreign native terminal"));
        }
        let life = tmux.pane_life(&owner.pane, owner.terminal).await
            .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
        if life != farcooler_tmux::PaneLife::Live {
            return Err(Error::Refused("native owner exited without observed process quiescence"));
        }
        let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native owner inventory unavailable"))?;
        if !panes.iter().any(|p| p.pane_id == owner.pane && p.terminal_id == owner.terminal
            && p.daemon_id == owner.terminal && p.worktree_id == owner.terminal && p.tty == owner.tty) {
            return Err(Error::Refused("native tty differs from retained owner"));
        }
        // Stop the tty roots and all descendants before terminating them so an
        // interactive shell's separate job groups cannot escape a leader kill.
        let mut owned = BTreeSet::new();
        let mut stable = false;
        for _ in 0..8 {
            let table = tokio::task::spawn_blocking(snapshot).await
                .map_err(|_| Error::Refused("native process read failed"))??;
            let current = table.under_tty(owner.tty.trim_start_matches("/dev/"));
            if current.is_empty() && owned.is_empty() { return Err(Error::Refused("native live tty has no process facts")); }
            let old = owned.len();
            for pid in current {
                if pid <= 1 || pid == std::process::id() as i32 { return Err(Error::Refused("invalid owned native pid")); }
                owned.insert(pid);
                // Only current live descendants of this exact tagged PTY.
                unsafe { libc::kill(pid, libc::SIGSTOP); }
            }
            if old == owned.len() { stable = true; break; }
        }
        if !stable { return Err(Error::Refused("native process tree did not settle")); }
        for &pid in owned.iter().rev() { unsafe { libc::kill(pid, libc::SIGKILL); } }
        // tmux is the wait/reap owner of the PTY child, not the workflow daemon.
        // A killed process stays findable by `kill(pid, 0)` until its parent reaps it, and a
        // pane's process is tmux's child: tmux 3.4 can miss the exit and leave it a zombie.
        // A zombie is dead, and the nudge makes tmux reap it.
        for round in 0..100 {
            if owned.iter().all(|&pid| gone(pid)) {
                if !tmux.kill_terminal_window(owner.terminal).await
                    .map_err(|_| Error::Refused("native terminal close failed"))? {
                    return Err(Error::Refused("native terminal close unproved"));
                }
                return Ok(());
            }
            if round % 10 == 9 { tmux.nudge_reap().await; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(Error::Refused("native descendants not quiescent"))
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
        heard_from: &std::sync::atomic::AtomicBool) -> Result<(bool, Vec<HookLine>)> {
        let agent = self.native_agent(binding)?;
        let tree = self.tree(binding);
        let mut assembler = MessageAssembler::new();
        let mut session = (agent == Agent::Claude).then(|| terminal(binding).to_string());
        let mut prompted = false;
        let mut hooks = Vec::new();
        // A reviewer's recorded verdict ends its turn once it has gone quiet, if its Stop never arrives (ov-1082).
        let mut look = tokio::time::interval(Duration::from_millis(250));
        let mut heard = tokio::time::Instant::now();
        let mut verdict = None;
        let building = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
        let mut unfinished: Option<tokio::time::Instant> = None;
        loop {
            let stream = tokio::select! {
                accepted = listener.accept() => accepted?.0,
                _ = look.tick() => {
                    if verdict.is_none() && self.recorded_verdict(binding) { verdict = Some(tokio::time::Instant::now()); }
                    if verdict.is_some_and(|at: tokio::time::Instant| at.max(heard).elapsed() >= VERDICT_QUIET) {
                        tracing::warn!(effect = %binding.effect.id, "the reviewer recorded its verdict and went quiet without its turn's end being heard; the verdict ends it");
                        return Ok((true, hooks));
                    }
                    if unfinished.is_some_and(|at| at.max(heard).elapsed() >= UNFINISHED_QUIET) {
                        tracing::warn!(effect = %binding.effect.id, "the build ended its turn without committing and was quiet after; its turn is over");
                        return Ok((true, hooks));
                    }
                    continue;
                }
            };
            if peer_uid(&stream) != Some(unsafe { libc::getuid() }) { continue; }
            let mut stream = tokio::io::BufReader::new(stream);
            let mut raw = Vec::new();
            // Bounded by the overall turn deadline as well as this connection.
            use tokio::io::AsyncReadExt;
            let n = tokio::time::timeout(Duration::from_secs(1), (&mut stream).take(65537).read_until(b'\n', &mut raw)).await;
            if !matches!(n, Ok(Ok(_))) || raw.len() > 65536 || raw.last() != Some(&b'\n') { continue; }
            let Ok(line) = serde_json::from_slice::<HookLine>(&raw) else { continue };
            // Never grant permissions or tool-fence bypass from this host. A hook that isn't gating has hung up
            // before this answer (crates/cli/src/hook.rs, `first_contact`), so a failed write is its news, not ours:
            // failing the turn on it ended every real build at its first hook, with none recorded (ov-1075).
            let _ = stream.get_mut().write_all(b"{}\n").await;
            let fact = facts(line.agent, &line.payload);
            // An agent's hooks name the directory it is in now, which is wherever it last moved to: ov-1064's reviewer
            // ended its turn in `apps/android`, and a hook that had to name the worktree itself was never heard (ov-1082).
            if line.agent != agent || !fact.cwd.as_deref().is_some_and(|cwd| cwd.starts_with(&tree)) { continue; }
            let Some(sid) = &fact.session_id else { continue };
            if session.is_none() && matches!(line.event.as_str(), "SessionStart" | "sessionStart") {
                session = Some(sid.clone());
            }
            if session.as_ref() != Some(sid) { continue; }
            heard = tokio::time::Instant::now();
            heard_from.store(true, std::sync::atomic::Ordering::Relaxed);
            if matches!(line.event.as_str(), "UserPromptSubmit" | "beforeSubmitPrompt") {
                // What the agent was launched with is the briefed prompt, so that is what its hook reports.
                if line.payload.get("prompt").and_then(|p| p.as_str()) == Some(self.native_prompt(binding)?.as_str()) { prompted = true; }
            }
            let events = assembler.accept(line.agent, &line.event, &line.payload);
            hooks.push(line);
            if hooks.len() > 4096 { return Err(Error::Refused("too many native hooks")); }
            if prompted {
                for event in events {
                    if let AgentEvent::TurnEnded { reason } = event {
                        // A build that ends its turn with nothing committed may be waiting on something it backgrounded:
                        // it stays, and its next turn's end is read the same way (ov-1090). A tree that can't be read
                        // ends the turn as before.
                        if building && reason == EndReason::EndTurn
                            && self.unfinished_build(binding, &tree).await.ok().flatten().is_some() {
                            tracing::info!(effect = %binding.effect.id, "the build ended its turn without committing; waiting for its next turn");
                            unfinished = Some(tokio::time::Instant::now());
                            continue;
                        }
                        return Ok((reason == EndReason::EndTurn, hooks));
                    }
                }
            }
        }
    }
    pub(super) async fn run_native(&self, binding: &Binding, command: &CodingCommand,
        mut cancel: tokio::sync::watch::Receiver<bool>) -> Result<Option<i32>> {
        self.native_hooks(binding)?;
        let socket = self.native_socket(binding);
        // No unlink-before-bind: a surviving owner is evidence, never a restart.
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let host = Host(server(&self.root, binding));
        let tmux = &host.0;
        let mut launch = String::from("exec env -i");
        for (key, value) in &command.environment {
            launch.push(' '); launch.push_str(&crate::service::shell_quote(&format!("{key}={value}")));
        }
        for arg in &command.argv { launch.push(' '); launch.push_str(&crate::service::shell_quote(arg)); }
        // Retained PTY stdin, not null: read gates launch until tags and owner are durable.
        let gated = format!("/bin/sh -c {}", crate::service::shell_quote(&format!("read -r workflow_start; {launch}")));
        let id = terminal(binding);
        let window = tmux.create_terminal_window(id, id, "workflow coding", &self.tree(binding).to_string_lossy(), &gated).await
            .map_err(|_| Error::Refused("native managed PTY open uncertain"))?;
        let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native PTY inventory unavailable"))?;
        let pane = panes.iter().find(|p| p.terminal_id == id && p.pane_id == window.pane_id)
            .ok_or(Error::Refused("native PTY tags missing"))?;
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone() };
        durable(&self.native_path(binding, "owner"), &serde_json::to_vec(&owner)?)?;
        // Listed for watching only (ov-972); declared after `host`, so it is unlisted before the server is reaped.
        let _watchable = crate::watch_view::publish(&binding.workspace, &binding.effect.item, tmux.clone(), pane.clone());
        tmux.send_keys(&window.pane_id, "\n").await.map_err(|_| Error::Refused("native launch delivery uncertain"))?;
        // A build stopped on a question, or never heard from, is told in Needs You (ov-1077).
        let heard_from = std::sync::atomic::AtomicBool::new(false);
        let stuck = crate::engine_stuck::watch(&binding.workspace, &binding.effect.item, self.native_agent(binding)?.as_str(),
            tmux, &window.pane_id, &heard_from);
        let result = tokio::select! {
            never = stuck => match never {},
            result = tokio::time::timeout(Duration::from_millis(command.timeout_ms), self.native_turn(&listener, binding, &heard_from)) => match result {
                Ok(Ok(turn)) => Some(turn),
                Ok(Err(error)) => { tracing::warn!(effect = %binding.effect.id, %error, "the native build's turn could not be read"); None }
                Err(_) => { tracing::warn!(effect = %binding.effect.id, "the native build's turn did not end in time"); None }
            },
            _ = async { while !*cancel.borrow_and_update() { if cancel.changed().await.is_err() { break; } } } => None,
        };
        // Capture actual PTY output before closing it; never parse it as success.
        let output = tmux.capture_scrollback(&window.pane_id).await.map_err(|_| Error::Refused("native PTY output unavailable"))?;
        std::fs::write(self.process_log(binding), output)?;
        self.quiesce_native(&owner).await?;
        let (mut completed, hooks) = result.map_or((None, Vec::new()), |(passed, hooks)| (Some(passed), hooks));
        // A recorded verdict is the review however the turn ended: cut short by a daemon stopping for an install, out
        // of time, or failing after it (ov-1082). Read after the reviewer is ended, so it is the last verdict given.
        if self.recorded_verdict(binding) { completed = Some(true); }
        durable(&self.native_path(binding, "settled"), &serde_json::to_vec(&NativeSettlement { owner, hooks, completed })?)?;
        drop(listener);
        std::fs::remove_file(socket)?;
        Ok(completed.map(|passed| if passed { 0 } else { 1 }))
    }
    /// Only an admitted Stop, or a reviewer's own recorded verdict (ov-1082), may terminate an interrupted native
    /// owner. Ordinary reconciliation never launches, sends input, or kills a discovered process. `completed` is what
    /// the settlement says of the turn; a settlement already written (the daemon died after it) is kept, and what it
    /// says is returned.
    pub(super) async fn cancel_interrupted_native(&self, binding: &Binding, completed: Option<bool>) -> Result<Option<bool>> {
        let owner: NativeOwner = serde_json::from_slice(&self.bytes(&self.native_path(binding, "owner"))?)?;
        if owner.binding != *binding { return Err(Error::Refused("foreign interrupted native owner")); }
        let settled = self.native_path(binding, "settled");
        let completed = if settled.try_exists()? {
            let settlement: NativeSettlement = serde_json::from_slice(&self.bytes(&settled)?)?;
            if settlement.owner.binding != *binding { return Err(Error::Refused("foreign native settlement")); }
            settlement.completed
        } else {
            let tmux = server(&self.root, binding);
            let output = tmux.capture_scrollback(&owner.pane).await.map_err(|_| Error::Refused("interrupted native output unavailable"))?;
            std::fs::write(self.process_log(binding), output)?;
            self.quiesce_native(&owner).await?;
            durable(&settled, &serde_json::to_vec(&NativeSettlement { owner, hooks: Vec::new(), completed })?)?;
            completed
        };
        // The socket is this build's alone, so a leftover one is stale by construction.
        let socket = self.native_socket(binding);
        if socket.try_exists()? { std::fs::remove_file(socket)?; }
        Ok(completed)
    }
    pub(super) fn verify_native_settlement(&self, binding: &Binding, exit: Option<i32>) -> Result<()> {
        let settlement: NativeSettlement = serde_json::from_slice(&self.bytes(&self.native_path(binding, "settled"))?)?;
        if settlement.owner.binding != *binding || settlement.owner.terminal != terminal(binding)
            || settlement.completed.map(|p| if p { 0 } else { 1 }) != exit {
            return Err(Error::Refused("foreign native settlement"));
        }
        Ok(())
    }
    pub(super) fn native_session(binding: &Binding) -> String { terminal(binding).to_string() }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    pub(crate) struct Fake;
    impl Drop for Fake { fn drop(&mut self) { TEST_ARGV.with(|p| *p.borrow_mut() = None); } }
    pub(crate) async fn native(mode: &str) -> (tempfile::TempDir, Runtime, Fake) { native_agent(mode, "claude").await }
    /// `agent` is `claude`, `codex` or `cursor`, each with its hooks pinned as before ov-1088 (Codex's and Cursor's
    /// committed in the repository), or `codex-engine`: Codex with the runner's own hooks file, outside the repository,
    /// which the engine copies into the worktree (ov-1088). That agent checks it finds the copy before it starts.
    async fn native_agent(mode: &str, agent: &str) -> (tempfile::TempDir, Runtime, Fake) {
        assert!(farcooler_core::programs::find("tmux").is_some(), "native ownership proof requires real tmux");
        let (dir, mut runtime) = configured(false, "pass").await;
        let socket = runtime.root.join("h.sock");
        let engine = agent == "codex-engine";
        let agent = if engine { "codex" } else { agent };
        let hooks = if agent == "claude" { dir.path().join("native-settings.json") }
            else if engine { dir.path().join("engine-codex-hooks.json") }
            else { runtime.repository().join(if agent == "codex" { ".codex/hooks.json" } else { ".cursor/hooks.json" }) };
        std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        let text = match agent {
            "claude" => serde_json::to_string(&crate::hook_install::claude_settings(&socket)).unwrap(),
            "codex" => crate::hook_install::merge_codex("{}", &socket),
            _ => crate::hook_install::merge_cursor("{}", &socket),
        };
        std::fs::write(&hooks, text).unwrap();
        if agent != "claude" && !engine {
            let repo = runtime.repository();
            adapters::git(&repo, &["add", "--", hooks.to_str().unwrap()]).await.unwrap();
            adapters::git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
                "commit", "--quiet", "-m", "existing project hooks"]).await.unwrap();
            let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
            adapters::git(&repo, &["update-ref", runtime.landing_ref(), &base]).await.unwrap();
            runtime.coding.as_mut().unwrap().base = base;
        }
        let script = dir.path().join("interactive.py");
        std::fs::write(&script, r#"import json, os, pathlib, socket, subprocess, sys, time
assert all(os.isatty(fd) for fd in (0,1,2)), 'native launch needs a real PTY'
assert os.environ['TERM'] == 'xterm-256color'
mode = sys.argv[1]
agent = sys.argv[2]
session = os.environ['FARCOOLER_NATIVE_SESSION']
# Where Codex reads its hooks: the engine's copy of the runner's file, when it supplies them (ov-1088).
if 'EXPECT_CODEX_HOOKS' in os.environ and pathlib.Path('.codex/hooks.json').read_bytes() != pathlib.Path(os.environ['EXPECT_CODEX_HOOKS']).read_bytes():
    sys.exit(7)
# As `farcooler hook` does (crates/cli/src/hook.rs, first_contact): a hook that isn't gating writes its
# line and hangs up at once, without reading the daemon's answer (ov-1075).
def hook(event, sid=session, cwd=os.getcwd(), **extra):
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
        s.sendall((json.dumps(dict(agent=agent, event=event, payload=dict(session_id=sid,cwd=cwd,workspace_roots=[cwd],**extra)))+'\n').encode())
hook('sessionStart' if agent == 'cursor' else 'SessionStart')
hook('beforeSubmitPrompt' if agent == 'cursor' else 'UserPromptSubmit', prompt=os.environ['TEST_LAUNCHED_PROMPT'])
pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
if mode == 'barrier':
    d = pathlib.Path(os.environ['STARTED']).parent / 'barrier'
    d.mkdir(exist_ok=True)
    (d / session).write_text(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
    t = time.time()
    while len(list(d.iterdir())) < 2:
        if time.time() - t > 4: sys.exit(9)
        time.sleep(0.05)
if mode in ('background', 'abandon'):
    # Ends its turn with its work uncommitted, as an agent that backgrounds a wait does (ov-1090). `background` is
    # woken when the wait is over and commits in its next turn; `abandon` never comes back.
    pathlib.Path('result.txt').write_text('started before the wait')
    hook('Stop')
    time.sleep(30 if mode == 'abandon' else 0.5)
if mode == 'cancel':
    subprocess.run(['/bin/sleep','30'])
else:
    pathlib.Path('result.txt').write_text('candidate native PTY')
    subprocess.check_call(['/usr/bin/git','add','--','result.txt'])
    subprocess.check_call(['/usr/bin/git','-c','commit.gpgsign=false','commit','--quiet','-m','native build'])
    stop = 'stop' if agent == 'cursor' else 'Stop'
    hook(stop, sid='foreign')
    hook(stop, cwd='/foreign')
    if agent == 'claude': hook(stop, agent_id='subagent')
    if mode != 'foreign-only': hook('StopFailure' if mode == 'failed' else stop, error='authentication_failed')
    time.sleep(30)
"#).unwrap();
        let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
        process.argv.clear(); process.timeout_ms = if mode == "foreign-only" { 300 } else { 5000 };
        if engine { process.environment.insert("EXPECT_CODEX_HOOKS".into(), hooks.to_string_lossy().into_owned()); }
        TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned(), mode.into(), agent.into()]));
        let root = runtime.root.clone();
        let config = runtime.coding.as_mut().unwrap();
        let file = LaunchFile { path: hooks.canonicalize().unwrap(), digest: digest(&std::fs::read(hooks).unwrap()) };
        config.harness = CodingHarness::Native { preset: agent.into(), settings: (agent == "claude").then(|| file.clone()),
            project_hooks: (agent != "claude" && !engine).then(|| file.clone()), codex_hooks: engine.then_some(file),
            plugin_dir: None, prompt: "implement the selected task".into(), process };
        // Persist private trusted config before enqueue, then exercise the public reopen path.
        std::fs::write(root.join("coding.json"), serde_json::to_vec(config).unwrap()).unwrap();
        drop(runtime);
        (dir, Runtime::open(&root).unwrap(), Fake)
    }
    #[tokio::test]
    async fn workflow_coding_native_real_pty_normalized_hooks_and_quiescence() {
        for agent in ["claude", "codex", "cursor", "codex-engine"] {
            let (dir, mut runtime, _fake) = native_agent("pass", agent).await;
            let definition = runtime.configured_definition().unwrap();
            enqueue(&mut runtime, "w", "one", definition);
            assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done, "{agent}");
            if agent == "codex-engine" {
                // Codex ran with the hooks, and they stayed the worktree's: what landed holds no hooks file.
                let landed = adapters::git(&runtime.repository(), &["ls-tree", "-r", "--name-only", runtime.landing_ref()]).await.unwrap();
                assert!(landed.contains("result.txt") && !landed.contains(".codex"), "{landed}");
            }
            let pid: i32 = std::fs::read_to_string(dir.path().join("started")).unwrap().parse().unwrap();
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "native TUI must be reaped before candidate success");
            let left: Vec<_> = std::fs::read_dir(&runtime.root).unwrap().map(|e| e.unwrap().file_name())
                .filter(|n| n.to_string_lossy().ends_with(".sock")).collect();
            assert!(left.is_empty(), "{agent}: {left:?}");
        }
    }
    #[tokio::test]
    async fn workflow_native_builds_overlap_each_on_its_own_hook_socket() {
        let (dir, mut runtime, _fake) = native("barrier").await;
        let definition = runtime.configured_definition().unwrap();
        super::super::concurrency_tests::enqueue_item(&mut runtime, "w", "one", definition.clone());
        super::super::concurrency_tests::enqueue_item(&mut runtime, "w", "two", definition);
        // Each build waits (up to four seconds) for the other to start, so only an overlap passes.
        for time in 2..5 { runtime.pump(time).await.unwrap(); }
        let state = runtime.state("w").unwrap().unwrap();
        for id in ["one", "two"] {
            assert!(state.items[id].stage >= 2 && state.items[id].phase == Phase::Running, "{id}: {state:?}");
        }
        let sockets: BTreeSet<String> = std::fs::read_dir(dir.path().join("barrier")).unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap()).collect();
        assert_eq!(sockets.len(), 2, "{sockets:?}");
        for socket in &sockets {
            assert!(socket.len() <= 103 && !socket.ends_with("/h.sock"), "{socket}");
            assert!(!Path::new(socket).exists(), "{socket} outlived its build");
        }
    }
    /// A build that backgrounds a wait ends its turn with its work uncommitted, and is woken when the wait is over
    /// (ov-1090): it is kept for that next turn, and what it commits then is the candidate.
    #[tokio::test]
    async fn workflow_native_build_that_ends_its_turn_uncommitted_is_kept_for_its_next_turn() {
        let (_dir, mut runtime, _fake) = native("background").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        assert_eq!((&state.items["one"].phase, state.items["one"].attempt), (&Phase::Done, 1), "{state:?}");
    }
    /// One that never comes back fails its step with what it left, which stays in its tree: never Uncertain, where
    /// no command reached ov-1076 and ov-1084.
    #[tokio::test]
    async fn workflow_native_build_that_never_commits_fails_with_its_work_kept_never_uncertain() {
        let (_dir, mut runtime, _fake) = native("abandon").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed, "{state:?}");
        assert_eq!(item.failure.as_deref(), Some("The build ended its turn with changes it hadn't committed: result.txt. \
            They're still in its worktree, and the build carries on from them."));
        assert_eq!(std::fs::read_to_string(runtime.item_tree("w", "one").join("result.txt")).unwrap(), "started before the wait");
    }
    #[tokio::test]
    async fn workflow_coding_native_foreign_subagent_hooks_and_failures_never_pass() {
        for mode in ["foreign-only", "failed"] {
            let (_dir, mut runtime, _fake) = native(mode).await;
            let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
            assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed, "{mode}");
        }
    }
    #[tokio::test]
    async fn workflow_coding_native_settlement_survives_crash_without_launching_again() {
        let (dir, mut runtime, _fake) = native("pass").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let binding = admitted(&runtime).await;
        let (_hold, cancelled) = tokio::sync::watch::channel(false);
        runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        let before = std::fs::read(dir.path().join("started")).unwrap();
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done);
        assert_eq!(std::fs::read(dir.path().join("started")).unwrap(), before);
    }
    #[tokio::test]
    async fn workflow_coding_native_owned_cancel_reaps_terminal_and_descendant() {
        let (dir, mut runtime, _fake) = native("cancel").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let binding = admitted(&runtime).await;
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let started = dir.path().join("started");
        let signal = async {
            for _ in 0..500 {
                if started.exists() { cancel.send_replace(true); return; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("interactive fixture did not start");
        };
        let (result, ()) = tokio::join!(runtime.execute_coding("w", &binding, cancelled), signal);
        assert_eq!(result.unwrap(), Outcome::SafeFailure);
        let pid: i32 = std::fs::read_to_string(started).unwrap().parse().unwrap();
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    #[cfg(target_os = "linux")]
    #[test]
    fn a_killed_process_nobody_has_reaped_yet_is_gone() {
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        assert!(!gone(pid), "a running process is not gone");
        unsafe { libc::kill(pid, libc::SIGKILL); }
        // Unreaped: `kill(pid, 0)` still succeeds, as it does for a pane tmux has not reaped.
        for _ in 0..500 {
            if std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z")) { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the zombie is still findable");
        assert!(gone(pid));
        child.wait().unwrap();
        assert!(gone(pid));
    }
    #[tokio::test]
    async fn workflow_coding_native_reopen_stop_owns_original_pty_without_relaunch() {
        let (dir, mut runtime) = configured(false, "pass").await;
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap()).unwrap();
        let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
        process.argv.clear();
        runtime.coding.as_mut().unwrap().harness = CodingHarness::Native { preset: "claude".into(),
            settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(settings).unwrap()) }),
            project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "unused crash fixture".into(), process };
        let root = runtime.root.clone();
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&runtime.coding.as_ref().unwrap()).unwrap()).unwrap();
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap(); let binding = admitted(&runtime).await;
        let host = Host(server(&root, &binding)); let id = terminal(&binding);
        let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
        let panes = host.0.list_tagged_panes().await.unwrap();
        let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone() };
        durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
        // Surviving private PTY plus bound owner, but no launch/process settlement.
        // Drop/reopen only the workflow owner, as after daemon death.
        drop(runtime); let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        let mut foreign = owner.clone(); foreign.tty = "/dev/foreign".into();
        assert!(runtime.quiesce_native(&foreign).await.is_err());
        assert_eq!(host.0.pane_life(&owner.pane, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
        let state = runtime.state("w").unwrap().unwrap();
        runtime.submit(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(),
            expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into() , reason: None } }, 201).unwrap();
        assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Cancelled);
        assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailure);
    }
    #[tokio::test]
    async fn workflow_service_native_stop_ignores_launch_drift_with_original_owner() {
        use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};
        for drift in ["changed-settings", "missing-settings", "missing-executable"] {
            let (dir, mut runtime) = configured(false, "pass").await;
            let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
            let repo_root = svc.store.create_repository_root(svc.host_id,dir.path().to_str().unwrap(),0).unwrap();
            let repo = svc.store.create_repository(svc.host_id,repo_root.id,"private",
                runtime.repository().join(".git").to_str().unwrap(),"").unwrap();
            let workspace = svc.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
            runtime.store = svc.store.clone();
            let settings = dir.path().join("settings.json");
            std::fs::write(&settings,serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap()).unwrap();
            let executable = dir.path().join("executable");
            std::fs::copy("/bin/sh",&executable).unwrap();
            std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
            let config = runtime.coding.as_mut().unwrap();
            config.checks[0].argv[0] = executable.to_string_lossy().into_owned();
            let CodingHarness::Command(mut process) = config.harness.clone() else { unreachable!() };
            process.argv.clear();
            config.harness = CodingHarness::Native { preset:"claude".into(),
                settings:Some(LaunchFile { path:settings.canonicalize().unwrap(),digest:digest(&std::fs::read(&settings).unwrap()) }),
                project_hooks:None, codex_hooks:None,plugin_dir:None,prompt:"unused".into(),process };
            let config = config.clone(); let definition = config.definition().unwrap();
            let adapter = Some(AdapterConfiguration::Coding(config.clone()));
            let pin = farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
            let m = |id: &str, revision, sequence| Mutation { id:id.into(),revision,epoch:1,sequence };
            runtime.wire_command(&workspace,"owner",Action::Publish { notifications:None, mutation:m("p",0,1),expected_pin:None,
                definition,adapter:adapter.map(Box::new) }).unwrap();
            runtime.wire_command(&workspace,"owner",Action::Enqueue { mutation:m("e",0,1),item:"one".into(),pin:pin.clone() }).unwrap();
            runtime.pump(2).await.unwrap();
            let state = runtime.state(&workspace).unwrap().unwrap();
            let stored = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap();
            let claim = runtime.store.workflow_claim_effect(&workspace,&stored.effect.id,"crash-test",state.revision,state.epoch,10,100).unwrap().unwrap();
            assert!(runtime.store.workflow_admit_effect(&claim,10).unwrap());
            let binding = Binding::new(&workspace,&stored.effect,claim.resource_generation,0,&runtime.item_base(&workspace,&stored.effect).unwrap());
            let root = runtime.root.clone();
            let host = Host(server(&root,&binding)); let id = terminal(&binding);
            let window = host.0.create_terminal_window(id,id,"crash fixture",runtime.tree(&binding).to_str().unwrap(),"/bin/sleep 30").await.unwrap();
            let panes = host.0.list_tagged_panes().await.unwrap(); let pane = panes.iter().find(|p| p.pane_id==window.pane_id).unwrap();
            let owner = NativeOwner { binding:binding.clone(),terminal:id,pane:pane.pane_id.clone(),tty:pane.tty.clone() };
            durable(&runtime.native_path(&binding,"owner"),&serde_json::to_vec(&owner).unwrap()).unwrap();
            drop(runtime);
            match drift {
                "changed-settings" => std::fs::write(&settings,"changed").unwrap(),
                "missing-settings" => std::fs::remove_file(&settings).unwrap(),
                _ => std::fs::remove_file(&executable).unwrap(),
            }
            assert!(config.validate().is_err(),"fresh launch must still refuse drift");
            let mut runtime = Runtime::open_service(&root,svc.store.clone()).unwrap();
            runtime.pump(200).await.unwrap();
            let state = runtime.state(&workspace).unwrap().unwrap();
            assert_eq!(state.items["one"].phase,Phase::Uncertain,"{drift}");
            assert_eq!(host.0.pane_life(&owner.pane,id).await.unwrap(),farcooler_tmux::PaneLife::Live);
            runtime.submit(&runtime.controller(),Request { id:"cancel".into(),workspace:workspace.clone(),
                expected_revision:state.revision,sequence:2,command:Command::Cancel { item:"one".into() , reason: None } },201).unwrap();
            assert_eq!(finish(&mut runtime,&workspace).await.items["one"].phase,Phase::Cancelled,"{drift}");
            assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(),Outcome::SafeFailure);
            assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["one"].policy_pin,pin);
            assert!(!dir.path().join("started").exists(),"recovery launched an adapter");
        }
    }
    /// ov-1082: the daemon died while a reviewer that had recorded its verdict was still in its pane. The restart
    /// settles the review from that verdict and ends the reviewer, instead of leaving the run uncertain.
    #[tokio::test]
    async fn workflow_native_review_recovers_from_its_recorded_verdict_after_a_crash() {
        use super::super::gate_tests::{three_gates, definition, at};
        let (dir, mut runtime, _fake) = three_gates("never").await;
        let definition = definition(&runtime); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..20 {
            if at(&runtime.state("w").unwrap().unwrap()).0 == "agent-review" { break; }
            runtime.pump(time).await.unwrap();
        }
        let binding = admitted(&runtime).await;
        let root = runtime.root.clone();
        let host = Host(server(&root, &binding)); let id = terminal(&binding);
        let window = host.0.create_terminal_window(id, id, "crash fixture", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
        let panes = host.0.list_tagged_panes().await.unwrap();
        let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone() };
        durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
        let command = runtime.coding_command(&binding).unwrap();
        super::super::write_agent_verdict(|key| command.environment.get(key).cloned(), true).unwrap();
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview));
        assert!(!dir.path().join("reviews").exists(), "recovery launched a reviewer");
        assert_ne!(host.0.pane_life(&owner.pane, id).await.ok(), Some(farcooler_tmux::PaneLife::Live), "the reviewer was left running");
    }
}
