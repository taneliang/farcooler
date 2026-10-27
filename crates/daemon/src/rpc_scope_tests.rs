//! The scope each method needs, and who may call what: moved out of
//! `rpc.rs` to keep it inside its size budget.

use super::*;

#[test]
fn scope_is_ordered_so_admin_can_do_everything() {
    assert!(satisfies(Scope::HostAdmin, Scope::Read));
    assert!(satisfies(Scope::HostAdmin, Scope::Control));
    assert!(satisfies(Scope::Control, Scope::Read));
    assert!(satisfies(Scope::Read, Scope::Read));
}

#[test]
fn a_lower_scope_cannot_reach_a_higher_method() {
    assert!(!satisfies(Scope::Read, Scope::Control));
    assert!(!satisfies(Scope::Control, Scope::HostAdmin));
    // An unnegotiated scope reaches nothing at all.
    assert!(!satisfies(Scope::Unspecified, Scope::Read));
}

/// Every method on the wire has a scope, so none of them is refused as a
/// method this runner does not know.
///
/// Enumerates `Method::ALL` rather than a list of names typed here, which
/// is what let the agent queue's three slip: they were in the protocol's
/// table and the hand-typed list below, which checked it, never named them.
#[test]
fn every_method_has_a_scope() {
    let unscoped: Vec<_> = farcooler_protocol::method::Method::ALL
        .iter()
        .filter(|m| required_scope(m.name()).is_none())
        .collect();
    assert!(unscoped.is_empty(), "refused as unknown by every runner: {unscoped:?}");
}

#[test]
fn an_unknown_method_is_refused_rather_than_defaulted() {
    // Deliberately a name nothing will ever take: this used to be
    // `terminal.write`, which became real, and a test asserting a method does
    // not exist has to name one that cannot.
    assert_eq!(required_scope("terminal.telepathy"), None);
    // The `layout.` handler arm is prefix-matched, so an unlisted layout
    // method must still be stopped by the table before it gets there.
    assert_eq!(required_scope("layout.nonsense"), None);
    assert_eq!(required_scope(""), None);
    assert_eq!(required_scope("host.get "), None, "no fuzzy matching");
}

#[test]
fn the_dangerous_methods_require_host_admin() {
    // Adding a repository root grants access to a directory tree, and
    // removing a worktree deletes files. Neither is a `control` action.
    assert_eq!(required_scope("repository_root.add"), Some(Scope::HostAdmin));
    assert_eq!(required_scope("worktree.remove"), Some(Scope::HostAdmin));
    assert_eq!(required_scope("repository_root.remove"), Some(Scope::HostAdmin));
    // Paths live behind the same gate, so listing roots is admin too.
    assert_eq!(required_scope("repository_root.list"), Some(Scope::HostAdmin));
}

#[test]
fn every_runner_setting_method_requires_host_admin() {
    // Writes, because they touch a file in the user's home directory on a
    // runner that may not be the one asking.
    for method in [
        "settings.set_branch_prefix",
        "settings.set_projector",
        "theme.upsert",
        "theme.delete",
    ] {
        assert_eq!(required_scope(method), Some(Scope::HostAdmin), "{method}");
    }
    // A color is not a secret.
    assert_eq!(required_scope("theme.list"), Some(Scope::Read));
    // The adapter family went with chat mode (ov-377): refused, not
    // defaulted to a scope.
    for method in ["adapter.list", "adapter.upsert", "adapter.delete", "adapter.test"] {
        assert_eq!(required_scope(method), None, "{method}");
    }
}

#[test]
fn enrolling_a_device_requires_control_and_looking_does_not() {
    // Revoking decides who may log in here at all. Enrolling is `control` at
    // the gate so the owner's phone can add a browser (ov-828), and
    // `enrollment::may_grant` keeps a control caller to grants no wider than
    // its own (`enrollment/grant_tests.rs`).
    assert_eq!(required_scope("client.enroll"), Some(Scope::Control));
    assert_eq!(required_scope("client.revoke"), Some(Scope::HostAdmin));
    // Reading is the shape of the fleet: no path, and a public key's
    // fingerprint is published by design.
    assert_eq!(required_scope("client.list"), Some(Scope::Read));
}

#[test]
fn tiling_is_control_not_admin() {
    // An agent has to be able to place its own panes, and none of this
    // touches a file or stops a process.
    for method in ["layout.split", "layout.zoom", "layout.move"] {
        assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
    }
    assert_eq!(required_scope("layout.list"), Some(Scope::Read));
}

// `every_method_is_dispatched_and_every_dispatched_route_is_a_method` is
// `tests/every_method_is_dispatched.rs` (ov-171): it parses this file, so
// it reads the arms rather than the text.

/// The board and workspace routes are split the way the scope table's
/// comments say they are, which no set comparison can see.
#[test]
fn the_board_reads_are_read_and_its_writes_are_control() {
    for method in ["task.list", "task.get", "task.get_by_key", "task.search"] {
        assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
    }
    for method in
        ["task.create", "task.update", "task.set_status", "task.note", "task.block"]
    {
        assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
    }
    assert_eq!(required_scope("workspace.list"), Some(Scope::Read));
    assert_eq!(required_scope("report.get"), Some(Scope::Read), "a report only reads");
    for method in [
        "workspace.create",
        "workspace.rename",
        "workspace.set_prefix",
        "workspace.set_settings",
        "workspace.delete",
        "task.move",
        "workspace.mark_read",
        "worktree.assign",
        "terminal.set_role",
        "workspace.start_orchestrator",
    ] {
        assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
    }
    // The one route that must never exist. Notes are append-only: the
    // store has a `BEFORE UPDATE` trigger that refuses unconditionally, so
    // a route here would fail on a caller's data rather than at review.
    assert_eq!(required_scope("task.note_update"), None);
    assert_eq!(required_scope("task.note_edit"), None);
}

#[test]
fn reads_never_require_more_than_read() {
    for method in ["host.get", "daemon.version", "worktree.list", "terminal.list"] {
        assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
    }
}
