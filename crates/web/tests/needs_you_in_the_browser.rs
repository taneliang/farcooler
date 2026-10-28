//! What the browser's Needs You reads and answers, against a fake runner that
//! plays the lists `test/fixtures/needs-you.json` holds (ov-773).
//!
//! That fixture is what the Mac and both phones decode, and what
//! `needs_you_json` is tested to write (`crates/client/src/needs_you_json.rs`).
//! Here the fake runner is given each of its two runners' lists as the daemon
//! sends them, on the wire, and the browser's `Runner::needs_you` must hand
//! back the fixture's JSON, byte for byte as a value: the same list the Mac
//! shows for the same runner.
//!
//! - Acceptance 1: break `Runner::needs_you` (drop an item, reorder, rewrite a
//!   key) and `the_browsers_list_is_the_one_the_mac_decodes` goes red.
//! - Acceptance 2: break `Runner::answer_ask` (a different request id or
//!   option, or an actor that is not a person) and the answer test goes red.
//! - Acceptance 3: a Read-scoped device sees its list, and its answer comes
//!   back refused with the runner's own code, which the page words.

mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use farcooler_protocol::v1 as pb;
use farcooler_web::error::WebError;
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use serde_json::Value;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, Seen, key, line};
use uuid::Uuid;

const FIXTURE: &str = include_str!("../../../test/fixtures/needs-you.json");

fn fixture_runner(name: &str) -> Value {
    let all: Value = serde_json::from_str(FIXTURE).expect("the fixture is JSON");
    all["runners"].as_array().unwrap().iter().find(|r| r["runner"] == name).expect("a runner of that name").clone()
}

// ---- the fixture, back into the daemon's messages ----
//
// The reverse of `needs_you_json`, written out here rather than shared with it,
// so a key that function renames is not renamed in both places at once.

fn bytes_of(v: &Value) -> Bytes {
    v.as_str().map(|s| Bytes::copy_from_slice(Uuid::parse_str(s).unwrap().as_bytes())).unwrap_or_default()
}

fn kind(word: &str) -> i32 {
    (match word {
        "ask" => pb::NeedsYouKind::Ask,
        "blocked" => pb::NeedsYouKind::Blocked,
        "decision" => pb::NeedsYouKind::Decision,
        "review" => pb::NeedsYouKind::Review,
        "pr_review" => pb::NeedsYouKind::PrReview,
        "pr_thread" => pb::NeedsYouKind::PrThread,
        "claude_login" => pb::NeedsYouKind::ClaudeLogin,
        "fyi" => pb::NeedsYouKind::Fyi,
        other => panic!("the fixture has a kind this test doesn't know: {other}"),
    }) as i32
}

fn task_ref(v: &Value) -> pb::TaskRef {
    pb::TaskRef {
        id: bytes_of(&v["id"]),
        key: v["key"].as_str().unwrap().into(),
        title: v["title"].as_str().unwrap().into(),
        status: (match v["status"].as_str().unwrap() {
            "needs_decision" => pb::TaskStatus::NeedsDecision,
            "in_review" => pb::TaskStatus::InReview,
            "in_progress" => pb::TaskStatus::InProgress,
            other => panic!("the fixture has a status this test doesn't know: {other}"),
        }) as i32,
    }
}

fn item(v: &Value) -> pb::NeedsYouItem {
    let text = |k: &str| v[k].as_str().map(str::to_string);
    let mut out = pb::NeedsYouItem {
        id: text("id").unwrap(),
        kind: kind(v["kind"].as_str().unwrap()),
        also: v["also"].as_array().unwrap().iter().map(|k| kind(k.as_str().unwrap())).collect(),
        rank: v["rank"].as_u64().unwrap() as u32,
        workspace_id: bytes_of(&v["workspace_id"]),
        workspace_name: text("workspace_name").unwrap(),
        repository_id: bytes_of(&v["repository_id"]),
        task: v["task"].as_object().map(|_| task_ref(&v["task"])),
        terminal: v["terminal"].as_object().map(|t| pb::TerminalRef {
            id: bytes_of(&t["id"]),
            worktree_id: bytes_of(&t["worktree_id"]),
            label: t["label"].as_str().unwrap().into(),
            role: match t["role"].as_str() {
                Some("agent") => pb::TerminalRole::Agent as i32,
                Some("orchestrator") => pb::TerminalRole::Orchestrator as i32,
                Some("shell") => pb::TerminalRole::Shell as i32,
                _ => 0,
            },
            pane_mode: pb::PaneMode::Terminal as i32,
        }),
        worktree: v["worktree"].as_object().map(|w| pb::WorktreeRef {
            id: bytes_of(&w["id"]),
            name: w["name"].as_str().unwrap().into(),
            branch: w["branch"].as_str().unwrap().into(),
            insertions: w["insertions"].as_u64().unwrap() as u32,
            deletions: w["deletions"].as_u64().unwrap() as u32,
        }),
        question: text("question").unwrap(),
        detail: text("detail"),
        ask_id: text("ask_id"),
        link: text("link"),
        actions: v["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| pb::NeedsYouAction {
                id: a["id"].as_str().unwrap().into(),
                title: a["title"].as_str().unwrap().into(),
                destructive: a["destructive"].as_bool().unwrap(),
                primary: a["primary"].as_bool().unwrap(),
            })
            .collect(),
        concern: v["concern"].as_object().map(|c| pb::NeedsYouConcern {
            handle: c["handle"].as_str().unwrap().into(),
            name: c["name"].as_str().unwrap().into(),
            yours: c["yours"].as_bool().unwrap(),
        }),
        ..Default::default()
    };
    if let Some(ms) = v["since"].as_i64() {
        let mut at = out.since.take().unwrap_or_default();
        at.seconds = ms / 1000;
        at.nanos = ((ms % 1000) * 1_000_000) as i32;
        out.since = Some(at);
    }
    out
}

fn list_of(runner: &str) -> pb::NeedsYouList {
    pb::NeedsYouList { items: fixture_runner(runner)["needs_you"]["items"].as_array().unwrap().iter().map(item).collect() }
}

// ---- a runner to read ----

/// A connected browser and the fake runner it is connected to, which holds
/// `runner`'s list from the fixture.
async fn connected(runner: &str) -> (Runner, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    *seen.needs_you.lock().await = Some(list_of(runner));
    support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
    let mut signer = Ed25519InMemory(key(BROWSER_KEY));
    let conn = ssh::connect(browser_side, &support::fingerprint(HOST_KEY), "owner", &line(BROWSER_KEY), &mut signer)
        .await
        .expect("connects");
    (Runner::start(conn).await.expect("starts"), seen)
}

/// ov-773 acceptance 1: for the same runner, the browser's list is the Mac's.
#[tokio::test]
async fn the_browsers_list_is_the_one_the_mac_decodes() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for name in ["studio", "build-box"] {
                let (runner, _) = connected(name).await;
                let shown = runner.needs_you().await.expect("needs_you.list");
                assert_eq!(shown, fixture_runner(name)["needs_you"], "{name}'s list is not what the Mac decodes");
            }
        })
        .await;
}

/// ov-773 acceptance 2: an Allow in the browser reaches the runner at once, as
/// a person's answer to that ask, and the runner's next list no longer holds
/// it. The Mac's list is the runner's, so what the runner drops the Mac drops;
/// the runner's own tests hold that half (`rpc_agent_answer_tests.rs`).
#[tokio::test]
async fn an_allow_reaches_the_runner_at_once_and_clears_the_item() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected("studio").await;
            let before = runner.needs_you().await.unwrap();
            let ask = before["items"][0].clone();
            assert_eq!(ask["kind"], "ask");
            let terminal = Uuid::parse_str(ask["terminal"]["id"].as_str().unwrap()).unwrap();

            tokio::time::timeout(
                Duration::from_secs(2),
                runner.answer_ask(terminal, ask["ask_id"].as_str().unwrap(), "allow"),
            )
            .await
            .expect("the runner takes the answer within two seconds")
            .expect("the answer is taken");

            let answers = seen.answers.lock().await.clone();
            assert_eq!(
                answers,
                vec![(
                    terminal.as_bytes().to_vec(),
                    ask["ask_id"].as_str().unwrap().to_string(),
                    "allow".to_string(),
                    // The browser is a person's, as a phone is (ov-493).
                    "user".to_string(),
                )]
            );
            let after = runner.needs_you().await.unwrap();
            let ids = |v: &Value| v["items"].as_array().unwrap().iter().map(|i| i["id"].clone()).collect::<Vec<_>>();
            assert_eq!(ids(&after).len(), ids(&before).len() - 1);
            assert!(!ids(&after).contains(&ask["id"]), "the answered ask is still listed");
        })
        .await;
}

/// A decision is answered with its option's text, as a note on its task.
#[tokio::test]
async fn a_decision_is_answered_with_the_options_text_on_its_task() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected("studio").await;
            let list = runner.needs_you().await.unwrap();
            let decision = list["items"].as_array().unwrap().iter().find(|i| i["kind"] == "decision").unwrap().clone();
            let task = Uuid::parse_str(decision["task"]["id"].as_str().unwrap()).unwrap();
            runner.answer_decision(task, "SQLite").await.expect("the answer is taken");
            assert_eq!(
                *seen.notes.lock().await,
                vec![(task.as_bytes().to_vec(), pb::TaskNoteKind::Answer as i32, "SQLite".to_string())]
            );
            let after = runner.needs_you().await.unwrap();
            assert!(after["items"].as_array().unwrap().iter().all(|i| i["id"] != decision["id"]));
        })
        .await;
}

/// ov-773 acceptance 3: a device enrolled to look reads its runner's list, and
/// its answer comes back as the runner's refusal, with the code the page words
/// it from: an error, not a crash and not a silent success.
#[tokio::test]
async fn a_read_scoped_device_sees_the_list_and_is_refused_by_the_runner() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected("build-box").await;
            *seen.refuse_answers.lock().await = Some((pb::ErrorCode::ScopeDenied, ""));

            let shown = runner.needs_you().await.expect("a read-scoped device still reads");
            assert_eq!(shown["items"].as_array().unwrap().len(), 2);
            let ask = &shown["items"][0];
            // Below Control the runner leaves out what an answer needs.
            assert_eq!(ask["ask_id"], Value::Null);
            assert_eq!(ask["actions"], serde_json::json!([]));

            let refused = runner
                .answer_ask(Uuid::from_u128(1), "hook-ask-x", "allow")
                .await
                .expect_err("the runner refuses an answer below Control");
            assert_eq!(refused.word(), "refused");
            assert_eq!(refused.refusal(), Some(("scope-denied", "")));
            assert!(matches!(refused, WebError::Refused { .. }));

            let refused = runner.answer_decision(Uuid::from_u128(2), "Yes").await.expect_err("and a decision's");
            assert_eq!(refused.refusal(), Some(("scope-denied", "")));
        })
        .await;
}

/// Two refusals with one code stay two: an ask someone else answered first and
/// one that never reached the agent (spec §2.5), told apart by `what`.
#[tokio::test]
async fn a_conflict_keeps_which_one_it_was() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected("studio").await;
            for what in ["not_held", "not_delivered"] {
                *seen.refuse_answers.lock().await = Some((pb::ErrorCode::ResourceConflict, what));
                let refused = runner.answer_ask(Uuid::from_u128(1), "hook-ask-x", "allow").await.unwrap_err();
                assert_eq!(refused.refusal(), Some(("resource-conflict", what)));
            }
        })
        .await;
}
