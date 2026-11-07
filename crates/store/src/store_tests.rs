use farcooler_protocol::v1::TerminalIntent;

use super::*;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

// ---- the one load-bearing rule ----

#[test]
fn terminals_table_has_no_runtime_state_column() {
    let s = store();
    let cols = s.column_names("terminals");
    for banned in ["state", "is_running", "running", "pid", "alive"] {
        assert!(
            !cols.iter().any(|c| c.eq_ignore_ascii_case(banned)),
            "terminals table must never carry a `{banned}` column: tmux is the sole \
             authority for whether a process is alive right now, found columns {cols:?}"
        );
    }
    // And exactly the durable columns the design calls for, nothing more.
    let expected = [
        "id",
        "worktree_id",
        "title",
        "command_preset",
        "intent",
        "runtime_confirmed",
        "exit_code",
        "exit_signal",
        "lease_generation",
        "epoch",
        "columns",
        "rows",
        "resource_version",
        // Both of these are intent, which is why they are allowed to live
        // here. `pane_mode` says what the pane is FOR, exactly as
        // `command_preset` does. `agent_session_id` names a conversation
        // that outlives every pane hosting it, which is what makes
        // toggling pane mode land on the same conversation instead of a
        // new one. The conversation itself is never stored.
        "pane_mode",
        "agent_session_id",
        // Intent again: the task this pane was opened to work.
        "task_id",
        // And again: whose work it is, and what it is to that workspace.
        // An orchestrator is one because somebody said so, not because
        // anything was seen running.
        "workspace_id",
        "role",
        // Where it came from: the terminal a split made it beside, and
        // whether that was the orchestrator then. Set at creation and
        // never moved.
        "split_of",
        "split_of_orchestrator",
        // Intent too: the page a web pane was opened on (ov-435).
        "web_url",
    ];
    assert_eq!(cols.len(), expected.len(), "unexpected column set: {cols:?}");
    for e in expected {
        assert!(cols.iter().any(|c| c == e), "missing expected column {e}, have {cols:?}");
    }
}

// ---- the task a terminal was opened for ----

/// A repository, a worktree in it, and a task on its board.
fn a_lane_with_a_task(s: &Store) -> (Uuid, Uuid) {
    let repo = s.register_repository_for_test("Far Cooler");
    let ws = s.create_worktree(repo, "feat/x", "/wt/terminal-task", false).unwrap();
    let task = s.create_task_for_test(repo, "a task");
    (ws.id, task)
}

#[test]
fn a_terminal_created_for_a_task_reads_it_back() {
    let s = store();
    let (ws, task) = a_lane_with_a_task(&s);
    let made = s
        .create_terminal_for_task(ws, "w", "claude", TerminalIntent::Running, 80, 24, Some(task))
        .unwrap();
    assert_eq!(made.task_id, Some(task));
    assert_eq!(s.get_terminal(made.id).unwrap().task_id, Some(task), "read back from disk");
    assert_eq!(
        s.list_terminals_for_worktree(ws).unwrap()[0].task_id,
        Some(task),
        "and through the listing every launch path reads"
    );

    let plain = s.create_terminal(ws, "p", "shell", TerminalIntent::Running, 80, 24).unwrap();
    assert_eq!(s.get_terminal(plain.id).unwrap().task_id, None);
}

/// A split names the terminal it was split from; any other terminal names
/// none. Read back through both reads every caller uses.
#[test]
fn a_split_terminal_names_what_it_was_split_from() {
    let s = store();
    let (ws, task) = a_lane_with_a_task(&s);
    let first = s.create_terminal(ws, "p", "shell", TerminalIntent::Running, 80, 24).unwrap();
    let split = s
        .create_split_terminal(ws, "w", "claude", TerminalIntent::Running, 80, 24, Some(task), first.id, false)
        .unwrap();
    assert_eq!(split.split_of, Some(first.id));
    assert_eq!(split.task_id, Some(task), "and keeps its task");
    assert_eq!(s.get_terminal(split.id).unwrap().split_of, Some(first.id), "read back from disk");
    let listed = s.list_terminals_for_worktree(ws).unwrap();
    assert_eq!(listed.iter().find(|t| t.id == split.id).unwrap().split_of, Some(first.id));
    assert_eq!(s.get_terminal(first.id).unwrap().split_of, None, "a window of its own names none");
}

/// A split records whether it was made from the orchestrator, either way,
/// and reads it back; a terminal that isn't a split says neither.
#[test]
fn a_split_records_whether_it_was_made_from_the_orchestrator() {
    let s = store();
    let (ws, _) = a_lane_with_a_task(&s);
    let first = s.create_terminal(ws, "p", "claude", TerminalIntent::Running, 80, 24).unwrap();
    for from_orchestrator in [true, false] {
        let split = s
            .create_split_terminal(ws, "w", "shell", TerminalIntent::Running, 80, 24, None, first.id, from_orchestrator)
            .unwrap();
        assert_eq!(split.split_of_orchestrator, Some(from_orchestrator));
        assert_eq!(s.get_terminal(split.id).unwrap().split_of_orchestrator, Some(from_orchestrator));
        let listed = s.list_terminals_for_worktree(ws).unwrap();
        assert_eq!(listed.iter().find(|t| t.id == split.id).unwrap().split_of_orchestrator, Some(from_orchestrator));
    }
    assert_eq!(s.get_terminal(first.id).unwrap().split_of_orchestrator, None, "not a split: not known");
}

/// A terminal outlives its ticket. Deleting the task must neither fail on
/// the foreign key nor leave the terminal naming a task that is gone.
#[test]
fn deleting_the_task_leaves_the_terminal_and_forgets_the_link() {
    let s = store();
    let (ws, task) = a_lane_with_a_task(&s);
    let term = s
        .create_terminal_for_task(ws, "w", "claude", TerminalIntent::Running, 80, 24, Some(task))
        .unwrap();

    s.conn()
        .execute("DELETE FROM tasks WHERE id = ?1", params![uuid_blob(task)])
        .expect("a task with a terminal opened for it can still be deleted");

    let after = s.get_terminal(term.id).expect("the terminal is still there");
    assert_eq!(after.task_id, None, "and names no task that is gone");
}

/// `migrate.rs`'s own append-only tests build a bare connection and turn
/// `recursive_triggers` on themselves to prove the trigger-plus-pragma
/// combination works at all; none of them go through `Store::init`, so
/// none of them actually prove `Store::init` sets that pragma. This one
/// does, end to end through `Store::open_in_memory`, so a future edit
/// that drops the pragma from `init` (rather than from the migration)
/// fails here instead of only in a schema-level test that never runs it.
#[test]
fn a_stores_init_actually_closes_the_replace_gap_in_task_notes() {
    let s = store();
    s.conn()
        .execute_batch(
            "INSERT INTO repository_roots VALUES (x'01', x'02', '/r', 0, 1);
             INSERT INTO repositories VALUES (x'03', x'02', x'01', 'r', '/r/.git', '', 1, '');
             INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
                 VALUES (x'06', x'03', 'Main', 'fc', 1, 0, 0);
             INSERT INTO tasks (id, repository_id, workspace_id, key, title, status, status_since,
                                created_at, resource_version)
                 VALUES (x'04', x'03', x'06', 'fc-1', 'a task', 'backlog', 0, 0, 1);
             INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (x'05', x'04', 'decision', 'user', 0, 'because', '{}');",
        )
        .unwrap();

    let err = s
        .conn()
        .execute(
            "INSERT OR REPLACE INTO task_notes (id, task_id, kind, actor, at, body, extra)
             VALUES (x'05', x'04', 'decision', 'user', 0, 'rewritten, same id', '{}')",
            [],
        )
        .expect_err("Store::init must turn recursive_triggers on, or REPLACE walks through");
    assert!(
        err.to_string().contains("append-only") || err.to_string().contains("trigger"),
        "the schema itself refuses, not a comment asking nicely: {err}"
    );

    let body: String = s
        .conn()
        .query_row("SELECT body FROM task_notes WHERE id = x'05'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(body, "because", "REPLACE keeps the id, so only the body proves nothing moved");
}

/// Round 1's `BEFORE DELETE` trigger on `task_notes` had no `WHEN`
/// clause, and `ON DELETE CASCADE` fires delete triggers unconditionally
/// — unlike `REPLACE`'s implicit delete, this is never gated by
/// `recursive_triggers`. So the moment any task on a repository carried
/// a note, deleting that repository aborted with "task_notes is
/// append-only" and left every row in place, including the repository
/// itself. Nothing in this file called the real `Store::delete_repository`
/// before this test, which is exactly how that shipped: 44/44 green with
/// the bug live.
///
/// Goes through `Store::delete_repository`, the real function, not raw
/// SQL, because a schema-level test proving the trigger has the right
/// `WHEN` clause would not have proven this call site actually reaches
/// it.
#[test]
fn deleting_a_repository_takes_its_whole_board_with_it() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "r", "/repos/one/.git", "").unwrap();
    let main = s.ensure_main_workspace(repo.id).unwrap();
    let side = s.create_workspace(repo.id, "Side", "side").unwrap();
    let task_id = Uuid::now_v7();

    {
        let conn = s.conn();
        conn.execute(
            "INSERT INTO tasks (id, repository_id, workspace_id, key, title, status, status_since,
                                created_at, resource_version)
             VALUES (?1, ?2, ?3, 'fc-1', 'a task', 'backlog', 0, 0, 1)",
            params![uuid_blob(task_id), uuid_blob(repo.id), uuid_blob(side.id)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
             VALUES (?1, ?2, 'decision', 'user', 0, 'because', '{}')",
            params![uuid_blob(Uuid::now_v7()), uuid_blob(task_id)],
        )
        .unwrap();
    } // drop the guard: delete_repository locks the same mutex itself.

    s.delete_repository(repo.id, repo.resource_version).unwrap();

    let conn = s.conn();
    let repos: i64 = conn
        .query_row(
            "SELECT count(*) FROM repositories WHERE id = ?1",
            params![uuid_blob(repo.id)],
            |r| r.get(0),
        )
        .unwrap();
    let tasks: i64 = conn
        .query_row(
            "SELECT count(*) FROM tasks WHERE repository_id = ?1",
            params![uuid_blob(repo.id)],
            |r| r.get(0),
        )
        .unwrap();
    let workspaces: i64 = conn
        .query_row(
            "SELECT count(*) FROM workspaces WHERE id IN (?1, ?2)",
            params![uuid_blob(main.id), uuid_blob(side.id)],
            |r| r.get(0),
        )
        .unwrap();
    let notes: i64 = conn
        .query_row(
            "SELECT count(*) FROM task_notes WHERE task_id = ?1",
            params![uuid_blob(task_id)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        (repos, tasks, notes, workspaces),
        (0, 0, 0, 0),
        "the repository, its task, the task's note, and its workspaces, Main included, must all be gone"
    );
}

// ---- round trips ----

#[test]
fn repository_root_round_trip() {
    let s = store();
    let host = Uuid::now_v7();
    let created = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    assert_eq!(created.resource_version, 1);

    let fetched = s.get_repository_root(created.id).unwrap();
    assert_eq!(fetched, created);

    let listed = s.list_repository_roots().unwrap();
    assert_eq!(listed, vec![created]);
}

#[test]
fn repository_round_trip_and_update() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/repos/one/.git", "origin").unwrap();
    assert_eq!(s.get_repository(repo.id).unwrap(), repo);

    let updated = s.update_repository(repo.id, 1, "renamed", "/repos/one/.git", "origin2").unwrap();
    assert_eq!(updated.display_name, "renamed");
    assert_eq!(updated.resource_version, 2);
    assert_eq!(s.get_repository(repo.id).unwrap(), updated);

    let listed = s.list_repositories_for_root(root.id).unwrap();
    assert_eq!(listed, vec![updated]);
}

#[test]
fn worktree_round_trip_and_update() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/worktree", false).unwrap();
    assert!(!ws.hidden);
    assert_eq!(s.get_worktree(ws.id).unwrap(), ws);

    let updated =
        s.update_worktree(ws.id, 1, "feature/x", "/wt/worktree", true, false).unwrap();
    assert!(updated.hidden);
    assert_eq!(updated.resource_version, 2);
}

// ---- ordering ----
//
// The rule these guard is the whole reason `ordinal` exists: a card's
// position is decided at creation and then only by the user. Nothing here
// may ever come to depend on activity, attention, or when a pane last said
// something — a list that rearranges itself is a list you cannot reach into
// without reading it first.

/// Three repositories, one runner, and everything the store hands back is
/// in the order the rows were created.
fn ordered_names(s: &Store, repo: Uuid) -> Vec<String> {
    s.list_worktrees_for_repository(repo).unwrap().iter().map(|w| w.name()).collect()
}

#[test]
fn a_new_worktree_lands_at_the_end() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();

    // Created out of alphabetical order on purpose: if anything fell back
    // to sorting by name or path, this would come back sorted.
    let zebra = s.create_worktree(repo.id, "feat/z", "/wt/zebra", false).unwrap();
    let apple = s.create_worktree(repo.id, "feat/a", "/wt/apple", false).unwrap();
    let mango = s.create_worktree(repo.id, "feat/m", "/wt/mango", false).unwrap();

    assert_eq!((zebra.ordinal, apple.ordinal, mango.ordinal), (0, 1, 2));
    assert_eq!(ordered_names(&s, repo.id), vec!["zebra", "apple", "mango"]);
}

/// The rank is across the runner, not restarted per repository, so two
/// projects' cards never collide on one number.
#[test]
fn the_rank_counts_across_every_repository_on_the_runner() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos", 1_000).unwrap();
    let one = s.create_repository(host, root.id, "one", "/one/.git", "").unwrap();
    let two = s.create_repository(host, root.id, "two", "/two/.git", "").unwrap();

    let a = s.create_worktree(one.id, "b", "/wt/a", false).unwrap();
    let b = s.create_worktree(two.id, "b", "/wt/b", false).unwrap();
    let c = s.create_worktree(one.id, "b", "/wt/c", false).unwrap();

    assert_eq!((a.ordinal, b.ordinal, c.ordinal), (0, 1, 2));
    assert_eq!(
        s.list_worktrees_in_order().unwrap().iter().map(|w| w.name()).collect::<Vec<_>>(),
        vec!["a", "b", "c"],
        "one list across the runner, in one order"
    );
}

/// Reordering is a permutation of the slots the named rows already hold.
#[test]
fn reordering_moves_the_cards_and_survives_a_reopen() {
    let dir = farcooler_core::scratch::dir("order");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");

    let (repo, a, b, c) = {
        let s = Store::open(&path).unwrap();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "").unwrap();
        let a = s.create_worktree(repo.id, "b", "/wt/a", false).unwrap();
        let b = s.create_worktree(repo.id, "b", "/wt/b", false).unwrap();
        let c = s.create_worktree(repo.id, "b", "/wt/c", false).unwrap();
        assert_eq!(ordered_names(&s, repo.id), vec!["a", "b", "c"]);

        s.reorder_worktrees(&[c.id, a.id, b.id]).unwrap();
        assert_eq!(ordered_names(&s, repo.id), vec!["c", "a", "b"]);
        (repo.id, a.id, b.id, c.id)
    };

    // A daemon restart is a reopen of this file. An order held in memory
    // would be gone here, and an order derived from the work would be
    // whatever the work looked like now.
    let s = Store::open(&path).unwrap();
    assert_eq!(ordered_names(&s, repo), vec!["c", "a", "b"]);

    // And the version moved, so a client holding the old row knows its copy
    // of the position is stale.
    assert!(s.get_worktree(a).unwrap().resource_version > 1);

    // Asking for the order it is already in changes nothing.
    s.reorder_worktrees(&[c, a, b]).unwrap();
    assert_eq!(ordered_names(&s, repo), vec!["c", "a", "b"]);

    std::fs::remove_dir_all(&dir).ok();
}

/// A client sends the cards it can SEE. Everything else must stay exactly
/// where it was — this is what stops a reorder in one project's group from
/// throwing another project's cards, or every hidden worktree, to the end.
#[test]
fn reordering_a_subset_leaves_every_other_card_where_it_was() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos", 1_000).unwrap();
    let one = s.create_repository(host, root.id, "one", "/one/.git", "").unwrap();
    let two = s.create_repository(host, root.id, "two", "/two/.git", "").unwrap();

    // The group that will be dragged sits at ranks 2 and 3, NOT at 0 and 1.
    // That is the whole point of the fixture: a reorder that renumbered the
    // cards it was handed from zero would land them on top of `x` and `y`
    // and pass every assertion about relative order inside one group.
    let x = s.create_worktree(two.id, "b", "/wt/x", false).unwrap();
    let y = s.create_worktree(two.id, "b", "/wt/y", false).unwrap();
    let a = s.create_worktree(one.id, "b", "/wt/a", false).unwrap();
    let c = s.create_worktree(one.id, "b", "/wt/c", false).unwrap();
    assert_eq!((x.ordinal, y.ordinal, a.ordinal, c.ordinal), (0, 1, 2, 3));

    s.reorder_worktrees(&[c.id, a.id]).unwrap();

    assert_eq!(ordered_names(&s, one.id), vec!["c", "a"], "the group that moved");
    assert_eq!(ordered_names(&s, two.id), vec!["x", "y"], "the group that did not");
    assert_eq!(
        (s.get_worktree(c.id).unwrap().ordinal, s.get_worktree(a.id).unwrap().ordinal),
        (2, 3),
        "the two cards swapped the slots they held; they did not move to the front"
    );
    assert_eq!(
        (s.get_worktree(x.id).unwrap().ordinal, s.get_worktree(y.id).unwrap().ordinal),
        (0, 1),
        "an untouched card keeps its exact rank, not merely its relative one"
    );
    assert_eq!(
        s.list_worktrees_in_order().unwrap().iter().map(|w| w.name()).collect::<Vec<_>>(),
        vec!["x", "y", "c", "a"],
        "and the whole runner's list is what a client would draw"
    );
}

/// Nonsense is refused whole rather than applied halfway, because half a
/// reorder is a layout nobody chose.
#[test]
fn a_reorder_naming_the_same_card_twice_or_a_stranger_is_refused() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "").unwrap();
    let a = s.create_worktree(repo.id, "b", "/wt/a", false).unwrap();
    let b = s.create_worktree(repo.id, "b", "/wt/b", false).unwrap();

    assert!(matches!(
        s.reorder_worktrees(&[a.id, a.id]),
        Err(DomainError::InvalidArgument { .. })
    ));
    assert!(matches!(
        s.reorder_worktrees(&[b.id, a.id, Uuid::now_v7()]),
        Err(DomainError::NotFound)
    ));
    assert_eq!(ordered_names(&s, repo.id), vec!["a", "b"], "nothing moved");
}

#[test]
fn worktree_flags_round_trip() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/flags", false).unwrap();
    assert!(!ws.hidden && !ws.worktree_missing);

    let hidden = s.set_worktree_flags(ws.id, ws.resource_version, true, false).unwrap();
    assert!(hidden.hidden, "hidden is set");
    assert_eq!(hidden.name(), "flags", "the name is the worktree, and hiding moves nothing");

    let missing = s.set_worktree_flags(hidden.id, hidden.resource_version, true, true).unwrap();
    assert!(missing.hidden && missing.worktree_missing);
}

#[test]
fn terminal_round_trip_and_update() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/terminal", false).unwrap();
    let term =
        s.create_terminal(ws.id, "shell", "claude", TerminalIntent::Running, 80, 24).unwrap();
    assert!(!term.runtime_confirmed);
    assert_eq!(s.get_terminal(term.id).unwrap(), term);

    let update = TerminalUpdate {
        title: "shell".into(),
        command_preset: "claude".into(),
        intent: TerminalIntent::Running,
        runtime_confirmed: true,
        exit_code: None,
        exit_signal: None,
        lease_generation: 1,
        epoch: 1,
        columns: 100,
        rows: 30,
    };
    let updated = s.update_terminal(term.id, 1, update).unwrap();
    assert!(updated.runtime_confirmed);
    assert_eq!(updated.columns, 100);
    assert_eq!(updated.resource_version, 2);

    let records = s.load_terminal_records(ws.id).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, term.id);
    assert_eq!(records[0].intent, TerminalIntent::Running);
    assert!(records[0].runtime_confirmed);
}

// ---- pane mode ----

#[test]
fn a_new_terminal_starts_in_terminal_pane_mode() {
    // Terminal-first is the product's default, and defaults belong in the
    // schema rather than in whichever caller remembered.
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/pane-mode", false).unwrap();
    let t = s.create_terminal(ws.id, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
    assert_eq!(t.pane_mode, PaneMode::Terminal);
    assert_eq!(t.agent_session_id, None);
}

#[test]
fn a_session_id_survives_a_reopen_because_it_is_intent_not_runtime() {
    // The one thing about an agent session that must outlive tmux. The
    // conversation itself is never stored.
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/session-id", false).unwrap();
    let t = s.create_terminal(ws.id, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
    let updated =
        s.set_pane_mode(t.id, t.resource_version, PaneMode::Agent, Some("abc-123".into()), false)
            .unwrap();
    assert_eq!(updated.pane_mode, PaneMode::Agent);

    let reopened = s.get_terminal(t.id).unwrap();
    assert_eq!(reopened.agent_session_id.as_deref(), Some("abc-123"));
}

/// The epoch is the caller's to ask for, and both answers have to work.
///
/// A pane-mode TOGGLE replaces the program in the pane, so every offset a
/// client holds against that terminal is stale and the epoch is how it is
/// told. Every other caller of this writes a session id onto a pane that
/// nothing has replaced, and a bump there would cost every attached client
/// a full re-read for no change. Read back off the row rather than off the
/// returned struct: the value that matters is the one SQLite holds.
#[test]
fn only_a_caller_that_asks_for_it_moves_the_epoch() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/epochs", false).unwrap();
    let t = s.create_terminal(ws.id, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
    let started = s.get_terminal(t.id).unwrap().epoch;

    let quiet = s
        .set_pane_mode(t.id, t.resource_version, PaneMode::Terminal, Some("s-1".into()), false)
        .unwrap();
    assert_eq!(
        s.get_terminal(t.id).unwrap().epoch,
        started,
        "writing down a conversation's name replaces nothing in the pane"
    );

    s.set_pane_mode(t.id, quiet.resource_version, PaneMode::Agent, None, true).unwrap();
    assert_eq!(
        s.get_terminal(t.id).unwrap().epoch,
        started + 1,
        "a toggle respawns the pane, and every offset held against it is stale"
    );
}

/// The join a live agent session arrives on.
///
/// A hook knows its own `session_id` and nothing else about Far Cooler, so
/// this is the only question it can be answered by. Two rows may hold the
/// same id — nothing constrains the column — so every claimant comes back
/// and the caller decides; picking one here would be this layer guessing
/// which pane somebody's conversation belongs to.
#[test]
fn a_session_id_finds_every_terminal_that_claims_it_and_no_others() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/sessions", false).unwrap();

    let mine = s.create_terminal(ws.id, "a", "claude", TerminalIntent::Running, 80, 24).unwrap();
    let mine =
        s.set_pane_mode(mine.id, mine.resource_version, PaneMode::Terminal, Some("s-1".into()), false)
            .unwrap();
    let other = s.create_terminal(ws.id, "b", "claude", TerminalIntent::Running, 80, 24).unwrap();
    s.set_pane_mode(other.id, other.resource_version, PaneMode::Terminal, Some("s-2".into()), false)
        .unwrap();
    // A terminal that has declared nothing must never be swept up by a
    // lookup for a session, which is what `agent_session_id = NULL`
    // matching a bound parameter would do under the wrong comparison.
    s.create_terminal(ws.id, "c", "shell", TerminalIntent::Running, 80, 24).unwrap();

    let found = s.terminals_with_agent_session("s-1").unwrap();
    assert_eq!(found.len(), 1, "one claimant, and not the other two rows");
    assert_eq!(found[0].id, mine.id);
    assert_eq!(found[0].agent_session_id.as_deref(), Some("s-1"), "the whole row comes back");

    assert!(
        s.terminals_with_agent_session("s-3").unwrap().is_empty(),
        "a session nobody declared has no claimant"
    );
}

/// Two rows on one id is the case the caller has to be able to see.
#[test]
fn two_terminals_claiming_one_session_both_come_back() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/ambiguous", false).unwrap();

    for title in ["a", "b"] {
        let t =
            s.create_terminal(ws.id, title, "claude", TerminalIntent::Running, 80, 24).unwrap();
        s.set_pane_mode(t.id, t.resource_version, PaneMode::Terminal, Some("shared".into()), false)
            .unwrap();
    }

    assert_eq!(
        s.terminals_with_agent_session("shared").unwrap().len(),
        2,
        "the ambiguity reaches the caller rather than being resolved here"
    );
}

// ---- optimistic concurrency ----

#[test]
fn stale_version_is_a_conflict_not_silently_applied() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();

    // First writer succeeds and moves the version to 2.
    s.update_repository(repo.id, 1, "first", "/gitdir", "origin").unwrap();

    // Second writer still thinks the version is 1: must be rejected, and
    // must not have applied its write.
    let err = s.update_repository(repo.id, 1, "second", "/gitdir", "origin").unwrap_err();
    assert!(matches!(err, DomainError::ResourceConflict));
    assert_eq!(s.get_repository(repo.id).unwrap().display_name, "first");
}

#[test]
fn updating_a_missing_resource_is_not_found_not_conflict() {
    let s = store();
    let err = s.update_repository(Uuid::now_v7(), 1, "x", "y", "z").unwrap_err();
    assert!(matches!(err, DomainError::NotFound));
}

#[test]
fn delete_also_honors_expected_version() {
    let s = store();
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();

    let err = s.delete_repository_root(root.id, 2).unwrap_err();
    assert!(matches!(err, DomainError::ResourceConflict));

    s.delete_repository_root(root.id, 1).unwrap();
    assert!(matches!(s.get_repository_root(root.id).unwrap_err(), DomainError::NotFound));
}

// ---- idempotency ----

#[test]
fn fresh_idempotency_key_is_recorded_and_not_a_replay() {
    let s = store();
    let client = Uuid::now_v7();
    let is_replay = s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap();
    assert!(!is_replay);
    assert_eq!(s.get_idempotency("op-1", client).unwrap().unwrap().request_hash, "hash-a");
}

#[test]
fn same_hash_replay_is_reported_and_not_double_recorded() {
    let s = store();
    let client = Uuid::now_v7();
    assert!(!s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap());
    assert!(s.check_idempotency("op-1", client, "hash-a", 2_000).unwrap(), "same hash replays");
    // Still exactly one record, with the original timestamp.
    let rec = s.get_idempotency("op-1", client).unwrap().unwrap();
    assert_eq!(rec.created_at, 1_000);
}

#[test]
fn different_hash_same_key_is_rejected() {
    let s = store();
    let client = Uuid::now_v7();
    assert!(!s.check_idempotency("op-1", client, "hash-a", 1_000).unwrap());
    let err = s.check_idempotency("op-1", client, "hash-b", 2_000).unwrap_err();
    assert!(matches!(err, DomainError::IdempotencyMismatch));
}

#[test]
fn different_clients_do_not_collide_on_the_same_key() {
    let s = store();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    assert!(!s.check_idempotency("shared-key", a, "hash-a", 1_000).unwrap());
    assert!(!s.check_idempotency("shared-key", b, "hash-b", 1_000).unwrap());
}

#[test]
fn pruning_removes_only_entries_past_retention() {
    let s = store();
    let client = Uuid::now_v7();
    s.check_idempotency("old", client, "h", 0).unwrap();
    s.check_idempotency("new", client, "h", 1_000).unwrap();

    let now = IDEMPOTENCY_RETENTION_MILLIS + 500;
    let pruned = s.prune_idempotency(now).unwrap();
    assert_eq!(pruned, 1, "only the entry past 24h should be pruned");
    assert!(s.get_idempotency("old", client).unwrap().is_none());
    assert!(s.get_idempotency("new", client).unwrap().is_some());
}

// ---- file-backed open / migration idempotency ----

#[test]
fn file_backed_store_persists_and_reopen_is_idempotent() {
    let dir = farcooler_core::scratch::dir("store-open");
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("db.sqlite3");

    let host = Uuid::now_v7();
    let root_id = {
        let s = Store::open(&db_path).unwrap();
        let root = s.create_repository_root(host, "/repos/one", 1_000).unwrap();
        root.id
    }; // dropped: connection closes, file remains on disk

    // Reopening an up-to-date database must not error and must not
    // re-run migrations against tables that already exist.
    let s = Store::open(&db_path).unwrap();
    let root = s.get_repository_root(root_id).unwrap();
    assert_eq!(root.path, "/repos/one");

    // Already current, so no backup should have been written.
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).filter(|n| !n.to_string_lossy().ends_with(".owners")).collect();
    assert_eq!(entries.len(), 1, "no backup file expected when reopening a current schema");

    std::fs::remove_dir_all(&dir).ok();
}
