use super::*;
use farcooler_workflow::{Definition, Grant, Input, Progression, step};
fn snapshot() -> WorkflowSnapshot {
    let workspace = Uuid::nil().to_string();
    let mut definition = Definition::coding();
    definition.stages[3].kind=farcooler_workflow::StageKind::Review(farcooler_workflow::Reviewer::Person("owner".into()));
    definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    let state = step(&State::empty(workspace.clone(), 1), &Input {
        schema:1, id:"enqueue".into(), workspace:workspace.clone(), expected_revision:0, epoch:1,
        source:"fixture".into(), source_sequence:1, now_ms:1, grant:Grant::Controller,
        command:Command::Enqueue { item:"item".into(),definition,policy_pin:"policy".into(),resource:"r".into(),
            dependencies:Default::default(),initial_candidate:None },busy_resources:Default::default(),
    }).unwrap().state;
    let context = WorkflowContext { capabilities:Capabilities { workspace,epoch:1,inspect:true,controller:true,
        decision_principal:Some("owner".into()) },binding:"generation-1".into() };
    WorkflowSnapshot { schema:2,presentation:project(&state,&context).unwrap(),state,context,
        configuration:serde_json::Value::Null,notices:serde_json::json!([]) }
}
fn identity(s: &WorkflowSnapshot) -> WorkflowCommandIdentity {
    WorkflowCommandIdentity { id:"approve".into(),sequence:2,revision:s.state.revision,epoch:s.state.epoch }
}
fn approve(s: &WorkflowSnapshot) -> Action {
    s.presentation.runs[0].actions.iter().find(|a| a.kind == cards::ActionKind::Approve).unwrap().clone()
}
#[test]
fn workflow_session_stale_observation_and_admission_binding() {
    let s = snapshot(); let selected = approve(&s);
    assert!(prepare(&s,Uuid::nil(),"item",selected.clone(),identity(&s)).is_ok());
    for change in 0..8 {
        let mut newer = snapshot();
        match change {
            0 => newer.state.revision += 1,
            1 => newer.state.epoch += 1,
            2 => newer.state.items.get_mut("item").unwrap().run.push_str("-new"),
            3 => newer.state.items.get_mut("item").unwrap().candidate = Some("unseen".into()),
            4 => newer.context.binding = "generation-2".into(),
            5 => newer.context.capabilities.decision_principal = Some("orchestrator".into()),
            6 => newer.context.capabilities.controller = false,
            _ => newer.state.items.get_mut("item").unwrap().policy_pin = "unseen-policy".into(),
        }
        assert!(prepare(&newer,Uuid::nil(),"item",selected.clone(),identity(&newer)).is_err(),"change {change}");
    }
    let mut id=identity(&s);id.revision+=1;
    assert!(prepare(&s,Uuid::nil(),"item",selected,id).is_err());
}
#[test]
fn workflow_session_prepared_retains_exact_observation_and_bytes() {
    let s=snapshot();
    let PreparedWorkflowResult::Mutation { prepared } = prepare(&s,Uuid::nil(),"item",approve(&s),identity(&s)).unwrap() else {panic!()};
    let restored:PreparedWorkflowAction=serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    assert_eq!(restored,prepared);
    assert_eq!(wire::parse(&prepared.json).unwrap(),wire::Action::Approve {
        mutation:Mutation {id:"approve".into(),sequence:2,revision:s.state.revision,epoch:1},
        item:"item".into(),run:s.state.items["item"].run.clone(),pin:s.state.items["item"].pin.clone(),candidate:None });
}

#[cfg(feature="native")]
mod socket;

fn observed(s: &WorkflowSnapshot, command: Command, grant: Grant) -> WorkflowSnapshot {
    let mut next=s.clone();
    next.state=step(&s.state,&Input { schema:1,id:format!("observe-{}",s.state.revision),workspace:s.state.workspace.clone(),
        expected_revision:s.state.revision,epoch:s.state.epoch,source:"fixture".into(),source_sequence:s.state.revision+1,
        now_ms:1,grant,command,busy_resources:Default::default() }).unwrap().state;
    next.presentation=project(&next.state,&next.context).unwrap();next
}
fn finish(s: &WorkflowSnapshot, candidate: &str) -> WorkflowSnapshot {
    let item=&s.state.items["item"]; let effect=item.pending.clone().unwrap();
    let evidence=farcooler_workflow::Evidence {candidate:candidate.into(),manifest:format!("manifest-{}",s.state.revision),
        receipt:format!("receipt-{}",s.state.revision),policy_pin:item.policy_pin.clone()};
    observed(s,Command::Observe {item:"item".into(),effect:effect.clone(),run:item.run.clone(),fence:item.fence,
        outcome:farcooler_workflow::Outcome::Passed(evidence)},Grant::Operation {effect,run:item.run.clone(),fence:item.fence})
}
#[test]
fn workflow_session_review_and_inspection_keep_exact_evidence() {
    let s=snapshot();let item=&s.state.items["item"];
    let running=observed(&s,Command::Approve {item:"item".into(),run:item.run.clone(),pin:item.pin.clone(),candidate:None},
        Grant::Decision {principal:"owner".into()});
    let build=finish(&running,"provision");let check=finish(&build,"candidate");let review=finish(&check,"candidate");
    let card=&review.presentation.runs[0];
    for kind in [cards::ActionKind::InspectLatestEvidence,cards::ActionKind::AcceptReview,cards::ActionKind::RequestRework] {
        let selected=card.actions.iter().find(|a|a.kind==kind).unwrap().clone();
        let result=prepare(&review,Uuid::nil(),"item",selected.clone(),identity(&review)).unwrap();
        match result {
            PreparedWorkflowResult::Inspection {intent:Intent::InspectLatestEvidence {evidence}} => assert_eq!(Some(evidence),card.latest_evidence),
            PreparedWorkflowResult::Mutation {prepared} => {
                let wire::Action::Review {mutation,run,pin,evidence,passed,..}=wire::parse(&prepared.json).unwrap() else {panic!()};
                assert_eq!(Some(evidence),card.latest_evidence);assert_eq!(pin,card.definition_pin);assert_eq!(run,card.run);
                assert_eq!(mutation.revision,review.state.revision);assert_eq!(passed,kind==cards::ActionKind::AcceptReview);
            },
            _ => panic!("wrong intent"),
        }
        let mut unseen=review.clone();
        unseen.state.items.get_mut("item").unwrap().last_evidence.as_mut().unwrap().receipt="newer-unseen-receipt".into();
        assert!(prepare(&unseen,Uuid::nil(),"item",selected,identity(&unseen)).is_err());
    }
    let item=&running.state.items["item"];let effect=item.pending.clone().unwrap();
    let uncertain=observed(&running,Command::Observe {item:"item".into(),effect:effect.clone(),run:item.run.clone(),fence:item.fence,
        outcome:farcooler_workflow::Outcome::Uncertain},Grant::Operation {effect:effect.clone(),run:item.run.clone(),fence:item.fence});
    let a=uncertain.presentation.runs[0].actions.iter().find(|a|a.kind==cards::ActionKind::InspectUncertainty).unwrap();
    let PreparedWorkflowResult::Inspection {intent:Intent::InspectUncertainty {effect:actual,run,fence}}=
        prepare(&uncertain,Uuid::nil(),"item",a.clone(),identity(&uncertain)).unwrap() else {panic!()};
    assert_eq!(actual,effect);assert_eq!(run,item.run);assert_eq!(fence,item.fence);
}
