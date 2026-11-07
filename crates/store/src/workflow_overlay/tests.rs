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
        rework_history: vec![], back_edges: BTreeMap::new(), retries: 0,
        members: members.iter().map(|m| Member { item: (*m).into(), candidate: format!("cand-{m}") }).collect(),
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
    assert_eq!(at(4, Phase::Failed), LandingPhase::CiFailed);
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
