//! `compose_codex`'s confirmation, read against rollout files directly.

use super::*;

/// A rollout read from its start holds earlier messages (review 1, L1): one
/// with the same words, dated before the Enter, doesn't confirm the send.
#[test]
fn an_earlier_message_with_the_same_words_does_not_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl");
    let line = r#"{"timestamp":"2026-10-07T16:05:00.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"continue"}]}}}"#;
    std::fs::write(&path, format!("{line}\n")).unwrap();
    let at = |entered| Sent { text: "continue".into(), images: 0, before: None, from: 0, entered };
    assert!(!records(&path, 0, &at(1_791_389_100_001)), "an earlier message");
    assert!(records(&path, 0, &at(1_791_389_100_000)), "its own, dated after the Enter");
}
