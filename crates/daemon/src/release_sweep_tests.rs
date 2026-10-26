//! The released devices' sweep (ov-783): a revoke that fails once, because
//! `authorized_keys` couldn't be written, is finished by the next
//! `member.set` and by the daemon's start, and nothing that isn't released
//! is touched along the way.

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope};

use super::*;
use crate::rpc_people::tests::{call, enroll, factory, set, with_bobs_phone_enrolled};
use crate::sessions::Session;

/// Bob with his phone and pad, Alice with hers, the owner's own phone, and a
/// pad Alice's device enrolled that's waiting on its first binding, each
/// with an open session; then Bob's pad taken off his list while `.ssh`
/// can't be written, so its revoke fails.
struct Failed {
    _dir: tempfile::TempDir,
    svc: Arc<Service>,
    sessions: Vec<(&'static str, Arc<Session>)>,
}

async fn a_failed_revoke() -> Failed {
    let (dir, svc) = with_bobs_phone_enrolled().await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi pad", "pad-b").await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz alice", "phone-a").await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIERERERERERERERERERERERERERERERERERERERERERE owner", "phone-o").await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b", "pad-b"], "", "")).await.expect("Bob bound");
    call(&admin, set("alice", &["phone-a"], "", "")).await.expect("Alice bound");
    // Enrolled by Alice's device, as `client.enroll` from it records it.
    svc.store.record_member_enrolled("pad-new", "alice").unwrap();
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV new", "pad-new").await;
    let sessions = ["pad-b", "phone-b", "phone-a", "phone-o", "pad-new"]
        .into_iter()
        .map(|d| (d, svc.sessions().open(Some(d.to_string()))))
        .collect();

    let ssh = svc.authorized_keys().parent().unwrap().to_path_buf();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o500)).unwrap();
    let refused = call(&admin, set("bob", &["phone-b"], "", "")).await;
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(refused.is_err(), "a revoke that couldn't write was answered as done: {refused:?}");
    assert_eq!(svc.store.member("bob").unwrap().devices, ["phone-b".to_string()], "the list is changed all the same");
    assert!(lines(&svc).contains("pad-b"), "the write failed, so the line is still there");
    let log = svc.store.people_events(1).unwrap();
    assert_eq!((log[0].kind.as_str(), log[0].handle.as_str(), log[0].detail.as_str()), ("revoke_failed", "bob", r#"{"device":"pad-b"}"#));
    Failed { _dir: dir, svc, sessions }
}

fn lines(svc: &Service) -> String {
    std::fs::read_to_string(svc.authorized_keys()).unwrap()
}

/// Bob's pad is cut off, its line gone and its session closed; every other
/// device, the one waiting on its binding included, keeps both.
fn only_the_pad_is_cut_off(failed: &Failed) {
    let keys = lines(&failed.svc);
    assert!(!keys.contains("pad-b"), "the pad still has a line:\n{keys}");
    for (device, session) in &failed.sessions {
        if *device == "pad-b" {
            assert!(session.is_closed(), "the pad's session still runs");
        } else {
            assert!(keys.contains(device), "{device} lost its line:\n{keys}");
            assert!(!session.is_closed(), "{device}'s session was closed");
        }
    }
}

/// The next `member.set`, for anyone, finishes what the failed one left,
/// and logs it under its caller.
#[tokio::test]
async fn the_next_member_set_finishes_a_failed_revoke() {
    let failed = a_failed_revoke().await;
    let admin = factory(&failed.svc, None, Scope::HostAdmin);
    call(&admin, set("alice", &["phone-a"], "", "")).await.expect("Alice set again, as she is");
    only_the_pad_is_cut_off(&failed);
    let log = failed.svc.store.people_events(1).unwrap();
    assert_eq!((log[0].kind.as_str(), log[0].actor.as_str(), log[0].detail.as_str()), ("revoked", "user", r#"{"device":"pad-b"}"#));
}

/// So does the daemon's start, as the runner's own.
#[tokio::test]
async fn the_sweep_at_start_finishes_a_failed_revoke() {
    let failed = a_failed_revoke().await;
    sweep_at_start(&failed.svc).await;
    only_the_pad_is_cut_off(&failed);
    let log = failed.svc.store.people_events(1).unwrap();
    assert_eq!((log[0].kind.as_str(), log[0].actor.as_str(), log[0].detail.as_str()), ("revoked", RUNNER, r#"{"device":"pad-b"}"#));
    // Nothing left: the pad's connection ended with its session closed.
    drop(failed.sessions);
    let again = sweep(&failed.svc).await.unwrap();
    assert!(again.revoked.is_empty() && again.closed.is_empty() && again.failed.is_empty(), "{again:?}");
}

/// A released device whose line is gone but whose session is still open
/// (a line taken out by hand) has the session closed.
#[tokio::test]
async fn a_released_device_with_no_line_has_its_sessions_closed() {
    let (_dir, svc) = with_bobs_phone_enrolled().await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b"], "", "")).await.expect("Bob bound");
    crate::enrollment::revoke(&svc, &pb::ClientRevoke { client_id: "phone-b".into() }).await.unwrap();
    let phone = svc.sessions().open(Some("phone-b".into()));
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    let swept = sweep(&svc).await.unwrap();
    assert_eq!(swept.closed, ["phone-b".to_string()]);
    assert!(phone.is_closed());
}

/// Moving a device from one member to another is enrolling it again: taken
/// off Bob, it's revoked, so binding it to Alice is refused as a device this
/// runner didn't enroll.
#[tokio::test]
async fn a_device_taken_off_one_member_cant_be_given_to_another() {
    let (_dir, svc) = with_bobs_phone_enrolled().await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b"], "", "")).await.expect("Bob bound");
    call(&admin, set("bob", &[], "", "")).await.expect("his phone taken off");
    let moved = call(&admin, set("alice", &["phone-b"], "", "")).await;
    assert_eq!(moved.err().as_deref(), Some("invalid-argument:device"));
}
