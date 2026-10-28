//! A member's role and the people log (ov-757), on an in-memory store.

use super::*;

fn store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    store.set_member("carol", "Carol", &[]).unwrap();
    store
}

/// Every member is a Member until the owner says otherwise, so a runner
/// where nobody was given a role reads exactly as before.
#[test]
fn every_member_starts_a_member() {
    let s = store();
    assert_eq!(s.member_role("bob").unwrap(), Role::Member);
    let roles = s.member_roles().unwrap();
    assert_eq!(roles.len(), 2);
    assert!(roles.values().all(|r| *r == Role::Member));
    assert!(s.people_events(10).unwrap().is_empty(), "nothing logged");
    assert!(matches!(s.member_role("nobody"), Err(DomainError::NotFound)));
}

/// A role given is logged with who gave it, from and to; given again it
/// writes nothing; renaming the member or changing their devices keeps it.
#[test]
fn a_role_change_is_logged_and_kept() {
    let s = store();
    assert_eq!(s.set_member_role("bob", Role::Maintainer, "user").unwrap(), Role::Member, "what he had");
    assert_eq!(s.member_role("bob").unwrap(), Role::Maintainer);
    assert_eq!(s.set_member_role("bob", Role::Maintainer, "user").unwrap(), Role::Maintainer);
    let log = s.people_events(10).unwrap();
    assert_eq!(log.len(), 1, "the second set changed nothing: {log:?}");
    let e = &log[0];
    assert_eq!((e.actor.as_str(), e.handle.as_str(), e.kind.as_str(), e.workspace), ("user", "bob", "role", None));
    let detail: serde_json::Value = serde_json::from_str(&e.detail).unwrap();
    assert_eq!(detail, serde_json::json!({ "from": "member", "to": "maintainer" }));

    s.set_member("bob", "Bob Ng", &["phone-b".to_string(), "mac-b".to_string()]).unwrap();
    assert_eq!(s.member_role("bob").unwrap(), Role::Maintainer, "kept through member.set");
    assert_eq!(s.member_role("carol").unwrap(), Role::Member, "nobody else's moved");

    s.set_member_role("bob", Role::Member, "user").unwrap();
    let log = s.people_events(10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&log[0].detail).unwrap()["to"], "member", "newest first");
    assert!(matches!(s.set_member_role("nobody", Role::Maintainer, "user"), Err(DomainError::NotFound)));
    assert_eq!(s.people_events(10).unwrap().len(), 2, "nobody's change logs nothing");
}

/// A member removed and added again under the handle starts a Member: the
/// role went with the row.
#[test]
fn a_member_added_again_starts_over() {
    let s = store();
    s.set_member_role("bob", Role::Maintainer, "user").unwrap();
    s.remove_member("bob").unwrap();
    s.set_member("bob", "Bob", &[]).unwrap();
    assert_eq!(s.member_role("bob").unwrap(), Role::Member);
}

/// Lines written for changes made elsewhere read back, workspace and all,
/// and `limit` keeps the newest.
#[test]
fn the_log_reads_back_newest_first() {
    let s = store();
    let ws = Uuid::now_v7();
    let detail = serde_json::json!({ "sha256": "ab" });
    s.record_people_event(NewPeopleEvent { actor: "member:bob", handle: "", workspace: Some(ws), kind: "charter", detail: &detail }).unwrap();
    s.record_people_event(NewPeopleEvent { actor: "user", handle: "carol", workspace: None, kind: "removed", detail: &serde_json::json!({}) })
        .unwrap();
    let log = s.people_events(1).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].kind, "removed");
    let all = s.people_events(10).unwrap();
    assert_eq!(all[1].workspace, Some(ws));
    assert_eq!(all[1].actor, "member:bob");
}

/// The column refuses any word but the two.
#[test]
fn the_column_holds_only_the_two_roles() {
    let s = store();
    let wrote = s.conn().execute("UPDATE members SET role = 'owner' WHERE handle = 'bob'", []);
    assert!(wrote.is_err(), "{wrote:?}");
}

/// The migration is the 51st (0060) and `Welcome`.
#[test]
fn the_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[50];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0060_member_roles as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 55);
}
