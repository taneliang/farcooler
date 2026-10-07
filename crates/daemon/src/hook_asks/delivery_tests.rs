//! A device's answer names it on the ask's row only once the hook took it
//! (ov-370 review 1 M1): an answer that never reached the hook leaves the row
//! naming nobody, so it can't say "Answered on Mac" over a dialog that is
//! still waiting at the keyboard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use farcooler_agent_hooks::wire::LONGEST_HOLD;
use uuid::Uuid;

use super::shape::notes_for;
use super::{AnswerRefused, AskShape, HookAsks};

fn plan() -> AskShape {
    AskShape::Plan { input: serde_json::json!({ "plan": "# Plan" }) }
}

#[tokio::test]
async fn an_answer_the_hook_took_names_the_device() {
    let asks = HookAsks::new(Arc::new(Mutex::new(None)));
    let pane = Uuid::now_v7();
    let (id, rx) = asks.hold_shaped(pane, Some("ExitPlanMode"), plan(), LONGEST_HOLD);
    let hook = tokio::spawn(async move {
        let settled = rx.await.expect("an ending");
        let _ = settled.ack.expect("a device's answer waits on its ack").send(());
    });
    assert_eq!(asks.answer_with(pane, &id, "allow", &HashMap::new(), "Mac").await, Ok(()));
    hook.await.unwrap();
    assert_eq!(notes_for(pane), [(id, Some("Mac".to_string()))]);
}

#[tokio::test]
async fn an_answer_that_never_reached_the_hook_names_nobody() {
    let asks = HookAsks::new(Arc::new(Mutex::new(None)));
    let pane = Uuid::now_v7();
    let (id, rx) = asks.hold_shaped(pane, Some("ExitPlanMode"), plan(), LONGEST_HOLD);
    // The hook went away: its connection drops the ack unsent.
    drop(rx);
    assert_eq!(asks.answer_with(pane, &id, "allow", &HashMap::new(), "Mac").await, Err(AnswerRefused::NotDelivered));
    assert_eq!(notes_for(pane), [(id, None)], "never \"Answered on Mac\" for an answer claude didn't get");
}
