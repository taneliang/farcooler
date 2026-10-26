//! The nudge's two settings (ov-580): off by default, set at Control and no
//! lower, the wait defaulting to a day, and a refusal changing nothing.

use farcooler_protocol::v1::{self as pb, Scope};
use uuid::Uuid;

use crate::test_support::fixture;
use crate::watch::Watcher;
use crate::workspace_ops::{list, set_settings};
use farcooler_core::DomainError;

#[test]
fn turning_it_on_is_a_control_scope_write() {
    assert_eq!(crate::rpc::required_scope("workspace.set_settings"), Some(Scope::Control));
}

#[tokio::test]
async fn a_board_never_nudges_until_told_and_the_wait_reads_as_a_day() {
    let (_dir, svc, repo) = fixture().await;
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    assert_eq!((main.pr_nudge, main.pr_nudge_hours), (Some(false), Some(24)));
}

#[tokio::test]
async fn the_switch_and_the_wait_are_set_read_back_and_the_wait_goes_back_to_a_day_at_zero() {
    let (_dir, svc, repo) = fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    let id = Uuid::from_slice(&main.id).unwrap();
    let on = pb::WorkspaceSetSettings { pr_nudge: Some(true), pr_nudge_hours: Some(48), ..Default::default() };
    let set = set_settings(&svc, &watcher, id, &on, Scope::Control, crate::workspace_ops::LOCAL).await.unwrap();
    assert_eq!((set.pr_nudge, set.pr_nudge_hours), (Some(true), Some(48)));
    let read = list(&svc, Some(repo), Scope::Read).unwrap().items.remove(0);
    assert_eq!((read.pr_nudge, read.pr_nudge_hours), (Some(true), Some(48)));

    let reset = pb::WorkspaceSetSettings { pr_nudge_hours: Some(0), ..Default::default() };
    let set = set_settings(&svc, &watcher, id, &reset, Scope::Control, crate::workspace_ops::LOCAL).await.unwrap();
    assert_eq!((set.pr_nudge, set.pr_nudge_hours), (Some(true), Some(24)), "the switch is left where it was");

    let too_long = pb::WorkspaceSetSettings { pr_nudge: Some(false), pr_nudge_hours: Some(100_000), ..Default::default() };
    assert!(matches!(
        set_settings(&svc, &watcher, id, &too_long, Scope::Control, crate::workspace_ops::LOCAL).await,
        Err(DomainError::InvalidArgument { what: "pr_nudge_hours" })
    ));
    let still = list(&svc, Some(repo), Scope::Control).unwrap().items.remove(0);
    assert_eq!(still.pr_nudge, Some(true), "a refusal changes nothing");
}
