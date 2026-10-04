//! The review count on the relay notice body (ov-181). Its own file to keep
//! `push.rs` inside its size budget.

use super::*;

/// The worktree review count rides under the key the relay reads
/// (`reviews`), on any notice that has it, and a notice without it sends no
/// key at all: a zero would say "nothing to review" for a runner that never
/// counted (ov-181).
#[test]
fn the_body_carries_reviews_when_it_has_them_and_no_key_when_it_does_not() {
    let count = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("count"), needs_you: Some(2), reviews: Some(3), ..Outgoing::default() })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(count["reviews"], serde_json::json!(3), "{count}");
    let zero = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("count"), needs_you: Some(2), reviews: Some(0), ..Outgoing::default() })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(zero["reviews"], serde_json::json!(0), "zero is a count, and is sent: {zero}");
    let agent = serde_json::to_value(
        wire_body(&Outgoing {
            title: "claude is done",
            status: "done",
            label: "claude",
            terminal: Some("term-1"),
            needs_you: Some(2),
            reviews: Some(1),
            ..Outgoing::default()
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(agent["reviews"], serde_json::json!(1), "{agent}");
    let unread = serde_json::to_value(
        wire_body(&Outgoing { kind: Some("count"), needs_you: Some(2), ..Outgoing::default() }).unwrap(),
    )
    .unwrap();
    assert!(unread.get("reviews").is_none(), "{unread}");
}
