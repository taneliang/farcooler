//! Members, leads and reps (ov-494), on an in-memory board.

use super::*;
use crate::models::Actor;
use farcooler_protocol::v1::TerminalIntent;

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let worktree = store.create_worktree(repo, "main", "/wt/people", true).unwrap().id;
    (store, main, worktree)
}

fn devices(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|d| d.to_string()).collect()
}

/// A member is a handle, a name and their devices; setting them again
/// renames them and replaces the devices; a device is one member's at most.
#[test]
fn a_member_holds_their_devices_and_no_one_elses() {
    let (store, _, _) = board();
    let alice = store.set_member("alice", " Alice Chen ", &devices(&["phone-a", "mac-a", "phone-a"])).unwrap();
    assert_eq!(
        alice,
        Member { handle: "alice".into(), name: "Alice Chen".into(), devices: devices(&["mac-a", "phone-a"]), account: None, email: None }
    );
    assert_eq!(store.member_of_device("phone-a").unwrap().as_deref(), Some("alice"));
    assert_eq!(store.member_of_device("owner-phone").unwrap(), None, "a device bound to nobody is the owner's");

    let taken = store.set_member("bob", "Bob", &devices(&["phone-b", "mac-a"]));
    assert!(matches!(taken, Err(DomainError::Conflict { what: "device_taken" })), "{taken:?}");
    assert!(store.member("bob").is_err(), "a refused set writes nothing");

    store.set_member("alice", "Alice", &devices(&["phone-a"])).unwrap();
    assert_eq!(store.member_of_device("mac-a").unwrap(), None, "a device left out is nobody's");
    assert!(store.device_released("mac-a").unwrap(), "and never the owner's");
    assert!(!store.device_released("owner-phone").unwrap());
    store.set_member("bob", "Bob", &devices(&["mac-a"])).unwrap();
    assert!(!store.device_released("mac-a").unwrap(), "bound again, it's Bob's");
    store.set_member("carol", "Carol", &devices(&["phone-c"])).unwrap();
    store.remove_member("carol").unwrap();
    assert!(store.device_released("phone-c").unwrap(), "a removed member's device is nobody's");
    let names: Vec<_> = store.members().unwrap().into_iter().map(|m| (m.handle, m.devices)).collect();
    assert_eq!(names, [("alice".to_string(), devices(&["phone-a"])), ("bob".to_string(), devices(&["mac-a"]))]);
}

/// A set answers with exactly the devices it took off this member's list
/// (ov-783): not one kept, not one added, not another member's, and nothing
/// when a set is refused.
#[test]
fn a_set_names_the_devices_it_released() {
    let (store, _, _) = board();
    let set = |h: &str, d: &[&str]| store.set_member_releasing(h, h, &devices(d), Directory::default());
    assert_eq!(set("alice", &["phone-a", "mac-a", "pad-a"]).unwrap().1, devices(&[]), "a new member released nothing");
    set("bob", &["phone-b"]).unwrap();
    let (alice, released) = set("alice", &["phone-a", "watch-a"]).unwrap();
    assert_eq!(alice.devices, devices(&["phone-a", "watch-a"]));
    assert_eq!(released, devices(&["mac-a", "pad-a"]));
    assert!(set("alice", &["phone-b"]).is_err(), "Bob's phone is his");
    assert_eq!(store.member("alice").unwrap().devices, devices(&["phone-a", "watch-a"]), "a refused set releases nothing");
    assert_eq!(set("alice", &[" watch-a ", "phone-a"]).unwrap().1, devices(&[]), "the same list, trimmed");
    assert_eq!(set("alice", &[]).unwrap().1, devices(&["phone-a", "watch-a"]));
    assert_eq!(store.member("bob").unwrap().devices, devices(&["phone-b"]));
}

/// What a sweep may revoke (ov-783): a device taken off a member's list, or
/// one a removed member had. Never one bound to a member, the owner's (bound
/// to nobody, never released), or one a member's device enrolled that's
/// waiting on its first binding. Bound once and taken off again, that one is
/// swept like any other.
#[test]
fn a_sweep_names_only_devices_taken_off_a_member() {
    let (store, _, _) = board();
    store.set_member("alice", "Alice", &devices(&["phone-a", "mac-a"])).unwrap();
    store.set_member("bob", "Bob", &devices(&["phone-b"])).unwrap();
    store.record_member_enrolled("pad-new", "alice").unwrap();
    store.record_member_enrolled("pad-bound", "alice").unwrap();
    assert_eq!(store.released_to_sweep().unwrap(), devices(&[]), "nothing taken off anyone yet");

    store.set_member("bob", "Bob", &devices(&["phone-b", "pad-bound"])).unwrap();
    store.set_member("alice", "Alice", &devices(&["phone-a"])).unwrap();
    store.set_member("bob", "Bob", &devices(&["phone-b"])).unwrap();
    store.set_member("carol", "Carol", &devices(&["phone-c"])).unwrap();
    store.remove_member("carol").unwrap();
    assert_eq!(store.released_to_sweep().unwrap(), devices(&["mac-a", "pad-bound", "phone-c"]));
    assert!(store.device_released("pad-new").unwrap(), "waiting on its binding, and not swept");
}

/// A handle is a short lowercase word the runner doesn't keep for itself,
/// and a name is one line.
#[test]
fn a_handle_and_a_name_are_checked() {
    let (store, _, _) = board();
    for bad in ["", "Alice", "1alice", "al ice", "owner", "orchestrator", &"a".repeat(33)] {
        assert!(matches!(store.set_member(bad, "A", &[]), Err(DomainError::InvalidArgument { what: "handle" })), "{bad:?}");
    }
    for bad in ["", "  ", "two\nlines", "Alice] [from the orchestrator", "The Orchestrator", "Far Cooler", "Alice’s rep"] {
        assert!(matches!(store.set_member("alice", bad, &[]), Err(DomainError::InvalidArgument { what: "member_name" })), "{bad:?}");
    }
    assert!(valid_handle("alice-2"));
}

/// The owner leads until the lead is handed to a member; there is one lead
/// at a time; removing that member gives it back to the owner.
#[test]
fn a_workspace_has_one_lead_and_the_owner_is_the_default() {
    let (store, main, _) = board();
    assert_eq!(store.workspace_lead(main).unwrap(), None, "the owner leads by default");
    assert!(matches!(store.set_workspace_lead(main, Some("alice")), Err(DomainError::NotFound)), "no such member");
    store.set_member("alice", "Alice", &[]).unwrap();
    store.set_member("bob", "Bob", &[]).unwrap();
    store.set_workspace_lead(main, Some("alice")).unwrap();
    store.set_workspace_lead(main, Some("bob")).unwrap();
    assert_eq!(store.workspace_lead(main).unwrap().as_deref(), Some("bob"), "handed over, not added");
    store.remove_member("bob").unwrap();
    assert_eq!(store.workspace_lead(main).unwrap(), None, "back to the owner");
    store.set_workspace_lead(main, Some("alice")).unwrap();
    store.set_workspace_lead(main, None).unwrap();
    assert_eq!(store.workspace_lead(main).unwrap(), None);
    assert!(matches!(store.remove_member("bob"), Err(DomainError::NotFound)));
}

/// A rep is one per member per workspace, found by its pane; it goes with
/// its pane or its member, and the desk card stays with the member.
#[test]
fn a_rep_is_bound_once_and_goes_with_its_pane_or_member() {
    let (store, main, worktree) = board();
    store.set_member("alice", "Alice", &[]).unwrap();
    let pane = |title| store.create_terminal(worktree, title, "claude", TerminalIntent::Running, 80, 24).unwrap().id;
    let (first, second) = (pane("one"), pane("two"));
    let rep = store.bind_rep(first, main, "alice").unwrap();
    assert_eq!(store.rep_of_terminal(first).unwrap(), Some(rep.clone()));
    assert_eq!(store.rep_for(main, "alice").unwrap(), Some(rep.clone()));
    assert_eq!(store.reps_of("alice").unwrap(), [rep]);
    assert!(matches!(store.bind_rep(second, main, "alice"), Err(DomainError::Conflict { what: "rep_taken" })));
    assert_eq!(store.rep_of_terminal(second).unwrap(), None);

    let desk = store.create_task(main, "Alice's requests", Actor::Runner).unwrap().id;
    store.set_rep_desk(main, "alice", desk).unwrap();
    let term = store.get_terminal(first).unwrap();
    store.delete_terminal(first, term.resource_version).unwrap();
    assert_eq!(store.rep_for(main, "alice").unwrap(), None, "the rep row goes with its pane");
    assert_eq!(store.rep_desk(main, "alice").unwrap(), Some(desk), "the desk outlives the pane");
    store.bind_rep(second, main, "alice").unwrap();
    store.remove_member("alice").unwrap();
    assert_eq!(store.rep_of_terminal(second).unwrap(), None, "and with its member");
    assert_eq!(store.rep_desk(main, "alice").unwrap(), None);
}

/// A member added by QR keeps the directory account they were added under
/// and its email (ov-525): one account is one member, an account never moves
/// to another person, and setting them again without one keeps it.
#[test]
fn a_member_keeps_the_directory_account_they_were_added_under() {
    let (store, _, _) = board();
    let bob = Directory { account: "user_bob", email: "bob@example.com" };
    let added = store.set_member_in("bob", "Bob", &devices(&["phone-b"]), bob).unwrap();
    assert_eq!((added.account.as_deref(), added.email.as_deref()), (Some("user_bob"), Some("bob@example.com")));

    // Renamed by hand, with no account given: it stays.
    store.set_member("bob", "Bob Smith", &devices(&["phone-b", "ipad-b"])).unwrap();
    let listed = store.member("bob").unwrap();
    assert_eq!((listed.name.as_str(), listed.account.as_deref()), ("Bob Smith", Some("user_bob")));
    assert_eq!(listed.email.as_deref(), Some("bob@example.com"));

    let taken = store.set_member_in("robert", "Robert", &[], bob);
    assert!(matches!(taken, Err(DomainError::Conflict { what: "account_taken" })), "{taken:?}");
    assert!(store.member("robert").is_err(), "a refused set writes nothing");
    let moved = store.set_member_in("bob", "Bob", &[], Directory { account: "user_mallory", email: "" });
    assert!(matches!(moved, Err(DomainError::Conflict { what: "account_differs" })), "{moved:?}");
    assert_eq!(store.member("bob").unwrap().devices, devices(&["ipad-b", "phone-b"]), "and changes nothing");

    for (account, email, what) in [
        ("user bob", "", "member_account"),
        ("user\nbob", "", "member_account"),
        ("user_carol", "not-an-email", "member_email"),
        ("user_carol", "carol@exa\nmple.com", "member_email"),
    ] {
        let odd = store.set_member_in("carol", "Carol", &[], Directory { account, email });
        assert!(matches!(odd, Err(DomainError::InvalidArgument { what: w }) if w == what), "{account:?} {email:?}: {odd:?}");
    }
    // Removed, the account is free for whoever is added under it next.
    store.remove_member("bob").unwrap();
    assert!(store.set_member_in("robert", "Robert", &[], bob).is_ok());
}

/// A database from before 0050 migrates with its members as they were, and
/// with no account on any of them.
#[test]
fn members_from_before_accounts_migrate_without_one() {
    use crate::migrate::{MIGRATIONS, migrate, migrate_only_to};
    let ours = MIGRATIONS
        .iter()
        .position(|(m, _)| std::ptr::fn_addr_eq(*m, migration_0050_member_accounts as fn(&rusqlite::Transaction) -> rusqlite::Result<()>))
        .expect("the migration is registered") as u32;
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    migrate_only_to(&mut conn, ours);
    conn.execute("INSERT INTO members (handle, name, created_at) VALUES ('alice', 'Alice', 1)", []).unwrap();
    conn.execute("INSERT INTO members (handle, name, created_at) VALUES ('bob', 'Bob', 2)", []).unwrap();
    migrate(&mut conn, ours).unwrap();
    let accounts: Vec<(String, Option<String>)> = conn
        .prepare("SELECT handle, account FROM members ORDER BY handle")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(accounts, vec![("alice".to_string(), None), ("bob".to_string(), None)]);
    // Two members with no account are not one account twice.
    conn.execute("INSERT INTO members (handle, name, created_at) VALUES ('carol', 'Carol', 3)", []).unwrap();
}
