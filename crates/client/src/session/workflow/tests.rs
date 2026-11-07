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
    WorkflowSnapshot { schema:2,host:Uuid::nil().to_string(),presentation:project(&state,&context).unwrap(),state,context,
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
        let result=prepare_selection_with_reason(&review,Uuid::nil(),"item",selected.clone(),review.state.revision,review.state.epoch,
            (kind==cards::ActionKind::RequestRework).then_some("Please fix the evidence.")).unwrap();
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

#[test]
fn workflow_selection_allocates_once_and_preserves_observation() {
    let mut s = snapshot();
    s.state.source_sequences.insert("unrelated-source".into(), u64::MAX);
    s.state.source_sequences.insert("wire:owner".into(), 41);
    s.state.revision = u64::MAX - 2;
    s.state.epoch = u64::MAX;
    s.context.capabilities.epoch = u64::MAX;
    s.presentation = project(&s.state, &s.context).unwrap();
    let selected = approve(&s);
    let PreparedWorkflowResult::Mutation { prepared } = prepare_selection(&s, Uuid::nil(), "item", selected.clone(),
        s.state.revision, s.state.epoch).unwrap() else { panic!() };
    let value: serde_json::Value = serde_json::from_str(&prepared.json).unwrap();
    assert_eq!(value["mutation"]["sequence"].as_u64(), Some(42));
    assert_eq!(value["mutation"]["revision"].as_u64(), Some(u64::MAX - 2));
    assert_eq!(value["mutation"]["epoch"].as_u64(), Some(u64::MAX));
    assert!(prepare_selection(&s, Uuid::nil(), "item", selected.clone(), s.state.revision - 1, s.state.epoch).is_err());
    let mut changed = s.clone(); changed.context.binding.push('x');
    assert!(prepare_selection(&changed, Uuid::nil(), "item", selected.clone(), s.state.revision, s.state.epoch).is_err());
    s.state.source_sequences.insert("wire:owner".into(), u64::MAX);
    s.presentation = project(&s.state, &s.context).unwrap();
    let exhausted = approve(&s);
    assert!(matches!(prepare_selection(&s, Uuid::nil(), "item", exhausted, s.state.revision, s.state.epoch),
        Err(SessionError::Protocol(message)) if message == "workflow source sequence exhausted"));
}

#[test]
fn workflow_selection_readonly_ignores_all_sequence_exhaustion() {
    let initial = snapshot(); let item = &initial.state.items["item"];
    let running = observed(&initial, Command::Approve { item:"item".into(), run:item.run.clone(), pin:item.pin.clone(), candidate:None },
        Grant::Decision { principal:"owner".into() });
    let mut s = finish(&running, "candidate");
    s.state.source_sequences.insert("wire:owner".into(), u64::MAX);
    s.state.source_sequences.insert("unrelated".into(), u64::MAX);
    s.context.capabilities.decision_principal = None;
    s.presentation = project(&s.state, &s.context).unwrap();
    let selected = s.presentation.runs[0].actions.iter().find(|a| a.kind == cards::ActionKind::InspectLatestEvidence).unwrap().clone();
    assert!(matches!(prepare_selection(&s, Uuid::nil(), "item", selected.clone(), s.state.revision, s.state.epoch).unwrap(),
        PreparedWorkflowResult::Inspection { .. }));
    assert!(prepare_selection(&s, Uuid::nil(), "item", selected, s.state.revision - 1, s.state.epoch).is_err());
}

#[test]
fn workflow_reasoned_selection_validation_and_immutable_bytes() {
    let initial=snapshot(); let i=&initial.state.items["item"];
    let s=observed(&initial,Command::Approve {item:"item".into(),run:i.run.clone(),pin:i.pin.clone(),candidate:None},
        Grant::Decision {principal:"owner".into()});
    let s=finish(&finish(&finish(&s,"provision"),"candidate"),"candidate");
    let card=&s.presentation.runs[0];
    let a=card.actions.iter().find(|a|a.kind==cards::ActionKind::RequestRework).unwrap();
    let fixture:serde_json::Value=serde_json::from_str(include_str!("../../testdata/workflow-rework.json")).unwrap();
    assert_eq!(serde_json::to_value(&card.forms[0]).unwrap(),fixture["form"]);
    for case in fixture["errors"].as_array().unwrap() {
        assert_eq!(cards::validate_input(&a.kind,case["reason"].as_str()).is_ok(),case["valid"].as_bool().unwrap());
    }
    assert_eq!(card.forms[0].max_bytes,4096);
    for reason in [None,Some(""),Some("\0"),Some(" \n")] {
        let PreparedWorkflowResult::Validation {error}=prepare_selection_with_reason(&s,Uuid::nil(),"item",a.clone(),s.state.revision,s.state.epoch,reason).unwrap() else {panic!("must not prepare")};
        assert_eq!(error.code,"workflow_reason"); assert!(!error.text.is_empty());
    }
    let reason="  reason with 界 <script>\n  ";
    let PreparedWorkflowResult::Mutation {prepared}=prepare_selection_with_reason(&s,Uuid::nil(),"item",a.clone(),s.state.revision,s.state.epoch,Some(reason)).unwrap() else {panic!()};
    let restored:PreparedWorkflowAction=serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    assert_eq!(restored,prepared);
    let wire::Action::Review {reason:actual,evidence,passed,..}=wire::parse(&restored.json).unwrap() else {panic!()};
    assert_eq!(actual.as_deref(),Some(reason));assert!(!passed);assert_eq!(Some(evidence),card.latest_evidence);
    let mut changed=s.clone();changed.context.binding.push('x');
    assert!(prepare_selection_with_reason(&changed,Uuid::nil(),"item",a.clone(),s.state.revision,s.state.epoch,Some(reason)).is_err());
    assert!(prepare_selection_with_reason(&s,Uuid::nil(),"item",a.clone(),s.state.revision-1,s.state.epoch,None).is_err());
}

#[test]
fn workflow_prepared_validation_is_shared_and_preserves_bytes() {
    let legacy=r#"{ "action":"Review", "mutation":{"id":"same","revision":1,"epoch":1,"sequence":2},"item":"a","run":"run","pin":"pin","evidence":{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"},"passed":false }"#;
    let mut p=PreparedWorkflowAction{schema:1,workspace:Uuid::nil(),json:legacy.into()};
    assert!(p.validate().is_ok());assert_eq!(p.json,legacy);
    p.schema=99;assert!(p.validate().is_err());assert_eq!(p.json,legacy);
    p.schema=1;p.json=r#"{"action":"Get"}"#.into();assert!(p.validate().is_err());
}
