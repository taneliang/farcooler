use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
pub(crate) struct Fake;
impl Drop for Fake { fn drop(&mut self) { TEST_ARGV.with(|p| *p.borrow_mut() = None); } }
pub(crate) async fn native(mode: &str) -> (tempfile::TempDir, Runtime, Fake) { native_agent(mode, "claude").await }
/// `agent` is `claude`, `codex` or `cursor`, each with its hooks pinned as before ov-1088 (Codex's and Cursor's
/// committed in the repository), or `codex-engine`: Codex with the runner's own hooks file, outside the repository,
/// which the engine copies into the worktree (ov-1088). That agent checks it finds the copy before it starts.
pub(crate) async fn native_agent(mode: &str, agent: &str) -> (tempfile::TempDir, Runtime, Fake) {
    let (dir, runtime) = configured(false, "pass").await;
    native_in(dir, runtime, mode, agent).await
}
pub(in crate::workflow_runtime) async fn native_in(dir: tempfile::TempDir, mut runtime: Runtime, mode: &str, agent: &str)
    -> (tempfile::TempDir, Runtime, Fake)
{
    assert!(farcooler_core::programs::find("tmux").is_some(), "native ownership proof requires real tmux");
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
        if takes or (mode == 'leftover-resolve' and event == 'Stop'):
            return json.loads(s.makefile().readline())
hook('sessionStart' if agent == 'cursor' else 'SessionStart')
prompt = pathlib.Path(os.environ['TEST_LAUNCHED_PROMPT_FILE']).read_text() if 'TEST_LAUNCHED_PROMPT_FILE' in os.environ else os.environ['TEST_LAUNCHED_PROMPT']
hook('beforeSubmitPrompt' if agent == 'cursor' else 'UserPromptSubmit', prompt=prompt)
pathlib.Path(os.environ['STARTED']).write_text(str(os.getpid()))
if mode == 'hook-disconnect':
    ready = pathlib.Path(os.environ['STARTED']).with_name('direction-ready')
    while not ready.exists():
        time.sleep(0.01)
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
    if mode in ('leftover-exit', 'leftover-resolve'):
        release = pathlib.Path(os.environ['STARTED']).with_name('release-leftover')
        argv = ['/bin/sleep', '3'] if mode == 'leftover-exit' else [
            '/bin/sh', '-c', 'i=0; while [ "$i" -lt 300 ] && [ ! -e "$1" ]; do sleep .1; i=$((i+1)); done', 'leftover', str(release)]
        job = subprocess.Popen(argv, start_new_session=True,
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        pathlib.Path(os.environ['STARTED']).with_name('leftover').write_text(str(job.pid))
        # Let the hold see the parent edge before macOS reparents this child.
        time.sleep(0.35)
        if mode == 'leftover-resolve':
            hook('Stop', last_assistant_message='Saved before quiescence.')
        sys.exit(0)
    stop = 'stop' if agent == 'cursor' else 'Stop'
    hook(stop, sid='foreign')
    hook(stop, cwd='/foreign')
    if agent == 'claude': hook(stop, agent_id='subagent')
    if mode != 'foreign-only': hook('StopFailure' if mode == 'failed' else stop, error='authentication_failed')
    time.sleep(30)
"#).unwrap();
    let CodingHarness::Command(mut process) = runtime.coding.as_ref().unwrap().harness.clone() else { unreachable!() };
    process.argv.clear();
    process.timeout_ms = match mode {
        "foreign-only" => 300,
        "hook-disconnect" => 15_000,
        _ => 5000,
    };
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
async fn ov_1308_codex_long_ticket_starts_with_bounded_window_command() {
    use farcooler_store::models::{Actor, AcceptanceItem, TaskUpdate};
    let (dir, mut runtime, _fake) = native_agent("pass", "codex-engine").await;
    // This executable receives the generated CLI arguments after both file substitutions
    // have run in the login shell. It never reads TEST_LAUNCHED_PROMPT[_FILE].
    let stand_in = dir.path().join("codex-stand-in");
    std::fs::write(&stand_in, r#"#!/usr/bin/python3
import json, os, pathlib, sys
assert sys.argv[1] == 'codex', sys.argv
assert os.getpid() == os.getpgrp(), 'the agent replaced its launch shells'
assert 'TEST_LAUNCHED_PROMPT' not in os.environ
assert 'TEST_LAUNCHED_PROMPT_FILE' not in os.environ
pathlib.Path(__file__).with_name('received.json').write_text(json.dumps(sys.argv[2:]))
os.environ['TEST_LAUNCHED_PROMPT'] = sys.argv[-1]
os.execv('/usr/bin/python3', ['/usr/bin/python3', str(pathlib.Path(__file__).with_name('interactive.py')), 'pass', 'codex'])
"#).unwrap();
    std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _stand_in = crate::service::test_agent::stand_in(&stand_in);
    TEST_ARGV.with(|p| p.borrow_mut().take());
    let host = Uuid::now_v7();
    let root = runtime.store.create_repository_root(host, "/long-ticket", 0).unwrap();
    let repo = runtime.store.create_repository(host, root.id, "long", "/long-ticket/.git", "").unwrap();
    let ws = runtime.store.ensure_main_workspace(repo.id).unwrap();
    let task = runtime.store.create_task(ws.id, "Long ticket", Actor::User).unwrap();
    let update = TaskUpdate { title: task.title.clone(), intent: "i'\"$`\n".repeat(2000),
        acceptance: vec![AcceptanceItem { id: Uuid::now_v7(), text: "a'\"$`".repeat(1600), met: false }],
        constraints: vec![], labels: vec![], worktree_id: None };
    assert_eq!(update.intent.len() + update.acceptance[0].text.len(), 20_000);
    runtime.store.update_task(task.id, task.resource_version, &update).unwrap();
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", &task.key, definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    runtime.install_codex_hooks(&binding, &runtime.tree(&binding)).await.unwrap();
    let command = runtime.coding_command(&binding).unwrap();
    assert!(command.argv.windows(2).any(|args| args == ["/bin/sh", "-c"]), "{command:?}");
    assert!(!command.environment.keys().any(|key| key.starts_with("TEST_LAUNCHED_PROMPT")));
    let window = native_launch(&command, &runtime.quarantine_receipt(&binding));
    assert!(window.len() < 8192, "{} bytes", window.len());
    assert!(window.contains("launch-brief") && window.contains("launch-hooks"));
    assert!(!window.contains("hooks="));
    let brief = runtime.root.join("receipts").join(format!("{}.launch-brief", digest(&binding.effect)));
    let text = std::fs::read_to_string(&brief).unwrap();
    assert!(text.contains(&update.intent) && text.contains(&update.acceptance[0].text));
    assert_eq!(runtime.coding_command(&binding).unwrap(), command);
    let (_cancel, orders) = tokio::sync::watch::channel(false);
    let outcome = runtime.execute_coding("w", &binding, orders).await.unwrap();
    assert!(matches!(outcome, Outcome::Passed(_)), "{outcome:?}");
    let received: Vec<String> = serde_json::from_slice(&std::fs::read(dir.path().join("received.json")).unwrap()).unwrap();
    assert_eq!(received.last(), Some(&text), "the complete published ticket reaches Codex as one argument");
    let configs: Vec<_> = received.windows(2).filter(|args| args[0] == "--config").map(|args| args[1].as_str()).collect();
    let hooks = runtime.codex_hook_override(&binding).unwrap().unwrap();
    assert!(configs.contains(&hooks.as_str()), "Codex receives the complete pinned hooks config: {configs:?}");
    let received_hooks: toml::Value = configs.iter().find(|c| c.starts_with("hooks=")).unwrap().parse().unwrap();
    let expected_hooks = crate::hook_install::native_codex_hooks(&runtime.root.join("h.sock"));
    assert_eq!(serde_json::to_value(&received_hooks["hooks"]).unwrap(), expected_hooks["hooks"]);
    assert!(!brief.exists(), "the generated shell consumed the brief file");
    // Recovery must reconstruct the same command after the shell consumes its input.
    assert_eq!(runtime.coding_command(&binding).unwrap(), command);
}

#[tokio::test]
async fn ov_1308_window_refusal_fails_with_tmux_reason() {
    for review in [false, true] {
        let (dir, mut runtime, _fake) = native_agent("pass", "codex").await;
        let mut definition = runtime.configured_definition().unwrap();
        if review {
            definition = super::super::gate_tests::definition(&runtime);
            definition.stages.retain(|s| !matches!(s.kind, StageKind::Review(Reviewer::Orchestrator(_) | Reviewer::Person(_))));
            definition.models.insert("agent-review".into(), vec!["codex".into()]);
        }
        enqueue(&mut runtime, "w", "one", definition);
        if review {
            let mut ready = false;
            for time in 2..20 {
                runtime.pump(time).await.unwrap();
                ready = runtime.store.workflow_outbox("w").unwrap().iter().any(|s|
                    s.active && s.effect.kind == EffectKind::Execute(StageKind::Review(Reviewer::Agent)));
                if ready { break; }
            }
            assert!(ready, "build and checks must pass before the review launch fails");
        }
        // Real tmux rejects this window before its launch barrier can be released.
        TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/bin/echo".into(), "x".repeat(40_000)]));
        let state = finish(&mut runtime, "w").await;
        let item = &state.items["one"];
        assert_eq!(item.phase, Phase::Failed);
        let reason = item.failure.as_ref().unwrap();
        assert!(reason.contains("native managed PTY could not open") && reason.contains("command too long"), "{reason}");
        // The workflow.get view serializes this execution state, and restart retains its reason.
        let state = runtime.execution_state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].failure.as_ref(), Some(reason));
        let view = runtime.wire_command("w", "orchestrator", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        assert_eq!(view["state"]["items"]["one"]["failure"].as_str(), Some(reason.as_str()));
        let outputs: Vec<_> = std::fs::read_dir(runtime.root.join("receipts")).unwrap()
            .filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "output")).collect();
        assert!(outputs.iter().any(|e| std::fs::read_to_string(e.path()).unwrap().contains("command too long")));
        let reason = reason.clone();
        drop(runtime);
        let reopened = Runtime::open(&dir.path().join("runtime")).unwrap();
        assert_eq!(reopened.state("w").unwrap().unwrap().items["one"].failure.as_ref(), Some(&reason));
    }
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
        assert_eq!(runtime.run_native(binding, &command, running[0].cancel.subscribe()).await.unwrap().code, Some(0));
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
async fn native_claude_configuration_refuses_missing_or_non_gating_context_hooks() {
    for gating in [None, Some(false)] {
        let (dir, runtime, _fake) = native("pass").await;
        runtime.exec.validate_native_host().unwrap();
        let mut config = runtime.coding.clone().unwrap();
        let CodingHarness::Native { settings: Some(file), .. } = &mut config.harness else { unreachable!() };
        let mut broken: serde_json::Value = serde_json::from_slice(&std::fs::read(&file.path).unwrap()).unwrap();
        match gating {
            None => { broken["hooks"].as_object_mut().unwrap().remove("PreToolUse"); }
            Some(_) => {
                let command = broken["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
                broken["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = command.replace(" --gating", "").into();
            }
        }
        let bytes = serde_json::to_vec(&broken).unwrap();
        std::fs::write(&file.path, &bytes).unwrap();
        file.digest = digest(&bytes);
        let error = runtime.exec.validate_native_configuration(&config).expect_err("missing context hook");
        assert!(error.to_string().contains("synchronous gating PreToolUse"), "{error}");
        let root = runtime.root.clone();
        std::fs::write(root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
        drop(runtime); // Release the writer lock so reopen reaches hook validation.
        assert!(matches!(Runtime::open(&root), Err(Error::Refused(reason))
            if reason.contains("synchronous gating PreToolUse")), "reopened {gating:?} hook");
        assert!(!dir.path().join("started").exists(), "a refused configuration never launches");
    }
}
#[tokio::test]
async fn workflow_coding_native_real_pty_normalized_hooks_and_quiescence() {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).try_init();
    for agent in ["claude", "codex", "cursor", "codex-engine"] {
        eprintln!("native fixture agent: {agent}");
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
        let NativeSettlement::Launched { owner, hooks, completed, .. } = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
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
async fn ov_1435_retry_keeps_committed_work_from_interrupted_attempt() {
    let (_dir, mut runtime, _fake) = native_agent("no-changes", "codex").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let tree = runtime.tree(&binding);
    adapters::git(&tree, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
        "commit", "--allow-empty", "-m", "earlier work"]).await.unwrap();
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let outcome = runtime.execute_coding("w", &binding, cancelled).await.unwrap();
    assert!(matches!(outcome, Outcome::Passed(_)), "{outcome:?}");
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
            if !dir.path().join("started").exists() { return false; }
            let id = crate::engine_steering::find(board.id, &task.key).expect("the running step is listed");
            assert_eq!(id, terminal(&binding));
            runtime.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
            std::fs::write(dir.path().join("direction-ready"), "").unwrap();
            true
        };
        let (outcome, started) = tokio::join!(runtime.execute_coding(&workspace, &binding, cancel), steer);
        assert!(started, "{agent} {mode}: the agent never started; the build ended {outcome:?}");
        assert!(matches!(outcome, Ok(Outcome::Passed(_))), "{agent} {mode}: {outcome:?}");
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
