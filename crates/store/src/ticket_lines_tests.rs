use super::*;

use crate::line_migration::store_with_the_line;

fn board(n: usize) -> (Store, Uuid, Vec<Uuid>) {
    let store = store_with_the_line();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("card {i}"), Actor::Manager).unwrap().id).collect();
    (store, main, tasks)
}

fn on_line(store: &Store, tasks: &[Uuid]) {
    for task in tasks {
        store.put_on_line(*task, &NewTicketLine::default(), Actor::Manager).unwrap();
    }
}

fn to(store: &Store, task: Uuid, stage: Stage) -> Result<TicketLine> {
    store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager)
}

/// Walk a ticket to `stage` along the usual path.
fn walk(store: &Store, task: Uuid, stage: Stage) {
    for s in [Stage::Building, Stage::Reviewing, Stage::Landing, Stage::Landed] {
        to(store, task, s).unwrap();
        if s == stage {
            return;
        }
    }
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A ticket goes on the line once, waiting, with its branch and worktree.
#[test]
fn a_ticket_goes_on_the_line_once() {
    let (store, _, t) = board(1);
    let new = NewTicketLine { branch: "store-872".into(), worktree_path: "/wt/store-872".into(), reason: "slice 2".into(), ..Default::default() };
    let line = store.put_on_line(t[0], &new, Actor::Manager).unwrap();
    assert_eq!((line.stage, line.branch.as_str(), line.worktree_path.as_str()), (Stage::Waiting, "store-872", "/wt/store-872"));
    assert_eq!(refused(store.put_on_line(t[0], &new, Actor::Manager)), "on_the_line");
    assert!(matches!(store.put_on_line(Uuid::now_v7(), &new, Actor::Manager), Err(DomainError::NotFound)));
    let events = store.line_events(LineSubject::Ticket(t[0]), 0).unwrap();
    assert_eq!(events[0].body, "Waiting.");
}

/// Stages move along their arrows: review back to building for a fix, a
/// failed landing back to building, anyone to needs you or dropped; a closed
/// ticket takes no more moves.
#[test]
fn stages_move_along_their_arrows() {
    let (store, _, t) = board(1);
    on_line(&store, &t);
    assert_eq!(refused(to(&store, t[0], Stage::Reviewing)), "stage", "waiting can't skip building");
    to(&store, t[0], Stage::Building).unwrap();
    assert_eq!(refused(to(&store, t[0], Stage::Landing)), "stage", "nor building skip review");
    to(&store, t[0], Stage::Reviewing).unwrap();
    to(&store, t[0], Stage::Building).unwrap();
    to(&store, t[0], Stage::Reviewing).unwrap();
    to(&store, t[0], Stage::NeedsYou).unwrap();
    to(&store, t[0], Stage::Landing).unwrap();
    to(&store, t[0], Stage::Building).unwrap();
    to(&store, t[0], Stage::Reviewing).unwrap();
    to(&store, t[0], Stage::Landing).unwrap();
    let landed = store
        .update_line(t[0], &LineUpdate { stage: Some(Stage::Landed), landed_sha: Some("abc".into()), ..Default::default() }, Actor::Manager)
        .unwrap();
    assert_eq!((landed.stage, landed.landed_sha.as_deref()), (Stage::Landed, Some("abc")));
    assert_eq!(refused(to(&store, t[0], Stage::Dropped)), "ticket_closed");
    for word in Stage::WORDS {
        assert_eq!(Stage::parse(word).map(Stage::as_str), Some(word));
    }
}

/// The plan orders waiting tickets; one that starts building leaves it.
#[test]
fn the_plan_orders_waiting_tickets() {
    let (store, main, t) = board(3);
    on_line(&store, &t);
    store.set_line_order(main, &[t[2], t[0]], Actor::Manager).unwrap();
    let ranks = |store: &Store| store.ticket_lines(main).unwrap().into_iter().map(|l| (l.task_id, l.plan_rank)).collect::<Vec<_>>();
    assert_eq!(ranks(&store), vec![(t[2], Some(1)), (t[0], Some(2)), (t[1], None)]);
    store.set_line_order(main, &[t[1]], Actor::Manager).unwrap();
    assert_eq!(ranks(&store)[0], (t[1], Some(1)));
    assert_eq!(ranks(&store).iter().filter(|(_, r)| r.is_some()).count(), 1, "the rest left the plan");
    to(&store, t[1], Stage::Building).unwrap();
    assert_eq!(store.ticket_line(t[1]).unwrap().plan_rank, None);
    assert_eq!(refused(store.set_line_order(main, &[t[1]], Actor::Manager)), "not_waiting");
    assert_eq!(refused(store.set_line_order(main, &[t[0], t[0]], Actor::Manager)), "duplicate");
}

/// A stacked ticket's parent is another ticket on the same board, never
/// itself or its own child, and the child lands only once the parent has.
#[test]
fn a_stacked_ticket_lands_after_its_parent() {
    let (store, _, t) = board(3);
    on_line(&store, &t[..2]);
    let stack = |child: Uuid, parent: Uuid| {
        store.update_line(child, &LineUpdate { stack_parent: Some(Some(parent)), ..Default::default() }, Actor::Manager)
    };
    assert_eq!(refused(stack(t[1], t[1])), "stack_parent");
    assert_eq!(refused(stack(t[1], t[2])), "stack_parent", "a parent must be on the line");
    stack(t[1], t[0]).unwrap();
    assert_eq!(refused(stack(t[0], t[1])), "stack_cycle");
    walk(&store, t[1], Stage::Landing);
    assert_eq!(refused(to(&store, t[1], Stage::Landed)), "parent_not_landed");
    walk(&store, t[0], Stage::Landed);
    assert_eq!(to(&store, t[1], Stage::Landed).unwrap().stage, Stage::Landed);
}

/// Tickets carried over from one lane share its branch: none lands alone.
#[test]
fn tickets_sharing_a_branch_dont_land_alone() {
    let (store, _, t) = board(2);
    on_line(&store, &t);
    let lane = Uuid::now_v7();
    store.conn().execute("UPDATE ticket_lines SET same_branch = ?1", [uuid_blob(lane)]).unwrap();
    walk(&store, t[0], Stage::Landing);
    assert_eq!(refused(to(&store, t[0], Stage::Landed)), "same_branch");
    assert_eq!(store.ticket_line(t[0]).unwrap().same_branch, Some(lane));
}

/// A landed SHA is only for a landed ticket; the other fields set and clear.
#[test]
fn fields_set_and_clear() {
    let (store, _, t) = board(1);
    on_line(&store, &t);
    let set = LineUpdate { pr_number: Some(Some(42)), branch: Some("b".into()), held_off: Some(true), ..Default::default() };
    let line = store.update_line(t[0], &set, Actor::Manager).unwrap();
    assert_eq!((line.pr_number, line.branch.as_str(), line.held_off), (Some(42), "b", true));
    let clear = LineUpdate { pr_number: Some(None), ..Default::default() };
    assert_eq!(store.update_line(t[0], &clear, Actor::Manager).unwrap().pr_number, None);
    let sha = LineUpdate { landed_sha: Some("abc".into()), ..Default::default() };
    assert_eq!(refused(store.update_line(t[0], &sha, Actor::Manager)), "not_landed");
    assert!(line.resource_version > 1);
}
