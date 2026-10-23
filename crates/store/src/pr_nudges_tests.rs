use super::*;
use crate::compat::Older;
use crate::landing::{LandingPatch, NUDGE_AFTER_HOURS, NUDGE_HOURS_MAX};

const REPO: &str = "github.com/acme/overnight";

fn board() -> (Store, &'static str, crate::models::Workspace) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap();
    (store, REPO, main)
}

#[test]
fn the_migration_is_welcome() {
    let last = &crate::migrate::MIGRATIONS[42];
    assert!(std::ptr::fn_addr_eq(last.0, migration_0052_pr_nudge as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 49);
}

#[test]
fn a_pull_request_is_nudged_once_and_a_second_claim_is_refused() {
    let (store, repo, _) = board();
    assert!(!store.pr_nudged(repo, 7).unwrap());
    assert!(store.claim_pr_nudge(repo, 7).unwrap(), "the first claim may post");
    assert!(!store.claim_pr_nudge(repo, 7).unwrap(), "the second may not");
    assert!(store.pr_nudged(repo, 7).unwrap());
    assert!(store.claim_pr_nudge(repo, 8).unwrap(), "another pull request is its own");
    assert!(store.claim_pr_nudge("github.com/acme/elsewhere", 7).unwrap(), "and so is the same number in another repository");
    assert!(!store.claim_pr_nudge("GitHub.com/ACME/Overnight", 7).unwrap(), "a repository's name has no case");
}

#[test]
fn a_claim_given_back_after_a_post_that_failed_may_be_taken_again() {
    let (store, repo, _) = board();
    assert!(store.claim_pr_nudge(repo, 7).unwrap());
    store.release_pr_nudge(repo, 7).unwrap();
    assert!(store.claim_pr_nudge(repo, 7).unwrap());
    assert!(!store.claim_pr_nudge(repo, 7).unwrap());
}

#[test]
fn a_restart_does_not_send_it_again() {
    let dir = farcooler_core::scratch::dir("nudges");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let repo = REPO;
    {
        let store = Store::open(&path).unwrap();
        assert!(store.claim_pr_nudge(repo, 31).unwrap());
    }
    let store = Store::open(&path).unwrap();
    assert!(store.pr_nudged(repo, 31).unwrap(), "the record is on disk");
    assert!(!store.claim_pr_nudge(repo, 31).unwrap());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nudges_are_off_until_somebody_turns_them_on_and_the_wait_defaults_to_a_day() {
    let (store, _, main) = board();
    let said = store.get_landing(main.id).unwrap();
    assert!(!said.pr_nudge, "it posts as the owner, so it is never on by default");
    assert_eq!((said.pr_nudge_hours, said.nudge_after_hours()), (None, NUDGE_AFTER_HOURS));
    assert_eq!(NUDGE_AFTER_HOURS, 24);

    let w = store.set_landing(main.id, main.resource_version, &LandingPatch { pr_nudge: Some(true), ..Default::default() }).unwrap();
    let on = store.get_landing(main.id).unwrap();
    assert!(on.pr_nudge && on.pr_nudge_hours.is_none());

    let w = store.set_landing(main.id, w.resource_version, &LandingPatch { pr_nudge_hours: Some(48), ..Default::default() }).unwrap();
    let two_days = store.get_landing(main.id).unwrap();
    assert!(two_days.pr_nudge, "the hours alone leave the switch where it was");
    assert_eq!(two_days.nudge_after_hours(), 48);

    let w = store.set_landing(main.id, w.resource_version, &LandingPatch { pr_nudge_hours: Some(0), ..Default::default() }).unwrap();
    assert_eq!(store.get_landing(main.id).unwrap().nudge_after_hours(), 24, "0 takes the choice away");

    let w = store.set_landing(main.id, w.resource_version, &LandingPatch { pr_nudge: Some(false), mode: None, ..Default::default() }).unwrap();
    assert!(!store.get_landing(main.id).unwrap().pr_nudge);
    let err = store.set_landing(main.id, w.resource_version, &LandingPatch { pr_nudge_hours: Some(NUDGE_HOURS_MAX + 1), ..Default::default() }).unwrap_err();
    assert!(matches!(err, farcooler_core::DomainError::InvalidArgument { what: "pr_nudge_hours" }), "{err:?}");
}

#[test]
fn the_same_repository_registered_twice_is_one_to_github() {
    // The store's own ids play no part: two registrations, one claim.
    let (store, repo, _) = board();
    let _again = store.register_repository_for_test("overnight-again");
    assert!(store.claim_pr_nudge(repo, 9).unwrap());
    assert!(!store.claim_pr_nudge(repo, 9).unwrap());
}
