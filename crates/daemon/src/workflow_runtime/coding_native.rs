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

impl Runtime {
    pub(super) fn is_native_build(&self, binding: &Binding) -> bool {
        matches!(binding.effect.kind, EffectKind::Execute(StageKind::Build))
            && matches!(self.coding.as_ref().map(|c| &c.harness), Some(CodingHarness::Native { .. }))
    }
    fn native_path(&self, binding: &Binding, suffix: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.native-{suffix}.json", digest(&binding.effect)))
    }
    fn native_agent(&self) -> Result<Agent> {
        let Some(CodingConfig { harness: CodingHarness::Native { preset, .. }, .. }) = &self.coding else {
            return Err(Error::Refused("not a native harness"));
        };
        preset.split(':').next().unwrap_or("").parse().map_err(|_| Error::Refused("unknown native agent"))
    }
    /// Pin the already-installed endpoint as well as its bytes. No hook writes.
    pub(super) fn validate_native_host(&self) -> Result<()> {
        if let Some(config) = &self.coding { self.validate_native_configuration(config)?; }
        Ok(())
    }
    pub(super) fn validate_native_configuration(&self, config: &CodingConfig) -> Result<()> {
        if matches!(config.harness, CodingHarness::Native { .. }) {
            if self.root.join("h.sock").as_os_str().len() > 103 {
                return Err(Error::Refused("native runtime needs a short private socket path"));
            }
            self.native_config_hooks(config)?;
        }
        Ok(())
    }
    fn native_hooks(&self) -> Result<()> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing native config"))?;
        self.native_config_hooks(config)
    }
    fn native_config_hooks(&self, config: &CodingConfig) -> Result<()> {
        let CodingHarness::Native { preset, settings, project_hooks, .. } = &config.harness else { return Ok(()) };
        let agent: Agent = preset.split(':').next().unwrap_or("").parse().map_err(|_| Error::Refused("unknown native agent"))?;
        let file = if agent == Agent::Claude { settings } else { project_hooks }
            .as_ref().ok_or(Error::Refused("missing native hooks"))?;
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
        for _ in 0..100 {
            if owned.iter().all(|pid| unsafe { libc::kill(*pid, 0) } != 0) {
                if !tmux.kill_terminal_window(owner.terminal).await
                    .map_err(|_| Error::Refused("native terminal close failed"))? {
                    return Err(Error::Refused("native terminal close unproved"));
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(Error::Refused("native descendants not quiescent"))
    }
    async fn native_turn(&self, listener: &tokio::net::UnixListener, binding: &Binding) -> Result<(bool, Vec<HookLine>)> {
        let agent = self.native_agent()?;
        let mut assembler = MessageAssembler::new();
        let mut session = (agent == Agent::Claude).then(|| terminal(binding).to_string());
        let mut prompted = false;
        let mut hooks = Vec::new();
        loop {
            let (stream, _) = listener.accept().await?;
            if stream.peer_cred()?.uid() != unsafe { libc::getuid() } { continue; }
            let mut stream = tokio::io::BufReader::new(stream);
            let mut raw = Vec::new();
            // Bounded by the overall turn deadline as well as this connection.
            use tokio::io::AsyncReadExt;
            let n = tokio::time::timeout(Duration::from_secs(1), (&mut stream).take(65537).read_until(b'\n', &mut raw)).await;
            if !matches!(n, Ok(Ok(_))) || raw.len() > 65536 || raw.last() != Some(&b'\n') { continue; }
            let Ok(line) = serde_json::from_slice::<HookLine>(&raw) else { continue };
            // Never grant permissions or tool-fence bypass from this host.
            stream.get_mut().write_all(b"{}\n").await?;
            let fact = facts(line.agent, &line.payload);
            if line.agent != agent || fact.cwd.as_deref() != Some(self.tree(binding).as_path()) { continue; }
            let Some(sid) = &fact.session_id else { continue };
            if session.is_none() && matches!(line.event.as_str(), "SessionStart" | "sessionStart") {
                session = Some(sid.clone());
            }
            if session.as_ref() != Some(sid) { continue; }
            if matches!(line.event.as_str(), "UserPromptSubmit" | "beforeSubmitPrompt") {
                let Some(CodingConfig { harness: CodingHarness::Native { prompt, .. }, .. }) = &self.coding else { unreachable!() };
                if line.payload.get("prompt").and_then(|p| p.as_str()) == Some(prompt.as_str()) { prompted = true; }
            }
            let events = assembler.accept(line.agent, &line.event, &line.payload);
            hooks.push(line);
            if hooks.len() > 4096 { return Err(Error::Refused("too many native hooks")); }
            if prompted {
                for event in events {
                    if let AgentEvent::TurnEnded { reason } = event {
                        return Ok((reason == EndReason::EndTurn, hooks));
                    }
                }
            }
        }
    }
    pub(super) async fn run_native(&self, binding: &Binding, command: &CodingCommand,
        mut cancel: tokio::sync::watch::Receiver<bool>) -> Result<Option<i32>> {
        self.native_hooks()?;
        let socket = self.root.join("h.sock");
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
        tmux.send_keys(&window.pane_id, "\n").await.map_err(|_| Error::Refused("native launch delivery uncertain"))?;
        let result = tokio::select! {
            result = tokio::time::timeout(Duration::from_millis(command.timeout_ms), self.native_turn(&listener, binding)) => result.ok().and_then(|r| r.ok()),
            _ = async { while !*cancel.borrow_and_update() { if cancel.changed().await.is_err() { break; } } } => None,
        };
        // Capture actual PTY output before closing it; never parse it as success.
        let output = tmux.capture_scrollback(&window.pane_id).await.map_err(|_| Error::Refused("native PTY output unavailable"))?;
        std::fs::write(self.process_log(binding), output)?;
        self.quiesce_native(&owner).await?;
        let (completed, hooks) = result.map_or((None, Vec::new()), |(passed, hooks)| (Some(passed), hooks));
        durable(&self.native_path(binding, "settled"), &serde_json::to_vec(&NativeSettlement { owner, hooks, completed })?)?;
        drop(listener);
        std::fs::remove_file(socket)?;
        Ok(completed.map(|passed| if passed { 0 } else { 1 }))
    }
    /// Only an admitted Stop may terminate an interrupted native owner. Ordinary
    /// reconciliation never launches, sends input, or kills a discovered process.
    pub(super) async fn cancel_interrupted_native(&self, binding: &Binding) -> Result<()> {
        let owner: NativeOwner = serde_json::from_slice(&self.bytes(&self.native_path(binding, "owner"))?)?;
        if owner.binding != *binding { return Err(Error::Refused("foreign interrupted native owner")); }
        let tmux = server(&self.root, binding);
        let output = tmux.capture_scrollback(&owner.pane).await.map_err(|_| Error::Refused("interrupted native output unavailable"))?;
        std::fs::write(self.process_log(binding), output)?;
        self.quiesce_native(&owner).await?;
        durable(&self.native_path(binding, "settled"), &serde_json::to_vec(&NativeSettlement {
            owner, hooks: Vec::new(), completed: None })?)?;
        // This root has one workflow/repository writer and no living old host.
        if self.root.join("h.sock").try_exists()? { std::fs::remove_file(self.root.join("h.sock"))?; }
        Ok(())
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
mod tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    struct Fake;
    impl Drop for Fake { fn drop(&mut self) { TEST_ARGV.with(|p| *p.borrow_mut() = None); } }
    async fn native(mode: &str) -> (tempfile::TempDir, Runtime, Fake) { native_agent(mode, "claude").await }
    async fn native_agent(mode: &str, agent: &str) -> (tempfile::TempDir, Runtime, Fake) {
        assert!(farcooler_core::programs::find("tmux").is_some(), "native ownership proof requires real tmux");
        let (dir, mut runtime) = configured(false, "pass").await;
        let socket = runtime.root.join("h.sock");
        let hooks = if agent == "claude" { dir.path().join("native-settings.json") }
            else { runtime.repository().join(if agent == "codex" { ".codex/hooks.json" } else { ".cursor/hooks.json" }) };
        std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        let text = match agent {
            "claude" => serde_json::to_string(&crate::hook_install::claude_settings(&socket)).unwrap(),
            "codex" => crate::hook_install::merge_codex("{}", &socket),
            _ => crate::hook_install::merge_cursor("{}", &socket),
        };
        std::fs::write(&hooks, text).unwrap();
        if agent != "claude" {
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
def hook(event, sid=session, cwd=os.getcwd(), **extra):
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(os.environ['FARCOOLER_NATIVE_HOOK_SOCKET'])
        s.sendall((json.dumps(dict(agent=agent, event=event, payload=dict(session_id=sid,cwd=cwd,workspace_roots=[cwd],**extra)))+'\n').encode())
        assert s.recv(64).strip() == b'{}'
hook('sessionStart' if agent == 'cursor' else 'SessionStart')
hook('beforeSubmitPrompt' if agent == 'cursor' else 'UserPromptSubmit', prompt='implement the selected task')
pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
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
        TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned(), mode.into(), agent.into()]));
        let config = runtime.coding.as_mut().unwrap();
        let file = LaunchFile { path: hooks.canonicalize().unwrap(), digest: digest(&std::fs::read(hooks).unwrap()) };
        config.harness = CodingHarness::Native { preset: agent.into(), settings: (agent == "claude").then(|| file.clone()),
            project_hooks: (agent != "claude").then_some(file), plugin_dir: None, prompt: "implement the selected task".into(), process };
        // Persist private trusted config before enqueue, then exercise the public reopen path.
        let root = runtime.root.clone();
        std::fs::write(root.join("coding.json"), serde_json::to_vec(config).unwrap()).unwrap();
        drop(runtime);
        (dir, Runtime::open(&root).unwrap(), Fake)
    }
    #[tokio::test]
    async fn workflow_coding_native_real_pty_normalized_hooks_and_quiescence() {
        for agent in ["claude", "codex", "cursor"] {
            let (dir, mut runtime, _fake) = native_agent("pass", agent).await;
            let definition = runtime.configured_definition().unwrap();
            enqueue(&mut runtime, "w", "one", definition);
            assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done, "{agent}");
            let pid: i32 = std::fs::read_to_string(dir.path().join("started")).unwrap().parse().unwrap();
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "native TUI must be reaped before candidate success");
            assert!(!runtime.root.join("h.sock").exists());
        }
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
    #[tokio::test]
    async fn workflow_coding_native_reopen_stop_owns_original_pty_without_relaunch() {
        let (dir, mut runtime) = configured(false, "pass").await;
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, serde_json::to_vec(&crate::hook_install::claude_settings(&runtime.root.join("h.sock"))).unwrap()).unwrap();
        let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
        process.argv.clear();
        runtime.coding.as_mut().unwrap().harness = CodingHarness::Native { preset: "claude".into(),
            settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(settings).unwrap()) }),
            project_hooks: None, plugin_dir: None, prompt: "unused crash fixture".into(), process };
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
            expected_revision: state.revision, sequence: 2, command: Command::Cancel { item: "one".into() } }, 201).unwrap();
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
                project_hooks:None,plugin_dir:None,prompt:"unused".into(),process };
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
                expected_revision:state.revision,sequence:2,command:Command::Cancel { item:"one".into() } },201).unwrap();
            assert_eq!(finish(&mut runtime,&workspace).await.items["one"].phase,Phase::Cancelled,"{drift}");
            assert_eq!(runtime.reconcile_coding(&binding).await.unwrap(),Outcome::SafeFailure);
            assert_eq!(runtime.state(&workspace).unwrap().unwrap().items["one"].policy_pin,pin);
            assert!(!dir.path().join("started").exists(),"recovery launched an adapter");
        }
    }

}
