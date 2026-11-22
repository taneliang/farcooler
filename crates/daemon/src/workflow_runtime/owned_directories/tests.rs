use std::path::Path;
use crate::workflow_runtime::{CodingReviewer, Command, Grant, Phase, Reviewer, StageKind};
use crate::workflow_runtime::coding_verbs_tests::{at_join, batched_push, command, lander, phase_of, until};

fn available(path: &Path) -> u64 {
    let stat = rustix::fs::statvfs(path).unwrap();
    stat.f_bavail * stat.f_frsize
}

#[tokio::test]
#[ignore = "requires real Codex login, scratch credentials and a built farcooler CLI"]
async fn native_codex_build_review_and_landing_release_owned_output() {
    use crate::workflow_runtime::{AGENT_REVIEWER, CodingConfigExt, CodingHarness, Progression, Stage, adapters};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    assert_eq!(std::env::var("FARCOOLER_LIVE_AGENTS").as_deref(), Ok("1"));
    let auth = std::env::var_os("OV1422_CODEX_AUTH").expect("path to scratch Codex auth.json");
    let rustup = std::env::var("OV1422_RUSTUP_HOME").expect("path to installed Rust toolchains");
    let cargo = farcooler_core::programs::find("cargo").unwrap();
    let tool_path = format!("{}:/opt/homebrew/bin:/usr/bin:/bin", cargo.parent().unwrap().display());
    let codex = farcooler_core::programs::find("codex").unwrap();
    let (dir, mut runtime) = lander("pass").await;
    let repo = runtime.repository();
    std::fs::create_dir(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"disk-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(repo.join("src/main.rs"), "fn main() { println!(\"before\"); }\n#[test]\nfn smoke() { main(); }\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "/target/\n/Cargo.lock\n").unwrap();
    adapters::git(&repo, &["add", "Cargo.toml", "src/main.rs", ".gitignore"]).await.unwrap();
    adapters::git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "commit", "-qm", "Rust project"]).await.unwrap();
    let config = runtime.coding.as_mut().unwrap();
    config.base = adapters::git(&repo, &["rev-parse", "HEAD"]).await.unwrap();
    adapters::git(&repo, &["update-ref", &config.landing_ref, &config.base]).await.unwrap();
    let remote = &config.landing.as_ref().unwrap().repository;
    let pushed = tokio::process::Command::new("/usr/bin/git").current_dir(&repo)
        .args(["-c", "core.hooksPath=/dev/null", "push", remote.to_str().unwrap(), "HEAD:refs/heads/main"])
        .status().await.unwrap();
    assert!(pushed.success());

    let (dir, mut runtime, fake) = super::super::coding_native::tests::native_in(dir, runtime, "pass", "codex-engine").await;
    drop(fake);
    eprintln!("Native Codex probe in {}", dir.path().display());
    let home = dir.path().join("codex-home");
    std::fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let credentials = home.join("auth.json");
    std::fs::copy(auth, &credentials).unwrap();
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(home.join("config.toml"), format!(
        "sandbox_mode = \"workspace-write\"\napproval_policy = \"never\"\nmodel_reasoning_effort = \"low\"\n[projects.{}]\ntrust_level = \"trusted\"\n",
        serde_json::to_string(&repo).unwrap())).unwrap();
    let exe = std::env::current_exe().unwrap();
    let profile = farcooler_core::scratch::cargo_profile(&exe).unwrap();
    let cli = profile.join("farcooler");
    assert!(cli.is_file(), "build the hook and verdict CLI with cargo build -p farcooler-cli");
    let bin = dir.path().join("hook-bin");
    std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(&cli, bin.join(farcooler_protocol::CHANNEL.cli_binary_name())).unwrap();
    let launch = dir.path().join("launch-codex");
    let quote = crate::service::shell_quote;
    std::fs::write(&launch, format!("#!/bin/sh\nshift\nexec {} --no-daemon --no-alt-screen --dangerously-bypass-hook-trust \"$@\"\n",
        quote(codex.to_str().unwrap()))).unwrap();
    std::fs::set_permissions(&launch, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _launch = crate::service::test_agent::stand_in(&launch);

    let config = runtime.coding.as_mut().unwrap();
    let CodingHarness::Native { preset, prompt, process, .. } = &mut config.harness else { unreachable!() };
    *preset = "codex:gpt-6.2-sol".into();
    *prompt = concat!(
        "This is a disposable Rust repository for a disk cleanup probe. Change only the word before to after in src/main.rs. ",
        "Run CARGO_TARGET_DIR=\"$FARCOOLER_SCRATCH/target\" cargo build --offline. Confirm the compiled binary in scratch prints after. ",
        "Then run touch \"$FARCOOLER_SCRATCH/build-ready\" and wait for \"$FARCOOLER_SCRATCH/committed\" before ending your turn. ",
        "The fixture will commit src/main.rs on your behalf. Do not access credentials, other projects, or external tools."
    ).into();
    process.timeout_ms = 120_000;
    process.environment.extend([
        ("CODEX_HOME".into(), home.to_str().unwrap().into()),
        ("HOME".into(), dir.path().to_str().unwrap().into()),
        ("PATH".into(), format!("{}:{tool_path}", bin.display())),
        ("RUSTUP_HOME".into(), rustup.clone()),
    ]);
    for check in &mut config.checks {
        check.argv = vec!["/bin/sh".into(), "-ec".into(), format!(
            "grep -Fq 'println!(\"after\");' src/main.rs\n{} test --offline\nif [ -n \"${{FARCOOLER_LANE_GATE_LOG_DIR:-}}\" ]; then printf passed > \"$FARCOOLER_LANE_GATE_LOG_DIR/rust.log\"; fi",
            quote(cargo.to_str().unwrap()))];
        check.environment.extend([("HOME".into(), dir.path().to_str().unwrap().into()),
            ("RUSTUP_HOME".into(), rustup.clone()), ("PATH".into(), tool_path.clone())]);
        check.timeout_ms = 30_000;
    }
    let mut definition = batched_push(&runtime, "any", 5000);
    let kind = StageKind::Review(Reviewer::Agent);
    let mut operation = runtime.coding.as_ref().unwrap().operation("coding-review/v1", AGENT_REVIEWER);
    operation.arguments.push(crate::review_agent::ReviewOptions {
        prompt: Some(format!("Verify only src/main.rs changed, replacing before with after. Run CARGO_TARGET_DIR=\"$FARCOOLER_SCRATCH/target\" cargo test --offline, then record your verdict with {} workflow verdict --passed true (or use --passed false and a summary of any findings). Do not access credentials, other projects, or external tools.", quote(cli.to_str().unwrap()))),
        ..Default::default()
    }.argument());
    definition.stages.insert(3, Stage { id: "agent-review".into(), kind, progression: Progression::Automatic,
        max_attempts: 1, operation, routes: Default::default(), limit: 0 });

    let before = available(&runtime.root);
    super::super::concurrency_tests::enqueue_item(&mut runtime, "w", "one", definition);
    let store = runtime.store.clone();
    let tree = runtime.item_tree("w", "one");
    // Codex protects the linked worktree's Git metadata even when the source
    // tree is writable. Commit the builder's checked change from the fixture.
    let commit = async {
        let scratch = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            loop {
                for record in store.workflow_cleanups("w").unwrap() {
                    if let Some(directory) = record.effect.directory {
                        let path = std::path::PathBuf::from(directory.path);
                        if path.join("build-ready").is_file() { return path; }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }).await.expect("Codex wrote the marker after compiling in scratch");
        assert!(scratch.join("target/debug/disk-probe").is_file());
        adapters::git(&tree, &["add", "--", "src/main.rs"]).await.unwrap();
        adapters::git(&tree, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "commit", "-qm", "Probe candidate"]).await.unwrap();
        std::fs::write(scratch.join("committed"), "").unwrap();
    };
    let (state, ()) = tokio::join!(
        until(&mut runtime, |state| matches!(phase_of(state, "one"), Some(Phase::WaitingJoin | Phase::Failed | Phase::Uncertain))),
        commit,
    );
    if phase_of(&state, "one") != Some(Phase::WaitingJoin) {
        for entry in std::fs::read_dir(runtime.root.join("receipts")).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "output") {
                eprintln!("{}", String::from_utf8_lossy(&std::fs::read(entry.path()).unwrap()));
            }
        }
        panic!("native Codex did not pass build and review: {:?}", phase_of(&state, "one"));
    }
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let scratch: Vec<_> = records.iter().filter(|record| record.effect.directory.is_some()).collect();
    assert_eq!(scratch.len(), 2);
    for record in scratch {
        let directory = record.effect.directory.as_ref().unwrap();
        let bytes = record.result.as_ref().unwrap().bytes_freed;
        assert!(bytes > 100_000, "expected compiled output, found {bytes} bytes");
        assert!(!Path::new(&directory.path).exists());
        eprintln!("Codex scratch removed: {} ({bytes} bytes)", directory.path);
    }
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    until(&mut runtime, |state| phase_of(state, "one") == Some(Phase::Done)).await;
    let after = available(&runtime.root);
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let total: u64 = records.iter().filter_map(|r| r.result.as_ref()).map(|r| r.bytes_freed).sum();
    assert!(total > 300_000);
    let landed = super::super::coding_verbs_tests::remote_head(&runtime).await;
    assert_eq!(adapters::git(&repo, &["show", &format!("{landed}:src/main.rs")]).await.unwrap().lines().next(),
        Some("fn main() { println!(\"after\"); }"));
    eprintln!("ov-1422 native Codex disk bytes available before={before} after={after}; cleanup reported={total}; landed={landed}");
}

#[tokio::test]
async fn build_review_send_back_and_landing_release_owned_output() {
    let (dir, mut runtime) = lander("pass").await;
    let repo = runtime.repository();
    std::fs::write(repo.join(".git/info/exclude"), "target/\n").unwrap();
    let adapter = dir.path().join("adapter.py");
    let script = std::fs::read_to_string(&adapter).unwrap()
        .replace("if stage == 'build':\n", concat!(
            "if stage == 'build':\n",
            "    scratch = pathlib.Path(os.environ['FARCOOLER_SCRATCH'])\n",
            "    assert scratch.is_dir() and not list(scratch.iterdir())\n",
            "    (scratch / 'objects').write_bytes(b'b' * (32 << 20))\n",
            "    assert not pathlib.Path('target/bin').exists()\n",
            "    pathlib.Path('target').mkdir(exist_ok=True)\n",
            "    pathlib.Path('target/bin').write_bytes(b't' * (8 << 20))\n"))
        .replace("elif stage == 'check':\n", "elif stage == 'check':\n    assert pathlib.Path('target/bin').stat().st_size == 8 << 20\n")
        .replace("elif stage == 'review':\n", concat!(
            "elif stage == 'review':\n",
            "    scratch = pathlib.Path(os.environ['FARCOOLER_SCRATCH'])\n",
            "    assert scratch.is_dir() and not list(scratch.iterdir())\n",
            "    (scratch / 'review').write_bytes(b'r' * (16 << 20))\n"))
        .replace("passed=iteration != '0' and mode != 'review-reject'", "passed=True")
        .replace("elif stage == 'gate':\n", concat!(
            "elif stage == 'gate':\n",
            "    logs = pathlib.Path(os.environ['FARCOOLER_LANE_GATE_LOG_DIR'])\n",
            "    assert logs.is_dir()\n",
            "    (logs / 'gate.log').write_bytes(b'log')\n",
            "    pathlib.Path('target').mkdir(exist_ok=True)\n",
            "    pathlib.Path('target/landing').write_bytes(b'l' * (32 << 20))\n"));
    std::fs::write(&adapter, script).unwrap();

    let config = runtime.coding.as_mut().unwrap();
    let mut review = config.checks[0].clone();
    review.argv[2] = "review".into();
    config.reviewer = Some(CodingReviewer { identity: "reviewer".into(), command: review });
    let mut definition = batched_push(&runtime, "any", 5000);
    let mut review = runtime.configured_definition().unwrap().stages.into_iter()
        .find(|stage| stage.kind == StageKind::Review(Reviewer::Dispatch)).unwrap();
    review.progression = farcooler_workflow::Progression::Automatic;
    definition.stages.insert(3, review);

    let before = available(&runtime.root);
    at_join(&mut runtime, "one", &definition).await;
    let after_first = runtime.store.workflow_cleanups("w").unwrap();
    let scratch: Vec<_> = after_first.iter().filter(|record| record.result.as_ref().is_some_and(|result|
        matches!(result.bytes_freed, 33_554_432 | 16_777_216))).collect();
    assert_eq!(scratch.len(), 2);
    for record in scratch {
        assert!(!Path::new(&record.effect.directory.as_ref().unwrap().path).exists());
        assert_eq!(Path::new(&record.effect.directory.as_ref().unwrap().path).parent(),
            Some(Path::new("/tmp").canonicalize().unwrap().as_path()));
    }
    let tree = runtime.item_tree("w", "one");
    assert!(!tree.join("target").exists());

    let sent_back = runtime.internal("w", Command::SendBack {
        item: "one".into(), reason: "Exercise a cold build".into(),
    }, Grant::Controller, 2).unwrap();
    assert!(sent_back.result.is_ok(), "{sent_back:?}");
    until(&mut runtime, |state| phase_of(state, "one") == Some(Phase::WaitingJoin)).await;
    assert!(!tree.join("target").exists(), "the rebuilt candidate passed its check and released target again");
    assert!(command(&runtime, Command::Queue { item: "one".into() }).result.is_ok());
    let state = until(&mut runtime, |state| phase_of(state, "one") == Some(Phase::Done)).await;
    let group = state.items.iter().find(|(_, item)| item.group_of.is_some()).map(|(id, _)| id)
        .or_else(|| state.archive.iter().find(|(_, item)| item.group.is_some()).map(|(id, _)| id)).unwrap();
    assert_eq!(phase_of(&state, group), Some(Phase::Done));
    assert!(!runtime.item_tree("w", group).exists());
    let after = available(&runtime.root);
    let records = runtime.store.workflow_cleanups("w").unwrap();
    let gate_logs = records.iter().filter(|r| r.effect.item == *group)
        .find_map(|r| r.effect.directory.as_ref()).unwrap();
    assert_eq!(std::fs::read(Path::new(&gate_logs.path).join("gate.log")).unwrap(), b"log");
    let total: u64 = records.iter().filter_map(|r| r.result.as_ref()).map(|r| r.bytes_freed).sum();
    assert!(total >= (128 << 20), "build, review and landing cleanup reported {total} bytes");
    eprintln!("ov-1422 disk bytes available before={before} after={after}; cleanup reported={total}");

    let unknown = runtime.root.join("lane-gate-logs-unrecorded");
    std::fs::create_dir(&unknown).unwrap();
    runtime.release_builds_at(&["w".into()], gate_logs.retain_until_ms - 1, || Ok(())).await.unwrap();
    assert!(Path::new(&gate_logs.path).exists());
    runtime.release_builds_at(&["w".into()], gate_logs.retain_until_ms, || Ok(())).await.unwrap();
    assert!(!Path::new(&gate_logs.path).exists());
    assert!(unknown.exists());
    let board = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
    assert!(board["build_cleanup"].as_array().unwrap().iter().any(|row|
        row["effect"]["item"] == *group && row["result"]["bytes_freed"] == 3));
}
