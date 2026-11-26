use super::*;
use super::super::recovery_backoff::FIRST_MS;

#[tokio::test]
async fn an_uncertain_push_settles_automatically_from_the_bare_target() {
    for landed in [false, true] {
        let (_dir, mut runtime, group) = uncertain_push(landed).await;
        let first = remote_head(&runtime).await;
        let initial = runtime.state("w").unwrap().unwrap();
        let candidate = initial.items[&group].candidate.clone().unwrap();
        assert_eq!(first == candidate, landed);
        runtime.pump(50 + FIRST_MS - 1_000).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items[&group].phase, Phase::Uncertain);
        runtime.pump(50 + FIRST_MS + 1_000).await.unwrap();
        let resolved = runtime.state("w").unwrap().unwrap();
        assert_ne!(resolved.items[&group].phase, Phase::Uncertain);
        assert_eq!(resolved.items[&group].pushed.as_deref(), landed.then_some(candidate.as_str()));
        assert_eq!(remote_head(&runtime).await, first, "the probe only reads the target");
        assert_eq!(outbox(&runtime, StageKind::Push), 1);
        let finished = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
        assert_eq!(candidate_of(&finished, "one"), Some(remote_head(&runtime).await.as_str()));
        assert_eq!(outbox(&runtime, StageKind::Push), if landed { 1 } else { 2 });
    }
}

#[tokio::test]
async fn an_uncertain_push_settles_after_the_bare_target_advances() {
    let (_dir, mut runtime, group) = uncertain_push(true).await;
    let candidate = remote_head(&runtime).await;
    let landing = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
    let tree = format!("{candidate}^{{tree}}");
    let head = adapters::git(&landing.repository, &["-c", "user.name=Test",
        "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false",
        "commit-tree", &tree, "-p", &candidate, "-m", "another landing"]).await.unwrap();
    adapters::git(&landing.repository, &["update-ref", &landing.target, &head, &candidate]).await.unwrap();

    runtime.pump(50 + FIRST_MS + 1_000).await.unwrap();
    let resolved = runtime.state("w").unwrap().unwrap();
    assert_ne!(resolved.items[&group].phase, Phase::Uncertain);
    assert_eq!(resolved.items[&group].pushed.as_deref(), Some(candidate.as_str()));
    assert_eq!(remote_head(&runtime).await, head);
    assert_eq!(outbox(&runtime, StageKind::Push), 1);
}

#[tokio::test]
async fn an_automatic_push_check_keeps_uncertain_when_the_bare_target_is_unreadable() {
    let (dir, mut runtime, group) = uncertain_push(false).await;
    let remote = &runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository;
    let moved = dir.path().join("remote-away.git");
    std::fs::rename(remote, &moved).unwrap();
    runtime.pump(50 + FIRST_MS + 1_000).await.unwrap();
    let state = runtime.state("w").unwrap().unwrap();
    assert_eq!(state.items[&group].phase, Phase::Uncertain);
    assert!(state.items[&group].pushed.is_none());
    assert!(state.items[&group].failure.as_deref().unwrap().contains("cannot be read"));
    assert_eq!(outbox(&runtime, StageKind::Push), 1);
}

#[tokio::test]
async fn a_delayed_bare_receive_can_land_after_the_runner_crashes() {
    const CRASH_ROOT: &str = "FARCOOLER_TEST_DELAYED_PUSH_ROOT";
    if let Some(root) = std::env::var_os(CRASH_ROOT) {
        let runtime = Runtime::open(Path::new(&root)).unwrap();
        let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.active).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(),
            state.items[&stored.effect.item].iteration, &runtime.item_base("w", &stored.effect).unwrap());
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        panic!("the runner should have been killed: {:?}", runtime.execute_coding("w", &binding, cancel).await);
    }

    let (_dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let binding = admitted(&runtime).await;
    let candidate = binding.effect.candidate.as_deref().unwrap();
    let first = remote_head(&runtime).await;
    let bare = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap().repository.clone();
    let hook = bare.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\n: > push-paused\nfor i in $(seq 1500); do\n  test ! -f push-release || exit 0\n  sleep 0.02\ndone\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let root = runtime.root.clone();
    drop(runtime);
    let mut sender = tokio::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(format!("{}::a_delayed_bare_receive_can_land_after_the_runner_crashes",
            module_path!().split_once("::").unwrap().1))
        .env(CRASH_ROOT, &root).kill_on_drop(true).spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !bare.join("push-paused").exists() {
            assert!(sender.try_wait().unwrap().is_none(), "the sender exited before Git reached the hook");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    sender.kill().await.unwrap();

    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(50).await.unwrap();
    let group = binding.effect.item.clone();
    let now = 50 + FIRST_MS + 1_000;
    runtime.pump(now).await.unwrap();
    let checked = runtime.state("w").unwrap().unwrap();
    let target_while_paused = remote_head(&runtime).await;

    std::fs::write(bare.join("push-release"), b"").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while remote_head(&runtime).await != candidate {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(target_while_paused, first);
    assert_eq!(checked.items[&group].phase, Phase::Uncertain);
    assert!(checked.items[&group].failure.as_deref().unwrap().contains("may still be running"));

    runtime.pump(now + 3 * FIRST_MS).await.unwrap();
    let finished = until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    assert_eq!(candidate_of(&finished, "one"), Some(candidate));
    assert_eq!(outbox(&runtime, StageKind::Push), 1);
}
