//! The old landings as history (ov-960): read, never moved.

use super::*;

use crate::landing_phase::LandingPhase;
use crate::models::Actor;
use crate::testing::old_landing;
use crate::ticket_lines::{LineUpdate, NewTicketLine, Stage};

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Bring the CI chip back", Actor::Manager).unwrap().id;
    store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
    for s in [Stage::Building, Stage::Reviewing, Stage::Landing] {
        store.update_line(task, &LineUpdate { stage: Some(s), ..Default::default() }, Actor::Manager).unwrap();
    }
    (store, main, task)
}

/// Landing 373 was pushed and waiting on CI when the engine took over. The
/// plan reads it as landed, with its ticket named, and no ticket as on it.
#[test]
fn a_landing_left_waiting_for_ci_reads_as_landed_history() {
    let (store, main, task) = board();
    let now = crate::tasks::now_millis();
    let id = old_landing(&store, main, 373, "pushed", Some("abcdef1"), &[task], now - 60_000);
    let plan = store.plan(main, now - 3_600_000).unwrap();
    let [view] = plan.landings.as_slice() else { panic!("{:?}", plan.landings) };
    assert_eq!((view.landing.id, view.phase, view.phase_since), (id, LandingPhase::Landed, now - 60_000));
    assert_eq!(view.title, "Landing 373");
    assert_eq!(view.summary, "Bring the CI chip back");
    assert!(plan.cards.iter().any(|c| c.task_id == task), "its ticket is named");
    assert!(plan.lines.iter().all(|l| l.landing.is_none()), "no ticket is on an old landing");
    // What it was is kept: `landing show` still has it as it was left.
    let kept = store.landing_numbered(main, 373).unwrap();
    assert_eq!((kept.state, kept.pushed_sha.as_deref(), kept.tickets.as_slice()), (LandingState::Pushed, Some("abcdef1"), &[task][..]));
}

/// An old landing of any state is settled: once its last move is older than
/// the plan's window it leaves the read, as a landed one does.
#[test]
fn every_old_landing_leaves_the_read_with_the_settled_ones() {
    let (store, main, _) = board();
    let now = crate::tasks::now_millis();
    for (n, state) in LandingState::WORDS.iter().enumerate() {
        old_landing(&store, main, n as u32 + 1, state, None, &[], now - 2 * 3_600_000);
    }
    let fresh = old_landing(&store, main, 99, "open", None, &[], now - 60_000);
    let plan = store.plan(main, now - 3_600_000).unwrap();
    let ids: Vec<Uuid> = plan.landings.iter().map(|v| v.landing.id).collect();
    assert_eq!(ids, vec![fresh], "only the one that moved within the window");
    assert_eq!(plan.landings[0].phase, LandingPhase::Dropped, "nothing was pushed, and nothing will push it now");
    assert_eq!(store.landings(main).unwrap().len(), LandingState::WORDS.len() + 1, "every one is kept");
}

/// Newest move first, and a deleted board with old landings says so.
#[test]
fn old_landings_read_newest_first_and_keep_their_board() {
    let (store, main, _) = board();
    let now = crate::tasks::now_millis();
    let older = old_landing(&store, main, 1, "landed", Some("abcdef1"), &[], now - 120_000);
    let newer = old_landing(&store, main, 2, "red", Some("abcdef2"), &[], now - 60_000);
    let plan = store.plan(main, now - 3_600_000).unwrap();
    let read: Vec<(Uuid, LandingPhase)> = plan.landings.iter().map(|v| (v.landing.id, v.phase)).collect();
    assert_eq!(read, vec![(newer, LandingPhase::CiFailed), (older, LandingPhase::Landed)]);
    assert!(has_history(&store.conn(), main).unwrap());
}

/// Review B2: on a board that never cut over, a ticket left on an open old
/// landing read as Dropped but stayed the landing's: the engine refused it and
/// left it untracked, and no writer was left to take it off. History holds no
/// ticket, so the engine takes it like any other card.
#[test]
fn a_ticket_on_an_old_open_landing_is_free_for_the_engine() {
    use farcooler_workflow::{Command, Definition, Grant, Input};
    let (store, main, task) = board();
    assert!(store.legacy_authority(), "a board that never cut over");
    let now = crate::tasks::now_millis();
    old_landing(&store, main, 13, "open", None, &[task], now);
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.landings[0].phase, LandingPhase::Dropped);
    let untracked = crate::workflow_projection::untracked(&store.conn(), Some(task)).unwrap();
    assert_eq!(untracked.len(), 1, "the projection offers it to the engine");
    let key = store.get_task(task).unwrap().key;
    let definition = Definition::ticket();
    let input = Input {
        schema: 1, id: "b2".into(), workspace: main.to_string(), expected_revision: 0, epoch: 1,
        source: "controller".into(), source_sequence: 1, now_ms: 0, grant: Grant::Controller, busy_resources: Default::default(),
        command: Command::Enqueue { item: key, policy_pin: definition.pin(), resource: "b2".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition },
    };
    assert_eq!(store.workflow_apply(&input).unwrap().result.map(|_| ()), Ok(()), "the engine takes it");
}
