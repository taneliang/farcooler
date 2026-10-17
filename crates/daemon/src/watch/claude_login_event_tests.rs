//! A terminal event names whose Claude login the pane uses, by point lookups
//! (ov-598). Its own file to keep `watch.rs` inside its size budget.

use super::*;
use crate::claude_logins::tests::{create, factory, runner};
use farcooler_protocol::v1::event::Payload;

/// `mark_seen` on a finished turn announces the terminal. The event names
/// Bob's login and the whole login tables are not read to say so: an event
/// fires on every activity change, so a read per event is a read per tick.
/// Goes red when the event path reads the tables for one pane.
///
/// A current-thread runtime on purpose: the count is per thread, so `mark_seen`
/// must read on this one. The control at the end reads a table on this thread
/// and expects the count to move, so a runtime that hid reads would fail it.
#[tokio::test(flavor = "current_thread")]
async fn a_terminal_event_names_the_login_without_reading_the_tables() {
    use farcooler_store::read_counts::whole_table_reads;

    let (_dir, svc, _main, checkout) = runner().await;
    let bobs = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's pane");
    let watcher = Watcher::new(svc.clone());
    let mut rx = watcher.subscribe();
    watcher
        .observe_for_tests(bobs, crate::needs_you::Observation {
            activity: AgentActivity::Done,
            state_since: now_millis(),
            command: "claude".into(),
            ..Default::default()
        })
        .await;
    let before = whole_table_reads();
    watcher.mark_seen(bobs).await;
    let spent = whole_table_reads() - before;

    let mut named = None;
    while let Ok(event) = rx.try_recv() {
        if let Some(Payload::TerminalChanged(t)) = event.payload {
            if t.id.as_ref() == bobs.as_bytes() {
                named = t.claude_login;
            }
        }
    }
    assert_eq!(named.map(|l| l.handle), Some("bob".into()), "the event names Bob's login");
    assert_eq!(spent, 0, "the event path read a whole table");
    let before = whole_table_reads();
    drop(svc.claude_logins_seen());
    assert!(whole_table_reads() > before, "this thread's reads are counted");
}
