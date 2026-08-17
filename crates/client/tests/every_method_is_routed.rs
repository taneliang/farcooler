//! The call table is how an app reaches a runner, and nothing in the compiler
//! checks that a method the daemon serves has an arm in it.
//!
//! This is the hole that shipped. `crates/daemon` grew `client.list`,
//! `client.enroll` and `client.revoke`; the protocol gave them a capability and
//! a scope; the daemon dispatched them; and `ffi::dispatch` routed none of the
//! three, so the whole of device enrollment was unreachable from every app with
//! nothing failing anywhere — the same shape as `header.rs`, where a function
//! exported and not declared is invisible to every client.
//!
//! Checked by reading the source rather than by calling, because `dispatch`
//! needs a live session and the failure being guarded against is precisely a
//! method nobody can call.

const FFI: &str = include_str!("../src/ffi.rs");

/// The methods this crate must route for the enrollment ceremony to end in an
/// enrollment. Their scopes and capabilities are `crates/protocol`'s, and the
/// rules about what may be written are `crates/daemon`'s; what belongs here is
/// only that an app can ask.
const ENROLLMENT: [&str; 3] = ["client.list", "client.enroll", "client.revoke"];

#[test]
fn every_enrollment_method_the_daemon_serves_can_be_called() {
    for method in ENROLLMENT {
        assert!(
            FFI.contains(&format!("\"{method}\" =>")),
            "the daemon serves {method} and no app can reach it: `dispatch` has no arm for it"
        );
    }
}

/// And the header says so, which is the other half of being reachable: an app
/// developer reads that file to find out what may be passed to
/// `farcooler_client_call`.
#[test]
fn the_header_tells_an_app_developer_these_exist() {
    const HEADER: &str = include_str!("../include/farcooler_client.h");
    for method in ENROLLMENT {
        assert!(
            HEADER.contains(method),
            "{method} is routed but undocumented: nobody will find it"
        );
    }
}
