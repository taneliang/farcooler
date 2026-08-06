//! The review buffer's durable behaviour.
//!
//! The load-bearing case is a daemon that dies mid-dispatch. ACP gives no
//! receipt, so "the agent got this" is never knowable from the send alone; the
//! only honest states are "seen it in the stream" and "cannot say". These tests
//! pin the second one down, because it is the one a hurried implementation turns
//! into a confident lie.

use farcooler_store::Store;
use farcooler_store::review::{DispatchState, Disposition, EntryStatus};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

/// A store with one workspace to hang entries off.
fn store_with_workspace() -> (Store, Uuid) {
    let s = Store::open_in_memory().expect("open");
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/tmp/root", NOW).expect("root");
    let repo = s
        .create_repository(host, root.id, "repo", "/tmp/root/repo/.git", "origin")
        .expect("repo");
    let ws = s
        .create_workspace(repo.id, "task", "feat/x", "/tmp/root/repo-wt", false)
        .expect("workspace");
    (s, ws.id)
}

fn capture(s: &Store, ws: Uuid, body: &str) -> farcooler_store::review::ReviewEntry {
    s.capture_review_entry(ws, body, Disposition::Fix, r#"{"kind":"none"}"#, "{}", NOW)
        .expect("capture")
}

#[test]
fn an_unanchored_entry_is_captured_without_a_file_or_a_line() {
    // The majority case. If this needed a path it would be the wrong data model.
    let (s, ws) = store_with_workspace();
    let e = capture(&s, ws, "the error copy doesn't match the rest of the app");
    assert_eq!(e.status, EntryStatus::Open);
    assert_eq!(e.anchor_json, r#"{"kind":"none"}"#);
    assert_eq!(e.resource_version, 1);
}

#[test]
fn the_outbox_row_and_the_entry_status_move_together_or_not_at_all() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let b = capture(&s, ws, "two");
    let terminal = Uuid::now_v7();

    let d = s
        .open_dispatch(
            ws,
            terminal,
            Disposition::Fix,
            &[(a.id, a.resource_version), (b.id, b.resource_version)],
            "Review of feat/x — 2 comments.",
            NOW,
        )
        .expect("dispatch opens");

    assert_eq!(d.state, DispatchState::Pending);
    for id in [a.id, b.id] {
        let e = s.review_entry(id).expect("read").expect("present");
        assert_eq!(e.status, EntryStatus::Dispatched);
        assert_eq!(e.dispatch_id, Some(d.id), "the entry knows which send it belongs to");
    }
}

#[test]
fn a_stale_entry_version_refuses_the_whole_dispatch_and_marks_nothing() {
    // Explicit ids stop a NEWLY CAPTURED entry being swept in. They do nothing
    // about one that was edited after the client drew it — that is what the
    // version is for, and a partial dispatch would be worse than none.
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let b = capture(&s, ws, "two");

    // Someone rewords `b` on another device.
    let b2 = s
        .update_review_entry(b.id, "two, but better", Disposition::Fix, b.resource_version, NOW)
        .expect("update");
    assert_eq!(b2.resource_version, 2);

    let err = s.open_dispatch(
        ws,
        Uuid::now_v7(),
        Disposition::Fix,
        &[(a.id, a.resource_version), (b.id, b.resource_version)],
        "prompt",
        NOW,
    );
    assert!(err.is_err(), "a stale version must refuse the dispatch");

    // And critically: `a` was NOT left marked by the half that succeeded.
    let a_after = s.review_entry(a.id).expect("read").expect("present");
    assert_eq!(
        a_after.status,
        EntryStatus::Open,
        "a refused dispatch must roll back every entry it touched"
    );
}

#[test]
fn an_already_dispatched_entry_is_not_dispatched_again() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let d = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(a.id, a.resource_version)], "p", NOW)
        .expect("first");
    s.mark_dispatch_observed(d.id, NOW).expect("observed");

    let current = s.review_entry(a.id).expect("read").expect("present");
    let again = s.open_dispatch(
        ws,
        Uuid::now_v7(),
        Disposition::Fix,
        &[(a.id, current.resource_version)],
        "p",
        NOW,
    );
    assert!(again.is_err(), "a dispatched entry is not open for dispatch");
}

#[test]
fn observing_the_prompt_in_the_stream_is_what_makes_a_dispatch_delivered() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let d = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(a.id, a.resource_version)], "p", NOW)
        .expect("dispatch");

    assert_eq!(s.dispatch(d.id).unwrap().unwrap().state, DispatchState::Pending);
    s.mark_dispatch_observed(d.id, NOW + 5).expect("observe");

    let after = s.dispatch(d.id).unwrap().unwrap();
    assert_eq!(after.state, DispatchState::Observed);
    assert_eq!(after.observed_at, Some(NOW + 5));
}

#[test]
fn a_restart_finds_every_pending_dispatch_so_none_is_silently_believed() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let b = capture(&s, ws, "two");

    let seen = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(a.id, a.resource_version)], "p", NOW)
        .expect("d1");
    let unseen = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(b.id, b.resource_version)], "p", NOW)
        .expect("d2");
    s.mark_dispatch_observed(seen.id, NOW).expect("observe");

    let pending = s.pending_dispatches().expect("pending");
    let ids: Vec<Uuid> = pending.iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![unseen.id], "only what was never observed comes back for resolution");
}

#[test]
fn an_unresolvable_dispatch_says_unknown_and_never_dispatched() {
    // The whole point. `Dispatched` claims the agent has it; the daemon cannot
    // know that, so the entry must say so and offer Send Again rather than sit
    // there looking handled.
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let d = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(a.id, a.resource_version)], "p", NOW)
        .expect("dispatch");

    s.mark_dispatch_unknown(d.id, NOW + 9).expect("unknown");

    assert_eq!(s.dispatch(d.id).unwrap().unwrap().state, DispatchState::Unknown);
    let e = s.review_entry(a.id).expect("read").expect("present");
    assert_eq!(e.status, EntryStatus::DispatchUnknown);
}

#[test]
fn marking_a_dispatch_unknown_leaves_an_observed_one_alone() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "one");
    let d = s
        .open_dispatch(ws, Uuid::now_v7(), Disposition::Fix, &[(a.id, a.resource_version)], "p", NOW)
        .expect("dispatch");
    s.mark_dispatch_observed(d.id, NOW).expect("observe");
    s.mark_dispatch_unknown(d.id, NOW + 1).expect("no-op");

    assert_eq!(
        s.dispatch(d.id).unwrap().unwrap().state,
        DispatchState::Observed,
        "something already proven delivered does not become uncertain later"
    );
}

#[test]
fn an_answer_lands_beside_the_entry_that_asked() {
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "why is this retrying three times?");
    let terminal = Uuid::now_v7();
    s.record_answer(a.id, terminal, "Because the transport retries idempotent reads.", "exact", NOW)
        .expect("answer");

    let e = s.review_entry(a.id).expect("read").expect("present");
    assert_eq!(e.status, EntryStatus::Answered);
    assert_eq!(e.answer_terminal_id, Some(terminal));
    assert_eq!(e.answer_correlation.as_deref(), Some("exact"));
    assert!(e.answer_text.unwrap().contains("idempotent"));
}

#[test]
fn an_uncorrelated_answer_is_recorded_as_uncorrelated_rather_than_dropped() {
    // A busy agent's turn that could not be split by number still reaches the
    // user — labelled, so they know the daemon is not vouching for the match.
    let (s, ws) = store_with_workspace();
    let a = capture(&s, ws, "q");
    s.record_answer(a.id, Uuid::now_v7(), "some prose that did not number itself", "uncorrelated", NOW)
        .expect("answer");
    let e = s.review_entry(a.id).expect("read").expect("present");
    assert_eq!(e.answer_correlation.as_deref(), Some("uncorrelated"));
}

#[test]
fn a_viewed_mark_clears_itself_when_the_file_changes() {
    // Keyed by content, so nothing has to notice an edit and go delete the row.
    let (s, ws) = store_with_workspace();
    s.mark_viewed(ws, "feat/x", "src/main.rs", "hash-of-what-i-read", NOW).expect("mark");

    assert!(s.is_viewed(ws, "feat/x", "src/main.rs", "hash-of-what-i-read").unwrap());
    assert!(
        !s.is_viewed(ws, "feat/x", "src/main.rs", "hash-after-the-agent-edited-it").unwrap(),
        "an agent changing the file un-reads it"
    );
}

#[test]
fn the_same_path_is_viewed_per_branch_not_per_workspace() {
    let (s, ws) = store_with_workspace();
    s.mark_viewed(ws, "feat/x", "src/main.rs", "h", NOW).expect("mark");
    assert!(s.is_viewed(ws, "feat/x", "src/main.rs", "h").unwrap());
    assert!(
        !s.is_viewed(ws, "feat/x-part-2", "src/main.rs", "h").unwrap(),
        "the same path on a stack link is a different thing to have read"
    );
}

#[test]
fn removing_a_workspace_takes_its_entries_with_it() {
    let (s, ws) = store_with_workspace();
    capture(&s, ws, "one");
    capture(&s, ws, "two");
    assert_eq!(s.list_review_entries(ws).unwrap().len(), 2);

    let ws_row = s.get_workspace(ws).expect("workspace");
    s.delete_workspace(ws, ws_row.resource_version).expect("delete");

    assert!(
        s.list_review_entries(ws).unwrap().is_empty(),
        "a buffer describing a worktree that no longer exists is not kept"
    );
}
