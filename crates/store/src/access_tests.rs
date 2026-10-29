use super::*;

fn plan() -> AccessPlan {
    AccessPlan {
        owner_devices: vec![("phone-o".into(), OwnerDeviceState::Confirmed), ("legacy".into(), OwnerDeviceState::Check)],
        member_access: vec![("bob".into(), Access::Member)],
        caps: vec![("phone-o".into(), Cap::Member), ("pad-b".into(), Cap::Viewer)],
    }
}

#[test]
fn a_plan_is_written_once_and_a_second_changes_nothing() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &["pad-b".into()]).unwrap();
    assert!(!store.access_migrated().unwrap());
    assert!(store.apply_access_plan(&plan()).unwrap());
    let first = store.stored_access().unwrap();
    assert_eq!(first.owner_devices, vec![("legacy".into(), OwnerDeviceState::Check), ("phone-o".into(), OwnerDeviceState::Confirmed)]);
    assert_eq!(first.member_access, vec![("bob".into(), Access::Member)]);
    assert_eq!(first.caps, vec![("pad-b".into(), Cap::Viewer), ("phone-o".into(), Cap::Member)]);
    assert!(store.access_migrated().unwrap());
    let other = AccessPlan { caps: vec![("phone-o".into(), Cap::Viewer)], ..plan() };
    assert!(!store.apply_access_plan(&other).unwrap(), "already ran");
    assert_eq!(store.stored_access().unwrap(), first);
}

/// A plan that fails halfway leaves nothing and doesn't mark itself done:
/// Bob isn't a member, so his access row is refused by its key.
#[test]
fn a_plan_that_fails_writes_nothing() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.apply_access_plan(&plan()).is_err());
    assert!(!store.access_migrated().unwrap());
    assert_eq!(store.stored_access().unwrap().caps, vec![]);
    assert_eq!(store.stored_access().unwrap().owner_devices, vec![]);
}

#[test]
fn nobody_stores_owner_as_a_members_access() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &[]).unwrap();
    let plan = AccessPlan { member_access: vec![("bob".into(), Access::Owner)], ..Default::default() };
    assert!(store.apply_access_plan(&plan).is_err());
}

#[test]
fn removing_a_member_takes_their_access_with_them() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &["pad-b".into()]).unwrap();
    store.apply_access_plan(&plan()).unwrap();
    store.remove_member("bob").unwrap();
    assert_eq!(store.stored_access().unwrap().member_access, vec![]);
}

#[test]
fn a_cap_can_only_be_viewer_or_member() {
    let store = Store::open_in_memory().unwrap();
    let conn = store.conn();
    assert!(conn.execute("INSERT INTO device_caps (client_id, cap) VALUES ('d', 'admin')", []).is_err());
    assert!(conn.execute("INSERT INTO step_up_keys (client_id, alg, pubkey, state, registered_at) VALUES ('d', 'p256', x'00', 'maybe', 0)", []).is_err());
    assert!(conn.execute("INSERT INTO step_up_keys (client_id, alg, pubkey, state, registered_at) VALUES ('d', 'p256', x'00', 'proposed', 0)", []).is_ok());
}

/// The migration is the 57th (0066) and `Welcome`: a build from before it
/// never names the tables.
#[test]
fn the_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[56];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0066_person_access as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 57);
}
