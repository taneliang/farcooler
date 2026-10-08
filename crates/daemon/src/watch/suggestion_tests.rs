//! claude's suggested next prompt, from the screen the watcher holds to the
//! turn row a follower is woken with (ov-409). Its own file to keep
//! `watch.rs` inside its size budget.

use std::time::Duration;

use farcooler_core::session_log::projector::RowKind;
use uuid::Uuid;

use super::registry_join::offered;
use crate::session_projectors::{Follow, RowChange, SessionProjectors};

fn capture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/../core/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The one real prediction in the corpus, drawn dim (see `composer::suggestion`).
fn predicted() -> String {
    capture("claude-idle-nothing-running.txt")
        .replace("❯\u{a0}wait for the background shell to finish", "❯\u{a0}\x1b[2mwait for the background shell to finish\x1b[0m")
}

#[test]
fn only_a_resting_claude_offers_its_prediction() {
    let want = Some("wait for the background shell to finish".to_string());
    assert_eq!(offered(Some("claude"), &predicted(), true), (want, None));
    assert_eq!(offered(Some("claude"), &predicted(), false), (None, None), "mid-turn, a dim line is a hint");
    assert_eq!(offered(Some("codex"), &predicted(), true), (None, None));
    assert_eq!(offered(None, &predicted(), true), (None, None));
    let fresh = capture("claude-2.1.292-idle-placeholder-160x45-e.txt");
    let example = Some("Try \"how does <filepath> work?\"".to_string());
    assert_eq!(offered(Some("claude"), &fresh, true), (None, example), "the generic example is a hint, not a prediction");
    assert_eq!(offered(Some("claude"), &fresh, false), (None, None));
}

#[tokio::test]
async fn a_follower_is_woken_with_the_suggestion_on_the_newest_turn() {
    let dir = std::env::temp_dir().join(format!("fc-suggest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"user","promptId":"p1","promptSource":"typed","timestamp":"2026-10-06T10:00:00Z","message":{"role":"user","content":"go"}}"#.to_string() + "\n",
    )
    .unwrap();
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path);
    let page = projectors.read_page(terminal, None, 10).unwrap();

    // The follower is already waiting when the tick reads the screen.
    let waiting = projectors.follow(terminal, page.epoch, page.rev, tokio::time::Instant::now() + Duration::from_secs(30), 100);
    let (suggestion, hint) = offered(Some("claude"), &predicted(), true);
    projectors.suggest(terminal, suggestion, hint);
    let Some(Follow::Changes { changes, .. }) = waiting.await else { panic!("a follow answers") };
    let [RowChange::Update(row)] = &changes[..] else { panic!("{changes:?}") };
    let RowKind::Turn(turn) = &row.kind else { panic!() };
    assert_eq!(turn.suggestion.as_deref(), Some("wait for the background shell to finish"));
    assert!(serde_json::to_string(row).unwrap().contains("\"suggestion\":\"wait for the background shell to finish\""));

    // A fresh session has no turn: the example rides its own row, emptied
    // when the box shows something else.
    let fresh = std::fs::read_to_string(format!("{}/../core/captures/claude-2.1.292-idle-placeholder-160x45-e.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let page = projectors.read_page(terminal, None, 10).unwrap();
    let waiting = projectors.follow(terminal, page.epoch, page.rev, tokio::time::Instant::now() + Duration::from_secs(30), 100);
    let (suggestion, hint) = offered(Some("claude"), &fresh, true);
    projectors.suggest(terminal, suggestion, hint);
    let Some(Follow::Changes { changes, .. }) = waiting.await else { panic!("a follow answers") };
    let hints: Vec<_> = changes
        .iter()
        .filter_map(|c| match c {
            RowChange::Insert(row) | RowChange::Update(row) => match &row.kind {
                RowKind::Hint(hint) => Some((row.id.clone(), hint.text.clone())),
                _ => None,
            },
            RowChange::Remove { .. } => None,
        })
        .collect();
    assert_eq!(hints, [("hint:composer".to_string(), "Try \"how does <filepath> work?\"".to_string())]);
    projectors.suggest(terminal, None, None);
    let rows = projectors.read_page(terminal, None, 10).unwrap().rows;
    let hint = rows.iter().find_map(|r| match &r.kind {
        RowKind::Hint(h) => Some(h.text.clone()),
        _ => None,
    });
    assert_eq!(hint.as_deref(), Some(""), "emptied, not removed");

    // A terminal with no projector open takes it without complaint.
    projectors.suggest(Uuid::now_v7(), Some("x".into()), Some("y".into()));
    let _ = std::fs::remove_dir_all(&dir);
}
