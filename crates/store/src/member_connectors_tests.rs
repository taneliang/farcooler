//! A member's connectors (ov-586), on an in-memory board.

use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

/// Keys are claude's own: anything outside `[A-Za-z0-9_-]` is `_`, runs are
/// one `_`, none at the ends, so no key holds `__`.
#[test]
fn a_key_is_the_server_segment_claude_builds() {
    for (name, key) in [
        ("Slack", "claude_ai_Slack"),
        ("Google Drive", "claude_ai_Google_Drive"),
        ("claude.ai Linear", "claude_ai_Linear"),
        ("Slack (2)", "claude_ai_Slack_2"),
        ("  Notion!  ", "claude_ai_Notion"),
        ("a__b", "claude_ai_a_b"),
        ("Café Ops", "claude_ai_Caf_Ops"),
        ("my-tool", "claude_ai_my-tool"),
        ("Asana \u{1F600}", "claude_ai_Asana"),
    ] {
        let got = connector_key(name);
        assert_eq!(got, key, "{name}");
        assert!(!got.contains("__"), "{got}");
    }
}

/// Empty by default; setting replaces the list; a name with claude's prefix
/// is kept without it; one key is one connector; and the list goes with its
/// member.
#[test]
fn a_members_connectors_are_replaced_whole_and_go_with_them() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("alice", "Alice", &[]).unwrap();
    assert!(store.member_connectors("alice").unwrap().is_empty(), "none until they turn one on");

    let kept = store.set_member_connectors("alice", &names(&["Slack", "claude.ai Google Drive", "Google  Drive", " Linear "])).unwrap();
    assert_eq!(kept, names(&["Google Drive", "Linear", "Slack"]));
    assert_eq!(store.set_member_connectors("alice", &names(&["Linear"])).unwrap(), names(&["Linear"]));
    assert!(store.set_member_connectors("alice", &[]).unwrap().is_empty(), "an empty list turns them all off");

    store.set_member_connectors("alice", &names(&["Slack"])).unwrap();
    store.remove_member("alice").unwrap();
    store.set_member("alice", "Another Alice", &[]).unwrap();
    assert!(store.member_connectors("alice").unwrap().is_empty(), "a reused handle starts with none");
}

/// A refused list writes nothing: no such member, a name with no letter, a
/// control character or too long, and too many.
#[test]
fn a_refused_list_writes_nothing() {
    let store = Store::open_in_memory().unwrap();
    assert!(matches!(store.set_member_connectors("nobody", &names(&["Slack"])), Err(DomainError::NotFound)));
    store.set_member("alice", "Alice", &[]).unwrap();
    store.set_member_connectors("alice", &names(&["Slack"])).unwrap();
    let long = "x".repeat(65);
    for bad in ["", "  ", "!!!", "claude.ai ", "Sl\nack", long.as_str()] {
        let refused = store.set_member_connectors("alice", &names(&["Linear", bad]));
        assert!(matches!(refused, Err(DomainError::InvalidArgument { what: "connector" })), "{bad:?}: {refused:?}");
    }
    let many: Vec<String> = (0..=MOST_CONNECTORS).map(|i| format!("Tool {i}")).collect();
    assert!(matches!(store.set_member_connectors("alice", &many), Err(DomainError::InvalidArgument { what: "connectors" })));
    assert_eq!(store.member_connectors("alice").unwrap(), names(&["Slack"]), "unchanged");
}

/// The migration is the 45th (0054) and `Welcome`: a build from before it never
/// names the table, and its reps run with no connector.
#[test]
fn the_migration_is_welcome() {
    use crate::compat::Older;
    let it = &crate::migrate::MIGRATIONS[44];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0054_member_connectors as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, Older::Welcome);
}
