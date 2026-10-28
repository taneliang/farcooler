//! An agent's conversation in the browser's client (ov-776), against the
//! in-process runner: the rows a page reads are the projector's own, a
//! follow brings only what changed, a message goes out as the person's
//! `terminal.compose`, and Stop is `terminal.interrupt`.
//!
//! Each test goes red without the line it names: take `compose` off the
//! `terminal.compose` call and the runner never sees the text; send
//! `terminal.write` instead of `terminal.interrupt` and the count stays zero.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use farcooler_ceremony::{Reach, RunnerEntry};
use farcooler_web::enroll;
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use support::{BROWSER_KEY, EPOCH, Ed25519InMemory, HOST_KEY, NODE_KEY, Seen, TERMINAL, fingerprint, key, line};

const TURN: &str = r#"{"ord":0,"rev":1,"id":"turn:a","turn":null,"provisional":false,"kind":{"Turn":{"prompt":"Fix the build","origin":"Typed","outcome":null,"background_running":0,"activity":"Busy"}}}"#;
const PROSE: &str = r#"{"ord":1,"rev":2,"id":"prose:1","turn":"turn:a","provisional":false,"kind":{"Prose":{"text":"Looking at the log","conclusion":false,"at_ms":null}}}"#;
const PROSE_MORE: &str = r#"{"ord":1,"rev":3,"id":"prose:1","turn":"turn:a","provisional":false,"kind":{"Prose":{"text":"Looking at the log now","conclusion":false,"at_ms":null}}}"#;

async fn connect() -> (Runner, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
    let offer = enroll::offer("Chrome on Linux", "acct-owner", &line(BROWSER_KEY), NODE_KEY);
    let entry = RunnerEntry {
        id: "01990000-0000-7000-8000-00000000a001".into(),
        label: "studio".into(),
        alias: "studio".into(),
        user: "owner".into(),
        host_key: fingerprint(HOST_KEY),
        reach: Reach::Tailcat { token: "tc-token".into(), client_key: String::new() },
        alternative: None,
        pending: false,
        pending_why: None,
    };
    let reply = farcooler_ceremony::encode_manifest(&farcooler_ceremony::manifest(&offer, vec![entry]));
    let manifest = enroll::accept(&reply, &offer, false, 0.0, 0.0).unwrap();
    let granted = enroll::granted(&manifest).remove(0);
    let mut signer = Ed25519InMemory(key(BROWSER_KEY));
    let conn = ssh::connect(browser_side, &granted.host_key, &granted.user, &line(BROWSER_KEY), &mut signer).await.unwrap();
    (Runner::start(conn).await.expect("connects"), seen)
}

/// The page learns which terminals are agents from the list, and reads their
/// rows as the apps' JSON: the epoch and revision to follow from, and each row
/// as the projector serialized it.
#[tokio::test]
async fn an_agents_rows_come_as_the_apps_json_and_the_list_says_it_is_an_agent() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connect().await;
            {
                let mut chat = seen.chat.lock().await;
                chat.put("turn:a", TURN);
                chat.put("prose:1", PROSE);
            }
            assert!(runner.terminals().await.unwrap()[0].agent, "a role of agent is an agent");

            let page = runner.agent_rows(TERMINAL, None, 100).await.expect("agent.rows");
            assert_eq!(page["epoch"], EPOCH);
            assert_eq!(page["rev"], 2);
            assert_eq!(page["moreBefore"], false);
            let rows = page["rows"].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0]["id"], "turn:a");
            assert_eq!(rows[0]["kind"]["Turn"]["prompt"], "Fix the build");
            assert_eq!(rows[1]["kind"]["Prose"]["text"], "Looking at the log");
        })
        .await;
}

/// A follow answers what changed after the revision it is given, as updates
/// and inserts by id, and a stranger's epoch is told to page again.
#[tokio::test]
async fn a_follow_brings_only_what_changed_and_a_strange_epoch_resets() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connect().await;
            {
                let mut chat = seen.chat.lock().await;
                chat.put("turn:a", TURN);
                chat.put("prose:1", PROSE);
            }
            let page = runner.agent_rows(TERMINAL, None, 100).await.unwrap();
            let (epoch, rev) = (page["epoch"].as_u64().unwrap(), page["rev"].as_u64().unwrap());

            seen.chat.lock().await.put("prose:1", PROSE_MORE);
            let follow = runner.agent_rows_follow(TERMINAL, epoch, rev, 0).await.expect("agent.rows_follow");
            assert_eq!(follow["reset"], false);
            assert_eq!(follow["rev"], 3);
            let changes = follow["changes"].as_array().unwrap();
            assert_eq!(changes.len(), 1, "only the row that changed");
            assert_eq!(changes[0]["kind"], "update");
            assert_eq!(changes[0]["id"], "prose:1");
            assert_eq!(changes[0]["row"]["kind"]["Prose"]["text"], "Looking at the log now");

            let stranger = runner.agent_rows_follow(TERMINAL, epoch + 1, rev, 0).await.unwrap();
            assert_eq!(stranger["reset"], true);
        })
        .await;
}

/// A message goes out as `terminal.compose`, the Mac composer's own call, and
/// says whether it waits in the agent's queue. Stop is `terminal.interrupt`.
#[tokio::test]
async fn a_message_is_composed_and_stop_is_an_interrupt() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connect().await;
            assert!(!runner.compose(TERMINAL, "Run the tests too").await.expect("terminal.compose"));
            seen.busy.store(true, Ordering::SeqCst);
            assert!(runner.compose(TERMINAL, "and lint").await.unwrap(), "a busy agent queues it");
            assert_eq!(*seen.composed.lock().await, ["Run the tests too", "and lint"]);
            assert!(seen.typed.lock().await.is_empty(), "a message is not keystrokes");

            runner.interrupt(TERMINAL).await.expect("terminal.interrupt");
            assert_eq!(seen.interrupts.load(Ordering::SeqCst), 1);
        })
        .await;
}
