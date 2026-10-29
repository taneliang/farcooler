use std::collections::BTreeMap;
use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::access::{Access, Cap, OwnerDeviceState};

use super::*;

fn key(n: u8) -> String {
    use ssh_key::public::{Ed25519PublicKey, KeyData};
    ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([n; 32])), "x").to_openssh().unwrap()
}

const NODE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

async fn enroll(svc: &Service, n: u8, id: &str, scope: Scope, shell: bool, node: &str) {
    let request = pb::ClientEnroll {
        public_key: key(n),
        label: id.into(),
        client_id: id.into(),
        scope: scope as i32,
        shell_access: shell,
        node_key: node.into(),
        for_teammate: false,
        step_up_key: None,
        teammate: None,
    };
    crate::enrollment::enroll(svc, &request).await.expect("enrolled");
}

/// A runner shaped like the real ones:
///
/// - the owner has control phones and a browser, a node key on one, a
///   read-only iPad, a Mac whose line is `host_admin` with its shell line;
/// - Bob is a member with a control phone, and a read pad;
/// - Carol has only a read display;
/// - Dan is a member whose phone is `host_admin`;
/// - Eve was removed, so her control phone is released and keeps its line;
/// - a legacy line added by Bob's phone and never bound.
struct Runner {
    _dir: tempfile::TempDir,
    svc: Arc<Service>,
}

async fn runner() -> Runner {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    enroll(&svc, 1, "phone-o", Scope::Control, false, NODE).await;
    enroll(&svc, 2, "phone-o2", Scope::Control, false, "").await;
    enroll(&svc, 3, "browser-o", Scope::Control, false, "").await;
    enroll(&svc, 4, "ipad-o", Scope::Read, false, "").await;
    enroll(&svc, 5, "mac-o", Scope::HostAdmin, false, "").await;
    enroll(&svc, 6, "mac-o", Scope::HostAdmin, true, "").await;
    enroll(&svc, 7, "phone-b", Scope::Control, false, "").await;
    enroll(&svc, 8, "pad-b", Scope::Read, false, "").await;
    enroll(&svc, 9, "tv-c", Scope::Read, false, "").await;
    enroll(&svc, 10, "phone-d", Scope::HostAdmin, false, "").await;
    enroll(&svc, 11, "phone-e", Scope::Control, false, "").await;
    enroll(&svc, 12, "legacy-t", Scope::Control, false, "").await;
    let store = &svc.store;
    store.set_member("bob", "Bob", &["phone-b".into(), "pad-b".into()]).unwrap();
    store.set_member("carol", "Carol", &["tv-c".into()]).unwrap();
    store.set_member("dan", "Dan", &["phone-d".into()]).unwrap();
    store.set_member("eve", "Eve", &["phone-e".into()]).unwrap();
    store.set_member("eve", "Eve", &[]).unwrap();
    store.record_added_by("legacy-t", "phone-b").unwrap();
    store.record_added_by("ipad-o", "").unwrap();
    Runner { _dir: dir, svc }
}

fn entries(svc: &Service) -> Vec<Entry> {
    fence::read(svc.authorized_keys(), fence::AUTHORIZED_KEYS).unwrap()
}

fn migrate(svc: &Service) -> bool {
    match migrate_in(&svc.store, svc.authorized_keys()) {
        Ok(wrote) => wrote,
        Err(Skipped::Breaks(device)) => panic!("the plan would change {device}"),
        Err(_) => panic!("the migration failed"),
    }
}

fn lines_by_device(entries: &[Entry]) -> BTreeMap<String, Scope> {
    entries.iter().filter(|e| key_a(e)).map(|e| (e.client_id.clone(), e.scope)).collect()
}

/// The migration, on shapes like the real ones: who is whose, each
/// member's access, and a cap for exactly the devices below their person,
/// the owner's read-only iPad among them.
#[tokio::test]
async fn the_migration_binds_and_caps_every_device_below_its_person() {
    let r = runner().await;
    assert!(migrate(&r.svc), "the first run writes");
    let stored = r.svc.store.stored_access().unwrap();
    use OwnerDeviceState::{Check, Confirmed};
    assert_eq!(
        stored.owner_devices,
        vec![
            ("browser-o".into(), Confirmed),
            ("ipad-o".into(), Confirmed),
            ("legacy-t".into(), Check),
            ("mac-o".into(), Confirmed),
            ("phone-o".into(), Confirmed),
            ("phone-o2".into(), Confirmed),
        ],
        "the owner's, with a teammate's old line to check; Eve's released phone is nobody's"
    );
    assert_eq!(
        stored.member_access,
        vec![
            ("bob".into(), Access::Member),
            ("carol".into(), Access::Viewer),
            ("dan".into(), Access::Admin),
            ("eve".into(), Access::Member),
        ]
    );
    assert_eq!(
        stored.caps,
        vec![
            ("browser-o".into(), Cap::Member),
            ("ipad-o".into(), Cap::Viewer),
            ("legacy-t".into(), Cap::Member),
            ("pad-b".into(), Cap::Viewer),
            ("phone-o".into(), Cap::Member),
            ("phone-o2".into(), Cap::Member),
        ],
        "a cap wherever a line is below its person's ceiling; none for a viewer's read line, an admin's, or the released phone"
    );
    assert!(stored.step_up_keys.is_empty(), "every device starts with no step-up key");
}

/// B1: after the migration, a rebuild of every line from the store alone
/// writes exactly today's lines: the same bytes, node key and shell line
/// and all. Nothing is raised, nothing lowered.
#[tokio::test]
async fn a_rebuild_after_the_migration_writes_exactly_todays_lines() {
    let r = runner().await;
    let before = entries(&r.svc);
    let file_before = std::fs::read(r.svc.authorized_keys()).unwrap();
    migrate(&r.svc);
    assert_eq!(std::fs::read(r.svc.authorized_keys()).unwrap(), file_before, "the migration wrote no line");
    let view = View::from_store(&r.svc.store).unwrap();
    let rebuilt = rebuild(&before, &view, &BTreeSet::new()).unwrap();
    let today: Vec<String> = before.iter().map(|e| e.line.clone()).collect();
    assert_eq!(rebuilt, today);
    assert!(before.iter().any(|e| e.shell_access) && before.iter().any(|e| !e.node_key.is_empty()), "the fixture has both");
    // And the rebuild is not a copy: with the iPad's cap gone, the view says
    // host_admin for it, and a rebuild told to raise it would write control
    // (never host_admin)...
    let mut uncapped = view.clone();
    uncapped.caps.remove("ipad-o");
    assert_eq!(uncapped.scope_of("ipad-o"), Some(Scope::HostAdmin));
    let raise = BTreeSet::from(["ipad-o".to_string()]);
    let raised = rebuild(&before, &uncapped, &raise).unwrap();
    assert_ne!(raised, today);
    let ipad = before.iter().position(|e| e.client_id == "ipad-o").unwrap();
    assert!(raised[ipad].contains("--scope control"), "{}", raised[ipad]);
    // ...but one that wasn't told to raise it never does (B1, the slice 1
    // review's SF2): a stale or missing cap can't widen a line.
    assert_eq!(rebuild(&before, &uncapped, &BTreeSet::new()).unwrap(), today);
}

/// A plan that would raise a line is never written (B1): it's refused
/// before the store is touched.
#[tokio::test]
async fn a_plan_that_would_raise_a_line_is_refused() {
    let r = runner().await;
    let entries = entries(&r.svc);
    let members = r.svc.store.members().unwrap();
    let added_by = r.svc.store.added_by_all().unwrap();
    let released = |id: &str| r.svc.store.device_released(id).unwrap();
    let mut plan = plan(&entries, &Facts { members: &members, released: &released, added_by: &added_by, access: &Default::default(), caps: &Default::default() });
    assert!(verify(&entries, &View::from_plan(&plan, &members)).is_ok());
    plan.caps.retain(|(d, _)| d != "ipad-o");
    assert_eq!(verify(&entries, &View::from_plan(&plan, &members)), Err("ipad-o".to_string()));
    let mut plan_pad = plan.clone();
    plan_pad.caps.push(("ipad-o".into(), Cap::Viewer));
    plan_pad.caps.retain(|(d, _)| d != "pad-b");
    assert_eq!(verify(&entries, &View::from_plan(&plan_pad, &members)), Err("pad-b".to_string()), "a member's read pad too");
}

/// A cap that is higher than the line is a raise as well: a member cap on
/// a read line is refused, and so is a viewer's access over a control line.
#[tokio::test]
async fn a_cap_or_access_that_misreads_a_line_is_refused() {
    let r = runner().await;
    let entries = entries(&r.svc);
    let members = r.svc.store.members().unwrap();
    let added_by = r.svc.store.added_by_all().unwrap();
    let released = |id: &str| r.svc.store.device_released(id).unwrap();
    let good = plan(&entries, &Facts { members: &members, released: &released, added_by: &added_by, access: &Default::default(), caps: &Default::default() });
    let mut raised = good.clone();
    for cap in raised.caps.iter_mut().filter(|(d, _)| d == "ipad-o") {
        cap.1 = Cap::Member;
    }
    assert_eq!(verify(&entries, &View::from_plan(&raised, &members)), Err("ipad-o".to_string()));
    let mut lowered = good;
    for access in lowered.member_access.iter_mut().filter(|(h, _)| h == "dan") {
        access.1 = Access::Member;
    }
    assert_eq!(verify(&entries, &View::from_plan(&lowered, &members)), Err("phone-d".to_string()), "an admin phone is not lowered");
}

/// The first run derives the tables and later starts only refresh them: a
/// restart keeps what a later change decided, adds the device an enroll
/// bound since, and writes no line.
#[tokio::test]
async fn the_migration_runs_once_and_later_starts_refresh() {
    let r = runner().await;
    assert!(migrate(&r.svc));
    assert!(r.svc.store.access_migrated().unwrap());
    let first = r.svc.store.stored_access().unwrap();
    enroll(&r.svc, 13, "phone-new", Scope::Control, false, "").await;
    let file = std::fs::read(r.svc.authorized_keys()).unwrap();
    assert!(!migrate(&r.svc), "the second run isn't the first");
    migrate_at_start(&r.svc).await;
    assert_eq!(std::fs::read(r.svc.authorized_keys()).unwrap(), file, "a refresh writes no line");
    let now = r.svc.store.stored_access().unwrap();
    let mut expected = first;
    expected.owner_devices.push(("phone-new".into(), OwnerDeviceState::Confirmed));
    expected.owner_devices.sort_by(|a, b| a.0.cmp(&b.0));
    expected.caps.push(("phone-new".into(), Cap::Member));
    expected.caps.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(now, expected, "the new device is the owner's, with its cap; nothing else moved");
}

/// Every Key A scope in the file is what the view says, device by device.
#[tokio::test]
async fn every_line_reads_back_at_its_scope() {
    let r = runner().await;
    migrate(&r.svc);
    let view = View::from_store(&r.svc.store).unwrap();
    for (device, scope) in lines_by_device(&entries(&r.svc)) {
        match device.as_str() {
            "phone-e" => assert_eq!(view.scope_of(&device), None, "a released device has no person"),
            _ => assert_eq!(view.scope_of(&device), Some(scope), "{device}"),
        }
    }
}

/// A runner with no `authorized_keys` yet has nothing to migrate and nobody
/// to bind; it's marked done, and no file appears.
#[tokio::test]
async fn a_runner_with_no_devices_migrates_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys"));
    assert!(migrate(&svc));
    assert_eq!(svc.store.stored_access().unwrap().caps, vec![]);
    assert!(!svc.authorized_keys().exists(), "migrating wrote no file");
}

/// `member.list` tells the owner every device's place and each member's
/// access, and tells a member neither: it's the owner's to check.
#[tokio::test]
async fn member_list_shows_the_owner_the_peoples_access_and_nobody_else() {
    use crate::rpc_people::tests::{call, factory};
    let r = runner().await;
    let list = |value| match value {
        Ok(pb::result::Value::MemberList(list)) => list,
        other => panic!("not a member list: {other:?}"),
    };
    let ask = || pb::Request { method: "member.list".into(), ..Default::default() };
    let before = list(call(&factory(&r.svc, None, Scope::HostAdmin), ask()).await);
    assert!(before.device_access.is_empty() && before.members.iter().all(|m| m.access.is_empty()), "nothing before it ran");
    migrate(&r.svc);
    let owner = list(call(&factory(&r.svc, None, Scope::HostAdmin), ask()).await);
    let row = |id: &str| owner.device_access.iter().find(|d| d.client_id == id).cloned().unwrap_or_else(|| panic!("no row for {id}"));
    assert_eq!((row("ipad-o").person.as_str(), row("ipad-o").access.as_str(), row("ipad-o").cap.as_str()), ("", "owner", "viewer"));
    assert_eq!((row("pad-b").person.as_str(), row("pad-b").access.as_str(), row("pad-b").cap.as_str()), ("bob", "member", "viewer"));
    assert!(row("legacy-t").check && !row("phone-o").check);
    assert!(owner.device_access.iter().all(|d| d.client_id != "phone-e"), "a released device is nobody's");
    let access = |handle: &str| owner.members.iter().find(|m| m.handle == handle).unwrap().access.clone();
    assert_eq!((access("bob"), access("carol"), access("dan")), ("member".into(), "viewer".into(), "admin".into()));
    let bob = list(call(&factory(&r.svc, Some("phone-b"), Scope::Control), ask()).await);
    assert!(!bob.device_access.is_empty() && bob.device_access.iter().all(|d| d.person == "bob"), "a member is told their own devices, never the owner's or another member's");
}

/// Each People row says what its device's step-up key is: trusted, proposed
/// with the code to check, or expired once its ten minutes pass (slice 4).
#[tokio::test]
async fn people_rows_carry_each_devices_step_up_key() {
    use farcooler_store::step_up::PROPOSAL_MS;
    let r = runner().await;
    migrate(&r.svc);
    let t0 = 5_000_000_i64;
    r.svc.store.put_step_up_key("ipad-o", "p256-sha256", &[4; 65], true, t0).unwrap();
    r.svc.store.put_step_up_key("pad-b", "p256-sha256", &[4; 65], false, t0).unwrap();
    let rows = |only: Option<&str>, now: i64| device_access(&r.svc.store, only, now).unwrap();
    let row = |rows: &[pb::DeviceAccess], id: &str| rows.iter().find(|d| d.client_id == id).cloned().unwrap_or_else(|| panic!("no row for {id}"));
    let all = rows(None, t0 + 1);
    assert_eq!(row(&all, "ipad-o").step_up, "trusted");
    assert_eq!(row(&all, "ipad-o").step_up_code, "", "a trusted key shows no code");
    let waiting = row(&all, "pad-b");
    assert_eq!((waiting.step_up.as_str(), waiting.step_up_at), ("proposed", t0));
    assert_eq!(waiting.step_up_code, crate::step_up::code_of("p256-sha256", &[4; 65]));
    assert_eq!(row(&all, "phone-o").step_up, "", "no key, no state");
    let late = rows(None, t0 + PROPOSAL_MS + 1);
    assert_eq!((row(&late, "pad-b").step_up.as_str(), row(&late, "pad-b").step_up_code.as_str()), ("expired", ""));
    let mine = rows(Some("bob"), t0 + 1);
    assert!(!mine.is_empty() && mine.iter().all(|d| d.person == "bob"), "one member's rows are theirs alone");
}

/// The slice 1 review's SF4: a rebuild returns every other shape of line
/// byte for byte as well: an id-less forced command, a foreign key, a line
/// another build spelled differently (another program, an extra option), a
/// `host_admin` line of a legacy teammate's, and two lines for one id.
#[test]
fn a_rebuild_returns_lines_in_other_shapes_as_they_were() {
    use farcooler_fence::{BEGIN, END, Grant, parse, render};
    let key = |n: u8| self::key(n);
    let ours = |n: u8, id: &str, scope: Scope| render(&key(n), id, id, scope, Grant::FarCooler, None).unwrap();
    let lines = vec![
        ours(1, "phone-o", Scope::Control),
        ours(2, "mac-legacy", Scope::HostAdmin),
        ours(3, "twin", Scope::Control),
        ours(4, "twin", Scope::Read),
        ours(5, "pending-t", Scope::Control),
        ours(6, "older", Scope::Control).replace("--stdio", "--stdio --legacy"),
        ours(7, "idless", Scope::Control).replace("--client idless ", ""),
        format!("ssh-ed25519 {} by-hand", key(8).split_whitespace().nth(1).unwrap()),
    ];
    let entries = parse(&format!("{BEGIN}\n{}\n{END}\n", lines.join("\n"))).unwrap();
    let released = |id: &str| id == "pending-t";
    let added_by = vec![("mac-legacy".to_string(), "someone".to_string())];
    let planned = plan(&entries, &Facts { members: &[], released: &released, added_by: &added_by, access: &Default::default(), caps: &Default::default() });
    let view = View::from_plan(&planned, &[]);
    verify(&entries, &view).expect("the plan reproduces every line it reads");
    assert_eq!(rebuild(&entries, &view, &BTreeSet::new()).unwrap(), lines);
    assert!(planned.owner_devices.iter().all(|(d, _)| d != "pending-t" && d != "idless"), "a released or id-less line isn't the owner's");
}
