use super::*;
use farcooler_workflow::{Definition, Grant, Input, Outcome, step};

fn auth() -> Capabilities {
    Capabilities { workspace: "workspace".into(), epoch: 1, inspect: true,
        controller: true, decision_principal: Some("owner".into()) }
}
fn apply(state: &State, command: Command, grant: Grant) -> State {
    step(state, &Input { schema: 1, id: format!("event-{}", state.revision),
        workspace: state.workspace.clone(), expected_revision: state.revision, epoch: state.epoch,
        source: "fixture".into(), source_sequence: state.revision + 1, now_ms: 1,
        grant, command, busy_resources: Default::default() }).unwrap().state
}
fn enqueue(definition: Definition) -> State {
    apply(&State::empty("workspace".into(), 1), Command::Enqueue {
        item: "item".into(), definition, policy_pin: "policy".into(), resource: "resource".into(),
        dependencies: Default::default(), initial_candidate: None }, Grant::Controller)
}
fn finish(state: &State, candidate: &str, passed: bool) -> State {
    let item = &state.items["item"];
    let evidence = Evidence { candidate: candidate.into(), manifest: format!("manifest-{}", state.revision),
        receipt: format!("receipt-{}", state.revision), policy_pin: "policy".into() };
    let effect = item.pending.clone().unwrap();
    apply(state, Command::Observe { item: "item".into(), effect: effect.clone(),
        run: item.run.clone(), fence: item.fence,
        outcome: if passed { Outcome::Passed(evidence) } else { Outcome::Rejected(evidence) } },
        Grant::Operation { effect, run: item.run.clone(), fence: item.fence })
}
fn fixtures() -> Vec<(&'static str, State)> {
    let automatic = enqueue(Definition::coding());
    let mut definition = Definition::coding();
    for stage in &mut definition.stages {
        stage.progression = Progression::AwaitApproval { principal: "owner".into() };
    }
    let approval = enqueue(definition);
    let build = finish(&automatic, "provision", true);
    let check = finish(&build, "candidate", true);
    let review = finish(&check, "candidate", true);
    let rework = finish(&review, "candidate", false);
    let mut uncertain = check.clone();
    let item = &check.items["item"];
    uncertain = apply(&uncertain, Command::Observe { item: "item".into(), effect: item.pending.clone().unwrap(),
        run: item.run.clone(), fence: item.fence, outcome: Outcome::Uncertain },
        Grant::Operation { effect: item.pending.clone().unwrap(), run: item.run.clone(), fence: item.fence });
    let invalidated = finish(&rework, "new-candidate", true);
    // Current snapshot after a replacement candidate retains the new build receipt.
    // Invalidation of an earlier retained receipt is represented directly in the immutable input.
    let mut invalidated = invalidated;
    invalidated.items.get_mut("item").unwrap().last_evidence = check.items["item"].last_evidence.clone();
    vec![("automatic", automatic), ("approval", approval), ("rework", rework),
        ("uncertain", uncertain), ("invalidated", invalidated)]
}

#[test]
fn workflow_presentation_golden() {
    let cases: Vec<_> = fixtures().into_iter().map(|(name, snapshot)| {
        let request = Request { schema: SCHEMA, snapshot, capabilities: auth(), selection: None };
        let input = serde_json::to_string(&request).unwrap();
        let output = json(&input);
        assert_eq!(output, json(&input));
        #[cfg(feature = "native")]
        unsafe {
            let request = std::ffi::CString::new(input.as_str()).unwrap();
            let bridge = crate::ffi::farcooler_client_workflow_presentation;
            let needed = bridge(request.as_ptr(), std::ptr::null_mut(), 0);
            let mut bytes = vec![0; needed];
            assert_eq!(bridge(request.as_ptr(), bytes.as_mut_ptr(), bytes.len()), needed);
            assert_eq!(bytes, output.as_bytes());
        }
        serde_json::json!({"name": name, "request": request,
            "response": serde_json::from_str::<serde_json::Value>(&output).unwrap()})
    }).collect();
    let actual = serde_json::to_string_pretty(&cases).unwrap() + "\n";
    if std::env::var_os("UPDATE_WORKFLOW_GOLDEN").is_some() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/workflow-presentation.json"), &actual).unwrap();
    }
    assert_eq!(actual, include_str!("../testdata/workflow-presentation.json"));
}

#[test]
fn workflow_presentation_stale_and_auth_boundaries() {
    let state = fixtures().remove(1).1;
    let card = project(&state, &auth()).unwrap().runs.remove(0);
    let approve = card.actions.iter().find(|a| a.kind == ActionKind::Approve).unwrap();
    assert!(matches!(action(&state, &auth(), "item", approve), Ok(Intent::Command(Command::Approve { .. }))));
    for field in ["revision", "epoch", "candidate", "evidence", "run", "policy"] {
        let mut changed = state.clone();
        match field {
            "revision" => changed.revision += 1,
            "epoch" => changed.epoch += 1,
            "candidate" => changed.items.get_mut("item").unwrap().candidate = Some("replacement".into()),
            "evidence" => changed.items.get_mut("item").unwrap().last_evidence = Some(Evidence {
                candidate: "old".into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy".into() }),
            "run" => changed.items.get_mut("item").unwrap().run = "replacement".into(),
            _ => changed.items.get_mut("item").unwrap().policy_pin = "new-policy".into(),
        }
        assert!(action(&changed, &auth(), "item", approve).is_err(), "{field}");
    }
    let mut revoked = auth(); revoked.decision_principal = None;
    assert_eq!(action(&state, &revoked, "item", approve), Err(Error::Stale));
    assert!(!project(&state, &revoked).unwrap().runs[0].actions.iter().any(|a| a.kind == ActionKind::Approve));
    revoked.inspect = false;
    assert_eq!(project(&state, &revoked), Err(Error::Unauthorized));
    let mut forged = approve.clone(); forged.kind = ActionKind::AcceptReview;
    assert_eq!(action(&state, &auth(), "item", &forged), Err(Error::Unavailable));
}

#[test]
fn workflow_presentation_invalidated_inspection_and_rework() {
    let cases = fixtures();
    for (_, state) in &cases {
        let card = &project(state, &auth()).unwrap().runs[0];
        if let Some(evidence) = &card.latest_evidence {
            let inspect = card.actions.iter().find(|a| a.kind == ActionKind::InspectLatestEvidence).unwrap();
            assert_eq!(action(state, &auth(), "item", inspect),
                Ok(Intent::InspectLatestEvidence { evidence: evidence.clone() }));
        }
    }
    let invalidated = &cases[4].1;
    assert!(project(invalidated, &auth()).unwrap().runs[0].reasons.iter().any(|r| r.kind == Reason::EvidenceInvalidated));
    let mut definition = Definition::coding();
    definition.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    let state = enqueue(definition);
    let state = finish(&finish(&finish(&state, "provision", true), "candidate", true), "candidate", true);
    let card = &project(&state, &auth()).unwrap().runs[0];
    let rework = card.actions.iter().find(|a| a.kind == ActionKind::RequestRework).unwrap();
    let Intent::Command(command) = action_with_reason(&state, &auth(), "item", rework, Some("Please fix the evidence.")).unwrap() else { panic!() };
    let next = apply(&state, command, Grant::Decision { principal: "owner".into() });
    assert_eq!(next.items["item"].iteration, 1);
    let mut stale = state.clone();
    stale.items.get_mut("item").unwrap().last_evidence.as_mut().unwrap().candidate = "old".into();
    let card = &project(&stale, &auth()).unwrap().runs[0];
    assert!(!card.actions.iter().any(|a| matches!(a.kind, ActionKind::AcceptReview | ActionKind::RequestRework)));
    assert_eq!(action(&stale, &auth(), "item", rework), Err(Error::Stale));
}

#[test]
fn workflow_presentation_errors_and_c_buffer() {
    assert!(json("{}").contains("Invalid"));
    assert!(json("{\"schema\":99}").contains("Invalid"));
    let state = fixtures().remove(0).1;
    let mut broken = state.clone(); broken.items.get_mut("item").unwrap().stage = usize::MAX;
    assert_eq!(project(&broken, &auth()), Err(Error::Invalid));
    #[cfg(feature = "native")]
    {
        use crate::ffi::farcooler_client_workflow_presentation as bridge;
        let request = std::ffi::CString::new(serde_json::to_string(&Request {
            schema: SCHEMA, snapshot: state, capabilities: auth(), selection: None }).unwrap()).unwrap();
        let expected = json(request.to_str().unwrap());
        unsafe {
            let needed = bridge(request.as_ptr(), std::ptr::null_mut(), 0);
            assert_eq!(needed, expected.len());
            let mut short = vec![42; needed - 1];
            assert_eq!(bridge(request.as_ptr(), short.as_mut_ptr(), short.len()), needed);
            assert!(short.iter().all(|b| *b == 42));
            let mut out = vec![0; needed];
            assert_eq!(bridge(request.as_ptr(), out.as_mut_ptr(), out.len()), needed);
            assert_eq!(out, expected.as_bytes());
            assert_eq!(bridge(std::ptr::null(), std::ptr::null_mut(), 0), 0);
        }
    }
}

#[test]
fn workflow_presentation_selected_and_every_stage_approval() {
    for every in [false, true] {
        let mut definition = Definition::coding();
        for (index, stage) in definition.stages.iter_mut().enumerate() {
            if every || index == 2 {
                stage.progression = Progression::AwaitApproval { principal: "owner".into() };
            }
        }
        let mut state = enqueue(definition);
        for index in 0..6 {
            let card = project(&state, &auth()).unwrap().runs.remove(0);
            if every || index == 2 {
                assert_eq!(card.gate, Some(display::gate_card(GateKind::Approval, "owner")));
                let approve = card.actions.iter().find(|a| a.kind == ActionKind::Approve).unwrap();
                let Intent::Command(command) = action(&state, &auth(), "item", approve).unwrap() else { panic!() };
                state = apply(&state, command, Grant::Decision { principal: "owner".into() });
            } else { assert!(card.gate.is_none()); }
            state = finish(&state, if index == 0 { "provision" } else { "candidate" }, true);
        }
        let card = project(&state, &auth()).unwrap().runs.remove(0);
        assert_eq!(card.phase, Phase::Done);
        assert!(card.stage.is_none() && card.gate.is_none());
        assert!(!card.actions.iter().any(|a| matches!(a.kind, ActionKind::Approve | ActionKind::Cancel)));
    }
}

#[test]
fn workflow_presentation_generic_display_slots() {
    // A renderer reads only shared text and forwards the supplied action descriptor.
    // There is no feature-specific kind/phase/stage switch or copy map in this renderer.
    fn text_slots(card: &RunCard) -> Vec<&str> {
        let mut slots = vec![card.label.as_str(), card.phase_label.as_str()];
        if let Some(stage) = &card.stage {
            slots.extend([stage.label.as_str(), stage.progression_label.as_str()]);
        }
        if let Some(gate) = &card.gate { slots.extend([gate.label.as_str(), gate.text.as_str()]); }
        for reason in &card.reasons { slots.extend([reason.label.as_str(), reason.text.as_str()]); }
        slots.extend(card.actions.iter().map(|a| a.label.as_str()));
        slots
    }
    for (_, state) in fixtures() {
        let view = project(&state, &auth()).unwrap();
        let slots = text_slots(&view.runs[0]);
        assert!(slots.iter().all(|s| !s.is_empty()));
        assert!(view.runs[0].stages.iter().all(|s| !s.label.is_empty() && !s.progression_label.is_empty()));
    }
    let state = fixtures().remove(4).1;
    let card = project(&state, &auth()).unwrap().runs.remove(0);
    let slots = text_slots(&card);
    assert!(slots.contains(&"Evidence Invalidated"));
    assert!(slots.contains(&"The latest evidence no longer matches the current candidate or policy."));
    let label = "Inspect Latest Evidence…";
    assert!(slots.contains(&label));
    let button = card.actions.iter().find(|a| a.label == label).unwrap();
    let request = Request { schema: SCHEMA, snapshot: state, capabilities: auth(),
        selection: Some(Selection { item: card.item.clone(), action: button.clone(), reason: None }) };
    let response: Response = serde_json::from_str(&json(&serde_json::to_string(&request).unwrap())).unwrap();
    assert_eq!(response.schema, 2);
    assert!(matches!(response.intent, Some(Intent::InspectLatestEvidence { .. })));
    assert!(response.error.is_none());
    // The v1 bare-enum contract is explicitly rejected rather than silently frozen by consumers.
    let mut old = request; old.schema = 1;
    let response: Response = serde_json::from_str(&json(&serde_json::to_string(&old).unwrap())).unwrap();
    assert_eq!(response.error, Some(Error::Invalid));
}
