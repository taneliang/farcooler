use super::*;

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn subagent(agent: &str) -> WorkerRecord {
    WorkerRecord {
        harness: "claude".into(),
        agent_id: agent.into(),
        session_id: Some("76e86926".into()),
        session_cwd: Some("/r".into()),
        orchestrator_terminal: None,
        label: Some("ov-1 Mac polish".into()),
        model: None,
        linked_by: LinkedBy::Orchestrator,
    }
}

fn worker_notes(store: &Store, task: Uuid) -> Vec<String> {
    store.notes_for(task, Some(NoteKind::Worker)).unwrap().into_iter().map(|n| n.body).collect()
}

/// A subagent at work means its task started: a backlog or todo task moves
/// to in progress in the same write, with one `status_change` note.
#[test]
fn recording_a_subagent_on_a_todo_task_starts_it() {
    let (store, _, t) = board(1);
    store.set_task_status(t[0].id, TaskStatus::Todo, Actor::Manager).unwrap();
    let task = store.record_worker(t[0].id, &subagent("a1"), Actor::Manager).unwrap();
    assert_eq!(task.status, TaskStatus::InProgress);
    let moves = store.notes_for(t[0].id, Some(NoteKind::StatusChange)).unwrap();
    assert_eq!(moves.len(), 2, "todo, then in progress: one note for this move");
    assert_eq!(moves[1].extra["to"], "in_progress");
    assert_eq!(worker_notes(&store, t[0].id), ["Claude subagent started: ov-1 Mac polish"]);
}

/// The same subagent recorded twice is one row; the second fills in what
/// it knows and writes no note.
#[test]
fn recording_the_same_subagent_twice_is_one_row() {
    let (store, _, t) = board(1);
    store.record_worker(t[0].id, &WorkerRecord { model: None, ..subagent("a1") }, Actor::Manager).unwrap();
    store.record_worker(t[0].id, &WorkerRecord { label: None, model: Some("opus".into()), ..subagent("a1") }, Actor::Manager).unwrap();
    let workers = store.workers_for(t[0].id).unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!((workers[0].label.as_str(), workers[0].model.as_str()), ("ov-1 Mac polish", "opus"), "kept, and filled in");
    assert_eq!(worker_notes(&store, t[0].id).len(), 1);
}

/// A task finishing closes its open subagents, `task_closed`, and the move
/// says how many.
#[test]
fn a_subagent_on_a_task_that_finishes_is_closed() {
    let (store, _, t) = board(1);
    store.record_worker(t[0].id, &subagent("a1"), Actor::Manager).unwrap();
    store.record_worker(t[0].id, &subagent("a2"), Actor::Manager).unwrap();
    store.set_task_status(t[0].id, TaskStatus::Done, Actor::Manager).unwrap();

    let workers = store.workers_for(t[0].id).unwrap();
    assert!(workers.iter().all(|w| w.ended_at.is_some() && w.end_reason == Some(EndReason::TaskClosed)), "{workers:?}");
    let moved = store.notes_for(t[0].id, Some(NoteKind::StatusChange)).unwrap().pop().unwrap();
    assert_eq!(moved.extra["workers_closed"], 2);
    let refused = store.record_worker(t[0].id, &subagent("a3"), Actor::Manager);
    assert!(matches!(refused, Err(DomainError::InvalidArgument { what: "task_closed" })));
}

/// An end is recorded once; a resume reopens the same row and says so.
#[test]
fn a_subagent_ends_once_and_a_resume_reopens_it() {
    let (store, _, t) = board(1);
    store.record_worker(t[0].id, &subagent("a1"), Actor::Manager).unwrap();
    store.end_worker(t[0].id, "claude", Some("a1"), EndReason::Finished, Actor::Manager).unwrap();
    store.end_worker(t[0].id, "claude", Some("a1"), EndReason::Stopped, Actor::Manager).unwrap();
    let ended = store.workers_for(t[0].id).unwrap().remove(0);
    assert_eq!(ended.end_reason, Some(EndReason::Finished), "the first end stands");

    store.record_worker(t[0].id, &WorkerRecord { label: None, ..subagent("a1") }, Actor::Manager).unwrap();
    let resumed = store.workers_for(t[0].id).unwrap();
    assert_eq!((resumed.len(), resumed[0].ended_at), (1, None));
    assert_eq!(
        worker_notes(&store, t[0].id),
        ["Claude subagent started: ov-1 Mac polish", "Claude subagent finished.", "Claude subagent resumed: ov-1 Mac polish"]
    );
    let missing = store.end_worker(t[0].id, "claude", Some("nobody"), EndReason::Finished, Actor::Manager);
    assert!(matches!(missing, Err(DomainError::NotFound)));
}

/// A board shows every open subagent, then the most recently closed one.
#[test]
fn a_board_shows_the_open_ones_and_the_last_closed() {
    let (store, _, t) = board(1);
    for agent in ["a1", "a2", "a3"] {
        store.record_worker(t[0].id, &subagent(agent), Actor::Manager).unwrap();
    }
    store.end_worker(t[0].id, "claude", Some("a1"), EndReason::Finished, Actor::Manager).unwrap();
    store.end_worker(t[0].id, "claude", Some("a2"), EndReason::Failed, Actor::Manager).unwrap();
    let task = store.get_task(t[0].id).unwrap();
    let shown = store.task_facts(&[task]).unwrap().remove(0).workers;
    assert_eq!(shown.iter().map(|w| w.agent_id.as_str()).collect::<Vec<_>>(), ["a3", "a2"]);
}

#[test]
fn a_key_links_only_where_the_description_starts_with_it() {
    assert_eq!(leading_key("ov-12: polish the sidebar"), Some("ov-12"));
    assert_eq!(leading_key("  ov-12 polish"), Some("ov-12"));
    assert_eq!(leading_key("ov-212"), Some("ov-212"));
    assert_eq!(leading_key("bil2-3: x"), Some("bil2-3"));
    assert_eq!(leading_key("after ov-92 lands"), None);
    assert_eq!(leading_key("ov-12a polish"), None);
    assert_eq!(leading_key("ov-: x"), None);
    assert_eq!(leading_key("-3: an old key"), None);
    assert_eq!(leading_key("Assess simulator feasibility"), None);
}

/// The runner's link from a description: only a leading key on this board,
/// never over what the orchestrator recorded, and noted on the card.
#[test]
fn a_subagent_links_from_a_leading_key_on_its_own_board() {
    let (store, main, t) = board(2);
    let key = t[0].key.clone();
    let linked = store.link_worker_by_description(main, &format!("{key}: Mac polish"), &subagent("a1")).unwrap();
    assert_eq!(linked.map(|t| t.id), Some(t[0].id));
    let worker = store.workers_for(t[0].id).unwrap().remove(0);
    assert_eq!(worker.linked_by, LinkedBy::Description);
    let note = store.notes_for(t[0].id, Some(NoteKind::Worker)).unwrap().remove(0);
    assert_eq!((note.actor, note.body.as_str()), (Actor::Runner, "Claude subagent linked from its description: ov-1 Mac polish"));

    let elsewhere = format!("after {} lands", t[1].key);
    assert_eq!(store.link_worker_by_description(main, &elsewhere, &subagent("a2")).unwrap(), None);
    store.record_worker(t[1].id, &subagent("a3"), Actor::Manager).unwrap();
    let already = store.link_worker_by_description(main, &format!("{key}: again"), &subagent("a3")).unwrap();
    assert_eq!(already, None, "recorded by the orchestrator on another task");
    let other_board = store.create_workspace(t[0].repository_id, "Billing", "bil").unwrap().id;
    assert_eq!(store.link_worker_by_description(other_board, &format!("{key}: x"), &subagent("a4")).unwrap(), None);
}

/// The orchestrator's record wins over the runner's guess: on the same task
/// the row becomes the orchestrator's; a guess open on another task is
/// closed there, `relinked`, and says so.
#[test]
fn the_orchestrator_s_record_overrides_a_description_link() {
    let (store, main, t) = board(2);
    let key = t[0].key.clone();
    store.link_worker_by_description(main, &format!("{key}: a guess"), &subagent("a1")).unwrap();
    store.record_worker(t[0].id, &subagent("a1"), Actor::Manager).unwrap();
    assert_eq!(store.workers_for(t[0].id).unwrap()[0].linked_by, LinkedBy::Orchestrator);

    store.link_worker_by_description(main, &format!("{key}: wrong"), &subagent("a2")).unwrap();
    store.record_worker(t[1].id, &subagent("a2"), Actor::Manager).unwrap();
    let wrong = store.workers_for(t[0].id).unwrap().into_iter().find(|w| w.agent_id == "a2").unwrap();
    assert_eq!(wrong.end_reason, Some(EndReason::Relinked));
    assert_eq!(worker_notes(&store, t[0].id).last().unwrap(), "Claude subagent was recorded on another task.");
    assert!(store.workers_for(t[1].id).unwrap()[0].ended_at.is_none(), "open where the orchestrator put it");
}

/// `task worker KEY --done` with no id: every open subagent on the task
/// ends; none open is `NotFound`.
#[test]
fn ending_with_no_id_ends_every_open_one() {
    let (store, _, t) = board(1);
    store.record_worker(t[0].id, &subagent("a1"), Actor::Manager).unwrap();
    store.record_worker(t[0].id, &WorkerRecord { harness: "codex".into(), ..subagent("/root/b") }, Actor::Manager).unwrap();
    store.end_worker(t[0].id, "claude", None, EndReason::Finished, Actor::Manager).unwrap();
    assert!(store.workers_for(t[0].id).unwrap().iter().all(|w| w.end_reason == Some(EndReason::Finished)));
    let none = store.end_worker(t[0].id, "claude", None, EndReason::Finished, Actor::Manager);
    assert!(matches!(none, Err(DomainError::NotFound)));
}
