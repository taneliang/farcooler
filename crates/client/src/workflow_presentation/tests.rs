use super::*;
use farcooler_workflow::{Definition, Grant, Input, Outcome, step};

/// The existing cases read Apple's title case; the per-platform cases below say their own.
const STYLE: Style = Style::Apple;

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
    // A run cancelled with a long reason, about 120 characters, for the captures at the narrowest width (ov-1053).
    let mut idle = automatic.clone();
    idle.items.get_mut("item").unwrap().pending = None;
    let cancelled = apply(&idle, Command::Cancel { item: "item".into(),
        reason: Some("This run was cancelled because its board was deleted. Nobody can approve it now, and its worktree was left as it was.".into()) },
        Grant::Controller);
    // A failed run with the sentence its Provision kept (ov-1058), long, for the captures (ov-1060).
    let mut failed = idle.clone();
    let item = failed.items.get_mut("item").unwrap();
    item.phase = Phase::Failed;
    item.failure = Some("The landing remote's main could not be downloaded in time, so nothing was built. Its worktree was never created; try the ticket again when the remote is reachable.".into());
    // A landed run: every step done (ov-1060).
    let mut landed = idle.clone();
    let item = landed.items.get_mut("item").unwrap();
    item.phase = Phase::Done;
    item.stage = item.definition.stages.len();
    vec![("automatic", automatic), ("approval", approval), ("rework", rework),
        ("uncertain", uncertain), ("invalidated", invalidated), ("cancelled", cancelled), ("failed", failed),
        ("landed", landed)]
}

#[test]
fn workflow_presentation_golden() {
    let cases: Vec<_> = fixtures().into_iter().map(|(name, snapshot)| {
        let request = Request { schema: SCHEMA, snapshot, capabilities: auth(), style: STYLE, selection: None };
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
    let card = project(&state, &auth(), STYLE).unwrap().runs.remove(0);
    let approve = card.actions.iter().find(|a| a.kind == ActionKind::Approve).unwrap();
    assert!(matches!(action(&state, &auth(), STYLE, "item", approve), Ok(Intent::Command(Command::Approve { .. }))));
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
        assert!(action(&changed, &auth(), STYLE, "item", approve).is_err(), "{field}");
    }
    let mut revoked = auth(); revoked.decision_principal = None;
    assert_eq!(action(&state, &revoked, STYLE, "item", approve), Err(Error::Stale));
    assert!(!project(&state, &revoked, STYLE).unwrap().runs[0].actions.iter().any(|a| a.kind == ActionKind::Approve));
    revoked.inspect = false;
    assert_eq!(project(&state, &revoked, STYLE), Err(Error::Unauthorized));
    let mut forged = approve.clone(); forged.kind = ActionKind::AcceptReview;
    assert_eq!(action(&state, &auth(), STYLE, "item", &forged), Err(Error::Unavailable));
}

#[test]
fn workflow_presentation_invalidated_inspection_and_rework() {
    let cases = fixtures();
    for (_, state) in &cases {
        let card = &project(state, &auth(), STYLE).unwrap().runs[0];
        if let Some(evidence) = &card.latest_evidence {
            let inspect = card.actions.iter().find(|a| a.kind == ActionKind::InspectLatestEvidence).unwrap();
            assert_eq!(action(state, &auth(), STYLE, "item", inspect),
                Ok(Intent::InspectLatestEvidence { evidence: evidence.clone() }));
        }
    }
    let invalidated = &cases[4].1;
    assert!(project(invalidated, &auth(), STYLE).unwrap().runs[0].reasons.iter().any(|r| r.kind == Reason::EvidenceInvalidated));
    let mut definition = Definition::coding();
    definition.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    let state = enqueue(definition);
    let state = finish(&finish(&finish(&state, "provision", true), "candidate", true), "candidate", true);
    let card = &project(&state, &auth(), STYLE).unwrap().runs[0];
    let rework = card.actions.iter().find(|a| a.kind == ActionKind::RequestRework).unwrap();
    let Intent::Command(command) = action_with_reason(&state, &auth(), STYLE, "item", rework, Some("Please fix the evidence.")).unwrap() else { panic!() };
    let next = apply(&state, command, Grant::Decision { principal: "owner".into() });
    assert_eq!(next.items["item"].iteration, 1);
    let mut stale = state.clone();
    stale.items.get_mut("item").unwrap().last_evidence.as_mut().unwrap().candidate = "old".into();
    let card = &project(&stale, &auth(), STYLE).unwrap().runs[0];
    assert!(!card.actions.iter().any(|a| matches!(a.kind, ActionKind::AcceptReview | ActionKind::RequestRework)));
    assert_eq!(action(&stale, &auth(), STYLE, "item", rework), Err(Error::Stale));
}

#[test]
fn workflow_presentation_errors_and_c_buffer() {
    assert!(json("{}").contains("Invalid"));
    assert!(json("{\"schema\":99}").contains("Invalid"));
    let state = fixtures().remove(0).1;
    let mut broken = state.clone(); broken.items.get_mut("item").unwrap().stage = usize::MAX;
    assert_eq!(project(&broken, &auth(), STYLE), Err(Error::Invalid));
    #[cfg(feature = "native")]
    {
        use crate::ffi::farcooler_client_workflow_presentation as bridge;
        let request = std::ffi::CString::new(serde_json::to_string(&Request {
            schema: SCHEMA, snapshot: state, capabilities: auth(), style: STYLE, selection: None }).unwrap()).unwrap();
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
            let card = project(&state, &auth(), STYLE).unwrap().runs.remove(0);
            if every || index == 2 {
                assert_eq!(card.gate, Some(display::gate_card(GateKind::Approval, "owner", STYLE)));
                let approve = card.actions.iter().find(|a| a.kind == ActionKind::Approve).unwrap();
                let Intent::Command(command) = action(&state, &auth(), STYLE, "item", approve).unwrap() else { panic!() };
                state = apply(&state, command, Grant::Decision { principal: "owner".into() });
            } else { assert!(card.gate.is_none()); }
            state = finish(&state, if index == 0 { "provision" } else { "candidate" }, true);
        }
        let card = project(&state, &auth(), STYLE).unwrap().runs.remove(0);
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
        let view = project(&state, &auth(), STYLE).unwrap();
        let slots = text_slots(&view.runs[0]);
        assert!(slots.iter().all(|s| !s.is_empty()));
        assert!(view.runs[0].stages.iter().all(|s| !s.label.is_empty() && !s.progression_label.is_empty()));
    }
    let state = fixtures().remove(4).1;
    let card = project(&state, &auth(), STYLE).unwrap().runs.remove(0);
    let slots = text_slots(&card);
    assert!(slots.contains(&"Evidence Invalidated"));
    assert!(slots.contains(&"The latest evidence no longer matches the current candidate or policy."));
    let label = "Inspect Latest Evidence…";
    assert!(slots.contains(&label));
    let button = card.actions.iter().find(|a| a.label == label).unwrap();
    let request = Request { schema: SCHEMA, snapshot: state, capabilities: auth(), style: STYLE,
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

/// A tracker-only ticket is not a run: it is not listed, so it shows no gate and offers a person nothing.
#[test]
fn a_tracker_only_ticket_offers_a_person_no_action() {
    let definition = Definition::ticket();
    let state = apply(&State::empty("workspace".into(), 1), Command::Enqueue {
        item: "item".into(), initial_candidate: Some(definition.pin()), definition, policy_pin: "policy".into(),
        resource: "resource".into(), dependencies: Default::default() }, Grant::Controller);
    assert!(project(&state, &auth(), STYLE).unwrap().runs.is_empty(), "a ticket is not a run");
    // And a hand-built Cancel, with the label a real one carries, is refused.
    let forged = Action { kind: ActionKind::Cancel, label: display::action_label(&ActionKind::Cancel, STYLE),
        guard: digest(&(digest(&("workflow-presentation/v2", &state, &auth())), "item")) };
    assert!(action(&state, &auth(), STYLE, "item", &forged).is_err());
}

/// ov-1053: a run cancelled with a reason says it on its Cancelled card; one cancelled without says nothing more.
#[test]
fn a_cancelled_run_says_why() {
    let state = enqueue(Definition::coding());
    let cancel = |reason: Option<&str>| apply(&state, Command::Cancel { item: "item".into(), reason: reason.map(str::to_owned) }, Grant::Controller);
    let text = |s: &State| project(s, &auth(), STYLE).unwrap().runs[0].reasons.iter()
        .find(|r| matches!(r.kind, Reason::Cancelled | Reason::Cancelling)).unwrap().text.clone();
    // The fixture's run has an operation in flight, so it is Cancelling until the engine stops it.
    let stopping = cancel(Some("This run was cancelled because its board was deleted."));
    assert_eq!(stopping.items["item"].phase, Phase::Cancelling);
    assert_eq!(text(&stopping), "This run was cancelled because its board was deleted. The engine is waiting for the operation to stop.");
    assert_eq!(text(&cancel(None)), "The engine is waiting for the operation to stop.");
    // A run with nothing in flight is Cancelled at once.
    let mut idle = state.clone();
    idle.items.get_mut("item").unwrap().pending = None;
    let done = apply(&idle, Command::Cancel { item: "item".into(), reason: Some("This run was cancelled because its board was deleted.".into()) }, Grant::Controller);
    assert_eq!(done.items["item"].phase, Phase::Cancelled);
    assert_eq!(text(&done), "This run was cancelled because its board was deleted.");
    let blank = apply(&idle, Command::Cancel { item: "item".into(), reason: Some("  ".into()) }, Grant::Controller);
    assert_eq!(text(&blank), "This run has been cancelled.");
}

/// ov-979: the runner's list of a named tree's changes reaches the run's card, and only for the run it stopped.
#[test]
fn a_stopped_run_shows_why() {
    let state = fixtures().into_iter().find(|(name, _)| *name == "uncertain").unwrap().1;
    let run = state.items["item"].run.clone();
    let text = "The run stopped because its worktree has changes, and it left them as they are: mine.txt.";
    let mut view = project(&state, &auth(), STYLE).unwrap();
    add_stops(&mut view, &[("item".to_string(), Stop { run: run.clone(), text: text.into() })].into(), STYLE);
    let reason = view.runs[0].reasons.last().unwrap();
    assert_eq!((&reason.kind, reason.label.as_str(), reason.text.as_str()), (&Reason::WorktreeChanged, "Worktree Has Changes", text));
    let mut view = project(&state, &auth(), STYLE).unwrap();
    let before = view.runs[0].reasons.clone();
    add_stops(&mut view, &[("item".to_string(), Stop { run: "an earlier run".into(), text: text.into() })].into(), STYLE);
    assert_eq!(view.runs[0].reasons, before, "a stop for another run is not this one's");
}

/// Each list below must hold one of every variant of its enum, and cannot go stale quietly: `slot` is an
/// exhaustive `match`, so a new variant does not compile until it has a slot, and a slot no entry fills
/// fails `every_variant_is_listed`. A label a new variant produces is then in the golden file that
/// `scripts/copy-lint.py` and `scripts/shared-core-lint.py` read, instead of being linted by nobody.
fn phases() -> Vec<Phase> {
    vec![Phase::Blocked, Phase::WaitingApproval, Phase::WaitingReview, Phase::Running, Phase::Uncertain,
        Phase::Cancelling, Phase::Done, Phase::Failed, Phase::Cancelled, Phase::WaitingJoin, Phase::Joined]
}
fn kinds() -> Vec<StageKind> {
    vec![StageKind::Provision, StageKind::Build, StageKind::Check, StageKind::Review(Reviewer::Dispatch),
        StageKind::Land, StageKind::Ci, StageKind::Tracker, StageKind::Action("x".into()), StageKind::Evaluate("x".into()),
        StageKind::Join { group: "x".into(), adopt: false }, StageKind::AwaitBase, StageKind::Fold, StageKind::Run,
        StageKind::Push, StageKind::AwaitCi, StageKind::Route, StageKind::PushBranch, StageKind::OpenPr,
        StageKind::PrReady, StageKind::AwaitPr]
}
fn reasons() -> Vec<Reason> {
    vec![Reason::Automatic, Reason::ApprovalRequired, Reason::ReviewRequired, Reason::DependenciesOrResource,
        Reason::EffectUncertain, Reason::Reworking, Reason::EvidenceInvalidated, Reason::Completed, Reason::Failed,
        Reason::Cancelling, Reason::Cancelled, Reason::WorktreeChanged]
}
fn actions() -> Vec<ActionKind> {
    vec![ActionKind::InspectLatestEvidence, ActionKind::InspectUncertainty, ActionKind::Approve,
        ActionKind::AcceptReview, ActionKind::RequestRework, ActionKind::RejectReview, ActionKind::Cancel, ActionKind::Retry]
}
fn phase_slot(p: &Phase) -> usize {
    match p { Phase::Blocked => 0, Phase::WaitingApproval => 1, Phase::WaitingReview => 2, Phase::Running => 3,
        Phase::Uncertain => 4, Phase::Cancelling => 5, Phase::Done => 6, Phase::Failed => 7, Phase::Cancelled => 8,
        Phase::WaitingJoin => 9, Phase::Joined => 10 }
}
fn kind_slot(k: &StageKind) -> usize {
    match k { StageKind::Provision => 0, StageKind::Build => 1, StageKind::Check => 2, StageKind::Review(_) => 3,
        StageKind::Land => 4, StageKind::Ci => 5, StageKind::Tracker => 6, StageKind::Action(_) => 7,
        StageKind::Evaluate(_) => 8, StageKind::Join { .. } => 9, StageKind::AwaitBase => 10, StageKind::Fold => 11,
        StageKind::Run => 12, StageKind::Push => 13, StageKind::AwaitCi => 14, StageKind::Route => 15,
        StageKind::PushBranch => 16, StageKind::OpenPr => 17, StageKind::PrReady => 18, StageKind::AwaitPr => 19 }
}
fn reason_slot(r: &Reason) -> usize {
    match r { Reason::Automatic => 0, Reason::ApprovalRequired => 1, Reason::ReviewRequired => 2,
        Reason::DependenciesOrResource => 3, Reason::EffectUncertain => 4, Reason::Reworking => 5,
        Reason::EvidenceInvalidated => 6, Reason::Completed => 7, Reason::Failed => 8, Reason::Cancelling => 9,
        Reason::Cancelled => 10, Reason::WorktreeChanged => 11 }
}
fn action_slot(a: &ActionKind) -> usize {
    match a { ActionKind::InspectLatestEvidence => 0, ActionKind::InspectUncertainty => 1, ActionKind::Approve => 2,
        ActionKind::AcceptReview => 3, ActionKind::RequestRework => 4, ActionKind::RejectReview => 5,
        ActionKind::Cancel => 6, ActionKind::Retry => 7 }
}

#[test]
fn every_variant_is_listed() {
    fn filled<T>(list: &[T], slot: fn(&T) -> usize, of: usize, what: &str) {
        let mut slots: Vec<usize> = list.iter().map(slot).collect();
        slots.sort();
        assert_eq!(slots, (0..of).collect::<Vec<_>>(), "the {what} list in `labels` misses a variant, or lists one twice");
    }
    filled(&phases(), phase_slot, 11, "Phase");
    filled(&kinds(), kind_slot, 20, "StageKind");
    filled(&reasons(), reason_slot, 12, "Reason");
    filled(&actions(), action_slot, 8, "ActionKind");
}

fn labels(style: Style) -> Vec<String> {
    let (phases, kinds, reasons, actions) = (phases(), kinds(), reasons(), actions());
    let mut out = vec![display::run_label(style)];
    out.extend(phases.iter().map(|p| display::phase_label(p, style)));
    out.extend(kinds.iter().map(|k| style.label(display::stage_label(k))));
    out.push(style.label("Automatic"));
    out.extend([GateKind::Approval, GateKind::Review].map(|k| display::gate_card(k, "owner", style).label));
    out.extend(reasons.into_iter().map(|r| display::reason_card(r, style).label));
    out.extend(actions.iter().map(|a| display::action_label(a, style)));
    out.push(display::close_label(style));
    let form = display::rework_form(1, style);
    out.extend([form.label, form.submit_label]);
    out.sort();
    out.dedup();
    out
}

/// Every label Rust can produce, per platform. `scripts/copy-lint.py` reads this file and holds the
/// Apple and Web labels to title case and the Material ones to sentence case, so a string whose case
/// the helper gets wrong, or one written in the wrong case at the source, fails there.
#[test]
fn workflow_label_golden_per_platform() {
    let all: std::collections::BTreeMap<_, _> = Style::ALL.iter()
        .map(|s| (format!("{s:?}").to_lowercase(), labels(*s))).collect();
    let actual = serde_json::to_string_pretty(&all).unwrap() + "\n";
    if std::env::var_os("UPDATE_WORKFLOW_GOLDEN").is_some() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/workflow-labels.json"), &actual).unwrap();
    }
    assert_eq!(actual, include_str!("../testdata/workflow-labels.json"));
}

/// Apple shows 'Awaiting Approval' and Android 'Awaiting approval' for the same run; the sentences do not move.
#[test]
fn each_platform_shows_its_own_case_and_the_sentences_do_not_move() {
    let state = fixtures().remove(1).1;
    let view = |style| project(&state, &auth(), style).unwrap().runs.remove(0);
    let (apple, material, web) = (view(Style::Apple), view(Style::Material), view(Style::Web));
    assert_eq!((apple.phase_label.as_str(), apple.label.as_str()), ("Awaiting Approval", "Workflow Run"));
    assert_eq!((material.phase_label.as_str(), material.label.as_str()), ("Awaiting approval", "Workflow run"));
    assert_eq!(web.phase_label, "Awaiting Approval");
    let names = |card: &RunCard| card.actions.iter().map(|a| a.label.clone()).collect::<Vec<_>>();
    assert_eq!(names(&apple), ["Approve", "Cancel Run"]);
    assert_eq!(names(&material), ["Approve", "Cancel run"]);
    let gate = |card: &RunCard| card.gate.clone().unwrap();
    assert_eq!((gate(&apple).label.as_str(), gate(&material).label.as_str()), ("Approval Required", "Approval required"));
    assert_eq!(gate(&apple).text, gate(&material).text);
    assert_eq!(apple.reasons.iter().map(|r| &r.text).collect::<Vec<_>>(), material.reasons.iter().map(|r| &r.text).collect::<Vec<_>>());
    // A principal's name is a value: neither style recases it.
    let stage = |card: &RunCard| card.stage.clone().unwrap().progression_label;
    assert_eq!((stage(&apple), stage(&material)), ("Approval from owner".into(), "Approval from owner".into()));
}

/// An action chosen from one platform's card is refused on another's: a click is matched by its label.
#[test]
fn an_action_is_matched_in_the_style_it_was_shown_in() {
    let state = fixtures().remove(1).1;
    let shown = project(&state, &auth(), Style::Material).unwrap().runs.remove(0);
    let cancel = shown.actions.iter().find(|a| a.kind == ActionKind::Cancel).unwrap();
    assert!(action(&state, &auth(), Style::Material, "item", cancel).is_ok());
    assert_eq!(action(&state, &auth(), Style::Apple, "item", cancel), Err(Error::Unavailable));
}

/// The style is required on the wire: a host that forgets it is told so instead of getting another platform's case.
#[test]
fn a_request_without_a_style_is_refused() {
    let state = fixtures().remove(0).1;
    let mut request = serde_json::to_value(Request { schema: SCHEMA, snapshot: state, capabilities: auth(),
        style: Style::Material, selection: None }).unwrap();
    let ok: Response = serde_json::from_str(&json(&request.to_string())).unwrap();
    assert_eq!(ok.card.unwrap().runs[0].phase_label, "Running");
    request.as_object_mut().unwrap().remove("style");
    let refused: Response = serde_json::from_str(&json(&request.to_string())).unwrap();
    assert_eq!(refused.error, Some(Error::Invalid));
}

/// ov-1060: a run's page leads with the ticket, the outcome and the reason, and shows no hash, stage id or counter by default.
#[test]
fn a_runs_page_reads_like_a_person_wrote_it() {
    let state = enqueue(Definition::coding());
    let cancelled = apply(&state, Command::Cancel { item: "item".into(), reason: Some("This run was cancelled because its board was deleted.".into()) }, Grant::Controller);
    let page = project(&cancelled, &auth(), STYLE).unwrap().runs.remove(0).page;
    assert_eq!((page.ticket.as_str(), page.outcome.as_str()), ("item", "Cancelling"));
    assert!(page.reason.starts_with("This run was cancelled because its board was deleted."));
    assert_eq!(page.steps[0].label, "Prepare workspace");
    assert_eq!(page.steps[0].state, "Stopping");
    assert_eq!(page.steps[1].state, "Not started");
    // Nothing a person reads by default carries a machine id.
    let shown: Vec<&str> = std::iter::once(page.reason.as_str()).chain(page.steps.iter().flat_map(|s| [s.label.as_str(), s.state.as_str()])).collect();
    assert!(shown.iter().all(|t| !t.contains("wf1:") && !t.contains("stage-") && !t.contains("Fence")), "{shown:?}");
    // The ids sit under Details, shortened, with the whole value to copy.
    let run = page.details.iter().find(|d| d.title == "Run").unwrap();
    let full = &cancelled.items["item"].run;
    if full.chars().count() > 12 { assert!(run.value.chars().count() <= 8 && run.whole.as_deref() == Some(full.as_str())); }
    assert!(page.details.iter().all(|d| !d.value.contains("wf1:")), "{:?}", page.details);
    assert!(!page.details.iter().any(|d| d.title.contains("Fence") || d.title.contains("Iteration") || d.title.contains("Policy")));
}

/// A failed run says why in the sentence it kept (ov-1058), not the generic "This run has failed."
#[test]
fn a_failed_runs_page_leads_with_its_sentence() {
    let mut state = enqueue(Definition::coding());
    let item = state.items.get_mut("item").unwrap();
    item.phase = Phase::Failed;
    item.failure = Some("The landing remote could not be reached.".into());
    let page = project(&state, &auth(), STYLE).unwrap().runs.remove(0).page;
    assert_eq!((page.outcome.as_str(), page.reason.as_str()), ("Failed", "The landing remote could not be reached."));
    assert_eq!(page.steps[0].state, "Failed");
}

#[test]
fn short_ids_drop_the_scheme_and_keep_the_front() {
    assert_eq!(page::short("wf1:0123456789abcdef0123456789abcdef"), "01234567");
    assert_eq!(page::short("item"), "item");
}

/// ov-1060 fix round: the page carries what a person must act on, every reason, and plain words for a live run.
#[test]
fn a_runs_page_holds_the_gate_every_reason_and_plain_words() {
    let fixtures = fixtures();
    let page = |name: &str| project(&fixtures.iter().find(|(n, _)| *n == name).unwrap().1, &auth(), STYLE).unwrap().runs.remove(0).page;
    // One plain sentence says what the run waits for and names the viewer as "you", never the raw principal.
    let approval = page("approval");
    assert_eq!(approval.outcome, "Awaiting approval");
    assert_eq!(approval.reason, "Waiting for your approval to start Prepare workspace.");
    let other = Capabilities { decision_principal: Some("someone".into()), ..auth() };
    let waiting = project(&fixtures.iter().find(|(n, _)| *n == "approval").unwrap().1, &other, STYLE).unwrap().runs.remove(0).page;
    assert_eq!(waiting.reason, "Waiting for owner's approval to start Prepare workspace.");
    assert!(!approval.more.iter().any(|m| m.contains("configured approval")), "{:?}", approval.more);
    // A running run names its stage; an uncertain one names the stage and what to do.
    assert_eq!(page("automatic").reason, "Running: Prepare workspace.");
    assert_eq!(page("uncertain").reason, "Run checks may or may not have finished. Check it before it continues.");
    // Rework and stale evidence stay on the page, with the reworks left in words.
    let rework = page("rework");
    assert!(rework.more.contains(&"This run has entered a rework iteration.".to_owned()), "{:?}", rework.more);
    assert!(rework.more.iter().any(|m| m.ends_with("reworks left.") || m.ends_with("rework left.")), "{:?}", rework.more);
    assert!(page("invalidated").more.iter().any(|m| m.contains("no longer matches")));
}

/// A run that landed reads "Completed" with every step done.
#[test]
fn a_landed_runs_page_is_all_done() {
    let mut state = enqueue(Definition::coding());
    let item = state.items.get_mut("item").unwrap();
    item.phase = Phase::Done;
    item.stage = item.definition.stages.len();
    let page = project(&state, &auth(), STYLE).unwrap().runs.remove(0).page;
    assert_eq!((page.outcome.as_str(), page.reason.as_str()), ("Completed", "All stages have completed."));
    assert!(page.steps.iter().all(|s| s.state == "Done"), "{:?}", page.steps);
}

/// A runner's stop explains a run that has no sentence of its own, but never overwrites a kept failure.
#[test]
fn a_stop_does_not_replace_a_kept_failure_sentence() {
    let fixtures = fixtures();
    let mut view = project(&fixtures.iter().find(|(n, _)| *n == "failed").unwrap().1, &auth(), STYLE).unwrap();
    let kept = view.runs[0].page.reason.clone();
    let stops = std::collections::BTreeMap::from([("item".to_owned(), Stop { run: view.runs[0].run.clone(), text: "Changes in the worktree.".into() })]);
    add_stops(&mut view, &stops, STYLE);
    assert_eq!(view.runs[0].page.reason, kept);
    assert!(view.runs[0].page.more.contains(&"Changes in the worktree.".to_owned()));
    let mut running = project(&fixtures.iter().find(|(n, _)| *n == "automatic").unwrap().1, &auth(), STYLE).unwrap();
    let stops = std::collections::BTreeMap::from([("item".to_owned(), Stop { run: running.runs[0].run.clone(), text: "Changes in the worktree.".into() })]);
    add_stops(&mut running, &stops, STYLE);
    assert_eq!(running.runs[0].page.reason, "Changes in the worktree.");
}

/// A run's actions are the ones the engine accepts for where it stands; a failed run is retried or closed.
#[test]
fn actions_follow_the_outcome() {
    let fixtures = fixtures();
    let state = |name: &str| fixtures.iter().find(|(n, _)| *n == name).unwrap().1.clone();
    let labels = |s: &State| project(s, &auth(), STYLE).unwrap().runs.remove(0).actions.iter().map(|a| a.label.clone()).collect::<Vec<_>>();
    // Nothing offers to cancel a run that has ended.
    for ended in ["cancelled", "landed"] { assert!(!labels(&state(ended)).iter().any(|l| l.starts_with("Cancel")), "{ended}"); }
    // A failed run can be tried again or closed, and "Try Again" does what the reducer's retry does.
    let failed = state("failed");
    assert_eq!(labels(&failed), ["Try Again", "Close Run"]);
    let card = project(&failed, &auth(), STYLE).unwrap().runs.remove(0);
    let retry = card.actions.iter().find(|a| a.kind == ActionKind::Retry).unwrap();
    let Intent::Command(command) = action(&failed, &auth(), STYLE, "item", retry).unwrap() else { panic!() };
    assert_eq!(command, Command::Retry { item: "item".into() });
    assert_ne!(apply(&failed, command, Grant::Controller).items["item"].phase, Phase::Failed);
    // Retries run out, and only a controller may retry.
    let mut spent = failed.clone();
    spent.items.get_mut("item").unwrap().retries = farcooler_workflow::MAX_RETRIES;
    assert_eq!(labels(&spent), ["Close Run"]);
    let viewer = Capabilities { controller: false, ..auth() };
    assert!(project(&failed, &viewer, STYLE).unwrap().runs[0].actions.is_empty());
}

/// Status words are sentence case on every platform; only buttons and headings follow the platform's case.
#[test]
fn status_words_are_sentence_case_on_every_platform() {
    for style in Style::ALL {
        for (name, state) in fixtures() {
            let card = project(&state, &auth(), style).unwrap().runs.remove(0);
            let words: Vec<&str> = std::iter::once(card.page.outcome.as_str()).chain(card.page.steps.iter().flat_map(|s| [s.label.as_str(), s.state.as_str()])).collect();
            for word in words {
                let rest: String = word.chars().skip(1).collect();
                assert!(rest == rest.to_lowercase() || word.contains("CI"), "{name} {style:?}: {word}");
            }
            assert_eq!(card.page.copy_label, style.label("Copy"));
        }
    }
}

#[test]
fn short_leaves_a_name_alone() {
    assert_eq!(page::short("new-candidate"), "new-candidate");
}
