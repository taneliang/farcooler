use super::*;
use crate::access::OwnerDeviceState;

fn store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &["pad-b".into()]).unwrap();
    store
}

fn insert(store: &Store, table: &str, id: &str) {
    let sql = match table {
        "owner_devices" => "INSERT INTO owner_devices (client_id) VALUES (?1)".to_string(),
        "member_devices" => "INSERT INTO member_devices (client_id, handle) VALUES (?1, 'bob')".to_string(),
        "device_caps" => "INSERT INTO device_caps (client_id, cap) VALUES (?1, 'viewer')".to_string(),
        "step_up_keys" => "INSERT INTO step_up_keys (client_id, alg, pubkey, state, registered_at) VALUES (?1, 'p256', x'00', 'proposed', 0)".to_string(),
        "pending_teammates" => "INSERT INTO pending_teammates (client_id, enrolled_at) VALUES (?1, 0)".to_string(),
        "released_devices" => "INSERT INTO released_devices (client_id) VALUES (?1)".to_string(),
        "device_enrollers" => "INSERT INTO device_enrollers (client_id, handle, enrolled_at) VALUES (?1, 'bob', 0)".to_string(),
        other => panic!("{other}"),
    };
    store.conn().execute(&sql, [id]).unwrap();
}

/// A bind is insert-only (S5): a row of any kind about the id, and the bind is
/// refused `device_taken` and writes nothing, whichever table holds it.
#[test]
fn a_bind_is_refused_when_any_row_about_the_id_is_left() {
    for table in DEVICE_TABLES {
        let store = store();
        insert(&store, table, "phone-x");
        assert!(store.device_has_rows("phone-x").unwrap(), "{table}");
        assert!(matches!(store.bind_owner_device("phone-x", None), Err(DomainError::Conflict { what: "device_taken" })), "{table}");
        let stored = store.stored_access().unwrap();
        assert_eq!(stored.owner_devices.iter().filter(|(d, _)| d == "phone-x").count(), usize::from(table == "owner_devices"), "{table}");
    }
    let store = store();
    assert!(!store.device_has_rows("phone-x").unwrap());
    store.bind_owner_device("phone-x", Some(Cap::Member)).unwrap();
    let stored = store.stored_access().unwrap();
    assert_eq!(stored.owner_devices, vec![("phone-x".into(), OwnerDeviceState::Confirmed)]);
    assert_eq!(stored.caps, vec![("phone-x".into(), Cap::Member)]);
    assert!(store.bind_owner_device("phone-x", None).is_err(), "a second bind of the same id");
}

/// A revoke clears the rows it owns for the id, whichever table holds them,
/// and nothing about any other id. The two older marks stay, as they did.
#[test]
fn clearing_a_device_removes_the_rows_a_revoke_owns_and_no_others() {
    for table in DEVICE_TABLES {
        let store = store();
        insert(&store, table, "phone-x");
        insert(&store, "device_caps", "phone-y");
        store.clear_device_rows("phone-x").unwrap();
        let kept = KEPT_MARKS.contains(&table);
        assert_eq!(store.device_has_rows("phone-x").unwrap(), kept, "{table}");
        assert!(store.device_has_rows("phone-y").unwrap(), "{table}: another id's rows went too");
    }
}

/// `access_generation` moves for a change to a person's authority, and not
/// for adding a device.
#[test]
fn the_generation_moves_with_access_and_caps_and_not_with_a_new_device() {
    let store = store();
    assert_eq!(store.access_generation("bob").unwrap(), 0);
    store.ensure_member_access("bob", Access::Member).unwrap();
    assert_eq!(store.access_generation("bob").unwrap(), 0, "a first access isn't a change");
    store.set_member_access("bob", Access::Viewer).unwrap();
    assert_eq!(store.access_generation("bob").unwrap(), 1);
    store.set_device_cap("pad-b", Some(Cap::Viewer)).unwrap();
    assert_eq!(store.access_generation("bob").unwrap(), 2, "a cap on bob's device is bob's change");
    assert_eq!(store.access_generation("").unwrap(), 0);
    store.bind_owner_device("phone-o", Some(Cap::Member)).unwrap();
    store.cap_new_device("pad-b", Cap::Member).unwrap();
    assert_eq!(store.access_generation("").unwrap(), 0, "adding an owner device changes nobody's authority");
    assert_eq!(store.access_generation("bob").unwrap(), 2);
    store.clear_device_rows("pad-b").unwrap();
    assert_eq!(store.access_generation("bob").unwrap(), 3, "revoking a capped device is a change");
    store.clear_device_rows("phone-o").unwrap();
    assert_eq!(store.access_generation("").unwrap(), 1, "and so is an owner's");
    assert!(matches!(store.set_member_access("ghost", Access::Member), Err(DomainError::NotFound)));
    assert!(store.set_member_access("bob", Access::Owner).is_err());
}

/// A refresh adds what a line has and the tables lack, drops what no line
/// backs, and leaves a row that already has a say.
#[test]
fn a_refresh_adds_prunes_and_keeps_what_has_a_say() {
    let store = store();
    store.ensure_member_access("bob", Access::Viewer).unwrap();
    store.bind_owner_device("gone-o", None).unwrap();
    store.bind_owner_device("kept-o", Some(Cap::Viewer)).unwrap();
    store.set_device_cap("stale-cap", Some(Cap::Member)).unwrap();
    let plan = AccessPlan {
        owner_devices: vec![("kept-o".into(), OwnerDeviceState::Confirmed), ("new-o".into(), OwnerDeviceState::Check)],
        member_access: vec![("bob".into(), Access::Admin)],
        caps: vec![("kept-o".into(), Cap::Member), ("new-o".into(), Cap::Member)],
    };
    let live = ["kept-o".to_string(), "new-o".to_string()].into_iter().collect();
    store.refresh_access(&plan, &live).unwrap();
    let stored = store.stored_access().unwrap();
    assert_eq!(stored.owner_devices, vec![("kept-o".into(), OwnerDeviceState::Confirmed), ("new-o".into(), OwnerDeviceState::Check)]);
    assert_eq!(stored.member_access, vec![("bob".into(), Access::Viewer)], "bob's recorded access has a say");
    assert_eq!(stored.caps, vec![("kept-o".into(), Cap::Viewer), ("new-o".into(), Cap::Member)], "a cap that exists wins; a cap no line backs goes");
}
