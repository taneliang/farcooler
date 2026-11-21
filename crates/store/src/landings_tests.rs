//! The old landings as history (ov-960): read, never moved.

use super::*;

use crate::models::Actor;
use crate::testing::old_landing;
use crate::ticket_lines::{LineUpdate, NewTicketLine, Stage};

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap().legacy_for_tests();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Bring the CI chip back", Actor::Manager).unwrap().id;
    store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
    for s in [Stage::Building, Stage::Reviewing, Stage::Landing] {
        store.update_line(task, &LineUpdate { stage: Some(s), ..Default::default() }, Actor::Manager).unwrap();
    }
    (store, main, task)
}

/// Historical rows remain readable but never enter the Plan, whatever their state or age.
#[test]
fn old_landings_remain_available_without_entering_the_plan() {
    let (store, main, task) = board();
    let now = crate::tasks::now_millis();
    for (n, state) in LandingState::WORDS.iter().enumerate() {
        old_landing(&store, main, n as u32 + 1, state, Some("abcdef1"), &[task], now);
    }
    assert!(store.plan(main, 0).unwrap().landings.is_empty());
    assert_eq!(store.landings(main).unwrap().len(), LandingState::WORDS.len());
    let kept = store.landing_numbered(main, 1).unwrap();
    assert_eq!((kept.pushed_sha.as_deref(), kept.tickets.as_slice()), (Some("abcdef1"), &[task][..]));
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
    assert!(plan.landings.is_empty());
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
