//! What the page needs to organize by workspace, send images and open a task's
//! detail (ov-871), against the in-process runner.
//!
//! - Break `TerminalSummary::workspace` (drop the field) and the list names no
//!   workspace: the terminals test goes red.
//! - Take the images out of `Runner::compose_images` (call `compose` with
//!   none) and the runner sees a message with no image: the images test goes
//!   red. Raise `compose_limits`' inline cap and the limits test goes red.
//! - Read another task, or write the note as another kind, and the task tests
//!   go red.

mod support;

use std::sync::Arc;

use farcooler_protocol::v1 as pb;
use farcooler_web::ssh;
use farcooler_web::wire::Runner;
use support::{BROWSER_KEY, Ed25519InMemory, HOST_KEY, Seen, TERMINAL, WORKSPACE, key, line};
use uuid::Uuid;

const TASK: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0d01);

async fn connected() -> (Runner, Arc<Seen>) {
    let (browser_side, runner_side) = tokio::io::duplex(1 << 16);
    let seen = Arc::new(Seen::default());
    support::serve(runner_side, HOST_KEY, Some(key(BROWSER_KEY).public_key().clone()), seen.clone());
    let mut signer = Ed25519InMemory(key(BROWSER_KEY));
    let conn = ssh::connect(browser_side, &support::fingerprint(HOST_KEY), "owner", &line(BROWSER_KEY), &mut signer).await.expect("connects");
    (Runner::start(conn).await.expect("starts"), seen)
}

#[tokio::test]
async fn a_terminal_names_the_workspace_it_works_for() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, _) = connected().await;
            let terminals = runner.terminals().await.unwrap();
            assert_eq!(terminals[0].workspace.as_deref(), Some(WORKSPACE.to_string().as_str()));
            let json = serde_json::to_value(&terminals[0]).unwrap();
            assert_eq!(json["workspace"], WORKSPACE.to_string());
        })
        .await;
}

#[tokio::test]
async fn images_go_with_the_message_and_the_limits_are_the_runners() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected().await;
            let limits = runner.compose_limits();
            assert!(limits.images, "the runner advertises compose");
            // No compose_upload: the images ride inside the request.
            assert_eq!((limits.each, limits.total), (900 * 1024, 900 * 1024));

            let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
            runner.compose_images(TERMINAL, "what is this?", &[("image/png".into(), png.clone()), ("image/jpeg".into(), vec![0xff, 0xd8])]).await.expect("terminal.compose");
            assert_eq!(*seen.composed.lock().await, ["what is this?"]);
            assert_eq!(*seen.composed_images.lock().await, [("image/png".to_string(), png.len()), ("image/jpeg".to_string(), 2)]);

            // Past the cap nothing is sent, and the refusal carries the word the page words.
            let big = vec![0u8; 901 * 1024];
            let refused = runner.compose_images(TERMINAL, "too big", &[("image/png".into(), big)]).await.unwrap_err();
            assert_eq!(refused.refusal().map(|(_, what)| what), Some("images_too_large"));
            assert_eq!(seen.composed.lock().await.len(), 1, "nothing went");
        })
        .await;
}

#[tokio::test]
async fn a_task_is_read_whole_and_a_note_is_a_comment() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (runner, seen) = connected().await;
            assert!(runner.task(TASK).await.is_err(), "a task the runner doesn't have");
            *seen.task.lock().await = Some(pb::TaskDetail {
                task: Some(pb::Task {
                    id: bytes::Bytes::copy_from_slice(TASK.as_bytes()),
                    key: "ov-9".into(),
                    title: "Pick a workspace".into(),
                    intent: "Scope the page".into(),
                    status: pb::TaskStatus::InProgress as i32,
                    acceptance: vec![pb::TaskAcceptanceItem { id: bytes::Bytes::from_static(&[3; 16]), text: "One at a time".into(), met: true }],
                    ..Default::default()
                }),
                notes: vec![pb::TaskNote { body: "Started".into(), actor: "agent".into(), kind: pb::TaskNoteKind::Progress as i32, at: 5, ..Default::default() }],
                ..Default::default()
            });
            let detail = runner.task(TASK).await.expect("task.get");
            assert_eq!(detail["task"]["key"], "ov-9");
            assert_eq!(detail["task"]["acceptance"][0]["met"], true);
            assert_eq!(detail["notes"][0]["body"], "Started");
            assert_eq!(detail["notes"][0]["kind"], "progress");

            runner.comment_on_task(TASK, "Looks right").await.expect("task.note");
            let notes = seen.notes.lock().await;
            assert_eq!(notes.len(), 1);
            assert_eq!((notes[0].0.as_slice(), notes[0].1, notes[0].2.as_str()), (TASK.as_bytes().as_slice(), pb::TaskNoteKind::Comment as i32, "Looks right"));
        })
        .await;
}
