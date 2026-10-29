//! Who enrolled a device (ov-719), on an in-memory store.

use super::*;

fn devices(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|d| d.to_string()).collect()
}

/// A device a member enrolled is released until it's bound, and listed as
/// theirs only while nobody is bound to it. One already bound keeps its
/// member. The owner's stay the owner's.
#[test]
fn a_device_a_member_enrolled_is_nobodys_until_bound() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &devices(&["mac-b"])).unwrap();
    store.set_member("tess", "Tess", &devices(&["phone-t"])).unwrap();
    assert!(store.device_is_owners("phone-o").unwrap());

    store.record_member_enrolled("phone-u", "bob").unwrap();
    store.record_member_enrolled("phone-t", "bob").unwrap();
    assert!(store.device_released("phone-u").unwrap());
    assert!(!store.device_is_owners("phone-u").unwrap());
    // Only a new line is recorded, so Tess's id here was revoked: Bob's key
    // under it isn't Tess's, and is nobody's until bound again (review F2).
    assert!(store.device_released("phone-t").unwrap(), "Bob's key would be Tess");
    assert_eq!(store.member_of_device("phone-t").unwrap(), None);
    assert_eq!(store.unbound_enrolled_by("bob").unwrap(), devices(&["phone-t", "phone-u"]));
    store.set_member("tess", "Tess", &devices(&["phone-t"])).unwrap();
    assert_eq!(store.unbound_enrolled_by("bob").unwrap(), devices(&["phone-u"]));

    store.set_member("una", "Una", &devices(&["phone-u"])).unwrap();
    assert!(!store.device_released("phone-u").unwrap(), "bound, it's Una's");
    assert!(store.unbound_enrolled_by("bob").unwrap().is_empty());
    assert!(!store.device_is_owners("mac-b").unwrap());
}

/// Removing a member answers with what they enrolled, in the same
/// transaction, and after it their device enrolls nothing: an enroll racing
/// the removal either is in the answer or writes no line.
#[test]
fn a_removed_member_enrolls_nothing_more() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &[]).unwrap();
    store.record_member_enrolled("phone-u", "bob").unwrap();
    assert_eq!(store.remove_member_with_enrolled("bob").unwrap(), devices(&["phone-u"]));
    assert!(store.record_member_enrolled("phone-v", "bob").is_err(), "refused once Bob is gone");
    assert!(!store.device_released("phone-v").unwrap(), "and wrote nothing");
}

/// Removing the enroller takes their rows with them, so the handle used
/// again inherits nothing, and what they enrolled stays released.
#[test]
fn the_record_goes_with_its_member() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &[]).unwrap();
    store.record_member_enrolled("phone-u", "bob").unwrap();
    store.remove_member("bob").unwrap();
    store.set_member("bob", "Another Bob", &[]).unwrap();
    assert!(store.unbound_enrolled_by("bob").unwrap().is_empty());
    assert!(store.device_released("phone-u").unwrap());
}

/// The migration is the 49th (0058) and `Welcome`: a build from before it
/// never names the table.
#[test]
fn the_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[48];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0058_device_enrollers as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 57);
}

/// 0063 (ov-828) is the 54th and `Welcome`.
#[test]
fn the_pending_teammates_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[53];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0063_pending_teammates as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
}

/// A teammate's device the owner is adding (ov-828 S1) is nobody's from its
/// write, and the sweep leaves it for the bind; bound, it is the member's
/// and waits on nothing, and taken off them it is swept as any other.
#[test]
fn a_pending_teammate_is_released_until_bound_and_never_swept_meanwhile() {
    let store = Store::open_in_memory().unwrap();
    store.record_teammate_pending("phone-t").unwrap();
    assert!(store.device_released("phone-t").unwrap());
    assert!(!store.device_is_owners("phone-t").unwrap());
    assert!(!store.released_to_sweep().unwrap().contains(&"phone-t".to_string()));

    store.set_member("tess", "Tess", &["phone-t".to_string()]).unwrap();
    assert!(!store.device_released("phone-t").unwrap());
    store.set_member("tess", "Tess", &[]).unwrap();
    assert!(store.released_to_sweep().unwrap().contains(&"phone-t".to_string()), "a pending mark outlived its bind");
}
