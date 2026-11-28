//! A provider at its limit fails the step or moves it to the step's fallback model, and a ticket moves to another
//! model with one command, keeping its worktree (ov-1515). Private fixtures only.
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: &str = "usage limit until Dec 1st, 2026 1:23 AM (Codex)";

struct Log { state: State, effects: Vec<Effect> }
impl Log {
    fn input(&self, command: Command, grant: Grant) -> Input {
        let s = &self.state;
        Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch,
            source: "fixture".into(), source_sequence: s.revision + 1, now_ms: 1000, grant, command }
    }
    fn try_run(&mut self, command: Command, grant: Grant) -> Result<Transition, Refusal> {
        let input = self.input(command, grant);
        let transition = step(&self.state, &input)?;
        assert_eq!(step(&self.state, &input).unwrap(), transition, "a step is a pure function");
        self.state = transition.state.clone();
        self.effects = transition.effects.clone();
        Ok(transition)
    }
    fn started(d: Definition) -> Self {
        let mut log = Log { state: State::empty("workspace".into(), 1), effects: Vec::new() };
        let ticket = Definition::ticket();
        log.try_run(Command::Enqueue { item: "ov-1".into(), policy_pin: "policy-1".into(), resource: "res".into(),
            dependencies: BTreeSet::new(), initial_candidate: Some(ticket.pin()), definition: ticket }, Grant::Controller).unwrap();
        log.try_run(Command::Start { start_load: None, item: "ov-1".into(), policy_pin: "policy-2".into(), resource: "run".into(),
            dependencies: BTreeSet::new(), initial_candidate: None, definition: d, models: Default::default(),
            tree: Some(Box::new(tree())), with: vec![] }, Grant::Controller).unwrap();
        log
    }
    fn answer(&mut self, effect: &Effect, outcome: Outcome) {
        self.try_run(Command::Observe { item: effect.item.clone(), effect: effect.id.clone(), run: effect.run.clone(),
            fence: effect.fence, outcome },
            Grant::Operation { effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence }).unwrap();
    }
    fn observe(&mut self, outcome: Outcome) { let effect = self.effects[0].clone(); self.answer(&effect, outcome); }
    fn model(&mut self, model: &str) -> Result<Transition, Refusal> {
        self.try_run(Command::SetModel { item: "ov-1".into(), model: model.into(), step: None }, Grant::Controller)
    }
    fn item(&self) -> &Item { &self.state.items["ov-1"] }
}
fn ev(candidate: &str) -> Evidence {
    Evidence { candidate: candidate.into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy-2".into() }
}
fn tree() -> RunTree {
    RunTree { name: "ov-1".into(), branch: "ov-1-fix".into(), path: "/trees/ov-1".into(), base: "a".repeat(40), adopted: true }
}
fn coding() -> Definition {
    let op = OperationSpec { adapter_version: "coding/v1".into(), arguments: vec![], environment: Default::default() };
    let stage = |id: &str, kind| Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: op.clone(), routes: Default::default(), limit: 0 };
    Definition { schema: 1, source: "coding/v1".into(), max_reworks: 0, rework_target: None, groups: Default::default(),
        models: BTreeMap::from([("build".into(), vec!["codex:sol".into(), "claude:sonnet".into(), "claude:opus".into()])]),
        fallbacks: Default::default(), send_backs: None, build_folders: None, script_policies: Default::default(),
        start_policy: None, limits: None, policies: Default::default(),
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check)] }
}
fn limited() -> Outcome { Outcome::UsageLimit { harness: "codex".into(), reason: LIMIT.into() } }

#[test]
fn a_usage_limit_fails_the_step_at_once_saying_until_when_and_the_retry_is_free() {
    // Two attempts: neither is spent on a provider that will refuse the next launch too.
    let mut d = coding();
    d.stages[0].max_attempts = 2;
    let mut log = Log::started(d);
    log.observe(limited());
    assert_eq!(log.item().phase, Phase::Failed);
    assert_eq!(log.item().failure.as_deref(), Some(LIMIT));
    assert!(log.item().reason.as_deref().unwrap().starts_with(&format!("build: {LIMIT}")), "{:?}", log.item().reason);
    assert!(log.effects.is_empty());
    log.try_run(Command::Retry { item: "ov-1".into() }, Grant::Controller).unwrap();
    assert_eq!(log.item().retries, 0);
}

#[test]
fn a_configured_fallback_takes_the_step_over_and_a_second_limit_there_fails_it() {
    let mut d = coding();
    d.fallbacks.insert("build".into(), "claude:sonnet".into());
    let mut log = Log::started(d);
    assert_eq!(log.effects[0].model.as_deref(), Some("codex:sol"));
    log.observe(limited());
    assert_eq!(log.item().phase, Phase::Running);
    let next = &log.effects[0];
    assert_eq!((next.model.as_deref(), next.model_chosen, next.tree.clone()), (Some("claude:sonnet"), true, Some(tree())));
    assert_eq!(log.item().attempt, 1, "no attempt was spent");
    assert!(log.item().reason.as_deref().unwrap().contains("claude:sonnet"), "{:?}", log.item().reason);
    // The fallback's own provider at its limit has nowhere further to go.
    log.observe(Outcome::UsageLimit { harness: "claude".into(), reason: "usage limit until 1pm (UTC) (Claude)".into() });
    assert_eq!(log.item().phase, Phase::Failed);
}

#[test]
fn a_fallback_on_the_limited_provider_is_not_used_and_none_is_used_unless_configured() {
    let mut d = coding();
    d.fallbacks.insert("build".into(), "codex:sol".into());
    let mut log = Log::started(d);
    log.observe(limited());
    assert_eq!(log.item().phase, Phase::Failed);
    let mut log = Log::started(coding());
    log.observe(limited());
    assert_eq!(log.item().phase, Phase::Failed);
}

#[test]
fn a_fallback_must_name_a_listed_model_of_its_step() {
    let mut d = coding();
    d.fallbacks.insert("build".into(), "claude:haiku".into());
    assert!(d.validate_detailed().unwrap_err().contains("fallback"));
    let mut d = coding();
    d.fallbacks.insert("check".into(), "claude:sonnet".into());
    assert!(d.validate_detailed().unwrap_err().contains("fallback"));
    // Left out, the canonical bytes (and so every published pin) are as they were.
    assert!(!serde_json::to_string(&coding()).unwrap().contains("fallbacks"));
}

#[test]
fn a_failed_ticket_moves_to_another_model_and_runs_again_in_the_same_tree() {
    let mut log = Log::started(coding());
    log.observe(limited());
    log.model("claude:opus").unwrap();
    assert_eq!(log.item().phase, Phase::Running);
    assert_eq!((log.item().retries, log.item().failure.clone()), (0, None));
    let build = &log.effects[0];
    assert_eq!((build.kind.clone(), build.model.as_deref(), build.model_chosen, build.tree.clone()),
        (EffectKind::Execute(StageKind::Build), Some("claude:opus"), true, Some(tree())));
}

#[test]
fn a_running_ticket_stops_its_agent_then_builds_on_the_new_model_in_the_same_tree() {
    let mut log = Log::started(coding());
    let old = log.effects[0].clone();
    log.model("claude:sonnet").unwrap();
    // Nothing starts beside the old agent: first it is stopped.
    assert_eq!(log.item().phase, Phase::Cancelling);
    assert_eq!(log.effects.len(), 1);
    assert_eq!(log.effects[0].kind, EffectKind::Stop { operation: old.id.clone() });
    let stop = log.effects[0].clone();
    log.observe(Outcome::Stopped { operation: old.id.clone(), receipt: "stopped".into() });
    assert_ne!(stop.id, old.id);
    assert_eq!(log.item().phase, Phase::Running);
    let build = &log.effects[0];
    assert_eq!((build.kind.clone(), build.model.as_deref(), build.tree.clone()),
        (EffectKind::Execute(StageKind::Build), Some("claude:sonnet"), Some(tree())));
    assert_ne!(build.fence, old.fence);
}

#[test]
fn a_queued_ticket_takes_the_new_model_without_an_agent_to_stop() {
    let mut d = coding();
    d.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    let mut log = Log::started(d);
    assert_eq!(log.item().phase, Phase::WaitingApproval);
    log.model("claude:sonnet").unwrap();
    assert_eq!(log.item().phase, Phase::WaitingApproval);
    assert!(log.effects.is_empty());
    assert_eq!(log.item().models["build"], "claude:sonnet");
}

#[test]
fn moving_the_build_while_a_check_runs_leaves_the_check_alone() {
    let mut log = Log::started(coding());
    log.observe(Outcome::Passed(ev("c1")));
    let check = log.item().pending.clone();
    log.model("claude:sonnet").unwrap();
    assert_eq!((log.item().phase.clone(), log.item().pending.clone()), (Phase::Running, check));
    assert!(log.effects.is_empty());
    assert_eq!(log.item().models["build"], "claude:sonnet");
}

#[test]
fn an_unlisted_model_is_refused_with_the_listed_ones_and_so_is_a_finished_ticket() {
    let mut log = Log::started(coding());
    let Err(Refusal::ModelRefused(why)) = log.model("claude:haiku") else { panic!("not refused") };
    assert!(why.contains("codex:sol") && why.contains("claude:opus"), "{why}");
    log.observe(Outcome::Passed(ev("c1")));
    log.observe(Outcome::Passed(ev("c1")));
    assert!(log.model("claude:sonnet").is_err());
    assert_eq!(log.try_run(Command::SetModel { item: "ov-1".into(), model: "claude:sonnet".into(), step: None },
        Grant::Decision { principal: "owner".into() }), Err(Refusal::Unauthorized));
}

#[test]
fn a_run_out_of_review_rounds_moves_as_retry_would_back_to_its_build() {
    let mut d = coding();
    d.stages.push({ let mut s = d.stages[1].clone(); s.id = "review".into(); s.kind = StageKind::Review(Reviewer::Orchestrator("orchestrator".into())); s });
    d.max_reworks = 1;
    d.rework_target = Some("build".into());
    let mut log = Log::started(d);
    for round in ["c1", "c2"] {
        log.observe(Outcome::Passed(ev(round)));
        log.observe(Outcome::Passed(ev(round)));
        let run = log.item().run.clone();
        log.try_run(Command::Review { item: "ov-1".into(), run, evidence: ev(round), passed: false, reason: Some("Again.".into()) },
            Grant::Decision { principal: "orchestrator".into() }).unwrap();
    }
    assert!(log.item().review_restart_stage().is_some(), "{:?}", log.item());
    log.model("claude:opus").unwrap();
    assert_eq!((log.item().stage, log.item().phase.clone(), log.item().iteration), (0, Phase::Running, 0));
    assert_eq!(log.effects[0].model.as_deref(), Some("claude:opus"));
}

/// A ticket waiting its turn to start has no run yet. Its pick is kept for the run, over the start policy's own.
#[test]
fn a_ticket_waiting_to_start_keeps_its_new_model_for_its_run() {
    let rest = || {
        let mut log = Log { state: State::empty("workspace".into(), 1), effects: Vec::new() };
        let ticket = Definition::ticket();
        log.try_run(Command::Enqueue { item: "ov-1".into(), policy_pin: "policy-1".into(), resource: "res".into(),
            dependencies: BTreeSet::new(), initial_candidate: Some(ticket.pin()), definition: ticket }, Grant::Controller).unwrap();
        log
    };
    let pick = |log: &mut Log, step: Option<&str>| log.try_run(Command::SetModel { item: "ov-1".into(),
        model: "claude:opus".into(), step: step.map(Into::into) }, Grant::Controller);
    let start = |log: &mut Log, models: &[(&str, &str)]| log.try_run(Command::Start { start_load: None, item: "ov-1".into(),
        policy_pin: "policy-2".into(), resource: "run".into(), dependencies: BTreeSet::new(), initial_candidate: None,
        definition: coding(), models: models.iter().map(|(s, m)| ((*s).into(), (*m).into())).collect(),
        tree: Some(Box::new(tree())), with: vec![] }, Grant::Controller).unwrap();
    // A ticket's own columns list no models, so the step has to be named.
    assert!(matches!(pick(&mut rest(), None), Err(Refusal::ModelRefused(_))));
    let mut log = rest();
    let before = log.item().clone();
    pick(&mut log, Some("build")).unwrap();
    assert_eq!((&log.item().phase, log.item().stage, &log.item().run), (&before.phase, before.stage, &before.run));
    assert!(log.effects.is_empty());
    start(&mut log, &[]);
    assert_eq!((log.effects[0].model.as_deref(), log.effects[0].model_chosen), (Some("claude:opus"), true));
    // A model named at Start is the later word.
    let mut log = rest();
    pick(&mut log, Some("build")).unwrap();
    start(&mut log, &[("build", "claude:sonnet")]);
    assert_eq!((log.effects[0].model.as_deref(), log.effects[0].model_chosen), (Some("claude:sonnet"), false));
}
