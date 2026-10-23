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
    assert!(!store.device_released("phone-t").unwrap(), "Tess's stays hers");
    assert_eq!(store.unbound_enrolled_by("bob").unwrap(), devices(&["phone-u"]));

    store.set_member("una", "Una", &devices(&["phone-u"])).unwrap();
    assert!(!store.device_released("phone-u").unwrap(), "bound, it's Una's");
    assert!(store.unbound_enrolled_by("bob").unwrap().is_empty());
    assert!(!store.device_is_owners("mac-b").unwrap());
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
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 49);
}
