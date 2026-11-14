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
/// Launched keeps the receipt shape previous engines wrote. A never-launched review records its binding
/// directly rather than inventing a pane owner, and can only supply an interrupted exit, never a verdict.
/// The optional failure preserves startup refusal reasons without changing older launched receipts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum NativeSettlement {
    Launched { owner: NativeOwner, hooks: Vec<HookLine>, completed: Option<bool>, #[serde(default, skip_serializing_if = "Option::is_none")] failure: Option<String> },
    NeverLaunched { binding: Binding },
}
impl NativeSettlement {
    fn completion(&self, binding: &Binding) -> Result<Option<bool>> {
        match self {
            Self::Launched { owner, completed, .. } if owner.binding == *binding && owner.terminal == terminal(binding) => Ok(*completed),
            Self::NeverLaunched { binding: original } if original == binding => Ok(None),
            _ => Err(Error::Refused("foreign native settlement")),
        }
    }
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

// Each effect has its own retained server, never the installed daemon's socket.
// A cancelled future tears down its server but cannot publish quiescence/success.
struct Host(TmuxServer);
impl Drop for Host {
    fn drop(&mut self) { farcooler_tmux::reap_server(self.0.socket()); }
}
pub(super) fn terminal(binding: &Binding) -> Uuid {
    let pin = digest(&("native-terminal/v1", binding));
    Uuid::parse_str(&pin[4..36]).expect("digest UUID")
}
fn server(root: &Path, binding: &Binding) -> TmuxServer {
    let id = terminal(binding);
    TmuxServer::in_dir(&format!("workflow-{id}"), id, root)
}
pub(super) fn snapshot() -> Result<crate::foreground::Foreground> {
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
            return Err(Error::Refused("native hooks must target this private host and include required active steering hooks"));
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
        let mut heard = clock.now();
        let mut verdict = None;
        let building = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
        let mut unfinished: Option<Duration> = None;
        loop {
            let stream = tokio::select! {
                accepted = listener.accept() => accepted?.0,
                _ = look.tick() => {
                    let owner: NativeOwner = serde_json::from_slice(&self.bytes(&self.native_path(binding, "owner"))?)?;
                    let alive = server(&self.root, binding).pane_life(&owner.pane, owner.terminal).await
                        .is_ok_and(|life| life == farcooler_tmux::PaneLife::Live);
                    if !alive {
                        let _ = super::execution::ACTIVITY.try_with(|a| {
                            a.session.store(false, std::sync::atomic::Ordering::SeqCst);
                            a.lost.store(true, std::sync::atomic::Ordering::SeqCst);
                        });
                        return Err(Error::Refused("the native session was lost without a receipt"));
                    }
                    if agent == Agent::Codex && startup_check.elapsed() >= Duration::from_secs(1) {
                        startup_check = tokio::time::Instant::now();
                        let output = server(&self.root, binding).capture_pane(&owner.pane, 40).await
                            .map_err(|_| Error::Refused("native startup output unavailable"))?;
                        if let Some(reason) = startup_refusal(agent, &output) { return Ok((false, hooks, Some(reason))); }
                    }
                    if verdict.is_none() && self.recorded_verdict(binding) { verdict = Some(clock.now()); }
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
            if line.agent != agent || !fact.cwd.as_deref().is_some_and(|cwd| cwd.starts_with(&tree)) { continue; }
            let Some(sid) = &fact.session_id else { continue };
            if session.is_none() && matches!(line.event.as_str(), "SessionStart" | "sessionStart") {
                session = Some(sid.clone());
            }
            if session.as_ref() != Some(sid) { continue; }
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
                stream.get_mut().write_all(reply).await?;
                stream.get_mut().write_all(b"\n").await?;
                continue;
            }
            let events = assembler.accept(line.agent, &line.event, &line.payload);
            let ended = prompted && events.iter().any(|event| matches!(event,
                AgentEvent::TurnEnded { reason: EndReason::EndTurn }));
            // Decide whether the build is waiting before the queue's last check, under the same
            // admission lock as message.send. An unfinished build keeps accepting direction.
            let waiting = ended && building && self.unfinished_build(binding, &tree).await.ok().flatten().is_some();
            let followup = if prompted && (ended || line.event == "PreToolUse") {
                self.native_followup(binding, &line, stream.get_mut(), ended && !waiting).await?
            } else { None };
            let continued = followup.is_some();
            if keyed_stop && let Some(reply) = followup { continued_stop = Some((line.payload.clone(), reply)); }
            if !continued { let _ = stream.get_mut().write_all(b"{}\n").await; }
            hooks.push(line);
            if hooks.len() > 4096 { return Err(Error::Refused("too many native hooks")); }
            if continued { unfinished = None; continue; }
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
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<Option<i32>> {
        self.native_hooks(binding)?;
        let socket = self.native_socket(binding);
        // No unlink-before-bind: a surviving owner is evidence, never a restart.
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let host = Host(server(&self.root, binding));
        let tmux = &host.0;
        let mut launch = String::from("env -i");
        for (key, value) in &command.environment {
            launch.push(' '); launch.push_str(&crate::service::shell_quote(&format!("{key}={value}")));
        }
        for arg in &command.argv { launch.push(' '); launch.push_str(&crate::service::shell_quote(arg)); }
        // Retained PTY stdin, not null: read gates launch until tags and owner are durable.
        // tmux automatically continues a stopped pane leader. Keep this waiting shell as the leader,
        // and pause only its agent child and descendants; the shell runs no work of its own.
        let gated = format!("exec /bin/sh -c {}", crate::service::shell_quote(&format!("read -r workflow_start; {launch}; status=$?; exit \"$status\"")));
        let id = terminal(binding);
        let window = tmux.create_terminal_window(id, id, "workflow coding", &self.tree(binding).to_string_lossy(), &gated).await
            .map_err(|_| Error::Refused("native managed PTY open uncertain"))?;
        let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native PTY inventory unavailable"))?;
        let pane = panes.iter().find(|p| p.terminal_id == id && p.pane_id == window.pane_id)
            .ok_or(Error::Refused("native PTY tags missing"))?;
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone() };
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
            .then(|| self.yielding.builder(crate::yielding::Target::Tty(pane.tty.clone())));
        // Listed for watching only (ov-972); declared after `host`, so it is unlisted before the server is reaped.
        let _watchable = crate::watch_view::publish(&binding.workspace, &binding.effect.item, tmux.clone(), pane.clone());
        tmux.send_keys(&window.pane_id, "\n").await.map_err(|_| Error::Refused("native launch delivery uncertain"))?;
        // A build stopped on a question, or never heard from, is told in Needs You (ov-1077).
        let heard_from = std::sync::atomic::AtomicBool::new(false);
        let stuck = crate::engine_stuck::watch(&binding.workspace, &binding.effect.item, self.native_agent(binding)?.as_str(),
            tmux, &window.pane_id, &heard_from);
        let (send, orders) = mpsc::channel(8);
        let review = !matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
        let _steerable = crate::engine_steering::publish(&binding.workspace, &binding.effect.item, id,
            self.native_agent(binding)?.as_str(), self.run_model(binding), review, self.steering_tickets(binding)?, send);
        let _inbox = super::native_messages::Inbox { store:&self.store, id };
        let clock = super::native_control::ActiveClock::new();
        let turn = self.native_turn(&listener, binding, &heard_from, &clock);
        let mut stop_reply = None;
        let mut frozen = None;
        let controlled = super::native_control::controlled(turn, tmux, &window.pane_id, &owner.tty, pane.stamp.pid as i32, id,
            &clock, Duration::from_millis(command.timeout_ms), orders, cancel, &mut frozen, &mut stop_reply);
        let _ = super::execution::ACTIVITY.try_with(|a| a.session.store(true, std::sync::atomic::Ordering::SeqCst));
        let result = tokio::select! {
            never = stuck => match never {},
            result = controlled => match result {
                Ok(turn) => turn,
                Err(error) => { tracing::warn!(effect = %binding.effect.id, %error, "the native turn could not be read"); None }
            },
        };
        let _ = super::execution::ACTIVITY.try_with(|a| {
            a.settle();
            a.session.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        self.quiesce_native(&owner).await?;
        if let Some(stopped) = frozen.take() { stopped.quiescent(); }
        // Closing the pane ends its fanout. Drain it before hashing the log into the receipt.
        match tokio::time::timeout(Duration::from_secs(5), &mut output).await {
            Ok(result) => result.map_err(|_| Error::Refused("native output capture failed"))??,
            Err(_) => { output.abort(); return Err(Error::Refused("native output did not drain")); }
        }
        let (mut completed, hooks, mut failure) = result.map_or((None, Vec::new(), None),
            |(passed, hooks, failure)| (Some(passed), hooks, failure));
        // A recorded verdict is the review however the turn ended: cut short by a daemon stopping for an install, out
        // of time, or failing after it (ov-1082). Read after the reviewer is ended, so it is the last verdict given.
        if self.recorded_verdict(binding) { completed = Some(true); failure = None; }
        durable(&self.native_path(binding, "settled"), &serde_json::to_vec(&NativeSettlement::Launched { owner, hooks, completed, failure })?)?;
        drop(listener);
        std::fs::remove_file(socket)?;
        if let Some(reply) = stop_reply { let _ = reply.send(Ok(())); }
        Ok(completed.map(|passed| if passed { 0 } else { 1 }))
    }
    /// Only an admitted Stop, or a reviewer's own recorded verdict (ov-1082), may terminate an interrupted native
    /// owner. Ordinary reconciliation never launches, sends input, or kills a discovered process. `completed` is what
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
            let out = server(&self.root, binding).run(&["list-panes", "-a", "-F", "#{pane_id}"]).await
                .map_err(|_| Error::Refused("native unstarted inventory unavailable"))?;
            let absent = out.stderr.contains("no server running") || out.stderr.contains("no current session")
                || (out.stderr.contains("error connecting") && (out.stderr.contains("No such file or directory")
                    || out.stderr.contains("Connection refused")));
            if !(out.ok() && out.stdout.trim().is_empty() || !out.ok() && absent) {
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
                let table = tokio::task::spawn_blocking(snapshot).await
                    .map_err(|_| Error::Refused("native process read failed"))??;
                if !table.under_tty(owner.tty.trim_start_matches("/dev/")).is_empty() {
                    return Err(Error::Refused("native vanished pane still has processes"));
                }
                // The pane cannot be captured or closed again. Its saved output and verdict survive the install.
                self.bytes(&self.process_log(binding))?;
            } else {
                self.capture_native_output(binding, &tmux, &owner.pane).await?;
                self.quiesce_native(&owner).await?;
            }
            durable(&settled, &serde_json::to_vec(&NativeSettlement::Launched { owner, hooks: Vec::new(), completed, failure: None })?)?;
            completed
        };
        // The socket is this build's alone, so a leftover one is stale by construction.
        let socket = self.native_socket(binding);
        if socket.try_exists()? { std::fs::remove_file(socket)?; }
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
    /// Only native reviews can recover a missing owner as never launched (ov-1130). A present but unreadable or
    /// malformed owner is still uncertain; absence alone is not enough until the private server is checked.
    pub(super) fn native_review_without_owner(&self, binding: &Binding) -> Result<bool> {
        Ok(matches!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent)))
            && !self.native_path(binding, "owner").try_exists()?)
    }
    pub(super) fn native_session(binding: &Binding) -> String { terminal(binding).to_string() }
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
pub(super) mod tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    pub(crate) struct Fake;
    impl Drop for Fake { fn drop(&mut self) { TEST_ARGV.with(|p| *p.borrow_mut() = None); } }
    pub(crate) async fn native(mode: &str) -> (tempfile::TempDir, Runtime, Fake) { native_agent(mode, "claude").await }
    /// `agent` is `claude`, `codex` or `cursor`, each with its hooks pinned as before ov-1088 (Codex's and Cursor's
    /// committed in the repository), or `codex-engine`: Codex with the runner's own hooks file, outside the repository,
    /// which the engine copies into the worktree (ov-1088). That agent checks it finds the copy before it starts.
    pub(crate) async fn native_agent(mode: &str, agent: &str) -> (tempfile::TempDir, Runtime, Fake) {
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
            "codex" => serde_json::to_string_pretty(&crate::hook_install::native_codex_hooks(&socket)).unwrap(),
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
        if mode == 'active-steering' and event == 'PreToolUse' and agent in ('claude', 'codex'):
            # Run the command from the accepted pinned settings, including --gating, through the
            # actual CLI. Assert the harness's output shape, not the daemon's internal wire response.
            settings = json.loads(pathlib.Path(os.environ['TEST_AGENT_SETTINGS']).read_text())
            command = settings['hooks']['PreToolUse'][0]['hooks'][0]['command']
            payload = dict(session_id=sid, cwd=cwd, **extra)
            reply = subprocess.run(command, shell=True, input=json.dumps(payload), text=True, capture_output=True, check=True)
            return json.loads(reply.stdout)['hookSpecificOutput']
        s.connect(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
        takes = ['followup'] if mode in ('steering', 'active-steering') and event == 'Stop' and sid == session and cwd == os.getcwd() and 'agent_id' not in extra else []
        s.sendall((json.dumps(dict(agent=agent, event=event, takes=takes, payload=dict(session_id=sid,cwd=cwd,workspace_roots=[cwd],**extra)))+'\n').encode())
        if takes:
            return json.loads(s.makefile().readline())
hook('sessionStart' if agent == 'cursor' else 'SessionStart')
prompt = pathlib.Path(os.environ['TEST_LAUNCHED_PROMPT_FILE']).read_text() if 'TEST_LAUNCHED_PROMPT_FILE' in os.environ else os.environ['TEST_LAUNCHED_PROMPT']
hook('beforeSubmitPrompt' if agent == 'cursor' else 'UserPromptSubmit', prompt=prompt)
pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
if mode in ('steering', 'active-steering'):
    ready = pathlib.Path(os.environ['STARTED']).with_name('direction-ready')
    for _ in range(500):
        if ready.exists(): break
        time.sleep(0.01)
    if mode == 'active-steering' and agent in ('claude', 'codex'):
        response = hook('PreToolUse', tool_name='Write', tool_input={'file_path':'result.txt'})
        assert response['hookEventName'] == 'PreToolUse', response
        assert response['permissionDecision'] == 'deny', response
        direction = response['permissionDecisionReason']
    else:
        # Between tool calls, direction gates the final Stop.
        response = hook('Stop', turn_id='probe-turn', last_assistant_message='first reply')
        if agent == 'codex':
            duplicate = hook('Stop', turn_id='probe-turn', last_assistant_message='first reply')
            assert duplicate == response, (duplicate, response)
        direction = response['followup_message']
    assert 'Use the new API' in direction, response
    pathlib.Path(os.environ['STARTED']).with_name('continued').write_text(direction)
if mode == 'model-refused':
    print("■ The 'gpt-6.1-luna' model is not supported when using Codex with a ChatGPT account", flush=True)
    time.sleep(30)
if mode == 'barrier':
    d = pathlib.Path(os.environ['STARTED']).parent / 'barrier'
    d.mkdir(exist_ok=True)
    (d / session).write_text(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
    t = time.time()
    while len(list(d.iterdir())) < 2:
        if time.time() - t > 4: sys.exit(9)
        time.sleep(0.05)
if mode in ('no-changes', 'no-changes-bare', 'dirty-no-commit'):
    if mode == 'dirty-no-commit':
        pathlib.Path('result.txt').write_text('work kept without a commit')
    if agent == 'cursor':
        # Cursor's stop hook carries no prose; it names the transcript, where the prose is (ov-1235).
        extra = {}
        if mode != 'no-changes-bare':
            t = pathlib.Path(os.environ['STARTED']).parent / 'cursor-transcript.jsonl'
            t.write_text(json.dumps(dict(role='user', message=dict(content=[dict(type='text', text='<user_query>go</user_query>')]))) + '\n'
                + json.dumps(dict(role='assistant', message=dict(content=[dict(type='text', text='Looking at the ticket.')]))) + '\n'
                + json.dumps(dict(role='assistant', message=dict(content=[dict(type='text', text='I could not read the ticket; the runner request hung.')]))) + '\n'
                + json.dumps(dict(type='turn_ended', status='success')) + '\n')
            extra = dict(transcript_path=str(t))
        hook('stop', **extra)
    else:
        hook('Stop', last_assistant_message='I could not read the ticket; the runner request hung.')
    time.sleep(30)
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
    print('native stdout evidence', flush=True)
    print('native stderr evidence', file=sys.stderr, flush=True)
    # A terminal UI can erase both the screen and scrollback before its final hook.
    print('\x1b[2J\x1b[3J\x1b[Hscreen after clearing evidence', flush=True)
    stop = 'stop' if agent == 'cursor' else 'Stop'
    hook(stop, sid='foreign')
    hook(stop, cwd='/foreign')
    if agent == 'claude': hook(stop, agent_id='subagent')
    if mode != 'foreign-only': hook('StopFailure' if mode == 'failed' else stop, error='authentication_failed')
    time.sleep(30)
"#).unwrap();
        let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
        process.argv.clear(); process.timeout_ms = if mode == "foreign-only" { 300 } else { 5000 };
        if mode == "active-steering" && matches!(agent, "claude" | "codex") {
            let exe = std::env::current_exe().unwrap();
            let profile = farcooler_core::scratch::cargo_profile(&exe).unwrap();
            assert!(profile.join("farcooler").is_file(), "build the hook CLI with cargo build -p farcooler-cli before this test");
            let bin = dir.path().join("hook-bin"); std::fs::create_dir(&bin).unwrap();
            std::os::unix::fs::symlink(profile.join("farcooler"), bin.join(farcooler_protocol::CHANNEL.cli_binary_name())).unwrap();
            process.environment.insert("PATH".into(), format!("{}:{}:/usr/bin:/bin", bin.display(), profile.display()));
            process.environment.insert("TEST_AGENT_SETTINGS".into(), hooks.to_string_lossy().into_owned());
        }
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
    #[test]
    fn workflow_native_startup_refusal_requires_the_codex_error_line() {
        let line = "■ The 'gpt-6.1-luna' model is not supported when using Codex with a ChatGPT account";
        assert!(startup_refusal(Agent::Codex, &line.replace("using Codex", "using\nCodex")).is_some());
        assert!(startup_refusal(Agent::Claude, line).is_none());
        assert!(startup_refusal(Agent::Codex, &format!("quoted: {line}")).is_none());
        assert!(startup_refusal(Agent::Codex, "Ready. All checks passed.").is_none());
    }

    #[tokio::test]
    async fn workflow_native_startup_model_refusal_fails_promptly_with_reason() {
        let (_dir, mut runtime, _fake) = native_agent("model-refused", "codex").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4), finish(&mut runtime, "w")).await
            .expect("a startup refusal must release its slot before the turn deadline");
        let item = &runtime.state("w").unwrap().unwrap().items["one"];
        assert_eq!(item.phase, Phase::Failed);
        assert!(item.failure.as_ref().unwrap().contains("not supported when using Codex"));
    }

    #[tokio::test]
    async fn workflow_execution_native_turn_stays_present_during_verification() {
        let (_dir, mut runtime, _fake) = native("pass").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let mut running = Vec::new();
        let mut futures = Vec::new();
        runtime.launch_ready(&["w".into()], 3, &mut running, &mut futures, &mut 0).await.unwrap();
        let binding = &running[0].binding;
        let command = runtime.coding_command(binding).unwrap();
        let activity = runtime.activities.lock().unwrap().get(&("w".into(), binding.effect.id.clone())).unwrap().clone();
        // Run just the native turn, pausing the adapter before its ProcessFact and candidate checks.
        super::super::execution::ACTIVITY.scope(activity, async {
            assert_eq!(runtime.run_native(binding, &command, running[0].cancel.subscribe()).await.unwrap(), Some(0));
            assert_eq!(runtime.execution_state("w").unwrap().unwrap().items["one"].phase, Phase::Running);
        }).await;
        drop(futures);
    }

    #[tokio::test]
    async fn workflow_native_session_lost_without_receipt_leaves_running_in_one_scan() {
        let (dir, mut runtime, _fake) = native("cancel").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap(); // Provision; the next effect launches the agent.
        let mut running = Vec::new();
        let mut futures = Vec::new();
        runtime.launch_ready(&["w".into()], 3, &mut running, &mut futures, &mut 0).await.unwrap();
        let started = dir.path().join("started");
        tokio::select! {
            _ = super::super::next_finished(&mut futures) => panic!("agent ended before it started"),
            _ = async {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
                }).await.unwrap();
            } => {}
        }
        assert_eq!(runtime.execution_state("w").unwrap().unwrap().items["one"].phase, Phase::Running);
        let pid: i32 = std::fs::read_to_string(started).unwrap().parse().unwrap();
        unsafe { libc::kill(pid, libc::SIGKILL); }
        let (index, _) = tokio::time::timeout(Duration::from_secs(2), super::super::next_finished(&mut futures))
            .await.expect("a dead session must not wait for the five-second turn deadline");
        let lost = runtime.execution_state("w").unwrap().unwrap();
        assert_eq!(lost.items["one"].phase, Phase::Uncertain);
        assert!(lost.items["one"].execution.as_ref().unwrap().text.contains("lost without a receipt"));
        let job = running.remove(index); futures.remove(index);
        runtime.activities.lock().unwrap().remove(&(job.workspace.clone(), job.effect.id.clone()));
        runtime.observe("w", &job.effect, Outcome::Uncertain, false, 4).unwrap();
        let item = &runtime.execution_state("w").unwrap().unwrap().items["one"];
        assert_eq!(item.phase, Phase::Uncertain);
        assert!(item.execution.is_none(), "settled uncertainty must not invent a new session-loss observation");
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
    async fn ov_1217_long_ticket_build_and_review_settle_from_native_hooks() {
        use farcooler_store::models::{Actor, AcceptanceItem, TaskUpdate};
        let (_dir, mut runtime, _fake) = super::super::gate_tests::three_gates("subdir").await;
        let host = Uuid::now_v7();
        let root = runtime.store.create_repository_root(host, "/long-brief", 0).unwrap();
        let repo = runtime.store.create_repository(host, root.id, "brief", "/long-brief/.git", "").unwrap();
        let workspace = runtime.store.ensure_main_workspace(repo.id).unwrap();
        let task = runtime.store.create_task(workspace.id, "Long native ticket", Actor::User).unwrap();
        // The prompt itself exceeds the old limit; quotes, newlines and non-ASCII text also grow on the wire.
        let intent = format!("{}{}", "x".repeat(70_000), "\"\\\n修复".repeat(500));
        let acceptance = "Keep every acceptance line intact.";
        runtime.store.update_task(task.id, task.resource_version, &TaskUpdate {
            title: task.title.clone(), intent: intent.clone(),
            acceptance: vec![AcceptanceItem { id: Uuid::now_v7(), text: acceptance.into(), met: false }],
            constraints: vec!["Keep native hook ownership checks.".into()], labels: vec![], worktree_id: None,
        }).unwrap();
        let mut definition = super::super::gate_tests::definition(&runtime);
        definition.stages.retain(|s| !matches!(s.kind, StageKind::Review(Reviewer::Orchestrator(_) | Reviewer::Person(_))));
        enqueue(&mut runtime, "w", &task.key, definition);
        let state = finish(&mut runtime, "w").await;
        assert_eq!(state.items[&task.key].phase, Phase::Done, "{state:?}");
        let mut settled = Vec::new();
        for entry in std::fs::read_dir(runtime.root.join("receipts")).unwrap() {
            let path = entry.unwrap().path();
            if !path.to_string_lossy().ends_with(".native-settled.json") { continue; }
            let NativeSettlement::Launched { owner, hooks, completed } = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
                else { panic!("the native step never launched") };
            assert_eq!(completed, Some(true));
            let prompt = hooks.iter().find(|h| h.event == "UserPromptSubmit").unwrap()
                .payload["prompt"].as_str().unwrap();
            assert!(prompt.len() > 65_536);
            assert!(prompt.contains(&task.title) && prompt.contains(&intent) && prompt.contains(acceptance));
            assert!(prompt.contains("Keep native hook ownership checks."));
            // A review's receipt can also settle it after quiet: require the actual Stop to have been read.
            assert!(hooks.iter().any(|h| h.event == "Stop"));
            settled.push(owner.binding.effect.kind);
        }
        assert_eq!(settled.len(), 2);
        assert!(settled.contains(&EffectKind::Execute(StageKind::Build)));
        assert!(settled.contains(&EffectKind::Execute(StageKind::Review(Reviewer::Agent))));
    }
    #[tokio::test]
    async fn ov_1217_build_without_a_commit_reports_ended_without_changes_and_last_message() {
        let (_dir, mut runtime, _fake) = native_agent("no-changes", "codex").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed, "{state:?}");
        assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message:\nI could not read the ticket; the runner request hung."));
        assert!(item.candidate.is_none());
    }
    #[tokio::test]
    async fn ov_1235_cursor_build_without_a_commit_reports_the_last_message_from_its_transcript() {
        let (_dir, mut runtime, _fake) = native_agent("no-changes", "cursor").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed, "{state:?}");
        assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message:\nI could not read the ticket; the runner request hung."));
    }
    #[tokio::test]
    async fn ov_1235_cursor_build_without_a_transcript_says_its_last_message_is_unavailable() {
        let (_dir, mut runtime, _fake) = native_agent("no-changes-bare", "cursor").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed, "{state:?}");
        assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\nThe agent's last message: unavailable for this agent."));
    }
    #[tokio::test]
    async fn ov_1217_dirty_build_without_a_commit_reports_reason_files_and_last_message() {
        let (_dir, mut runtime, _fake) = native_agent("dirty-no-commit", "codex").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed, "{state:?}");
        assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\n\
            The build ended its turn with changes it hadn't committed: result.txt. \
            They're still in its worktree, and the build carries on from them.\n\n\
            The agent's last message:\nI could not read the ticket; the runner request hung."));
        assert!(item.candidate.is_none());
        assert_eq!(std::fs::read_to_string(runtime.item_tree("w", "one").join("result.txt")).unwrap(), "work kept without a commit");
    }
    #[tokio::test]
    async fn ov_1217_no_commit_does_not_reuse_work_already_ahead_of_base() {
        let (_dir, mut runtime, _fake) = native_agent("no-changes", "codex").await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let binding = admitted(&runtime).await;
        let tree = runtime.tree(&binding);
        adapters::git(&tree, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
            "commit", "--allow-empty", "-m", "earlier work"]).await.unwrap();
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let outcome = runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        assert!(matches!(outcome, Outcome::SafeFailureBecause(ref why) if why.contains("ended without changes")
            && why.contains("the runner request hung")), "{outcome:?}");
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
        assert_eq!(item.failure.as_deref(), Some("The build ended without changes: it made no new commit.\n\n\
            The build ended its turn with changes it hadn't committed: result.txt. \
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
    async fn workflow_native_failed_build_keeps_output_even_after_terminal_clears() {
        let (_dir, mut runtime, _fake) = native("failed").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed);
        let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        let logs = reply["process_logs"].as_array().unwrap();
        let log = logs.iter().find(|log| log["stage"] == serde_json::json!({"Execute":"Build"})).unwrap();
        let output = std::fs::read_to_string(log["log_path"].as_str().unwrap()).unwrap();
        for evidence in ["native stdout evidence", "native stderr evidence"] {
            assert!(output.contains(evidence), "missing {evidence}: {output:?}");
            assert!(log["tail"].as_str().unwrap().contains(evidence));
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
    async fn workflow_native_steering_continues_the_same_codex_and_claude_session_before_settlement() {
        use farcooler_store::models::Actor;
        for (agent, mode) in [("claude", "steering"), ("codex", "steering"), ("claude", "active-steering"), ("codex", "active-steering")] {
            let (dir, mut runtime, _fake) = native_agent(mode, agent).await;
            let host = Uuid::now_v7();
            let repository = runtime.repository();
            let root = runtime.store.create_repository_root(host, dir.path().to_str().unwrap(), 0).unwrap();
            let repo = runtime.store.create_repository(host, root.id, "Native", repository.join(".git").to_str().unwrap(), "").unwrap();
            let board = runtime.store.ensure_main_workspace(repo.id).unwrap();
            let task = runtime.store.create_task(board.id, "Steer this", Actor::User).unwrap();
            let workspace = board.id.to_string();
            let definition = runtime.configured_definition().unwrap();
            enqueue(&mut runtime, &workspace, &task.key, definition);
            runtime.pump(2).await.unwrap();
            let state = runtime.state(&workspace).unwrap().unwrap();
            let effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
            let claim = runtime.store.workflow_claim_effect(&workspace, &effect.id, "steering", state.revision, state.epoch, 10, 100).unwrap().unwrap();
            let base = runtime.pin_base(&workspace, &effect).await.unwrap();
            assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
            let binding = Binding::new(&workspace, &effect, claim.resource_generation, 0, &base);
            let (_cancel, cancel) = tokio::sync::watch::channel(false);
            let steer = async {
                for _ in 0..500 {
                    if dir.path().join("started").exists() { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(dir.path().join("started").exists());
                let id = crate::engine_steering::find(board.id, &task.key).expect("the running step is listed");
                assert_eq!(id, terminal(&binding));
                runtime.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
                std::fs::write(dir.path().join("direction-ready"), "").unwrap();
            };
            let (outcome, ()) = tokio::join!(runtime.execute_coding(&workspace, &binding, cancel), steer);
            assert!(matches!(outcome.unwrap(), Outcome::Passed(_)));
            assert!(std::fs::read_to_string(dir.path().join("continued")).unwrap().contains("Use the new API"));
            assert!(runtime.store.pending_message_wakes().unwrap().is_empty());
            assert_eq!(runtime.state(&workspace).unwrap().unwrap(), state, "steering changes no workflow state");
            assert!(crate::engine_steering::find(board.id, &task.key).is_none(), "unlisted after settlement");
        }
    }

    #[tokio::test]
    async fn workflow_native_stop_reaps_a_paused_agent_without_cancelling_its_run() {
        let (dir, mut runtime, _fake) = native("cancel").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        runtime.pump(2).await.unwrap();
        let binding = admitted(&runtime).await;
        let state = runtime.state("w").unwrap().unwrap();
        let (_cancel, cancel) = tokio::sync::watch::channel(false);
        let stop = async {
            for _ in 0..500 {
                if dir.path().join("started").exists() { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(dir.path().join("started").exists());
            let id = terminal(&binding);
            crate::engine_steering::control(id, crate::engine_steering::Control::Pause).await.unwrap();
            assert_eq!(crate::engine_steering::row(id).unwrap().doing_now, "Paused");
            crate::engine_steering::control(id, crate::engine_steering::Control::Stop).await.unwrap();
            let pid: i32 = std::fs::read_to_string(dir.path().join("started")).unwrap().parse().unwrap();
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "Stop acknowledges only after process quiescence");
        };
        let (outcome, ()) = tokio::join!(runtime.execute_coding("w", &binding, cancel), stop);
        assert_eq!(outcome.unwrap(), Outcome::SafeFailure);
        assert_eq!(runtime.state("w").unwrap().unwrap(), state, "Stop leaves settlement to the engine");
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
        for (agent, drift) in ["claude", "codex"].into_iter().flat_map(|agent|
            ["changed-settings", "missing-settings", "missing-executable"].map(|drift| (agent, drift))) {
            let (dir, mut runtime) = configured(false, "pass").await;
            let svc = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
            let repo_root = svc.store.create_repository_root(svc.host_id,dir.path().to_str().unwrap(),0).unwrap();
            let repo = svc.store.create_repository(svc.host_id,repo_root.id,"private",
                runtime.repository().join(".git").to_str().unwrap(),"").unwrap();
            let workspace = svc.store.ensure_main_workspace(repo.id).unwrap().id.to_string();
            runtime.store = svc.store.clone();
            let settings = dir.path().join("settings.json");
            let socket = runtime.root.join("h.sock");
            let hooks = if agent == "claude" { crate::hook_install::claude_settings(&socket) } else { crate::hook_install::native_codex_hooks(&socket) };
            std::fs::write(&settings,serde_json::to_vec(&hooks).unwrap()).unwrap();
            let executable = dir.path().join("executable");
            std::fs::copy("/bin/sh",&executable).unwrap();
            std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
            let config = runtime.coding.as_mut().unwrap();
            config.checks[0].argv[0] = executable.to_string_lossy().into_owned();
            let CodingHarness::Command(mut process) = config.harness.clone() else { unreachable!() };
            process.argv.clear();
            let hooks = LaunchFile { path:settings.canonicalize().unwrap(),digest:digest(&std::fs::read(&settings).unwrap()) };
            config.harness = CodingHarness::Native { preset:agent.into(),
                settings:(agent == "claude").then(|| hooks.clone()),
                project_hooks:None, codex_hooks:(agent == "codex").then_some(hooks),plugin_dir:None,prompt:"unused".into(),process };
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
    /// ov-1130: an install removed the server after the verdict and output were saved, before settlement.
    #[tokio::test]
    async fn workflow_native_review_vanished_server_keeps_output_and_recorded_verdict() {
        use super::super::gate_tests::{three_gates, held_review, at};
        for passed in [true, false] {
            let (dir, mut runtime, _fake) = three_gates("never").await;
            let binding = held_review(&mut runtime).await;
            let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
            let window = host.0.create_terminal_window(id, id, "lost reviewer", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
            let panes = host.0.list_tagged_panes().await.unwrap();
            let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
            // Model the vanished pane's now-empty tty with a reserved PTY. Holding its master is the
            // synchronization point: another concurrent test cannot reuse this tty before recovery reads it.
            use std::os::fd::FromRawFd;
            let (mut master, mut slave) = (-1, -1);
            let mut name = [0 as libc::c_char; 128];
            assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, name.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut()) }, 0);
            let (_master, _slave) = unsafe { (std::fs::File::from_raw_fd(master), std::fs::File::from_raw_fd(slave)) };
            let tty = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_str().unwrap().to_owned();
            let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: pane.pane_id.clone(), tty };
            durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
            let command = runtime.coding_command(&binding).unwrap();
            super::super::write_agent_verdict(|key| command.environment.get(key).cloned(), passed).unwrap();
            std::fs::write(runtime.process_log(&binding), b"saved reviewer output").unwrap();
            assert!(host.0.run(&["kill-server"]).await.unwrap().ok());
            let root = runtime.root.clone(); drop(runtime);
            let mut runtime = Runtime::open(&root).unwrap();
            runtime.pump(200).await.unwrap();
            let expected = if passed { ("orchestrator-review".into(), Phase::WaitingReview) }
                else { ("agent-review".into(), Phase::Failed) };
            assert_eq!(at(&runtime.state("w").unwrap().unwrap()), expected);
            assert_eq!(std::fs::read(runtime.process_log(&binding)).unwrap(), b"saved reviewer output");
            assert!(runtime.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect))).exists());
            assert!(runtime.native_path(&binding, "settled").exists());
            assert!(!dir.path().join("reviews").exists(), "recovery launched another reviewer");
        }
    }
    /// A review refused before opening its PTY has no owner file. Its admitted Stop must still finish.
    #[tokio::test]
    async fn workflow_native_review_never_launched_stop_ends_cancelled() {
        use super::super::gate_tests::{three_gates, held_review};
        let (dir, mut runtime, _fake) = three_gates("never").await;
        let binding = held_review(&mut runtime).await;
        let state = runtime.state("w").unwrap().unwrap();
        assert!(runtime.submit(&runtime.controller(), Request { id: "cancel".into(), workspace: "w".into(),
            expected_revision: state.revision, sequence: 2,
            command: Command::Cancel { item: "one".into(), reason: None } }, 200).unwrap().result.is_ok());
        runtime.pump(201).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
        let fact_path = runtime.root.join("receipts").join(format!("{}.process.json", digest(&binding.effect)));
        let fact: serde_json::Value = serde_json::from_slice(&runtime.bytes(&fact_path).unwrap()).unwrap();
        assert!(fact["exit"].is_null());
        assert!(!dir.path().join("reviews").exists());
    }
    /// Without a Stop, that same unstarted effect is safe to retry, including after it was marked Uncertain.
    #[tokio::test]
    async fn workflow_native_review_never_launched_runs_again() {
        use super::super::gate_tests::{three_gates, held_review, at};
        let (dir, mut runtime, _fake) = three_gates("subdir").await;
        let binding = held_review(&mut runtime).await;
        runtime.observe("w", &binding.effect, Outcome::Uncertain, false, 200).unwrap();
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(201).await.unwrap();
        assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(), Outcome::SafeFailure);
        for time in 202..240 {
            if at(&runtime.state("w").unwrap().unwrap()).0 == "orchestrator-review" { break; }
            runtime.pump(time).await.unwrap();
        }
        assert_eq!(at(&runtime.state("w").unwrap().unwrap()), ("orchestrator-review".into(), Phase::WaitingReview));
        assert_eq!(std::fs::read_to_string(dir.path().join("reviews")).unwrap(), "claude:sonnet\n");
    }
    /// Missing ownership is not proof that nothing launched while the effect's server still has a pane.
    #[tokio::test]
    async fn workflow_native_review_without_owner_refuses_an_unfinished_open() {
        use super::super::gate_tests::{three_gates, held_review};
        let (_dir, mut runtime, _fake) = three_gates("never").await;
        let binding = held_review(&mut runtime).await;
        let host = Host(server(&runtime.root, &binding));
        // Deliberately untagged: a tagged-only inventory must not mistake it for an empty server.
        assert!(host.0.run(&["new-session", "-d", "/bin/sleep 30"]).await.unwrap().ok());
        assert!(runtime.cancel_interrupted_native(&binding, None).await.is_err());
        assert!(!runtime.native_path(&binding, "settled").exists());
        assert!(!runtime.process_log(&binding).exists());
    }
    /// A tty that still has processes (including one reused since the crash) is never killed or called stopped.
    #[tokio::test]
    async fn workflow_native_review_vanished_pane_refuses_a_busy_tty() {
        use super::super::gate_tests::{three_gates, held_review};
        let (_dir, mut runtime, _fake) = three_gates("never").await;
        let binding = held_review(&mut runtime).await;
        let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
        let window = host.0.create_terminal_window(id, id, "busy tty", runtime.tree(&binding).to_str().unwrap(), "/bin/sleep 30").await.unwrap();
        let panes = host.0.list_tagged_panes().await.unwrap();
        let pane = panes.iter().find(|p| p.pane_id == window.pane_id).unwrap();
        let owner = NativeOwner { binding: binding.clone(), terminal: id, pane: "%999999".into(), tty: pane.tty.clone() };
        durable(&runtime.native_path(&binding, "owner"), &serde_json::to_vec(&owner).unwrap()).unwrap();
        std::fs::write(runtime.process_log(&binding), b"saved output").unwrap();
        assert!(runtime.cancel_interrupted_native(&binding, Some(true)).await.is_err());
        assert!(!runtime.native_path(&binding, "settled").exists());
        assert_eq!(std::fs::read(runtime.process_log(&binding)).unwrap(), b"saved output");
        assert_eq!(host.0.pane_life(&window.pane_id, id).await.unwrap(), farcooler_tmux::PaneLife::Live);
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
        let log = runtime.process_log(&binding);
        durable(&log, b"evidence saved before the crash\n").unwrap();
        drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap();
        runtime.pump(200).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(at(&state), ("orchestrator-review".into(), Phase::WaitingReview));
        assert!(std::fs::read_to_string(log).unwrap().contains("evidence saved before the crash"));
        assert!(!dir.path().join("reviews").exists(), "recovery launched a reviewer");
        assert_ne!(host.0.pane_life(&owner.pane, id).await.ok(), Some(farcooler_tmux::PaneLife::Live), "the reviewer was left running");
    }
}
