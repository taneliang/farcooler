//! Who added each device (ov-838): the record, the one-step revoke of what
//! a device added, the daily limit, and who may start the tunnel.

use std::sync::Arc;

use farcooler_core::DomainError;
use farcooler_protocol::v1::{ClientEnroll, ClientRevoke, Scope};

use super::Caller;
use crate::service::Service;
use crate::tunnel_gate::recording::Recording;

/// A distinct ed25519 key for each `n`.
fn key(n: u8) -> String {
    let key = ssh_key::PublicKey::from(ssh_key::public::Ed25519PublicKey([n; 32]));
    key.to_openssh().unwrap()
}

const NODE: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

async fn runner() -> (tempfile::TempDir, Arc<Service>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    (dir, svc)
}

fn asking(n: u8, client_id: &str, scope: Scope) -> ClientEnroll {
    ClientEnroll { public_key: key(n), label: "A device".into(), client_id: client_id.into(), scope: scope as i32, ..Default::default() }
}

/// `client_id`'s session, at `scope`.
fn as_device(svc: &Service, client_id: &str, scope: Scope) -> Caller {
    Caller::new(svc.sessions().open(Some(client_id.into())), scope)
}

async fn add(svc: &Service, by: &Caller, n: u8, client_id: &str) -> farcooler_core::Result<farcooler_protocol::v1::ClientEnrollResult> {
    super::super::enroll_as(svc, &asking(n, client_id, Scope::Control), None, Some(by.clone())).await
}

async fn lines(svc: &Service) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> =
        super::super::list(svc).await.unwrap().items.into_iter().map(|e| (e.client_id, e.added_by)).collect();
    lines.sort();
    lines
}

/// The phone the owner's Mac paired, a browser the phone added, a tablet the
/// browser added, and a laptop the Mac added. Each line names its enroller;
/// revoking the phone alone leaves what it added, named by `added_by`, and
/// `with_added` takes the phone and all it added in one step.
#[tokio::test]
async fn each_device_names_its_enroller_and_a_revoke_takes_what_it_added() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    super::super::enroll(&svc, &asking(2, "laptop", Scope::Control)).await.unwrap();
    add(&svc, &as_device(&svc, "phone", Scope::Control), 3, "browser").await.unwrap();
    add(&svc, &as_device(&svc, "browser", Scope::Control), 4, "tablet").await.unwrap();
    let named = |pairs: &[(&str, &str)]| pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    assert_eq!(lines(&svc).await, named(&[("browser", "phone"), ("laptop", ""), ("phone", ""), ("tablet", "browser")]));

    let rows = svc.store.added_by_all().unwrap();
    assert_eq!(farcooler_store::device_added_by::added_under(&rows, "phone"), vec!["browser", "tablet"], "the phone's descendants");

    super::super::revoke(&svc, &ClientRevoke { client_id: "phone".into(), with_added: true }).await.unwrap();
    assert_eq!(lines(&svc).await, named(&[("laptop", "")]), "the revoke left what the phone added");
    for gone in ["phone", "browser", "tablet"] {
        assert!(svc.sessions().open(Some(gone.into())).is_closed(), "{gone} wasn't marked revoked");
    }
}

/// Without `with_added`, only the device goes, as before.
#[tokio::test]
async fn a_plain_revoke_takes_the_one_device() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    add(&svc, &as_device(&svc, "phone", Scope::Control), 3, "browser").await.unwrap();
    super::super::revoke(&svc, &ClientRevoke { client_id: "phone".into(), with_added: false }).await.unwrap();
    assert_eq!(lines(&svc).await, vec![("browser".to_string(), "phone".to_string())]);
}

/// What a member's device revokes along with another spares the owner's.
#[tokio::test]
async fn a_members_revoke_spares_the_owners_devices() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone-b", Scope::Control)).await.unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    add(&svc, &as_device(&svc, "phone-b", Scope::Control), 3, "browser-b").await.unwrap();
    svc.store.record_member_device_added("browser-b", "bob").unwrap();
    super::super::enroll(&svc, &asking(4, "owners-tablet", Scope::Control)).await.unwrap();
    // Recorded as the phone's, as a line from before Bob was bound would be.
    svc.store.record_added_by("owners-tablet", "phone-b").unwrap();
    assert!(svc.store.device_is_owners("owners-tablet").unwrap());

    let bob = crate::people::Person::Member("bob".into());
    super::revoke_as(&svc, &ClientRevoke { client_id: "phone-b".into(), with_added: true }, &bob).await.unwrap();
    let left: Vec<String> = lines(&svc).await.into_iter().map(|(id, _)| id).collect();
    assert_eq!(left, vec!["owners-tablet".to_string()], "a member's revoke took the owner's device, or left Bob's");
}

/// N6 (ov-828): a device below `host_admin` adds `ADD_LIMIT` devices a day
/// and no more; `host_admin` isn't counted.
#[tokio::test]
async fn a_control_device_adds_so_many_a_day() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    let phone = as_device(&svc, "phone", Scope::Control);
    for n in 0..super::ADD_LIMIT as u8 {
        add(&svc, &phone, 10 + n, &format!("browser-{n}")).await.expect("within the limit");
    }
    let over = add(&svc, &phone, 30, "browser-over").await;
    assert!(matches!(over, Err(DomainError::Conflict { what: "add_limit" })), "past the limit: {over:?}");
    assert!(!lines(&svc).await.iter().any(|(id, _)| id == "browser-over"), "a line past the limit was written");
    let admin = as_device(&svc, "mac", Scope::HostAdmin);
    for n in 0..=super::ADD_LIMIT as u8 {
        add(&svc, &admin, 40 + n, &format!("by-mac-{n}")).await.expect("host_admin isn't limited");
    }
}

/// An id a revoke marked can't be taken by a device below `host_admin`:
/// a still-open ssh master under it would be served again.
#[tokio::test]
async fn a_control_device_cant_take_a_revoked_id() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    super::super::enroll(&svc, &asking(2, "old", Scope::Control)).await.unwrap();
    super::super::revoke(&svc, &ClientRevoke { client_id: "old".into(), with_added: false }).await.unwrap();
    let taken = add(&svc, &as_device(&svc, "phone", Scope::Control), 3, "old").await;
    assert!(matches!(taken, Err(DomainError::Conflict { what: "device_taken" })), "{taken:?}");
    super::super::enroll(&svc, &asking(3, "old", Scope::Control)).await.expect("the owner's socket may");
}

/// A device below `host_admin` can't enroll under an id that already has a
/// line: the owner's line and its `added_by` stay as they were.
#[tokio::test]
async fn a_control_device_cant_reenroll_an_existing_id() {
    let (_dir, svc) = runner().await;
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    // A shell-only line: the shape a control device's own enroll doesn't match.
    super::super::enroll(&svc, &ClientEnroll { shell_access: true, ..asking(2, "owners-mac", Scope::Control) }).await.unwrap();
    let before = lines(&svc).await;
    let taken = add(&svc, &as_device(&svc, "phone", Scope::Control), 3, "owners-mac").await;
    assert!(matches!(taken, Err(DomainError::Conflict { what: "device_taken" })), "{taken:?}");
    assert_eq!(lines(&svc).await, before, "the owner's line or its added_by changed");
}

/// The ruling on N5 (ov-828): a device below `host_admin` is admitted to a
/// tunnel that runs, and never starts one. `host_admin` does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_control_device_never_starts_the_tunnel() {
    let (_dir, svc) = runner().await;
    let tunnel = Arc::new(Recording::default());
    svc.tunnel.use_backend(tunnel.clone());
    super::super::enroll(&svc, &asking(1, "phone", Scope::Control)).await.unwrap();
    let browser = ClientEnroll { node_key: NODE.into(), ..asking(3, "browser", Scope::Control) };
    let paired = super::super::enroll_as(&svc, &browser, None, Some(as_device(&svc, "phone", Scope::Control))).await.unwrap();
    assert_eq!(paired.conn_blob, "", "a control device was handed a tunnel");
    assert_eq!(tunnel.admits(), None, "a control device started the tunnel");
    assert!(!svc.tailcat_key().exists(), "a control device minted a tunnel identity");

    let laptop = ClientEnroll { node_key: "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE".into(), ..asking(4, "laptop", Scope::Control) };
    let paired = super::super::enroll_as(&svc, &laptop, None, Some(as_device(&svc, "mac", Scope::HostAdmin))).await.unwrap();
    assert_eq!(paired.conn_blob, "token", "host_admin didn't start the tunnel");
}

/// R4 (the ov-828 re-review): a revoked teammate's pending row goes with
/// its line, so the sweep can take it if it's ever back.
#[tokio::test]
async fn a_revoke_clears_a_pending_teammate() {
    let (_dir, svc) = runner().await;
    let teammate = ClientEnroll { for_teammate: true, ..asking(1, "phone-t", Scope::Control) };
    let mark: super::super::MemberEnrolled = {
        let svc = svc.clone();
        Box::new(move |id: &str| svc.store.record_teammate_pending(id))
    };
    super::super::enroll_as(&svc, &teammate, Some(mark), None).await.unwrap();
    assert!(!svc.store.released_to_sweep().unwrap().contains(&"phone-t".to_string()));
    super::super::revoke(&svc, &ClientRevoke { client_id: "phone-t".into(), with_added: false }).await.unwrap();
    assert!(svc.store.released_to_sweep().unwrap().contains(&"phone-t".to_string()), "the pending row outlived the revoke");
}
