//! Add Teammate in one write (ov-856): a person, an access and a device, bound
//! under one fence lock hold and one step-up. Every test goes through `Rpc`
//! the way a phone's connection does.

use farcooler_protocol::v1::{self as pb, Request, Scope, request};
use farcooler_store::access::{Access, Cap};

use crate::rpc_people::tests::call;
use crate::step_up::tests::{Device, Runner, asking, lines, nonce, runner, word};

fn teammate(handle: &str, account: &str, access: &str) -> pb::TeammateBind {
    pb::TeammateBind { handle: handle.into(), name: "Erin".into(), account: account.into(), email: String::new(), access: access.into() }
}

fn add_req(n: u8, id: &str, scope: Scope, bind: pb::TeammateBind) -> Request {
    let p = pb::ClientEnroll { for_teammate: true, teammate: Some(bind), ..asking(n, id, scope) };
    Request { method: "client.enroll".into(), payload: Some(request::Payload::ClientEnroll(p)), ..Default::default() }
}

/// `phone` signs `req` for one nonce, as its Secure Enclave would.
async fn signed(conn: &crate::rpc::RpcFactory, r: &Runner, phone: &Device, req: Request) -> Request {
    let n = nonce(conn).await;
    phone.step(&r.pin, req, &n)
}

fn owner_phone(r: &Runner) -> Device {
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, crate::step_up::ALG_DEVICE);
    phone
}

fn dan_phone(r: &Runner) -> Device {
    let phone = Device::new("phone-d", 12);
    r.trust(&phone, crate::step_up::ALG_DEVICE);
    phone
}

fn access_of(r: &Runner, handle: &str) -> Option<Access> {
    r.svc.store.member_access(handle).unwrap()
}

/// The owner's phone adds Erin as a Member: the line is written, the device is
/// Erin's and nobody else's, and she has the access that was asked for.
#[tokio::test]
async fn the_owners_phone_adds_a_person_with_an_access_in_one_signed_write() {
    let r = runner().await;
    let phone = owner_phone(&r);
    let conn = r.conn("phone-o");
    let req = add_req(40, "erin-phone", Scope::Control, teammate("erin", "acct-erin", "member"));
    call(&conn, signed(&conn, &r, &phone, req).await).await.expect("a signed Add Teammate");
    assert!(lines(&r).contains(&"erin-phone".to_string()), "the line is written");
    let erin = r.svc.store.member("erin").unwrap();
    assert_eq!(erin.devices, ["erin-phone"], "bound to her in the same write");
    assert_eq!(access_of(&r, "erin"), Some(Access::Member));
    assert_eq!(r.svc.store.member_of_device("erin-phone").unwrap().as_deref(), Some("erin"));
    assert!(!r.svc.store.device_is_owners("erin-phone").unwrap(), "never the owner's, not for a moment");
}

/// A Viewer is added on a read line, with no cap to carry; a Member on a read
/// line is held at Viewer by a cap.
#[tokio::test]
async fn the_line_fits_the_access() {
    let r = runner().await;
    let phone = owner_phone(&r);
    let conn = r.conn("phone-o");
    call(&conn, signed(&conn, &r, &phone, add_req(41, "vera-tv", Scope::Read, teammate("vera", "acct-vera", "viewer"))).await).await.expect("a viewer");
    assert_eq!(access_of(&r, "vera"), Some(Access::Viewer));
    assert_eq!(r.svc.store.device_cap("vera-tv").unwrap(), None);

    let wide = add_req(42, "vera-wide", Scope::Control, teammate("vera2", "acct-vera2", "viewer"));
    let refused = word(call(&conn, signed(&conn, &r, &phone, wide).await).await);
    assert!(refused.ends_with("scope_access"), "{refused}: a control line for a Viewer");
    assert!(!lines(&r).contains(&"vera-wide".to_string()), "nothing was written");
    assert!(r.svc.store.member("vera2").is_err(), "and nobody was added");

    call(&conn, signed(&conn, &r, &phone, add_req(43, "max-tv", Scope::Read, teammate("max", "acct-max", "member"))).await).await.expect("a member on a read line");
    assert_eq!(r.svc.store.device_cap("max-tv").unwrap(), Some(Cap::Viewer));
}

/// Admin is the owner's to give. The owner's phone gives it; an Admin's does
/// not, whatever the signature says, and nothing is written.
#[tokio::test]
async fn only_the_owner_makes_an_admin() {
    let r = runner().await;
    let owner = owner_phone(&r);
    let conn = r.conn("phone-o");
    call(&conn, signed(&conn, &r, &owner, add_req(44, "ann-phone", Scope::Control, teammate("ann", "acct-ann", "admin"))).await)
        .await
        .expect("the owner makes an Admin");
    assert_eq!(access_of(&r, "ann"), Some(Access::Admin));

    let dan = dan_phone(&r);
    let conn = r.conn("phone-d");
    let refused = word(call(&conn, signed(&conn, &r, &dan, add_req(45, "cy-phone", Scope::Control, teammate("cy", "acct-cy", "admin"))).await).await);
    assert!(refused.ends_with("admin_is_owners"), "{refused}");
    assert!(!lines(&r).contains(&"cy-phone".to_string()), "no line");
    assert!(r.svc.store.member("cy").is_err(), "no person");
    // An Admin does add a Member.
    call(&conn, signed(&conn, &r, &dan, add_req(46, "di-phone", Scope::Control, teammate("di", "acct-di", "member"))).await).await.expect("an Admin adds a Member");
    assert_eq!(access_of(&r, "di"), Some(Access::Member));
}

/// A Member and a Viewer add nobody, signed or not.
#[tokio::test]
async fn a_member_or_a_viewer_cannot_add_a_teammate() {
    let r = runner().await;
    let bob = Device::new("phone-b", 13);
    r.trust(&bob, crate::step_up::ALG_DEVICE);
    let conn = r.conn("phone-b");
    let refused = word(call(&conn, signed(&conn, &r, &bob, add_req(47, "x-phone", Scope::Control, teammate("x", "acct-x", "member"))).await).await);
    assert!(refused.contains("not_owners_device") || refused.contains("not_admin"), "{refused}");
    assert!(!lines(&r).contains(&"x-phone".to_string()));
}

/// The step-up is for that person, that access and that device: a signature
/// over one Add Teammate carries no other. Each swap is refused and writes
/// nothing.
#[tokio::test]
async fn the_step_up_is_bound_to_the_person_the_access_and_the_device() {
    let r = runner().await;
    let phone = owner_phone(&r);
    let conn = r.conn("phone-o");
    let base = add_req(48, "gus-phone", Scope::Control, teammate("gus", "acct-gus", "member"));
    let with = |change: &dyn Fn(&mut pb::ClientEnroll)| {
        let mut req = base.clone();
        let Some(request::Payload::ClientEnroll(p)) = req.payload.as_mut() else { unreachable!() };
        change(p);
        req.payload
    };
    let swaps = [
        ("the access", with(&|p| p.teammate.as_mut().unwrap().access = "admin".into())),
        ("the person", with(&|p| p.teammate.as_mut().unwrap().handle = "mallory".into())),
        ("the account", with(&|p| p.teammate.as_mut().unwrap().account = "acct-mallory".into())),
        ("the device", with(&|p| p.client_id = "other-phone".into())),
        ("the key", with(&|p| p.public_key = crate::step_up::tests::key(99))),
        ("no person at all", with(&|p| p.teammate = None)),
    ];
    for (what, payload) in swaps {
        let mut req = signed(&conn, &r, &phone, base.clone()).await;
        req.payload = payload;
        let refused = word(call(&conn, req).await);
        assert!(refused.ends_with("step_up_failed"), "{what}: {refused}");
    }
    assert!(r.svc.store.member("gus").is_err() && r.svc.store.member("mallory").is_err());
    assert!(!lines(&r).contains(&"gus-phone".to_string()) && !lines(&r).contains(&"other-phone".to_string()));

    // And another device can't use the owner phone's signature.
    let ipad = Device::new("pad-d", 14);
    r.trust(&ipad, crate::step_up::ALG_DEVICE);
    let theirs = signed(&conn, &r, &phone, base.clone()).await;
    let refused = word(call(&r.conn("pad-d"), theirs).await);
    assert!(refused.contains("challenge_expired") || refused.ends_with("step_up_failed"), "{refused}");
    assert!(r.svc.store.member("gus").is_err());
}

/// A device that stepped up for nothing, or whose key isn't asked for yet,
/// still doesn't add a person without a signature over this request.
#[tokio::test]
async fn adding_a_person_needs_a_signature_over_this_request() {
    let r = runner().await;
    // Keyless: phone-o has no trusted key, so the old rule would let the
    // plain call through; Add Teammate with a person is stricter.
    let plain = word(call(&r.conn("phone-o"), add_req(49, "hal-phone", Scope::Control, teammate("hal", "acct-hal", "member"))).await);
    assert!(plain.ends_with("step_up_needed"), "{plain}");
    assert!(r.svc.store.member("hal").is_err() && !lines(&r).contains(&"hal-phone".to_string()));
    // The runner's own socket needs none.
    call(&r.socket(), add_req(50, "hal-phone", Scope::Control, teammate("hal", "acct-hal", "member"))).await.expect("the socket");
    assert_eq!(access_of(&r, "hal"), Some(Access::Member));
}

/// A person who is here keeps their access, their name and their devices; the
/// handle is theirs only with their account; an Admin leaves another Admin alone.
#[tokio::test]
async fn a_person_who_is_here_keeps_what_they_have() {
    let r = runner().await;
    let phone = owner_phone(&r);
    let conn = r.conn("phone-o");
    // Bob is a Member with an account.
    r.svc.store.set_member_in("bob", "Bob", &["phone-b".into(), "pad-b".into()], farcooler_store::people::Directory { account: "acct-bob", email: "" }).unwrap();

    let other_access = add_req(51, "bob-3", Scope::Read, teammate("bob", "acct-bob", "viewer"));
    let refused = word(call(&conn, signed(&conn, &r, &phone, other_access).await).await);
    assert!(refused.ends_with("access_exists"), "{refused}");
    assert!(!lines(&r).contains(&"bob-3".to_string()));

    let stranger = add_req(52, "bob-4", Scope::Control, teammate("bob", "acct-someone-else", "member"));
    let refused = word(call(&conn, signed(&conn, &r, &phone, stranger).await).await);
    assert!(refused.ends_with("handle_taken"), "{refused}");
    assert_eq!(r.svc.store.member("bob").unwrap().name, "Bob", "not renamed");

    call(&conn, signed(&conn, &r, &phone, add_req(53, "bob-5", Scope::Control, teammate("bob", "acct-bob", ""))).await).await.expect("another device for Bob");
    let bob = r.svc.store.member("bob").unwrap();
    assert_eq!(bob.devices, ["bob-5", "pad-b", "phone-b"], "his other devices stay");
    assert_eq!(bob.name, "Bob");
    assert_eq!(access_of(&r, "bob"), Some(Access::Member));

    // Dan (an Admin) can't add a device to another Admin.
    crate::person_access::set_access(&r.svc, "bob", Access::Admin, || ()).await.unwrap();
    let dan = dan_phone(&r);
    let conn = r.conn("phone-d");
    let refused = word(call(&conn, signed(&conn, &r, &dan, add_req(54, "bob-6", Scope::Control, teammate("bob", "acct-bob", ""))).await).await);
    assert!(refused.ends_with("not_yours"), "{refused}");
    assert!(!lines(&r).contains(&"bob-6".to_string()));
}

/// A teammate request that wasn't `for_teammate`, or that asks for a shell or
/// `host_admin`, is refused.
#[tokio::test]
async fn it_never_writes_a_shell_or_a_host_admin_line() {
    let r = runner().await;
    let phone = owner_phone(&r);
    let conn = r.conn("phone-o");
    let mut shell = add_req(55, "sh-phone", Scope::HostAdmin, teammate("sh", "acct-sh", "member"));
    let Some(request::Payload::ClientEnroll(p)) = shell.payload.as_mut() else { unreachable!() };
    p.shell_access = true;
    assert!(call(&conn, signed(&conn, &r, &phone, shell).await).await.is_err());
    let wide = add_req(56, "wide-phone", Scope::HostAdmin, teammate("wide", "acct-wide", "member"));
    assert!(call(&conn, signed(&conn, &r, &phone, wide).await).await.is_err());
    let mut plain = add_req(57, "pl-phone", Scope::Control, teammate("pl", "acct-pl", "member"));
    let Some(request::Payload::ClientEnroll(p)) = plain.payload.as_mut() else { unreachable!() };
    p.for_teammate = false;
    assert!(word(call(&conn, signed(&conn, &r, &phone, plain).await).await).ends_with("teammate"));
    for id in ["sh-phone", "wide-phone", "pl-phone"] {
        assert!(!lines(&r).contains(&id.to_string()), "{id}");
    }
}
