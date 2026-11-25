//! ov-1178: accepted settings must deliver through the CLI and the real native turn host.
use super::*;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::models::Actor;

#[tokio::test]
async fn accepted_native_claude_delivers_message_and_acceptance_before_turn_ends() {
    let exe = std::env::current_exe().unwrap();
    let cli = farcooler_core::scratch::cargo_profile(&exe).unwrap().join("farcooler");
    assert!(cli.is_file(), "build the real hook CLI first: cargo build -p farcooler-cli --bin farcooler");
    let (dir, mut runtime, _fake) = tests::native("pass").await;
    runtime.exec.validate_native_host().unwrap();
    let (_service_dir, svc, repo) = crate::test_support::fixture().await;
    runtime.store = svc.store.clone(); // The native host and watcher share the runner's durable message queue.
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let primary = svc.store.create_task(ws, "native primary", Actor::Manager).unwrap();
    let task = svc.store.create_task(ws, "native grouped ticket", Actor::Manager).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());

    // Provision and admit a real build, then include a secondary ticket in this run's publication.
    let workspace = ws.to_string();
    let definition = runtime.configured_definition().unwrap();
    super::super::tests::enqueue(&mut runtime, &workspace, &primary.key, definition);
    runtime.pump(2).await.unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    let mut effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &effect.id, "native-delivery", state.revision,
        state.epoch, 10, 100).unwrap().unwrap();
    let base = runtime.pin_base(&workspace, &effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
    effect.with.push(task.key.clone());
    let binding = Binding::new(&workspace, &effect, claim.resource_generation, state.items[&primary.key].iteration, &base);

    // The validated command may use the channel's bare CLI name. Resolve it only inside this
    // scratch agent's PATH, to the binary built from this checkout, never an installed CLI.
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let hook_binary = crate::hook_install::binary_path();
    assert!(!Path::new(&hook_binary).is_absolute(), "unit harness should use the channel CLI name");
    std::os::unix::fs::symlink(&cli, bin.join(hook_binary)).unwrap();
    let agent = dir.path().join("claude");
    std::fs::copy(farcooler_core::programs::find("perl").unwrap(), &agent).unwrap();
    let script = dir.path().join("delivery.pl");
    std::fs::write(&script, include_str!("coding_native_delivery_test.pl")).unwrap();
    let CodingHarness::Native { settings: Some(settings), .. } = &runtime.coding.as_ref().unwrap().harness else { unreachable!() };
    TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec![agent.to_string_lossy().into_owned(),
        script.to_string_lossy().into_owned(), settings.path.to_string_lossy().into_owned()]));
    let mut command = runtime.exec.coding_command(&binding).unwrap();
    command.timeout_ms = 20_000;
    command.environment.insert("PATH".into(), format!("{}:/usr/bin:/bin", bin.display()));
    command.environment.insert("CLAUDE_CONFIG_DIR".into(), dir.path().join("config").to_string_lossy().into_owned());
    let log = dir.path().join("context.jsonl");
    let finish = dir.path().join("finish");
    command.environment.insert("CONTEXT_LOG".into(), log.to_string_lossy().into_owned());
    command.environment.insert("FINISH".into(), finish.to_string_lossy().into_owned());
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let run = runtime.exec.run_native(&binding, &command, cancelled);
    tokio::pin!(run);
    let delivery = async {
        let started = dir.path().join("started");
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(60),
            Duration::from_millis(20), || started.exists()).await, "native agent never submitted its prompt");
        for key in [&primary.key, &task.key] {
            assert_eq!(crate::watch_view::lookup(&workspace, key).unwrap().1.terminal_id, terminal(&binding));
        }
        let sent = watcher.message_send(pb::Request { method: "message.send".into(),
            payload: Some(request::Payload::MessageSend(pb::MessageSend {
                to: task.key.clone(), text: "Rebase while this turn is active".into(), actor: "manager".into(),
                workspace_id: ws.as_bytes().to_vec().into(), ..Default::default()
            })), ..Default::default()
        }, crate::people::Person::Owner).await.unwrap();
        assert!(matches!(sent, result::Value::MessageSent(_)));
        for text in ["Rebase while this turn is active", "Acceptance changed on"] {
            if text.starts_with("Acceptance") {
                crate::task_ops::update(&svc, &watcher, &pb::TaskUpdate {
                    task_id: task.id.as_bytes().to_vec().into(), expected_version: task.resource_version,
                    title: task.title.clone(), intent: task.intent.clone(), actor: "manager".into(),
                    acceptance: vec![pb::TaskAcceptanceItem { text: "New mid-turn requirement".into(), ..Default::default() }],
                    ..Default::default()
                }).unwrap();
            }
            assert!(wait_until_async!(tokio::time::Instant::now() + Duration::from_secs(60),
                Duration::from_millis(20), async || {
                    watcher.pump_wakes().await;
                    std::fs::read_to_string(&log).unwrap_or_default().contains(text)
                }), "CLI never delivered {text}");
            let received = std::fs::read_to_string(&log).unwrap_or_default();
            let replies: Vec<serde_json::Value> = received.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            let field = if text.starts_with("Acceptance") { "additionalContext" } else { "permissionDecisionReason" };
            let contexts: Vec<_> = replies.iter().filter(|r|
                r["hookSpecificOutput"][field].as_str().is_some_and(|c| c.contains(text))).collect();
            assert_eq!(contexts.len(), 1, "CLI never delivered {text}: {received}");
            assert_eq!(contexts[0]["hookSpecificOutput"]["hookEventName"], "PreToolUse");
            if text.starts_with("Acceptance") {
                assert!(contexts[0]["hookSpecificOutput"].get("permissionDecision").is_none());
            } else {
                assert_eq!(contexts[0]["hookSpecificOutput"]["permissionDecision"], "deny");
            }
            assert!(svc.store.pending_message_wakes().unwrap().is_empty());
            assert!(!finish.exists(), "delivery must precede Stop");
            let pid: i32 = std::fs::read_to_string(&started).unwrap().parse().unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the native turn must still be running");
        }
        std::fs::write(&finish, "end the turn").unwrap();
    };
    tokio::select! {
        result = &mut run => panic!("native turn ended before both deliveries: {result:?}"),
        _ = delivery => {}
    }
    assert_eq!(run.await.unwrap(), Some(0));
    let settled: NativeSettlement = serde_json::from_slice(&std::fs::read(runtime.exec.native_path(&binding, "settled")).unwrap()).unwrap();
    let NativeSettlement::Launched { hooks, .. } = settled else { panic!("never launched") };
    assert!(hooks.iter().any(|h| h.event == "PreToolUse" && h.takes.iter().any(|t| t == "additional_context")));
    assert!(hooks.iter().any(|h| h.event == "Stop"));
    for key in [&primary.key, &task.key] { assert!(crate::watch_view::lookup(&workspace, key).is_none()); }
}

#[tokio::test]
async fn ov_1464_closed_tool_hook_does_not_end_the_build() {
    use farcooler_agent_hooks::wire::TAKES_UPDATED_INPUT;
    use std::io::Write;

    let (dir, mut runtime, _fake) = tests::native_agent("disconnected-hook", "codex").await;
    let host = Uuid::now_v7();
    let repository = runtime.repository();
    let root = runtime.store.create_repository_root(host, dir.path().to_str().unwrap(), 0).unwrap();
    let repo = runtime.store.create_repository(host, root.id, "Native", repository.join(".git").to_str().unwrap(), "").unwrap();
    let board = runtime.store.ensure_main_workspace(repo.id).unwrap();
    let task = runtime.store.create_task(board.id, "Steer this", Actor::User).unwrap();
    let workspace = board.id.to_string();
    let definition = runtime.configured_definition().unwrap();
    super::super::tests::enqueue(&mut runtime, &workspace, &task.key, definition);
    runtime.pump(2).await.unwrap();
    let binding = super::super::coding_tests::admitted_in(&runtime, &workspace).await;
    let (_cancel, cancel) = tokio::sync::watch::channel(false);
    let run = runtime.execute_coding(&workspace, &binding, cancel);
    tokio::pin!(run);

    let id = terminal(&binding);
    let started = dir.path().join("started");
    let steer = async {
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_millis(10), || started.exists()).await, "the build did not start");
        runtime.store.send_manual_message(task.id, Actor::Manager, "Use the new API", Some(id), serde_json::json!({})).unwrap();
        // Emulate a CLI that has sent its gating request and dropped the socket before the reply.
        // Perform send and close in one poll so the runner cannot win the race to reply.
        let mut socket = std::os::unix::net::UnixStream::connect(runtime.native_socket(&binding)).unwrap();
        let line = HookLine { agent: Agent::Codex, event: "PreToolUse".into(), takes: vec![TAKES_UPDATED_INPUT.into()],
            payload: serde_json::json!({"session_id": id, "cwd": runtime.tree(&binding), "tool_name": "Write"}), launch: None };
        let mut frame = serde_json::to_vec(&line).unwrap();
        frame.push(b'\n');
        socket.write_all(&frame).unwrap();
        drop(socket);
        assert!(super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_millis(10), || runtime.store.notes_for(task.id, None).unwrap().iter()
                .any(|n| n.body.contains("Couldn't confirm the engine agent got the message"))).await,
            "the failed delivery must be recorded");
        std::fs::write(dir.path().join("direction-ready"), "").unwrap();
    };
    tokio::select! {
        outcome = &mut run => panic!("the disconnected hook ended the build: {outcome:?}"),
        _ = steer => {}
    }
    let outcome = run.await;
    assert!(matches!(outcome, Ok(Outcome::Passed(_))), "the same build must commit and pass: {outcome:?}");
    assert!(runtime.store.pending_message_wakes().unwrap().is_empty());
}
