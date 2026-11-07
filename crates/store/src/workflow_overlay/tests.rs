//! Private fixtures only: a scratch store, throwaway tasks, and a group state written as the engine would.
use super::*;
use crate::{Store, models::Actor};
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

fn stage(id: &str, kind: StageKind) -> Stage {
    Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: OperationSpec { adapter_version: "fixture/v1".into(), arguments: vec![id.into()], environment: BTreeMap::new() },
        routes: BTreeMap::new(), limit: 0 }
}
fn steps() -> Vec<Stage> {
    vec![stage("base", StageKind::AwaitBase), stage("fold", StageKind::Fold), stage("gate", StageKind::Run),
         stage("push", StageKind::Push), stage("ci", StageKind::AwaitCi)]
}
fn group(at: usize, phase: Phase, candidate: Option<&str>, members: &[&str]) -> Item {
    let definition = Definition { schema: 2, source: "group/landing".into(), stages: steps(), max_reworks: 0,
        rework_target: None, groups: BTreeMap::new() };
    Item { definition, pin: "pin".into(), policy_pin: "policy".into(), dependencies: BTreeSet::new(),
        resource: "ref:origin:main".into(), stage: at, iteration: 0, attempt: 0, fence: 1, phase, run: "run".into(),
        pending: None, candidate: candidate.map(Into::into), last_evidence: None, stopping: None,
        rework_history: vec![], back_edges: BTreeMap::new(), retries: 0, pushed: None,
        members: members.iter().map(|m| Member { item: (*m).into(), candidate: format!("cand-{m}") }).collect(),
        handed: vec![], since_ms: 0, wait_since: None,
        group_of: Some(GroupOf { name: "landing".into(), pin: "pin".into() }), grouped_in: None }
}
fn ci(sha: &str, status: CiStatus) -> CiRead {
    CiRead { subject: sha_subject(sha), sha: sha.into(), status, url: String::new(), jobs: vec![], fetched_at: 1, changed_at: 1, asked_at: 1 }
}
fn view(item: Item) -> GroupLanding {
    let mut state = State::empty("w".into(), 1);
    state.items.insert("group-1".into(), item);
    groups_in(&state, &|_| None, &|_| 0).remove(0)
}

#[test]
fn each_verb_maps_to_the_phase_a_person_would_name() {
    let at = |i, p| view(group(i, p, Some("abc1234"), &[])).phase(None);
    assert_eq!(at(0, Phase::Blocked), LandingPhase::WaitingForPreviousLanding);
    assert_eq!(at(0, Phase::Running), LandingPhase::WaitingForMain);
    assert_eq!(at(1, Phase::Running), LandingPhase::Integrating);
    assert_eq!(at(2, Phase::Running), LandingPhase::GatingLocally);
    assert_eq!(at(3, Phase::Running), LandingPhase::GatingLocally);
    assert_eq!(at(4, Phase::Running), LandingPhase::WaitingForCi);
    assert_eq!(at(4, Phase::Failed), LandingPhase::Stalled, "a failure with no word from CI is not a red run");
    assert_eq!(at(2, Phase::Failed), LandingPhase::Stalled);
    assert_eq!(at(3, Phase::Uncertain), LandingPhase::Stalled, "an unknown push is not claimed as landed or not");
    assert_eq!(at(2, Phase::WaitingApproval), LandingPhase::ReadyToLand);
}

#[test]
fn a_commit_is_pushed_only_once_the_push_step_is_behind_the_group() {
    assert_eq!(view(group(2, Phase::Running, Some("abc1234"), &[])).pushed_sha, None, "gating: the candidate is not out yet");
    assert_eq!(view(group(3, Phase::Running, Some("abc1234"), &[])).pushed_sha, None, "pushing: nobody knows yet");
    assert_eq!(view(group(4, Phase::Running, Some("abc1234"), &[])).pushed_sha.as_deref(), Some("abc1234"));
    assert_eq!(view(group(1, Phase::Running, Some("abc1234"), &[])).pushed_sha, None, "a moved base sent it back");
}

#[test]
fn ci_on_the_pushed_commit_moves_the_phase_and_the_stored_state_word() {
    let g = view(group(4, Phase::Running, Some("abc1234"), &[]));
    assert_eq!(g.state(None), LandingState::Pushed);
    assert_eq!(g.state(Some(&ci("abc1234", CiStatus::Running))), LandingState::Pushed);
    assert_eq!(g.state(Some(&ci("abc1234", CiStatus::Passed))), LandingState::Green);
    assert_eq!(g.phase(Some(&ci("abc1234", CiStatus::Failed))), LandingPhase::CiFailed);
    assert_eq!(g.state(Some(&ci("abc1234", CiStatus::Failed))), LandingState::Red);
}

fn board_with_group(item: Item, members: &[&str]) -> (Store, Uuid, Vec<crate::models::Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("Overlay");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let tasks: Vec<_> = members.iter().map(|m| store.create_task(ws, m, Actor::User).unwrap()).collect();
    let mut state = State::empty(ws.to_string(), 1);
    state.revision = 3;
    for (m, t) in members.iter().zip(&tasks) {
        let _ = m;
        let mut ticket = group(0, Phase::Joined, None, &[]);
        ticket.group_of = None;
        state.items.insert(t.key.clone(), ticket);
    }
    let mut item = item;
    item.members = tasks.iter().map(|t| Member { item: t.key.clone(), candidate: "c".into() }).collect();
    state.items.insert("group-00000000000000000002-landing".into(), item);
    let conn = store.conn();
    conn.execute("INSERT INTO workflow_states(workspace,revision,epoch,state) VALUES(?1,3,1,?2)", params![ws.to_string(), serde_json::to_string(&state).unwrap()]).unwrap();
    for t in &tasks {
        conn.execute("INSERT INTO workflow_owned_items(workspace,item,task_id) VALUES(?1,?2,?3)", params![ws.to_string(), t.key, t.id.as_bytes().as_slice()]).unwrap();
    }
    drop(conn);
    (store, ws, tasks)
}

#[test]
fn the_plan_draws_a_live_group_as_a_landing_with_its_tickets_and_phase() {
    let (store, ws, tasks) = board_with_group(group(2, Phase::Running, Some("abc1234"), &[]), &["first", "second"]);
    let plan = store.plan(ws, i64::MIN).unwrap();
    let [view] = plan.landings.as_slice() else { panic!("one group, one landing: {:?}", plan.landings.len()) };
    assert_eq!(view.phase, LandingPhase::GatingLocally);
    assert_eq!(view.title, "Landing");
    assert_eq!(view.landing.tickets, tasks.iter().map(|t| t.id).collect::<Vec<_>>());
    assert_eq!(view.summary, "First; Second");
    assert_eq!(plan.landings[0].landing.id, store.plan(ws, i64::MIN).unwrap().landings[0].landing.id, "the row keeps its id between reads");
}

#[test]
fn a_pushed_group_names_its_commit_for_the_ci_watch_and_shows_what_ci_says() {
    let (store, ws, _) = board_with_group(group(4, Phase::Running, Some("ABC1234"), &[]), &["only"]);
    assert_eq!(store.workflow_ci_subjects().unwrap(), vec![(ws, "sha:abc1234".to_string())]);
    store.record_ci(ws, &ci("abc1234", CiStatus::Failed)).unwrap();
    let plan = store.plan(ws, i64::MIN).unwrap();
    assert_eq!(plan.landings[0].phase, LandingPhase::CiFailed);
    assert_eq!(plan.landings[0].landing.ci_subject().as_deref(), Some("sha:abc1234"), "the chip reads under the same subject");
}

#[test]
fn a_board_with_no_engine_state_has_no_group_landings() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("Plain");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    assert!(store.workflow_groups(ws).unwrap().is_empty());
    assert!(store.workflow_ci_subjects().unwrap().is_empty());
}

// Groups from real reducer steps: the engine hands a finished group's members back in the step that
// ends it, so a group that has ended must still name the tickets it ran.

fn act(id: &str, kind: StageKind) -> Stage { stage(id, kind) }
fn ticket_definition() -> Definition {
    let mut d = Definition::coding();
    d.schema = 2; d.max_reworks = 0; d.rework_target = None;
    d.stages.truncate(3);
    d.stages[2].routes.insert("rejected".into(), Target::Step("stage-1".into()));
    d.stages[2].limit = 2;
    let mut join = act("join", StageKind::Join { group: "landing".into(), adopt: true });
    join.routes.insert("rejected".into(), Target::Step("stage-1".into()));
    join.limit = 1;
    d.stages.push(join);
    d.stages.push(act("tidy", StageKind::Ci));
    d.groups.insert("landing".into(), GroupDef { limit: 2, exclusive: "ref:origin:main".into(), steps: steps() });
    d
}
struct Engine { state: State }
impl Engine {
    fn run(&mut self, command: Command, grant: Grant) {
        let s = &self.state;
        let input = Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch, source: "fixture".into(),
            source_sequence: s.revision + 1, now_ms: 1000 + s.revision as i64, grant, command };
        self.state = step(s, &input).unwrap().state;
    }
    fn observe(&mut self, item: &str, outcome: impl FnOnce(Evidence) -> Outcome) {
        let i = &self.state.items[item];
        let (effect, run, fence) = (i.pending.clone().unwrap(), i.run.clone(), i.fence);
        let evidence = Evidence { candidate: i.candidate.clone().unwrap_or_else(|| "f".repeat(40)), manifest: "m".into(),
            receipt: "r".into(), policy_pin: i.policy_pin.clone() };
        self.run(Command::Observe { item: item.into(), effect: effect.clone(), run, fence, outcome: outcome(evidence) },
            Grant::Operation { effect, run: self.state.items[item].run.clone(), fence });
    }
}
/// One ticket, queued into a group, walked to the stage `at` with every earlier step passed.
fn engine_group(ticket: &str, at: usize) -> (Engine, String) {
    let mut e = Engine { state: State::empty("w".into(), 1) };
    e.run(Command::Enqueue { item: ticket.into(), definition: ticket_definition(), policy_pin: "policy".into(),
        resource: format!("res-{ticket}"), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    while e.state.items[ticket].phase != Phase::WaitingJoin { e.observe(ticket, Outcome::Passed); }
    e.run(Command::Queue { item: ticket.into() }, Grant::Controller);
    let group = e.state.items[ticket].grouped_in.clone().unwrap();
    while e.state.items[&group].stage < at {
        e.observe(&group, |ev| Outcome::Passed(Evidence { candidate: "f".repeat(40), ..ev }));
    }
    (e, group)
}
fn viewed(e: &Engine, ticket: &str, group: &str) -> GroupLanding {
    let id = Uuid::from_u128(7);
    let key = ticket.to_string();
    groups_in(&e.state, &|i| (i == key).then_some(id), &|_| 0).into_iter().find(|g| g.group == group).unwrap()
}

#[test]
fn a_group_whose_ci_failed_still_names_its_tickets() {
    let (mut e, group) = engine_group("a", 4);
    e.observe(&group, Outcome::Rejected);
    assert_eq!(e.state.items[&group].phase, Phase::Failed);
    assert!(e.state.items[&group].members.is_empty(), "the engine has handed them back already");
    let g = viewed(&e, "a", &group);
    assert_eq!(g.tickets.len(), 1, "the landing still shows its ticket");
    assert_eq!(g.phase(Some(&ci(&"f".repeat(40), CiStatus::Failed))), LandingPhase::CiFailed);
}

#[test]
fn a_wait_that_failed_is_not_called_a_failed_ci_run() {
    let (mut e, group) = engine_group("a", 4);
    e.observe(&group, |ev| Outcome::Labeled("timeout".into(), ev));
    let g = viewed(&e, "a", &group);
    assert_eq!(e.state.items[&group].phase, Phase::Failed);
    assert_eq!(g.phase(None), LandingPhase::Stalled);
    assert_eq!(g.phase(Some(&ci(&"f".repeat(40), CiStatus::Running))), LandingPhase::Stalled);
}

#[test]
fn a_group_says_when_it_reached_the_step_it_is_at() {
    let (mut e, group) = engine_group("a", 2);
    let at_gate = e.state.items[&group].since_ms;
    e.observe(&group, Outcome::Passed);
    assert!(e.state.items[&group].since_ms > at_gate, "moving to the next step moves the time");
    assert!(at_gate > 0);
}

#[test]
fn the_notice_that_a_group_failed_names_the_tickets_it_ran() {
    let (e, group) = engine_group("a", 4);
    let before = e.state.clone();
    let mut e = e;
    e.observe(&group, Outcome::Rejected);
    let input = Input { busy_resources: BTreeSet::new(), schema: 1, id: "fail".into(), workspace: "w".into(),
        expected_revision: before.revision, epoch: 1, source: "fixture".into(), source_sequence: 1, now_ms: 2000,
        grant: Grant::Controller, command: Command::Advance };
    let store = Store::open_in_memory().unwrap();
    let mut conn = store.conn();
    conn.execute("INSERT INTO workflow_states(workspace,revision,epoch,state) VALUES('w',?1,1,?2)",
        params![e.state.revision as i64, serde_json::to_string(&e.state).unwrap()]).unwrap();
    conn.execute("INSERT INTO workflow_events(workspace,revision,id,payload,payload_hash,notices) VALUES('w',?1,'fail','{}','h','[]')",
        params![e.state.revision as i64]).unwrap();
    let tx = conn.transaction().unwrap();
    crate::workflow_notices::select(&tx, &input, &before, &e.state).unwrap();
    tx.commit().unwrap();
    drop(conn);
    let notices = store.workflow_pending_notices(0).unwrap();
    let failed = notices.iter().map(|(_, d)| &d.notice).find(|n| n.item == group).expect("a notice for the group");
    assert_eq!(failed.tickets, vec!["a".to_string()]);
}
