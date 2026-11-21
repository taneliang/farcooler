use super::*;

#[tokio::test]
async fn workflow_get_names_the_paused_holder_after_five_minutes_in_the_heavy_queue() {
    let (dir, mut runtime) = lander("gate-slow").await;
    let queue = runtime.root.join("lane-gate.lock.queue");
    std::fs::create_dir(&queue).unwrap();
    let mut holder = tokio::process::Command::new("/bin/sleep").arg("60").kill_on_drop(true).spawn().unwrap();
    let pid = holder.id().unwrap();
    unsafe { libc::kill(pid as i32, libc::SIGSTOP); }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !crate::proc_table::stopped_or_gone(pid as i32) {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let holder_ticket = queue.join(format!("{:020}-{pid}.ticket", 1));
    std::fs::write(&holder_ticket, format!("worktree /fixture/paused-builder, PID {pid}, started earlier, running tests\n")).unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    let state = until(&mut runtime, |s| s.items.values().any(|g|
        g.group_of.is_some() && g.definition.stages[g.stage].kind == StageKind::Run)).await;
    let group = group_id(&state);
    let read = runtime.exec.clone();
    let ticket_dir = queue.clone();
    let effect = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s|
        s.active && s.effect.item == group && s.effect.kind == EffectKind::Execute(StageKind::Run)).unwrap().effect;
    let owner = runtime.root.join("receipts").join(format!("{}.owner.json", digest(&effect)));
    let diagnosis = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        let owner: crate::effect_runner::Owner = loop {
            if let Ok(raw) = std::fs::read(&owner) { break serde_json::from_slice(&raw).unwrap(); }
            assert!(tokio::time::Instant::now() < deadline, "gate failed to launch in {:?}", dir.path());
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let waiter = ticket_dir.join(format!("{now:020}-{}.ticket", owner.pid));
        std::fs::write(&waiter, "landing ticket").unwrap();
        let get = || read.read_get("w").unwrap()["state"]["items"][&group]["execution"]["heavy_lock_wait"].clone();
        assert!(get().is_null(), "a recent wait stays quiet");
        std::fs::rename(waiter, ticket_dir.join(format!("{:020}-{}.ticket", now - Duration::from_secs(301).as_nanos(), owner.pid))).unwrap();
        let message = get();
        std::fs::remove_file(holder_ticket).unwrap();
        assert!(get().is_null(), "the diagnostic clears when the holder releases the ticket");
        message
    };
    let message = tokio::select! {
        answer = diagnosis => answer,
        result = runtime.pump(1) => panic!("gate ended before the wait could be inspected: {result:?}"),
    };
    holder.kill().await.unwrap();
    let message = message.as_str().unwrap_or_default();
    assert!(message.contains(&format!("PID {pid}")) && message.contains("/fixture/paused-builder")
        && message.contains("paused"), "expected the paused holder, got {message:?}");
}
