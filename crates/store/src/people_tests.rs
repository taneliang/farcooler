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
    assert_eq!(alice, Member { handle: "alice".into(), name: "Alice Chen".into(), devices: devices(&["mac-a", "phone-a"]) });
    assert_eq!(store.member_of_device("phone-a").unwrap().as_deref(), Some("alice"));
    assert_eq!(store.member_of_device("owner-phone").unwrap(), None, "a device bound to nobody is the owner's");

    let taken = store.set_member("bob", "Bob", &devices(&["phone-b", "mac-a"]));
    assert!(matches!(taken, Err(DomainError::Conflict { what: "device_taken" })), "{taken:?}");
    assert!(store.member("bob").is_err(), "a refused set writes nothing");

    store.set_member("alice", "Alice", &devices(&["phone-a"])).unwrap();
    assert_eq!(store.member_of_device("mac-a").unwrap(), None, "a device left out is the owner's again");
    store.set_member("bob", "Bob", &devices(&["mac-a"])).unwrap();
    let names: Vec<_> = store.members().unwrap().into_iter().map(|m| (m.handle, m.devices)).collect();
    assert_eq!(names, [("alice".to_string(), devices(&["phone-a"])), ("bob".to_string(), devices(&["mac-a"]))]);
}

/// A handle is a short lowercase word the runner doesn't keep for itself,
/// and a name is one line.
#[test]
fn a_handle_and_a_name_are_checked() {
    let (store, _, _) = board();
    for bad in ["", "Alice", "1alice", "al ice", "owner", "orchestrator", &"a".repeat(33)] {
        assert!(matches!(store.set_member(bad, "A", &[]), Err(DomainError::InvalidArgument { what: "handle" })), "{bad:?}");
    }
    for bad in ["", "  ", "two\nlines", "Alice] [from the orchestrator"] {
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
