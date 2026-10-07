//! The capability table against the method table: every name a method needs
//! is offered, no name twice, and `terminal.compose`'s two (ov-372, ov-367).

use crate::{capability, method};

#[test]
fn every_capability_a_method_names_is_one_this_build_advertises() {
    // A method mapped to a capability absent from `ALL` would be
    // permanently unreachable: the daemon refuses anything whose capability
    // it does not advertise, so the typo would present as a feature that
    // silently does not exist. Every method, not a sample of them.
    for method in method::Method::ALL {
        let cap = method.capability();
        assert!(capability::ALL.contains(&cap), "{method:?} names {cap}, which is not advertised");
    }
}

#[test]
fn capability_names_are_unique() {
    let unique: std::collections::BTreeSet<_> = capability::ALL.iter().collect();
    assert_eq!(unique.len(), capability::ALL.len(), "a duplicate name hides one of them");
}

/// `terminal.compose` is served behind `agent_compose`, and `compose` says
/// it takes line breaks, images and commands: both offered.
#[test]
fn compose_is_served_and_says_what_it_takes() {
    assert_eq!(capability::for_method("terminal.compose"), Some(capability::AGENT_COMPOSE));
    assert!(capability::ALL.contains(&capability::COMPOSE));
    assert_ne!(capability::COMPOSE, capability::AGENT_COMPOSE);
}
