//! External completion is checked against this run's pinned landing target, never a caller's branch.
use super::*;
use farcooler_protocol::workflow_commands::Action;

impl Runtime {
    pub(super) async fn verify_close(&self, workspace: &str, principal: &str, action: &Action) -> Result<bool> {
        let Action::Close { mutation, item, run, pin, resolution: CloseResolution::Landed { sha } } = action else { return Ok(false) };
        let payload = serde_json::to_string(&(principal, action))?;
        let id = format!("wire:{principal}:{}", mutation.id);
        // Replaying an accepted command needs no network, even after the run was archived.
        if self.store.workflow_wire_receipt(workspace, &id, &payload)?.is_some() { return Ok(false); }
        let Some(state) = self.state(workspace)? else { return Ok(false) };
        if state.revision != mutation.revision || state.epoch != mutation.epoch { return Ok(false); }
        let Some(live) = state.items.get(item).filter(|i| &i.run == run && &i.pin == pin) else { return Ok(false) };
        if !coding::sha(sha) || live.stopping.is_some() { return Ok(false); }
        let mut exec = self.exec.clone();
        if self.service_mode {
            let bundle = self.store.workflow_bundle(&live.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
            exec.coding = bundle.adapter.map(|farcooler_protocol::workflow_config::AdapterConfiguration::Coding(config)| config);
        }
        let log = exec.root.join("receipts").join(format!("{}.close.fetch.output", digest(&payload)));
        // Failure to read the target grants no proof. The wire writer records an Evidence refusal.
        let landed = match exec.coding.as_ref().and_then(|c| c.landing.as_ref()) {
            Some(identity) => exec.on_target(identity, sha, &log).await,
            None => super::coding_verbs::on_remote(&exec.repository(), exec.landing_ref(), sha).await,
        };
        Ok(landed.unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    use super::super::{coding_tests::configured, tests::{enqueue, finish}};

    fn close(state: &State, id: &str, resolution: CloseResolution) -> Action {
        let item = &state.items["old"];
        Action::Close { mutation: Mutation { id:id.into(), revision:state.revision, epoch:state.epoch, sequence:state.revision+1 },
            item:"old".into(), run:item.run.clone(), pin:item.pin.clone(), resolution }
    }
    async fn send(runtime: &Runtime, action: Action) -> serde_json::Value {
        let proof = runtime.verify_close("w", "orchestrator", &action).await.unwrap();
        runtime.wire_command_with("w", "orchestrator", &["orchestrator".into()], None, action, super::super::wire_commands::Prepared { tree:None, close_verified:proof }).unwrap()
    }
    #[tokio::test]
    async fn workflow_close_verifies_landing_and_replays_without_reading_the_target() {
        let (dir, mut runtime) = configured(false, "check-fail").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "old", definition);
        let state = finish(&mut runtime, "w").await;
        assert_eq!(state.items["old"].phase, Phase::Failed);
        let built = state.items["old"].candidate.clone().unwrap();
        let unlanded = close(&state, "unlanded", CloseResolution::Landed { sha:built });
        assert_eq!(send(&runtime, unlanded).await["receipt"]["result"]["Err"], "Evidence");
        assert_eq!(runtime.state("w").unwrap().unwrap(), state);
        let landed = runtime.coding.as_ref().unwrap().base.clone();
        let request = close(&state, "landed", CloseResolution::Landed { sha:landed.clone() });
        // Knowing a hash on main without server verification is not enough.
        assert_eq!(runtime.wire_command("w", "orchestrator", close(&state, "unverified", CloseResolution::Landed { sha:landed.clone() }))
            .unwrap()["receipt"]["result"]["Err"], "Evidence");
        let reply = send(&runtime, request.clone()).await;
        assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
        let next = runtime.state("w").unwrap().unwrap();
        assert_eq!(next.items["old"].phase, Phase::Done);
        assert_eq!(next.items["old"].candidate.as_deref(), Some(landed.as_str()));
        let events = runtime.events("w").unwrap();
        let mut replay = State::empty("w".into(), 1);
        for event in &events { replay = step(&replay, event).unwrap().state; }
        assert_eq!(replay, next);
        drop(runtime);
        let runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        assert_eq!(send(&runtime, request.clone()).await, reply);
        let mut changed = request;
        if let Action::Close { resolution, .. } = &mut changed { *resolution = CloseResolution::Landed { sha:"f".repeat(40) }; }
        assert!(runtime.verify_close("w", "orchestrator", &changed).await.is_err(), "identity cannot change its evidence");
    }
    #[tokio::test]
    async fn workflow_close_continuation_requires_another_open_ticket_and_current_run() {
        let (_dir, mut runtime) = configured(false, "check-fail").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "old", definition);
        let state = finish(&mut runtime, "w").await;
        let absent = close(&state, "missing", CloseResolution::Continued { item:"new".into() });
        assert_eq!(send(&runtime, absent).await["receipt"]["result"]["Err"], "Evidence");
        runtime.track_tickets(&[("w".into(), "new".into(), "todo".into())]).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        let mut stale = close(&state, "stale", CloseResolution::Continued { item:"new".into() });
        if let Action::Close { run, .. } = &mut stale { *run = "old-run".into(); }
        assert_eq!(send(&runtime, stale).await["receipt"]["result"]["Err"], "Stale");
        let reply = send(&runtime, close(&state, "continued", CloseResolution::Continued { item:"new".into() })).await;
        assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["old"].phase, Phase::Cancelled);
        assert_eq!(state.items["new"].phase, Phase::WaitingApproval);
    }
    #[tokio::test]
    async fn workflow_close_stops_an_uncertain_operation_and_recovers_to_done() {
        let (dir, mut runtime) = configured(false, "ci-wrong").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "old", definition);
        for time in 2..15 { runtime.pump(time).await.unwrap(); }
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["old"].phase, Phase::Uncertain);
        let sha = runtime.coding.as_ref().unwrap().base.clone();
        let request = close(&state, "close-uncertain", CloseResolution::Landed { sha:sha.clone() });
        let reply = send(&runtime, request.clone()).await;
        assert!(reply.pointer("/receipt/result/Ok").is_some(), "{reply}");
        assert_eq!(runtime.state("w").unwrap().unwrap().items["old"].phase, Phase::Cancelling);
        drop(runtime);
        let mut runtime = Runtime::open(&dir.path().join("runtime")).unwrap();
        runtime.pump(20).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["old"].phase, Phase::Done);
        assert_eq!(state.items["old"].candidate.as_deref(), Some(sha.as_str()));
        assert_eq!(send(&runtime, request).await, reply);
    }

}
