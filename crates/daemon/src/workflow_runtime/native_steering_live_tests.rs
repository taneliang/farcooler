//! Opt-in probe against the installed Codex, using only a disposable repository and Codex home.
use super::*;
use farcooler_store::models::Actor;

#[tokio::test]
#[ignore = "requires a real Codex login and FARCOOLER_LIVE_AGENTS=1"]
async fn running_codex_receives_ticket_direction_before_its_first_stop() {
    assert_eq!(std::env::var("FARCOOLER_LIVE_AGENTS").as_deref(), Ok("1"));
    let auth = PathBuf::from(std::env::var_os("OV1205_CODEX_AUTH").expect("path to a scratch auth.json"));
    let codex = farcooler_core::programs::find("codex").expect("installed Codex");
    let (dir, mut runtime, _fake) = super::coding_native::tests::native_agent("pass", "codex-engine").await;
    let home = dir.path().join("codex-home"); std::fs::create_dir(&home).unwrap();
    std::fs::copy(auth, home.join("auth.json")).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"gpt-6.1-sol\"\nmodel_reasoning_effort = \"low\"\n").unwrap();
    let exe = std::env::current_exe().unwrap();
    let profile = farcooler_core::scratch::cargo_profile(&exe).unwrap();
    let bin = dir.path().join("hook-bin"); std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(profile.join("farcooler"), bin.join(farcooler_protocol::CHANNEL.cli_binary_name())).unwrap();
    let git_dir = runtime.repository().join(".git");
    let CodingHarness::Native { prompt, process, .. } = &mut runtime.coding.as_mut().unwrap().harness else { unreachable!() };
    *prompt = "Controlled steering test. First run a shell tool: touch probe-started; sleep 10. Then use a SEPARATE shell tool to write ORIGINAL to result.txt. Remove probe-started and commit result.txt with message probe. Follow any subsequent direction, including a tool-denial reason. Do not delegate, read tickets, use network tools, or edit other files.".into();
    process.timeout_ms = 120_000;
    process.environment.extend([
        ("CODEX_HOME".into(), home.to_string_lossy().into_owned()),
        ("HOME".into(), dir.path().to_string_lossy().into_owned()),
        ("OV1205_GIT_DIR".into(), git_dir.to_string_lossy().into_owned()),
        ("PATH".into(), format!("{}:/opt/homebrew/bin:/usr/bin:/bin", bin.display())),
    ]);
    let (_board, svc, repo) = crate::test_support::fixture().await;
    runtime.store = svc.store.clone();
    let board = svc.store.ensure_main_workspace(repo).unwrap();
    let task = svc.store.create_task(board.id, "Live Codex steering", Actor::User).unwrap();
    let workspace = board.id.to_string();
    let definition = runtime.configured_definition().unwrap();
    super::tests::enqueue(&mut runtime, &workspace, &task.key, definition);
    runtime.pump(2).await.unwrap();
    let state = runtime.state(&workspace).unwrap().unwrap();
    let effect = runtime.store.workflow_outbox(&workspace).unwrap().into_iter().find(|s| s.active).unwrap().effect;
    let claim = runtime.store.workflow_claim_effect(&workspace, &effect.id, "live-steering", state.revision, state.epoch, 10, 100).unwrap().unwrap();
    let base = runtime.pin_base(&workspace, &effect).await.unwrap();
    assert!(runtime.store.workflow_admit_effect(&claim, 10).unwrap());
    let binding = Binding::new(&workspace, &effect, claim.resource_generation, 0, &base);
    let tree = runtime.tree(&binding);
    std::fs::write(home.join("config.toml"), format!("model = \"gpt-6.1-sol\"\nmodel_reasoning_effort = \"low\"\n[projects.{}]\ntrust_level = \"trusted\"\n", serde_json::to_string(&tree.to_string_lossy()).unwrap())).unwrap();
    use std::io::Write;
    writeln!(std::fs::OpenOptions::new().append(true).open(home.join("config.toml")).unwrap(), "[projects.{}]\ntrust_level = \"trusted\"", serde_json::to_string(&runtime.repository().to_string_lossy()).unwrap()).unwrap();
    eprintln!("scratch probe root: {}", dir.path().display());
    let hooks = runtime.codex_hook_override(&binding).unwrap().unwrap();
    super::coding_native::TEST_ARGV.with(|p| *p.borrow_mut() = Some(vec!["/bin/sh".into(), "-c".into(),
        format!("exec {} --no-daemon --no-alt-screen --dangerously-bypass-hook-trust --approve-for-me --add-dir \"$OV1205_GIT_DIR\" --config {} \"$TEST_LAUNCHED_PROMPT\"", crate::service::shell_quote(&codex.to_string_lossy()), crate::service::shell_quote(&hooks))]));
    let watcher = crate::watch::Watcher::new(svc.clone());
    let (_cancel, cancel) = tokio::sync::watch::channel(false);
    let steer = async {
        tokio::time::timeout(Duration::from_secs(90), async {
            while !tree.join("probe-started").exists() { tokio::time::sleep(Duration::from_millis(50)).await; }
        }).await.expect("Codex ran its first tool");
        let reply = watcher.message_send(farcooler_protocol::v1::Request { method:"message.send".into(),
            payload:Some(farcooler_protocol::v1::request::Payload::MessageSend(farcooler_protocol::v1::MessageSend {
                workspace_id:crate::wire::id_bytes(board.id), to:task.key.clone(), actor:"user".into(),
                text:"Write STEERED instead of ORIGINAL to result.txt before committing. Commit only result.txt.".into(),
                ..Default::default() })), ..Default::default() }, crate::people::Person::Owner).await.unwrap();
        assert!(matches!(reply, farcooler_protocol::v1::result::Value::MessageSent(_)));
        eprintln!("scratch direction queued while the first tool is running");
    };
    let (outcome, ()) = tokio::join!(runtime.execute_coding(&workspace, &binding, cancel), steer);
    eprintln!("scratch live output: {}", std::fs::read_to_string(runtime.process_log(&binding)).unwrap_or_default());
    let outcome = outcome.unwrap();
    eprintln!("scratch live outcome: {outcome:?}");
    let result = std::fs::read_to_string(tree.join("result.txt")).unwrap();
    let receipt = runtime.root.join("receipts").join(format!("{}.native-settled.json", digest(&binding.effect)));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    let hooks = value["hooks"].as_array().unwrap();
    eprintln!("scratch hook events: {:?}", hooks.iter().map(|h| &h["event"]).collect::<Vec<_>>());
    assert_eq!(result.trim(), "STEERED");
    assert!(matches!(outcome, Outcome::Passed(_)));
    assert!(hooks.iter().any(|h| h["event"] == "PreToolUse"), "direction needs a tool boundary");
    assert_eq!(hooks.iter().filter(|h| h["event"] == "Stop").count(), 1, "no Stop continuation needed");
    assert!(svc.store.pending_message_wakes().unwrap().is_empty());
}
