//! Only the RPC admission seam supplies principal; the wire vocabulary has none.
use super::*;
use farcooler_protocol::workflow_commands::{Action, MAX_BYTES};
use farcooler_store::workflow_publication::Write;

impl CommandHandle {
    pub(crate) async fn wire_command(&self, workspace: String, principal: String, action: Action) -> Result<serde_json::Value> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::Wire(workspace, principal, Box::new(action), send)).await
            .map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
}
impl Runtime {
    pub(super) fn wire_command(&self, workspace: &str, principal: &str, action: Action) -> Result<serde_json::Value> {
        let payload = serde_json::to_string(&(principal, &action))?;
        if payload.len() > MAX_BYTES { return Err(Error::Refused("oversized command")); }
        if matches!(action, Action::Get) {
            return Ok(serde_json::json!({"schema":1,"configuration": self.store.workflow_configuration(workspace)?,
                "state":self.state(workspace)?, "notices":self.notices(workspace,0)?}));
        }
        let mutation = match &action {
            Action::Publish { mutation, .. } | Action::Enqueue { mutation, .. }
            | Action::Review { mutation, .. } | Action::Approve { mutation, .. } | Action::Cancel { mutation, .. } => mutation,
            Action::Get => unreachable!(),
        };
        if !valid_id(&mutation.id) || mutation.id.len() > 100 { return Err(Error::Refused("invalid command id")); }
        let id = format!("wire:{principal}:{}", mutation.id);
        // Durable lookup precedes revision, epoch, run and configuration checks.
        if let Some(receipt) = self.store.workflow_wire_receipt(workspace, &id, &payload)? {
            return Ok(serde_json::json!({"schema":1,"receipt":receipt}));
        }
        let state = self.state(workspace)?.unwrap_or_else(|| State::empty(workspace.into(), 1));
        let stale = state.epoch != mutation.epoch || state.revision != mutation.revision;
        let controller = self.controller();
        self.authorize(controller.instance)?;
        let session = self.decision_session(&controller, principal)?;
        let input = |command, grant| Input { schema:1,id:id.clone(),workspace:workspace.into(),
            expected_revision:mutation.revision,epoch:mutation.epoch,source:format!("wire:{principal}"),
            source_sequence:mutation.sequence,now_ms:now(),grant,command,busy_resources:BTreeSet::new() };
        let receipt = if stale {
            self.store.workflow_wire_write(workspace,&id,&payload,Write::Refuse(Refusal::Stale))?
        } else {
            match &action {
                Action::Publish { expected_pin, definition, .. } => {
                    // A valid pure definition can still name an unsupported fixture adapter.
                    adapters::validate(definition)?;
                    self.store.workflow_wire_write(workspace,&id,&payload,Write::Publish {
                        expected_pin:expected_pin.as_deref(),definition,revision:state.revision })?
                }
                Action::Enqueue { item, pin, .. } => {
                    let configuration = self.store.workflow_configuration(workspace)?;
                    match configuration.filter(|c| &c.pin == pin) {
                        None => self.store.workflow_wire_write(workspace,&id,&payload,Write::Refuse(Refusal::Stale))?,
                        Some(configuration) => {
                            adapters::validate(&configuration.definition)?;
                            let initial_candidate = matches!(configuration.definition.stages[0].kind, StageKind::Tracker).then(|| pin.clone());
                            let input = input(Command::Enqueue { item:item.clone(),definition:configuration.definition,
                                policy_pin:pin.clone(),resource:self.resource.clone(),dependencies:BTreeSet::new(),initial_candidate },Grant::Controller);
                            self.store.workflow_wire_write(workspace,&id,&payload,Write::Apply(&input))?
                        }
                    }
                }
                Action::Approve { item, run, pin, candidate, .. } => {
                    let input = input(Command::Approve { item:item.clone(),run:run.clone(),pin:pin.clone(),candidate:candidate.clone() },Grant::Decision { principal:session.principal.clone() });
                    self.store.workflow_wire_write(workspace,&id,&payload,Write::Apply(&input))?
                }
                Action::Review { item, run, pin, evidence, passed, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin && i.last_evidence.as_ref() == Some(evidence)) {
                        self.store.workflow_wire_write(workspace,&id,&payload,Write::Refuse(Refusal::Stale))?
                    } else {
                        let input = input(Command::Review { item:item.clone(),run:run.clone(),evidence:evidence.clone(),passed:*passed },Grant::Decision { principal:session.principal.clone() });
                        self.store.workflow_wire_write(workspace,&id,&payload,Write::Apply(&input))?
                    }
                }
                Action::Cancel { item, run, pin, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write(workspace,&id,&payload,Write::Refuse(Refusal::Stale))?
                    } else {
                        let input = input(Command::Cancel { item:item.clone() },Grant::Controller);
                        self.store.workflow_wire_write(workspace,&id,&payload,Write::Apply(&input))?
                    }
                }
                Action::Get => unreachable!(),
            }
        };
        Ok(serde_json::json!({"schema":1,"receipt":receipt}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    fn m(id:&str,revision:u64,sequence:u64) -> Mutation { Mutation { id:id.into(),revision,epoch:1,sequence } }
    fn ok(value:&serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(),"{value}"); }
    #[tokio::test]
    async fn workflow_wire_pins_policy_and_replays_after_restart_without_reexecution() {
        let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("fixture");
        let mut runtime = Runtime::create(&root).await.unwrap();
        let mut definition = coding_definition(false);
        definition.stages[0].progression = Progression::AwaitApproval { principal:"owner".into() };
        let pin = definition.pin();
        let publication = Action::Publish { mutation:m("publish",0,1),expected_pin:None,definition };
        ok(&runtime.wire_command("ws","owner",publication.clone()).unwrap());
        let enqueue = Action::Enqueue { mutation:m("enqueue",0,1),item:"a".into(),pin:pin.clone() };
        let receipt = runtime.wire_command("ws","owner",enqueue.clone()).unwrap(); ok(&receipt);
        runtime.pump(10).await.unwrap();
        let state = runtime.state("ws").unwrap().unwrap();
        assert_eq!(state.items["a"].phase,Phase::WaitingApproval);
        let new_definition = coding_definition(true);
        ok(&runtime.wire_command("ws","owner",Action::Publish { mutation:m("publish2",state.revision,2),expected_pin:Some(pin.clone()),definition:new_definition }).unwrap());
        assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].pin,pin);
        assert_eq!(runtime.wire_command("ws","owner",enqueue.clone()).unwrap(),receipt);
        assert!(runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("enqueue",0,1),item:"different".into(),pin:pin.clone() }).is_err());
        drop(runtime);
        let runtime = Runtime::open(&root).unwrap();
        assert_eq!(runtime.wire_command("ws","owner",enqueue).unwrap(),receipt);
        ok(&runtime.wire_command("ws","owner",publication).unwrap());
        assert_eq!(runtime.state("ws").unwrap().unwrap().items.len(),1);
    }
    #[tokio::test]
    async fn workflow_wire_stale_epoch_pin_run_and_wrong_decider_refuse() {
        let dir = tempfile::tempdir().unwrap(); let mut runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
        let definition = Definition::tracker("owner"); let pin = definition.pin();
        ok(&runtime.wire_command("ws","owner",Action::Publish { mutation:m("p",0,1),expected_pin:None,definition }).unwrap());
        ok(&runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap());
        runtime.pump(10).await.unwrap();
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        let approve = |mutation,run,pin| Action::Approve { mutation,item:"a".into(),run,pin,candidate:item.candidate.clone() };
        let refused = runtime.wire_command("ws","orchestrator",approve(m("wrong-person",state.revision,1),item.run.clone(),pin.clone())).unwrap();
        assert!(refused.to_string().contains("Unauthorized"),"{refused}");
        let mut stale = m("epoch",state.revision,2); stale.epoch = 99;
        assert!(runtime.wire_command("ws","owner",approve(stale,item.run.clone(),pin.clone())).unwrap().to_string().contains("Stale"));
        for (id,run,pin) in [("run","old".into(),pin.clone()),("pin",item.run.clone(),"old".into())] {
            let refused = runtime.wire_command("ws","owner",approve(m(id,state.revision,2),run,pin)).unwrap();
            assert!(refused.pointer("/receipt/result/Err").is_some(),"{refused}");
        }
        ok(&runtime.wire_command("ws","owner",approve(m("approve",state.revision,2),item.run.clone(),pin.clone())).unwrap());
        runtime.pump(20).await.unwrap();
        assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].phase,Phase::Done);
    }
    #[tokio::test]
    async fn workflow_wire_auto_and_every_stage_approval_preserve_policy() {
        for every in [false,true] {
            let dir = tempfile::tempdir().unwrap(); let mut runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
            let mut definition = coding_definition(false);
            if every { for stage in &mut definition.stages { stage.progression=Progression::AwaitApproval { principal:"orchestrator".into() }; } }
            let pin=definition.pin();
            ok(&runtime.wire_command("ws","owner",Action::Publish { mutation:m("p",0,1),expected_pin:None,definition }).unwrap());
            ok(&runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap());
            let mut approvals=0;
            for time in 1..40 {
                runtime.pump(time).await.unwrap();
                let state=runtime.state("ws").unwrap().unwrap(); let item=&state.items["a"];
                if item.phase==Phase::Done { break; }
                if item.phase==Phase::WaitingApproval {
                    assert!(every); approvals+=1;
                    ok(&runtime.wire_command("ws","orchestrator",Action::Approve { mutation:m(&format!("a{approvals}"),state.revision,approvals),item:"a".into(),run:item.run.clone(),pin:pin.clone(),candidate:item.candidate.clone() }).unwrap());
                }
            }
            assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].phase,Phase::Done);
            assert_eq!(approvals,if every {5} else {0});
        }
    }
}

#[cfg(test)]
mod semantic_review_tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    #[tokio::test]
    async fn workflow_wire_semantic_review_requires_exact_observation_and_principal() {
        for passed in [false,true] {
            let dir=tempfile::tempdir().unwrap();let mut runtime=Runtime::create(&dir.path().join("fixture")).await.unwrap();
            let mut definition=coding_definition(true);
            definition.stages.iter_mut().find(|s| matches!(s.kind,StageKind::Review(_))).unwrap().kind=StageKind::Review(Reviewer::Person("owner".into()));
            let pin=definition.pin();
            let m=|id:&str,revision,sequence| Mutation { id:id.into(),revision,epoch:1,sequence };
            runtime.wire_command("ws","owner",Action::Publish { mutation:m("p",0,1),expected_pin:None,definition }).unwrap();
            runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap();
            for time in 1..15 {
                runtime.pump(time).await.unwrap();
                if runtime.state("ws").unwrap().unwrap().items["a"].phase==Phase::WaitingReview {break;}
            }
            let state=runtime.state("ws").unwrap().unwrap();let item=&state.items["a"];
            assert_eq!(item.phase,Phase::WaitingReview);
            let evidence=item.last_evidence.clone().unwrap();
            let review=|id:&str,sequence,run,pin,evidence| Action::Review { mutation:m(id,state.revision,sequence),item:"a".into(),run,pin,evidence,passed };
            let wrong=runtime.wire_command("ws","orchestrator",review("wrong",1,item.run.clone(),pin.clone(),evidence.clone())).unwrap();
            assert!(wrong.to_string().contains("Unauthorized"),"{wrong}");
            let mut old=evidence.clone();old.receipt="unseen".into();
            for (id,run,pin,evidence) in [("old-evidence",item.run.clone(),pin.clone(),old),("old-run","stale".into(),pin.clone(),evidence.clone()),("old-pin",item.run.clone(),"stale".into(),evidence.clone())] {
                assert!(runtime.wire_command("ws","owner",review(id,2,run,pin,evidence)).unwrap().to_string().contains("Stale"));
            }
            let accepted=runtime.wire_command("ws","owner",review("review",2,item.run.clone(),pin.clone(),evidence.clone())).unwrap();
            assert!(accepted.pointer("/receipt/result/Ok").is_some(),"{accepted}");
            assert_eq!(runtime.wire_command("ws","owner",review("review",2,item.run.clone(),pin,evidence)).unwrap(),accepted);
            let next=runtime.state("ws").unwrap().unwrap();
            assert_eq!(next.items["a"].iteration,if passed {0} else {1});
            assert_eq!(next.items["a"].stage,if passed {4} else {1});
        }
    }
}
