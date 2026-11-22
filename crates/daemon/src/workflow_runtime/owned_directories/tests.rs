use std::path::Path;
use crate::workflow_runtime::{CodingReviewer, Command, Grant, Phase, Reviewer, StageKind};
use crate::workflow_runtime::coding_verbs_tests::{at_join, batched_push, command, lander, phase_of, until};

fn available(path: &Path) -> u64 {
    let stat = rustix::fs::statvfs(path).unwrap();
    stat.f_bavail * stat.f_frsize
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
