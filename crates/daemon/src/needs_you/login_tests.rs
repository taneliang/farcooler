use std::time::{Duration, UNIX_EPOCH};

use farcooler_protocol::v1::NeedsYouKind;

use super::*;
use crate::claude_login_renewal::{LoginNeed, Why};
use crate::needs_you::{assemble, redact_below_control};

fn inputs(viewer: Option<Person>) -> Inputs {
    Inputs {
        logins: vec![
            LoginNeed { handle: "alice".into(), name: "Alice Chen".into(), since_ms: 1_000, why: Why::Stopped },
            LoginNeed { handle: "bob".into(), name: "Bob".into(), since_ms: 2_000, why: Why::Asked },
        ],
        runner: "Studio".into(),
        viewer,
        ..Inputs::default()
    }
}

fn now() -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_secs(2_000_000)
}

fn seen_by(viewer: Option<Person>) -> Vec<pb::NeedsYouItem> {
    assemble(&inputs(viewer), now())
}

/// A member sees their own login and no one else's, in their own words and
/// with the one button; the owner sees every member's by name with none; a
/// released device and a reader with no viewer see nothing.
#[test]
fn each_viewer_sees_their_own_part() {
    let alices = seen_by(Some(Person::Member("alice".into())));
    assert_eq!(alices.len(), 1);
    assert_eq!(alices[0].question, "Your Claude login on Studio stopped working.");
    assert_eq!(alices[0].kind(), NeedsYouKind::ClaudeLogin);
    assert_eq!(alices[0].id, "claude-login:alice");
    assert_eq!(alices[0].actions.iter().map(|a| (a.id.as_str(), a.title.as_str(), a.primary)).collect::<Vec<_>>(), vec![("sign_in", "Sign In Again", true)]);

    let bobs = seen_by(Some(Person::Member("bob".into())));
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0].question, "Sign in to Claude on Studio to start your rep.");
    assert_eq!(bobs[0].actions[0].title, "Sign In");

    let owner = seen_by(Some(Person::Owner));
    let said: Vec<&str> = owner.iter().map(|i| i.question.as_str()).collect();
    assert_eq!(said, vec!["Alice Chen’s Claude login stopped working", "Bob needs to sign in to Claude"], "oldest first");
    assert!(owner.iter().all(|i| i.actions.is_empty()), "nothing for the owner to press");

    assert!(seen_by(Some(Person::Member("carol".into()))).is_empty(), "a member with no login trouble");
    assert!(seen_by(Some(Person::Released)).is_empty());
    assert!(seen_by(None).is_empty(), "the runner-wide count has none");
}

/// A login item names a person and offers a button answered at any scope, so
/// the redaction a Read peer gets keeps both.
#[test]
fn a_read_peer_keeps_the_question_and_the_button() {
    let item = seen_by(Some(Person::Member("alice".into()))).remove(0);
    let read = redact_below_control(item.clone());
    assert_eq!(read, item);
}

/// It ranks with a block: after an ask, before a decision.
#[test]
fn it_ranks_with_the_blocks() {
    use crate::needs_you::{TIER_SPAN, tier};
    assert_eq!(tier(NeedsYouKind::ClaudeLogin), tier(NeedsYouKind::Blocked));
    let item = seen_by(Some(Person::Owner)).remove(0);
    assert_eq!(item.rank / TIER_SPAN, 1);
}

/// A refused Codex or Cursor pane is a line with no button (neither has a
/// sign-in yet), under an id of its own so it sits beside Claude's (ov-652).
#[test]
fn a_codex_or_cursor_ask_has_its_line_and_no_button() {
    let mut inputs = inputs(None);
    inputs.logins = vec![
        LoginNeed { handle: "bob".into(), name: "Bob".into(), since_ms: 1_000, why: Why::Asked },
        LoginNeed { handle: "bob".into(), name: "Bob".into(), since_ms: 2_000, why: Why::CodexAsked },
        LoginNeed { handle: "bob".into(), name: "Bob".into(), since_ms: 3_000, why: Why::CursorAsked },
    ];
    inputs.viewer = Some(Person::Member("bob".into()));
    let bobs = assemble(&inputs, now());
    assert_eq!(
        bobs.iter().map(|i| (i.id.as_str(), i.question.as_str(), i.actions.len())).collect::<Vec<_>>(),
        vec![
            ("claude-login:bob", "Sign in to Claude on Studio to start your rep.", 1),
            ("codex-login:bob", "Codex isn’t connected to your account on Studio yet.", 0),
            ("cursor-login:bob", "Cursor isn’t connected to your account on Studio yet.", 0),
        ]
    );
    let ranks: Vec<u32> = bobs.iter().map(|i| i.rank).collect();
    let blocked = rank(NeedsYouKind::Blocked, now_ms(), now_ms());
    assert!(ranks[1] > blocked && ranks[2] > blocked, "below even the newest Blocked: {ranks:?} vs {blocked}");
    inputs.viewer = Some(Person::Owner);
    assert_eq!(assemble(&inputs, now())[1].question, "Bob needs Codex connected to their account");
}

fn now_ms() -> i64 {
    2_000_000_000
}
