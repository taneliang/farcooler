//! A member's actor word (ov-707): `member:<handle>`, read back as written,
//! and nothing that isn't a member's handle taken for one.

use super::{Actor, Handle};

#[test]
fn a_members_word_reads_back_as_the_member() {
    let alice = Actor::Member { handle: Handle::new("alice-chen").unwrap() };
    assert_eq!(alice.to_string(), "member:alice-chen");
    assert_eq!(Actor::parse("member:alice-chen"), Some(alice));
    assert!(alice.is_person());
    assert!(Actor::User.is_person());
    assert!(!Actor::Manager.is_person() && !Actor::Runner.is_person() && !Actor::Unknown.is_person());
}

#[test]
fn nothing_but_a_handle_a_member_could_have_is_a_member() {
    let longest = format!("member:{}", "a".repeat(32));
    assert!(Actor::parse(&longest).is_some(), "32 characters is a handle");
    for raw in ["member:", "member:Alice", "member:owner", "member:user", "member:9lives", "member:a b", &format!("member:{}", "a".repeat(33))] {
        assert_eq!(Actor::parse(raw), None, "{raw}");
    }
    assert_eq!(Handle::new("al\u{e9}"), None);
}
