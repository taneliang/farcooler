use super::*;

#[test]
fn every_enroller_is_kept_and_counted_by_day() {
    let store = Store::open_in_memory().unwrap();
    store.record_added_by_at("browser", "phone", 1_000).unwrap();
    store.record_added_by_at("tablet", "phone", 5_000).unwrap();
    store.record_added_by_at("phone", "", 500).unwrap();
    assert_eq!(store.added_by_since("phone", 0).unwrap(), 2);
    assert_eq!(store.added_by_since("phone", 2_000).unwrap(), 1, "an add before the window was counted");
    assert_eq!(store.added_by_since("tablet", 0).unwrap(), 0);
    // Enrolled again by someone else, the row is theirs.
    store.record_added_by_at("browser", "", 6_000).unwrap();
    assert_eq!(store.added_by_since("phone", 0).unwrap(), 1);
    assert_eq!(
        store.added_by_all().unwrap(),
        vec![("browser".into(), "".into()), ("phone".into(), "".into()), ("tablet".into(), "phone".into())]
    );
}

#[test]
fn what_a_device_added_is_found_through_every_generation_and_any_cycle() {
    let rows: Vec<(String, String)> = [("b", "a"), ("c", "b"), ("d", "c"), ("a", "d"), ("x", "y"), ("e", "a")]
        .iter()
        .map(|(d, by)| (d.to_string(), by.to_string()))
        .collect();
    assert_eq!(added_under(&rows, "a"), vec!["b", "c", "d", "e"]);
    assert_eq!(added_under(&rows, "c"), vec!["a", "b", "d", "e"]);
    assert!(added_under(&rows, "x").is_empty());
}

/// R4 (the ov-828 re-review): a revoked device's pending row and its
/// member enrollment go; who added what stays, for the chain.
#[test]
fn a_revoked_device_waits_on_nothing() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &[]).unwrap();
    store.record_teammate_pending("phone-t").unwrap();
    store.record_member_enrolled("browser-b", "bob").unwrap();
    store.record_added_by("browser-b", "phone-b").unwrap();
    store.forget_revoked_device("phone-t").unwrap();
    store.forget_revoked_device("browser-b").unwrap();
    assert!(store.released_to_sweep().unwrap().contains(&"phone-t".to_string()), "the pending row stayed");
    assert!(store.unbound_enrolled_by("bob").unwrap().is_empty(), "the member enrollment stayed");
    assert_eq!(store.added_by_all().unwrap(), vec![("browser-b".into(), "phone-b".into())]);
}

/// R3 (the ov-828 re-review): a teammate's line from before ov-828, never
/// bound, read as the owner's and could add devices. Bound now, what it
/// added while the owner's is the member's enrollment: released until
/// bound, and revoked with them.
#[test]
fn binding_a_device_hands_over_what_it_added_as_the_owners() {
    let store = Store::open_in_memory().unwrap();
    store.record_added_by("browser", "legacy").unwrap();
    store.record_added_by("tablet", "browser").unwrap();
    store.record_added_by("owners-mac", "").unwrap();
    assert!(store.device_is_owners("browser").unwrap());

    store.set_member("bob", "Bob", &["legacy".to_string()]).unwrap();
    for device in ["browser", "tablet"] {
        assert!(store.device_released(device).unwrap(), "{device} stayed the owner's");
    }
    assert!(store.device_is_owners("owners-mac").unwrap(), "a device nobody bound added was handed over");
    let removed = store.remove_member_with_enrolled("bob").unwrap();
    assert_eq!(removed, vec!["browser".to_string(), "tablet".to_string()], "removing bob left what his device added");
}

/// 0065 (ov-838) is the 56th and `Welcome`.
#[test]
fn the_added_by_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[55];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0065_device_added_by as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
}

/// R-88 (ov-838): a member's own phone adds their browser, theirs from its
/// write; another member's id is refused, and so is a member who is gone.
#[test]
fn a_members_own_device_is_bound_to_them_from_its_write() {
    let store = Store::open_in_memory().unwrap();
    store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    store.record_member_device_added("browser-b", "bob").unwrap();
    assert_eq!(store.member_of_device("browser-b").unwrap().as_deref(), Some("bob"));
    assert!(!store.device_released("browser-b").unwrap());
    let taken = store.record_member_device_added("phone-c", "bob");
    assert!(matches!(taken, Err(farcooler_core::DomainError::Conflict { what: "device_taken" })), "{taken:?}");
    assert!(store.record_member_device_added("browser-x", "nobody").is_err(), "a gone member gained a device");
    store.remove_member("bob").unwrap();
    assert!(store.released_to_sweep().unwrap().contains(&"browser-b".to_string()), "bob's browser outlived him");
}

/// ov-982: how a device was paired, and when, for the owner to read before
/// vouching. Enrolled again, the newer route wins; one paired before any of
/// this was recorded says nothing rather than guessing.
#[test]
fn how_and_when_a_device_was_paired_is_kept() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!((store.paired_via("phone").unwrap(), store.added_by_of("phone").unwrap()), (None, None), "never recorded");
    store.record_added_by_at("phone", "", 7_000).unwrap();
    store.record_paired_via("phone", PairedVia::Terminal).unwrap();
    assert_eq!(store.added_by_of("phone").unwrap(), Some(("".into(), 7_000)));
    assert_eq!(store.paired_via("phone").unwrap(), Some(PairedVia::Terminal));
    store.record_paired_via("phone", PairedVia::App).unwrap();
    assert_eq!(store.paired_via("phone").unwrap(), Some(PairedVia::App), "enrolled again");
    for via in [PairedVia::App, PairedVia::Device, PairedVia::Terminal] {
        assert_eq!(PairedVia::parse(via.as_str()), Some(via));
    }
    assert_eq!(PairedVia::parse("socket"), None);
}

/// ov-989: a device's model, as it said at pairing, is kept and cleaned to one short printable line.
#[test]
fn the_model_a_device_said_is_kept_clean() {
    use crate::device_added_by::clean_model;
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.device_model("phone").unwrap(), "", "never said");
    store.record_device_model("phone", "iPhone 17 Pro").unwrap();
    assert_eq!(store.device_model("phone").unwrap(), "iPhone 17 Pro");
    store.record_device_model("phone", "").unwrap();
    assert_eq!(store.device_model("phone").unwrap(), "", "forgotten");
    assert_eq!(clean_model("  Pixel\n\t9  Pro\u{202e} "), "Pixel 9 Pro");
    assert_eq!(clean_model(&"x".repeat(100)).chars().count(), 40);
    // The ones a first pass missed: Arabic letter mark, Mongolian vowel separator, soft hyphen, tag characters,
    // variation selectors, and a stack of combining marks.
    assert_eq!(clean_model("A\u{061C}B\u{180E}C\u{00AD}D\u{E0041}\u{E007F}E\u{FE0F}F"), "ABCDEF");
    assert_eq!(clean_model(&format!("Pi{}xel", "\u{0301}".repeat(30))), "Pixel", "marks don't stack");
}
