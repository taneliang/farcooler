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
