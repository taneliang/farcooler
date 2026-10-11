//! The capability table against the method table: every name a method needs
//! is offered, no name twice, each method's wire name, `terminal.compose`'s
//! two (ov-372, ov-367), and the interrupt's (ov-368).

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

/// Each method is one wire name and back again, and no two share a name.
///
/// The macro writes the name into `name` and `parse` from one row, so the
/// round trip can only fail on a duplicate: `parse` would answer the first
/// row for both, and the second would be unreachable on the wire.
#[test]
fn every_method_round_trips_through_its_wire_name() {
    let names: std::collections::BTreeSet<_> = method::Method::ALL.iter().map(|m| m.name()).collect();
    assert_eq!(names.len(), method::Method::ALL.len(), "two methods share a wire name");
    for &m in method::Method::ALL {
        assert_eq!(method::Method::parse(m.name()), Some(m), "{m:?}");
        assert_eq!(capability::for_method(m.name()), Some(m.capability()), "{m:?}");
    }
}

/// Chat mode's methods are gone (ov-377): a runner refuses each as a method
/// it has never heard of, and what a terminal pane still uses stays under
/// `agent`.
#[test]
fn chat_modes_methods_are_retired() {
    for name in [
        "terminal.set_pane_mode",
        "terminal.agent_prompt",
        "terminal.agent_set_mode",
        "terminal.agent_set_model",
        "terminal.agent_set_config",
        "terminal.agent_edit_queued",
        "terminal.agent_cancel_queued",
        "terminal.agent_steer_queued",
        "terminal.agent_cancel",
        "adapter.list",
        "adapter.upsert",
        "adapter.delete",
        "adapter.test",
    ] {
        assert_eq!(method::Method::parse(name), None, "{name}");
    }
    for name in ["terminal.agent_subscribe", "terminal.agent_answer", "terminal.draft_prompt", "terminal.tell"] {
        assert_eq!(capability::for_method(name), Some(capability::AGENT), "{name}");
    }
}

/// A retired capability's word is never advertised, and never given to
/// something new.
#[test]
fn a_retired_capability_is_never_advertised_again() {
    for retired in capability::RETIRED {
        assert!(!capability::ALL.contains(retired), "{retired} is advertised");
    }
    assert!(capability::RETIRED.contains(&"agent_queue"));
    assert!(capability::RETIRED.contains(&"adapters"));
}

/// Stop and Send Now are served behind one word of their own (ov-368), which
/// no other method needs.
#[test]
fn the_interrupt_is_served_behind_its_own_word() {
    for name in ["terminal.interrupt", "terminal.send_now"] {
        assert_eq!(capability::for_method(name), Some(capability::TERMINAL_INTERRUPT), "{name}");
    }
    assert!(capability::ALL.contains(&capability::TERMINAL_INTERRUPT));
    assert_eq!(method::Method::ALL.iter().filter(|m| m.capability() == capability::TERMINAL_INTERRUPT).count(), 2);
}

/// Bring Here is served behind a word of its own (ov-369), which no other
/// method needs.
#[test]
fn bring_here_is_served_behind_its_own_word() {
    assert_eq!(capability::for_method("terminal.bring_draft"), Some(capability::BRING_DRAFT));
    assert!(capability::ALL.contains(&capability::BRING_DRAFT));
    assert_eq!(method::Method::ALL.iter().filter(|m| m.capability() == capability::BRING_DRAFT).count(), 1);
}
