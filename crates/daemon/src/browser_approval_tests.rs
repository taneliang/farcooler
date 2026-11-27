//! A browser approving another browser, or removing a device (ov-1490):
//! `client.enroll` and `client.revoke` signed with the passkey on that
//! browser's own line, and nothing else that way.
//! And every approval and removal in the runner's audit log.

use base64ct::{Base64, Encoding};
use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};

use super::tests::{Device, Runner, enroll_req, key, lines, nonce, revoke_req, runner, word};
use super::{ALG_DEVICE, ALG_PASSKEY_LINE};
use crate::rpc_people::tests::{call, factory};

const UP: u8 = 0x01;
const UP_UV: u8 = 0x05;

/// This runner offering `passkey_keys`, for as long as the guard lives.
async fn offering() -> tokio::sync::MutexGuard<'static, ()> {
    let turn = crate::sshd_version::OFFER_TESTS.lock().await;
    farcooler_protocol::capability::offer(farcooler_protocol::capability::PASSKEY_KEYS);
    turn
}

/// The OpenSSH line of a passkey whose private half is `device`'s.
fn passkey(device: &Device) -> String {
    let mut blob = Vec::new();
    for field in [&b"sk-ecdsa-sha2-nistp256@openssh.com"[..], b"nistp256", &device.public(), farcooler_fence::app_host().as_bytes()] {
        blob.extend_from_slice(&(field.len() as u32).to_be_bytes());
        blob.extend_from_slice(field);
    }
    format!("sk-ecdsa-sha2-nistp256@openssh.com {} webauthn@browser", Base64::encode_string(&blob))
}

/// `id`'s line made `device`'s passkey line at `control`, as an enrollment of
/// that browser writes it.
fn browser_line(r: &Runner, id: &str, device: &Device) {
    let line = farcooler_fence::render(&passkey(device), "Browser", id, Scope::Control, farcooler_fence::Grant::FarCooler, None).unwrap();
    let path = r.svc.authorized_keys();
    let kept: Vec<String> = farcooler_fence::read(path, farcooler_fence::AUTHORIZED_KEYS).unwrap().into_iter().map(|e| if e.client_id == id { line.clone() } else { e.line }).collect();
    farcooler_fence::write(path, farcooler_fence::AUTHORIZED_KEYS, &kept, &[], farcooler_fence::Placement::Last).unwrap();
}

fn origin() -> String {
    format!("https://{}", farcooler_fence::app_host())
}

/// An enroll of another browser's passkey at `scope`.
fn approve(id: &str, key: &str, scope: Scope) -> Request {
    let mut req = enroll_req(1, id, scope, false, None);
    let Some(request::Payload::ClientEnroll(p)) = req.payload.as_mut() else { unreachable!() };
    p.public_key = key.to_string();
    req
}

/// `req` as `device`'s passkey signs it for the browser approval route.
fn signed(device: &Device, pin: &str, req: Request, nonce: &[u8], flags: u8, origin: &str) -> Request {
    let mut req = device.assert(pin, req, nonce, flags, 0, origin);
    // The same assertion under the route's name: the signature covers the
    // statement, which doesn't name an algorithm.
    req.step_up.as_mut().unwrap().alg = ALG_PASSKEY_LINE.into();
    req
}

fn events(r: &Runner) -> Vec<(String, String)> {
    r.svc.store.people_events(50).unwrap().into_iter().filter(|e| e.kind.starts_with("device_")).map(|e| (e.kind, e.detail)).collect()
}

#[tokio::test]
async fn a_browser_approves_another_browser_with_its_own_passkey_and_it_is_logged() {
    let _offer = offering().await;
    let r = runner().await;
    let web = Device::new("web-o", 15);
    browser_line(&r, "web-o", &web);
    let conn = r.conn("web-o");
    let new = passkey(&Device::new("new-o", 16));
    let n = nonce(&conn).await;
    let req = signed(&web, &r.pin, approve("new-o", &new, Scope::Control), &n, UP_UV, &origin());
    let result::Value::ClientEnroll(done) = call(&conn, req.clone()).await.expect("an approval with a verified passkey") else { panic!() };
    assert!(!done.already_enrolled);
    assert!(lines(&r).contains(&"new-o".to_string()));
    assert_eq!(r.svc.store.added_by_of("new-o").unwrap().unwrap().0, "web-o");
    let logged = events(&r);
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0].0, "device_added");
    assert!(logged[0].1.contains("\"new-o\"") && logged[0].1.contains("\"by\":\"web-o\""), "{}", logged[0].1);

    // The same assertion again: its challenge is spent.
    let again = word(call(&conn, req).await);
    assert!(again.ends_with("challenge_expired"), "{again}");

    // With no signature a `control` browser removes nothing.
    let bare = word(call(&conn, revoke_req("new-o", false)).await);
    assert!(bare.starts_with("scope-denied"), "{bare}");
    // And removed again, from the browser, at once and in the log.
    let n = nonce(&conn).await;
    call(&conn, signed(&web, &r.pin, revoke_req("new-o", false), &n, UP_UV, &origin())).await.expect("a browser removes a device");
    assert!(!lines(&r).contains(&"new-o".to_string()));
    let logged = events(&r);
    assert_eq!(logged[0].0, "device_removed", "{logged:?}");
    assert!(logged[0].1.contains("\"new-o\""), "{}", logged[0].1);
}

#[tokio::test]
async fn an_approval_needs_the_person_the_apps_origin_and_this_browsers_own_passkey() {
    let r = runner().await;
    let web = Device::new("web-o", 15);
    browser_line(&r, "web-o", &web);
    let conn = r.conn("web-o");
    let new = passkey(&Device::new("new-o", 16));
    let before = lines(&r);
    let other = Device::new("web-o", 17);
    for (what, signer, flags, from) in [
        ("a touch with nobody verified", &web, UP, origin()),
        ("another page", &web, UP_UV, "https://evil.example".to_string()),
        ("another passkey", &other, UP_UV, origin()),
    ] {
        let n = nonce(&conn).await;
        let refused = word(call(&conn, signed(signer, &r.pin, approve("new-o", &new, Scope::Control), &n, flags, &from)).await);
        assert!(refused.ends_with("step_up_failed"), "{what}: {refused}");
    }
    // Signed for one device, sent for another.
    let n = nonce(&conn).await;
    let mut swapped = signed(&web, &r.pin, approve("new-o", &new, Scope::Control), &n, UP_UV, &origin());
    let Some(request::Payload::ClientEnroll(p)) = swapped.payload.as_mut() else { unreachable!() };
    p.public_key = passkey(&Device::new("new-o", 18));
    let refused = word(call(&conn, swapped).await);
    assert!(refused.ends_with("step_up_failed"), "{refused}");
    // With no step-up at all, as before.
    let refused = word(call(&conn, approve("new-o", &new, Scope::Control)).await);
    assert!(refused.ends_with("defensive_only"), "{refused}");
    // A challenge this connection never got.
    let refused = word(call(&conn, signed(&web, &r.pin, approve("new-o", &new, Scope::Control), &[7u8; 32], UP_UV, &origin())).await);
    assert!(refused.ends_with("challenge_expired"), "{refused}");
    assert_eq!(lines(&r), before, "nothing was written");
    assert!(events(&r).is_empty());
}

#[tokio::test]
async fn a_browsers_approval_adds_only_a_browser_at_control_or_below() {
    let r = runner().await;
    let web = Device::new("web-o", 15);
    browser_line(&r, "web-o", &web);
    let conn = r.conn("web-o");
    let new = passkey(&Device::new("new-o", 16));
    let before = lines(&r);
    let mut shell = approve("new-o", &new, Scope::Control);
    let mut teammate = approve("new-o", &new, Scope::Control);
    let mut with_key = approve("new-o", &new, Scope::Control);
    for (req, change) in [(&mut shell, 0), (&mut teammate, 1), (&mut with_key, 2)] {
        let Some(request::Payload::ClientEnroll(p)) = req.payload.as_mut() else { unreachable!() };
        match change {
            0 => p.shell_access = true,
            1 => p.for_teammate = true,
            _ => p.step_up_key = Some(Device::new("new-o", 16).wire_key(ALG_DEVICE)),
        }
    }
    for (what, req) in [
        ("a device that isn't a browser", approve("new-o", &key(70), Scope::Control)),
        ("host_admin", approve("new-o", &new, Scope::HostAdmin)),
        ("a shell line", shell),
        ("a teammate's device", teammate),
        ("a step-up key", with_key),
    ] {
        let n = nonce(&conn).await;
        let refused = word(call(&conn, signed(&web, &r.pin, req, &n, UP_UV, &origin())).await);
        assert!(refused.ends_with("defensive_only") || refused.ends_with("socket_only"), "{what}: {refused}");
    }
    assert_eq!(lines(&r), before, "nothing was written");
}

/// The route is approving and removing and nothing else, a browser's passkey line and
/// no other device's, and the owner's or an Admin's.
#[tokio::test]
async fn the_passkey_route_serves_no_other_method_no_other_line_and_no_member() {
    let r = runner().await;
    let web = Device::new("web-o", 15);
    browser_line(&r, "web-o", &web);
    let conn = r.conn("web-o");
    let n = nonce(&conn).await;
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "carol".into() })),
        ..Default::default()
    };
    let refused = word(call(&conn, signed(&web, &r.pin, remove, &n, UP_UV, &origin())).await);
    assert!(refused.ends_with("defensive_only"), "{refused}");

    // A phone's line holds no passkey.
    let new = passkey(&Device::new("new-o", 16));
    let phone = Device::new("phone-o", 21);
    let conn = r.conn("phone-o");
    let n = nonce(&conn).await;
    let refused = word(call(&conn, signed(&phone, &r.pin, approve("new-o", &new, Scope::Control), &n, UP_UV, &origin())).await);
    assert!(refused.ends_with("key_untrusted"), "{refused}");

    // Bob is a Member: his browser approves nothing this way (ov-778 is open).
    let bobs = Device::new("phone-b", 22);
    browser_line(&r, "phone-b", &bobs);
    let conn = r.conn("phone-b");
    let n = nonce(&conn).await;
    let refused = word(call(&conn, signed(&bobs, &r.pin, approve("new-b", &new, Scope::Control), &n, UP_UV, &origin())).await);
    assert!(refused.ends_with("not_admin"), "{refused}");
    assert!(!lines(&r).iter().any(|l| l.starts_with("new-")));
}

/// Recovery with no enrolled device left is the runner's own socket, from a
/// person's terminal: no connection over SSH, and no pane, gets it.
#[tokio::test]
async fn with_every_device_gone_the_runners_socket_enrolls_a_browser_and_logs_it() {
    let _offer = offering().await;
    let r = runner().await;
    let socket = r.socket();
    for device in lines(&r) {
        call(&socket, revoke_req(&device, false)).await.unwrap_or_else(|e| panic!("{device}: {e}"));
    }
    assert!(lines(&r).is_empty());
    let new = passkey(&Device::new("new-o", 16));
    // A device that was removed has no line to say who it is.
    let gone = factory(&r.svc, Some("web-o"), Scope::Control);
    let refused = word(call(&gone, approve("new-o", &new, Scope::Control)).await);
    assert!(refused != "(served)", "{refused}");
    assert!(lines(&r).is_empty());
    call(&socket, approve("new-o", &new, Scope::Control)).await.expect("the runner's own socket");
    assert_eq!(lines(&r), vec!["new-o".to_string()]);
    let added = events(&r).into_iter().find(|(kind, _)| kind == "device_added").expect("logged");
    assert!(added.1.contains("\"by\":\"\""), "{}", added.1);
}
