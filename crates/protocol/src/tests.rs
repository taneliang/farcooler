//! The crate's own tests, moved out of `lib.rs` (ov-455), which was over
//! the 1,500-line budget.

use super::*;

/// `TaskBlockSet.reason` carries presence, and the bytes have to agree
/// with what its comment in the proto promises, because that promise is
/// the whole argument that adding `optional` did not break the wire.
///
/// Three claims, each checked as bytes rather than reasoned about:
///
///   - `None` puts no field 3 on the wire, so an old daemon reading it
///     sees the `string` default `""` -- which is what an old client
///     meaning "no reason given" already sent.
///   - `Some("")` DOES put field 3 on the wire (tag byte `0x1a`, length
///     `0`), which is the difference presence buys: an implicit-presence
///     `string` cannot say "empty on purpose" at all.
///   - a plain proto3 `string` still vanishes when empty -- `actor` here,
///     the neighbouring field -- which is why an OLD client that sent
///     `reason: ""` is read as absent by a new daemon and so preserves a
///     reason rather than clearing it.
///
/// That last one is the compatibility note worth having pinned: the
/// upgrade makes old clients safe by default and costs them only the
/// ability to clear a reason, which they had no way to ask for anyway.
#[test]
fn an_absent_block_reason_is_absent_on_the_wire_and_an_empty_one_is_not() {
    use prost::Message;

    let absent =
        v1::TaskBlockSet { reason: None, actor: String::new(), ..Default::default() };
    assert!(
        !absent.encode_to_vec().contains(&0x1au8),
        "no reason means no field 3, which an old daemon reads as the default"
    );

    let empty =
        v1::TaskBlockSet { reason: Some(String::new()), ..Default::default() };
    assert_eq!(
        empty.encode_to_vec(),
        vec![0x1a, 0x00],
        "an empty reason asked for on purpose is sent, or nobody could ever clear one"
    );

    // The neighbouring plain `string`, to show that vanishing-when-empty
    // is proto3's rule and not something this field opted into.
    let actor = v1::TaskBlockSet { actor: String::new(), ..Default::default() };
    assert!(
        actor.encode_to_vec().is_empty(),
        "an implicit-presence string is not sent when it is empty"
    );
}

/// Renaming a message is free on the wire only because its field numbers
/// do not move: protobuf never sends a name. This pins every number the
/// worktree rename touched, read by name out of the compiled descriptor,
/// so an app built before the rename still means what it sends.
///
/// Each row is (message, field, number, type). A message-typed field also
/// pins which message sits at that number, so two renamed messages
/// swapped between tags fail here too. The messages renamed whole are
/// pinned field by field and must have no field the table does not name,
/// so a field dropped or added in passing fails as well.
#[test]
fn renamed_messages_keep_their_field_numbers() {
    use prost::Message;
    use prost_types::FileDescriptorSet;

    let set = FileDescriptorSet::decode(
        &include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..],
    )
    .expect("the build writes a descriptor");
    let file = set
        .file
        .iter()
        .find(|f| f.package() == "farcooler.v1")
        .expect("farcooler.v1 is in the descriptor");
    let message = |name: &str| {
        file.message_type
            .iter()
            .find(|m| m.name() == name)
            .unwrap_or_else(|| panic!("no message {name}"))
    };

    const WT: &str = ".farcooler.v1.Worktree";
    let fields: &[(&str, &str, i32, &str)] = &[
        ("Request", "worktree_create", 23, ".farcooler.v1.WorktreeCreate"),
        ("Request", "worktree_reorder", 70, ".farcooler.v1.WorktreeReorder"),
        ("Result", "worktree", 6, WT),
        ("Result", "worktree_list", 7, ".farcooler.v1.WorktreeList"),
        ("Result", "discovered_worktree_list", 15, ".farcooler.v1.DiscoveredWorktreeList"),
        ("Event", "worktree_changed", 13, WT),
        ("Worktree", "id", 1, ""),
        ("Worktree", "resource_version", 2, ""),
        ("Worktree", "repository_id", 3, ""),
        ("Worktree", "task_name", 4, ""),
        ("Worktree", "branch", 5, ""),
        ("Worktree", "worktree_path_token", 6, ""),
        ("Worktree", "worktree_path", 7, ""),
        ("Worktree", "state", 8, ".farcooler.v1.WorktreeState"),
        ("Worktree", "is_main_checkout", 9, ""),
        ("Worktree", "ordinal", 10, ""),
        // Added after the rename by workspaces-as-workstreams, on fresh
        // tags. Listed so the whole-message check below still holds.
        ("Worktree", "workspace_id", 11, ""),
        ("Worktree", "claim_source", 12, ""),
        ("Worktree", "foreign_writer_workspace_ids", 13, ""),
        // Added by the needs-you rollup, on a fresh tag.
        ("Worktree", "open_tasks", 14, ".farcooler.v1.TaskRef"),
        // Added by ov-199, on a fresh tag.
        ("Worktree", "lfs_pointers", 15, ""),
        ("WorktreeList", "items", 1, WT),
        ("WorktreeReorder", "worktree_ids", 1, ""),
        ("WorktreeCreate", "task_name", 1, ""),
        ("WorktreeCreate", "branch", 2, ""),
        ("WorktreeCreate", "base_revision", 3, ""),
        ("WorktreeCreate", "terminal_preset", 4, ""),
        ("WorktreeCreate", "adopt_existing", 5, ""),
        ("WorktreeCreate", "fork_only", 6, ""),
        ("WorktreeCreate", "workspace_id", 7, ""),
        ("DiscoveredWorktree", "path", 1, ""),
        ("DiscoveredWorktree", "branch", 3, ""),
        ("DiscoveredWorktree", "head", 4, ""),
        ("DiscoveredWorktree", "suggested_name", 5, ""),
        ("DiscoveredWorktree", "locked", 6, ""),
        ("DiscoveredWorktreeList", "items", 1, ".farcooler.v1.DiscoveredWorktree"),
        ("InboxWorktree", "worktree_id", 1, ""),
        ("InboxWorktree", "task_name", 2, ""),
        ("InboxWorktree", "branch", 3, ""),
        ("InboxWorktree", "changed_since_reviewed", 8, ""),
        ("InboxWorktree", "insertions", 9, ""),
        ("InboxWorktree", "deletions", 10, ""),
        ("ChangesInbox", "items", 1, ".farcooler.v1.InboxWorktree"),
        ("ChangeSetSelector", "worktree", 1, ".farcooler.v1.Empty"),
        ("WorktreeFileSearch", "worktree_id", 1, ""),
        ("Terminal", "worktree_id", 4, ""),
        ("PaneGroup", "worktree_id", 2, ""),
        ("PaneGroupList", "worktree_id", 1, ""),
        ("ChangeSetRequest", "worktree_id", 1, ""),
        ("ChangeSetChanged", "worktree_id", 1, ""),
        ("ChangeSet", "worktree_id", 1, ""),
        ("CommitFilesRequest", "worktree_id", 1, ""),
        ("FileDiffRequest", "worktree_id", 1, ""),
        ("ChangesSetBase", "worktree_id", 1, ""),
        ("ChangesMarkRead", "worktree_id", 1, ""),
        ("Task", "worktree_id", 11, ""),
        ("TaskCreate", "worktree_id", 7, ""),
        ("TaskUpdate", "worktree_id", 8, ""),
    ];
    for &(m, f, number, type_name) in fields {
        let field = message(m)
            .field
            .iter()
            .find(|x| x.name() == f)
            .unwrap_or_else(|| panic!("{m} has no field {f}"));
        assert_eq!(field.number(), number, "{m}.{f} moved off tag {number}");
        if !type_name.is_empty() {
            assert_eq!(field.type_name(), type_name, "{m}.{f} carries the wrong message");
        }
    }

    // Renamed whole: every field is in the table above, so none went
    // missing and none arrived with the rename.
    for m in [
        "Worktree",
        "WorktreeList",
        "WorktreeReorder",
        "WorktreeCreate",
        "DiscoveredWorktree",
        "DiscoveredWorktreeList",
        "InboxWorktree",
    ] {
        let mut have: Vec<_> =
            message(m).field.iter().map(|x| (x.name().to_string(), x.number())).collect();
        let mut want: Vec<_> = fields
            .iter()
            .filter(|r| r.0 == m)
            .map(|r| (r.1.to_string(), r.2))
            .collect();
        have.sort();
        want.sort();
        assert_eq!(have, want, "{m} is not the message it was before the rename");
    }

    let enum_values = |name: &str| -> Vec<(String, i32)> {
        file.enum_type
            .iter()
            .find(|e| e.name() == name)
            .unwrap_or_else(|| panic!("no enum {name}"))
            .value
            .iter()
            .map(|v| (v.name().to_string(), v.number()))
            .collect()
    };
    let state: Vec<(String, i32)> = [
        ("WORKTREE_STATE_UNSPECIFIED", 0),
        ("WORKTREE_STATE_CREATING", 1),
        ("WORKTREE_STATE_READY", 2),
        ("WORKTREE_STATE_ACTIVE", 3),
        ("WORKTREE_STATE_ERROR", 4),
        ("WORKTREE_STATE_HIDDEN", 5),
        ("WORKTREE_STATE_WORKTREE_MISSING", 6),
    ]
    .map(|(n, v)| (n.to_string(), v))
    .into();
    assert_eq!(enum_values("WorktreeState"), state, "WorktreeState's numbers moved");
    assert!(
        enum_values("ErrorCode").contains(&("ERROR_CODE_WORKTREES_EXIST".to_string(), 20)),
        "ERROR_CODE_WORKTREES_EXIST moved off 20"
    );
}

/// The workspace additions, pinned the way the rename's numbers are
/// above: each on a tag nothing held before, so an app built before them
/// decodes every message exactly as it did.
#[test]
fn workspace_additions_take_fresh_tags() {
    use prost::Message;
    use prost_types::FileDescriptorSet;

    let set = FileDescriptorSet::decode(
        &include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..],
    )
    .expect("the build writes a descriptor");
    let file = set.file.iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .unwrap_or_else(|| panic!("no message {m}"))
            .field
            .iter()
            .find(|x| x.name() == f)
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    for (m, f, n) in [
        ("Request", "workspace_create", 80),
        ("Request", "workspace_rename", 81),
        ("Request", "workspace_set_prefix", 82),
        ("Request", "task_move", 83),
        ("Request", "worktree_assign", 84),
        ("Request", "terminal_set_role", 85),
        ("Request", "workspace_start_orchestrator", 86),
        ("Result", "workspace", 43),
        ("Result", "workspace_list", 44),
        ("Terminal", "workspace_id", 39),
        ("Terminal", "role", 40),
        ("Task", "workspace_id", 15),
        ("TaskCreate", "workspace_id", 9),
        ("TaskListRequest", "workspace_id", 4),
        ("TaskChanged", "workspace_id", 4),
        ("TaskChanged", "from_workspace_id", 5),
    ] {
        assert_eq!(number(m, f), n, "{m}.{f} moved off tag {n}");
    }
    // The last field each of these carried before, still where it was.
    assert_eq!(number("Request", "task_get_by_key"), 79);
    assert_eq!(number("Result", "task_note_hit_list"), 42);
    assert_eq!(number("Terminal", "task_id"), 37);
    assert_eq!(number("Task", "updated_at"), 14);
    assert_eq!(number("TaskChanged", "actor"), 3);
}

/// The needs-you additions, each on a tag nothing held before: the
/// event after `events_missed`, the result after `workspace_list` (27 to
/// 31 stay unused), and the worktree's tasks after its foreign writers.
#[test]
fn needs_you_takes_fresh_tags() {
    use prost::Message;
    use prost_types::FileDescriptorSet;

    let set = FileDescriptorSet::decode(
        &include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..],
    )
    .expect("the build writes a descriptor");
    let file = set.file.iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .unwrap_or_else(|| panic!("no message {m}"))
            .field
            .iter()
            .find(|x| x.name() == f)
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    for (m, f, n) in [
        ("Event", "needs_you_changed", 24),
        ("Result", "needs_you_list", 45),
        ("Worktree", "open_tasks", 14),
        ("Worktree", "lfs_pointers", 15),
    ] {
        assert_eq!(number(m, f), n, "{m}.{f} moved off tag {n}");
    }
    // The last field each of these carried before, still where it was.
    assert_eq!(number("Event", "events_missed"), 23);
    assert_eq!(number("Result", "workspace_list"), 44);
    assert_eq!(number("Worktree", "foreign_writer_workspace_ids"), 13);
    // And the gap in `Result` stays a gap.
    let result = file.message_type.iter().find(|x| x.name() == "Result").expect("Result");
    for n in 27..=31 {
        assert!(result.field.iter().all(|x| x.number() != n), "Result reused tag {n}");
    }
}

/// `report.get` on tags nothing held before: the request after
/// `workspace_set_settings`, the result after `needs_you_list`.
#[test]
fn report_takes_fresh_tags() {
    use prost::Message;
    use prost_types::FileDescriptorSet;

    let set = FileDescriptorSet::decode(
        &include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..],
    )
    .expect("the build writes a descriptor");
    let file = set.file.iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .and_then(|x| x.field.iter().find(|x| x.name() == f))
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    assert_eq!(number("Request", "report_request"), 88);
    assert_eq!(number("Request", "workspace_set_settings"), 87);
    assert_eq!(number("Result", "report"), 46);
    assert_eq!(number("Result", "needs_you_list"), 45);
    assert_eq!(capability::for_method("report.get"), Some(capability::REPORT));
    assert!(capability::ALL.contains(&capability::REPORT), "the daemon would not advertise it");
}

/// The list is asked for by a word of its own, and the daemon
/// advertises it: a method whose capability is missing from `ALL` is one
/// the daemon refuses on every runner.
#[test]
fn needs_you_list_is_gated_on_its_own_advertised_capability() {
    assert_eq!(capability::NEEDS_YOU, "needs_you");
    assert_eq!(capability::for_method("needs_you.list"), Some(capability::NEEDS_YOU));
    assert!(capability::ALL.contains(&capability::NEEDS_YOU), "the daemon would not advertise it");
    assert_eq!(capability::STREAM_SIZE_MARKERS, "stream_size_markers");
    assert!(
        capability::ALL.contains(&capability::STREAM_SIZE_MARKERS),
        "the daemon would not advertise it"
    );
}

/// Terminal names and ports (ov-234) take tags nothing held before, each
/// behind a capability of its own that the daemon advertises.
#[test]
fn terminal_names_and_ports_take_fresh_tags() {
    use prost::Message;
    use prost_types::FileDescriptorSet;

    let set = FileDescriptorSet::decode(
        &include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..],
    )
    .expect("the build writes a descriptor");
    let file = set.file.iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    let number = |m: &str, f: &str| {
        file.message_type
            .iter()
            .find(|x| x.name() == m)
            .and_then(|x| x.field.iter().find(|x| x.name() == f))
            .unwrap_or_else(|| panic!("{m} has no field {f}"))
            .number()
    };
    assert_eq!(number("Request", "terminal_rename"), 140);
    assert_eq!(number("Terminal", "ports"), 44);
    // The last field each carried before, still where it was.
    assert_eq!(number("Request", "task_worker_set"), 132);
    assert_eq!(number("Terminal", "split_of_orchestrator"), 42);
    assert_eq!(capability::for_method("terminal.rename"), Some(capability::TERMINAL_NAMES));
    for word in [capability::TERMINAL_NAMES, capability::TERMINAL_PORTS] {
        assert!(capability::ALL.contains(&word), "the daemon would not advertise {word}");
    }
}

/// `workspaces` already means worktrees and is frozen; workspaces as
/// workstreams are asked for by a word of their own.
#[test]
fn workstreams_is_not_the_frozen_worktrees_word() {
    assert_eq!(capability::WORKTREES, "workspaces", "shipped clients ask for this word");
    assert_eq!(capability::WORKSTREAMS, "workstreams");
    assert!(capability::ALL.contains(&capability::WORKSTREAMS));
    for method in [
        "workspace.list",
        "workspace.create",
        "workspace.rename",
        "workspace.set_prefix",
        "workspace.delete",
        "task.move",
        "worktree.assign",
        "terminal.set_role",
        "workspace.start_orchestrator",
    ] {
        assert_eq!(capability::for_method(method), Some(capability::WORKSTREAMS), "{method}");
    }
}

/// `Terminal.notice_task_id` is a field, so no method names its capability.
/// This is the only thing that keeps the daemon from forgetting to say it
/// has the field, which would leave every client folding nothing.
#[test]
fn notice_task_is_advertised() {
    assert_eq!(capability::NOTICE_TASK, "notice_task");
    assert!(capability::ALL.contains(&capability::NOTICE_TASK), "the daemon would not advertise it");
}

/// Every channel, so a match arm added to one list and forgotten in another
/// fails here rather than shipping.
const ALL_CHANNELS: [Channel; 4] =
    [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable];

#[test]
fn a_channel_round_trips_through_its_name() {
    for c in ALL_CHANNELS {
        assert_eq!(Channel::from_str_or_local(c.as_str()), c);
    }
}

#[test]
fn an_unknown_channel_is_local_not_stable() {
    // Defaulting the other way would let a hand-made build pass itself off
    // as a release. scripts/version.sh makes the same choice for the same
    // reason.
    assert_eq!(Channel::from_str_or_local(""), Channel::Local);
    assert_eq!(Channel::from_str_or_local("nonsense"), Channel::Local);
    assert_eq!(Channel::from_str_or_local("STABLE"), Channel::Local);
    // The names this replaced. A build stamped by an older checkout must
    // not silently land on the wrong channel — `local` is where anything
    // unreadable belongs.
    assert_eq!(Channel::from_str_or_local("release"), Channel::Local);
    assert_eq!(Channel::from_str_or_local("beta"), Channel::Local);
    assert_eq!(Channel::from_str_or_local("dev"), Channel::Local);
}

#[test]
fn the_stamped_channel_is_one_of_the_four() {
    assert!(ALL_CHANNELS.contains(&CHANNEL));
}

#[test]
fn an_unknown_method_names_no_capability() {
    // What makes "this runner is too old" distinguishable from "no such
    // thing": a method nobody implements has no capability, and the daemon
    // turns that into CAPABILITY_UNSUPPORTED rather than NOT_FOUND.
    assert_eq!(capability::for_method("something.invented"), None);
    assert_eq!(capability::for_method(""), None);
    // Exact, as the daemon's scope table always was. This used to answer
    // `layout` for any name under `layout.`, so a layout verb this build
    // lacks read as a capability it has.
    assert_eq!(capability::for_method("layout.nonsense"), None);
}

#[test]
fn the_floor_capabilities_are_always_present() {
    // Every daemon that has ever existed does worktrees and terminals, so
    // a client may assume them without asking. If either ever left this
    // list, every shipped client would break at once.
    assert!(capability::ALL.contains(&capability::WORKTREES));
    assert!(capability::ALL.contains(&capability::TERMINALS));
}

#[test]
fn release_binaries_keep_their_bare_names() {
    // Every release client already in the field resolves
    // `~/.local/bin/farcoolerd`. A suffix here strands all of them at once,
    // and an App Store build cannot be corrected for days.
    assert_eq!(Channel::Stable.daemon_binary_name(), "farcoolerd");
    assert_eq!(Channel::Stable.cli_binary_name(), "farcooler");
    assert_eq!(Channel::Stable.tunnel_binary_name(), "farcooler-tunnel");
}

#[test]
fn each_channel_has_its_own_binary_name() {
    for names in [
        ALL_CHANNELS.map(Channel::daemon_binary_name),
        ALL_CHANNELS.map(Channel::cli_binary_name),
        ALL_CHANNELS.map(Channel::tunnel_binary_name),
    ] {
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), ALL_CHANNELS.len(), "two channels cannot share one path: {names:?}");
    }
}

#[test]
fn a_lookup_prefers_this_channels_name_over_the_bare_one() {
    // The order is the whole isolation property. A `~/.local/bin` holding
    // both a stable install and a preview one holds two files, and a
    // preview client that checked the bare name first would find the
    // stable daemon sitting there and talk to it — which is precisely the
    // meeting the channels exist to prevent.
    for c in ALL_CHANNELS {
        assert_eq!(c.daemon_binary_candidates().first(), Some(&c.daemon_binary_name()));
        assert_eq!(c.cli_binary_candidates().first(), Some(&c.cli_binary_name()));
        assert_eq!(c.tunnel_binary_candidates().first(), Some(&c.tunnel_binary_name()));
    }
}

#[test]
fn every_channel_also_answers_to_the_name_cargo_gives_it() {
    // `[[bin]] name = "farcoolerd"` is what a build produces whatever
    // channel stamped it, and `build-linux.sh`, `release.yml` and
    // `build-app.sh` all copy that name out of `target/` unchanged. A
    // lookup that accepted only the channel name would mean a checkout
    // stops being able to find what it just built the moment it is not
    // stable.
    for c in ALL_CHANNELS {
        assert!(c.daemon_binary_candidates().contains(&"farcoolerd"), "{c:?}");
        assert!(c.cli_binary_candidates().contains(&"farcooler"), "{c:?}");
        assert!(c.tunnel_binary_candidates().contains(&"farcooler-tunnel"), "{c:?}");
    }
}

#[test]
fn stable_looks_for_exactly_one_name() {
    // Its channel name and cargo's name are the same string, and a list
    // that repeated it would make every caller do redundant work and
    // every error message list a path twice.
    assert_eq!(Channel::Stable.daemon_binary_candidates(), &["farcoolerd"]);
    assert_eq!(Channel::Stable.cli_binary_candidates(), &["farcooler"]);
    assert_eq!(Channel::Stable.tunnel_binary_candidates(), &["farcooler-tunnel"]);
}
