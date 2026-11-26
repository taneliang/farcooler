/// **A connection that fell behind is a resync** (ov-20 R-M9): the daemon
/// dropped events addressed to it, and the one answer to "you missed some"
/// is to re-read everything, which is what the phones already do on the
/// queue's own overflow.
#[test]
fn events_missed_is_a_resync() {
    use farcooler_protocol::v1::event::Payload;
    assert_eq!(
        super::FleetEvent::of(Payload::EventsMissed(farcooler_protocol::v1::Empty {})),
        Some(super::FleetEvent::Resync)
    );
}

/// The needs-you event is news of its own, not `Fleet` and not nothing:
/// an answered decision moves no terminal, so a client that read this as
/// anything else would never re-read the list.
#[test]
fn a_needs_you_event_is_a_fleet_event() {
    use farcooler_protocol::v1::event::Payload;
    assert_eq!(
        super::FleetEvent::of(Payload::NeedsYouChanged(farcooler_protocol::v1::Empty {})),
        Some(super::FleetEvent::NeedsYou)
    );
}

/// A runner that does not advertise `tasks` is refused a board read here,
/// with the code the runner itself would have used, and one that does is
/// let through. An empty list is an old runner, which has no board.
#[test]
fn a_board_read_is_refused_by_name_on_a_runner_without_a_board() {
    use farcooler_protocol::capability::TASKS;
    let old: Vec<String> = Vec::new();
    let without = vec!["workspaces".to_string(), "terminals".to_string()];
    let with = vec!["workspaces".to_string(), TASKS.to_string()];
    for advertised in [&old, &without] {
        match super::require(advertised, TASKS, "task.list") {
            Err(super::SessionError::Refused { code, retryable, .. }) => {
                assert_eq!(
                    code,
                    farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32
                );
                assert!(!retryable, "asking again will not grow the runner a board");
            }
            other => panic!("{advertised:?} was not refused: {other:?}"),
        }
    }
    assert!(super::require(&with, TASKS, "task.list").is_ok());
}

/// A fleet row names its workspace, how it was claimed, who else is
/// writing in it, and the tasks working in it; each of its terminals names its own workspace and role.
/// A pane in another worktree does not lend its fields to this one's.
#[test]
fn a_fleet_row_carries_its_workspace_and_its_panes_roles() {
    let b = |u: Uuid| bytes::Bytes::copy_from_slice(u.as_bytes());
    let (lane, other, main, billing) =
        (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let w = Worktree {
        id: b(lane),
        workspace_id: Some(b(main)),
        claim_source: Some("hook".into()),
        foreign_writer_workspace_ids: vec![b(billing)],
        lfs_pointers: 4,
        open_tasks: vec![farcooler_protocol::v1::TaskRef {
            id: b(other),
            key: "bil-9".into(),
            title: "Invoice PDF export".into(),
            status: farcooler_protocol::v1::TaskStatus::InReview as i32,
        }],
        ..Default::default()
    };
    let pane = |worktree: Uuid, workspace: Uuid, role: farcooler_protocol::v1::TerminalRole| {
        Terminal {
            worktree_id: b(worktree),
            workspace_id: Some(b(workspace)),
            role: role as i32,
            ..Default::default()
        }
    };
    use farcooler_protocol::v1::TerminalRole;
    let terminals = vec![
        pane(other, billing, TerminalRole::Orchestrator),
        pane(lane, billing, TerminalRole::Agent),
        Terminal { worktree_id: b(lane), ..Default::default() },
    ];
    let known = vec![farcooler_protocol::v1::Workspace {
        id: b(billing),
        name: "Billing".into(),
        ..Default::default()
    }];
    let row = serde_json::json!({ "terminals": [{ "id": "a" }, { "id": "b" }] });
    let row = super::with_workspaces(row, &w, &terminals, &known);
    assert_eq!(row["workspace"], main.to_string(), "{row}");
    assert_eq!(row["claim_source"], "hook");
    assert_eq!(row["foreign_writers"], serde_json::json!(["Billing"]));
    assert_eq!(row["lfs_pointers"], 4, "how many large files weren't downloaded (ov-199)");
    assert_eq!(
        row["open_tasks"],
        serde_json::json!([{
            "id": other.to_string(),
            "key": "bil-9",
            "title": "Invoice PDF export",
            "status": "in_review",
        }]),
        "{row}"
    );
    assert_eq!(row["terminals"][0]["workspace"], billing.to_string(), "{row}");
    assert_eq!(row["terminals"][0]["role"], "agent", "another worktree's pane lent its role");
    assert!(row["terminals"][1]["workspace"].is_null(), "{row}");
    assert!(row["terminals"][1]["role"].is_null(), "an old runner's pane was given a role");
}

/// A board read keyed by workspace asks for that board, and says it needs
/// `workstreams` to; on a runner without it, the read is the repository's
/// one board, and nothing is required that the runner would refuse.
#[test]
fn a_board_read_by_workspace_names_the_board_only_where_the_runner_has_boards() {
    use farcooler_protocol::capability::{TASKS, WORKSTREAMS};
    let repository = Uuid::now_v7();
    let billing = Uuid::now_v7();
    let new = vec![TASKS.to_string(), WORKSTREAMS.to_string()];
    let (list, required) = super::task_list_request(&new, repository, Some(billing));
    assert_eq!(list.workspace_id.as_deref(), Some(billing.as_bytes().as_slice()));
    assert_eq!(list.repository_id.as_ref(), repository.as_bytes());
    assert_eq!(required, vec![WORKSTREAMS.to_string()]);

    let (whole, required) = super::task_list_request(&new, repository, None);
    assert_eq!(whole.workspace_id, None, "no workspace asked for is the whole repository");
    assert!(required.is_empty());

    let old = vec![TASKS.to_string()];
    let (list, required) = super::task_list_request(&old, repository, Some(billing));
    assert_eq!(list.workspace_id, None, "an old runner was sent a board it cannot name");
    assert!(required.is_empty(), "an old runner would refuse the read outright");
    assert_eq!(list.repository_id.as_ref(), repository.as_bytes());
}

/// A worktree made from a phone is claimed for its repository's Main on
/// a runner with workstreams, naming the capability, and for nothing on
/// one without them.
#[test]
fn a_worktree_made_from_a_phone_is_claimed_for_main() {
    use farcooler_protocol::v1 as pb;
    let bytes = |u: Uuid| bytes::Bytes::copy_from_slice(u.as_bytes());
    let (repository, other) = (Uuid::now_v7(), Uuid::now_v7());
    let (main, billing, elsewhere) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let listed = [
        pb::Workspace { id: bytes(elsewhere), repository_id: bytes(other), is_main: true, ..Default::default() },
        pb::Workspace { id: bytes(billing), repository_id: bytes(repository), ..Default::default() },
        pb::Workspace { id: bytes(main), repository_id: bytes(repository), is_main: true, ..Default::default() },
    ];
    let (claim, required) = super::claim(repository, None, Some(&listed));
    assert_eq!(claim, Some(bytes(main)), "this repository's Main");
    assert_eq!(required, vec![farcooler_protocol::capability::WORKSTREAMS.to_string()]);

    assert_eq!(super::claim(repository, None, None), (None, Vec::new()), "a runner without workstreams");
    assert_eq!(super::claim(repository, None, Some(&listed[..2])), (None, Vec::new()), "no Main listed");
}

/// A pane's task is a uuid string or nothing: never the nil uuid, which
/// would match no card and be drawn as a link to nowhere.
#[test]
fn a_pane_names_its_task_or_nothing() {
    let task = uuid::Uuid::now_v7();
    let mut t = farcooler_protocol::v1::Terminal {
        task_id: Some(bytes::Bytes::copy_from_slice(task.as_bytes())),
        ..Default::default()
    };
    assert_eq!(super::task_of(&t), Some(task.to_string()));
    t.task_id = None;
    assert_eq!(super::task_of(&t), None);
    t.task_id = Some(bytes::Bytes::new());
    assert_eq!(super::task_of(&t), None, "empty bytes are not the nil uuid");
    t.task_id = Some(bytes::Bytes::from_static(b"short"));
    assert_eq!(super::task_of(&t), None);
}

/// A split names the pane it was split from, and anything else names
/// none: never the nil uuid.
#[test]
fn a_pane_names_what_it_was_split_from_or_nothing() {
    let from = uuid::Uuid::now_v7();
    let w = farcooler_protocol::v1::Worktree::default();
    let pane = |split_of| farcooler_protocol::v1::Terminal { split_of, ..Default::default() };
    let terminals = [
        pane(Some(bytes::Bytes::copy_from_slice(from.as_bytes()))),
        pane(None),
        pane(Some(bytes::Bytes::new())),
        pane(Some(bytes::Bytes::from_static(b"short"))),
    ];
    let row = serde_json::json!({ "terminals": [{}, {}, {}, {}] });
    let row = super::with_workspaces(row, &w, &terminals, &[]);
    assert_eq!(row["terminals"][0]["splitOf"], from.to_string(), "{row}");
    for i in 1..4 {
        assert_eq!(row["terminals"][i]["splitOf"], serde_json::json!(null), "{row}");
    }
}

/// Whether a split was made from the orchestrator rides beside `splitOf`,
/// either way, and never without it (ov-76).
#[test]
fn a_pane_says_whether_it_was_split_from_the_orchestrator() {
    let from = Some(bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()));
    let w = farcooler_protocol::v1::Worktree::default();
    let pane = |split_of, said| farcooler_protocol::v1::Terminal {
        split_of,
        split_of_orchestrator: said,
        ..Default::default()
    };
    let terminals =
        [pane(from.clone(), Some(true)), pane(from.clone(), Some(false)), pane(from, None), pane(None, Some(true))];
    let row = serde_json::json!({ "terminals": [{}, {}, {}, {}] });
    let row = super::with_workspaces(row, &w, &terminals, &[]);
    assert_eq!(row["terminals"][0]["splitOfOrchestrator"], true, "{row}");
    assert_eq!(row["terminals"][1]["splitOfOrchestrator"], false, "{row}");
    assert_eq!(row["terminals"][2]["splitOfOrchestrator"], serde_json::json!(null), "{row}");
    assert_eq!(row["terminals"][3]["splitOfOrchestrator"], serde_json::json!(null), "{row}");
}

/// The fleet trace's anchor reaches the app as its own number, and never
/// without the trace it anchors.
#[test]
fn the_fleet_traces_anchor_travels_only_with_the_trace() {
    let list = farcooler_protocol::v1::TerminalList {
        items: Vec::new(),
        fleet_trace: vec![0x10; farcooler_core::trace::ENCODED_LEN].into(),
        fleet_trace_anchor: Some(5_960_000),
    };
    assert_eq!(super::fleet_trace_anchor(&list), Some(5_960_000));
    assert_eq!(super::fleet_trace(&list).map(|t| t.len()), Some(88));

    // Spelled exactly as `CoreModel.Fleet` decodes it, and a number. A
    // rename here would be a phone that silently packs every runner again.
    let sent = super::fleet_json(true, serde_json::json!(1), &list, Vec::new(), None);
    assert_eq!(sent["fleetTraceAnchor"], serde_json::json!(5_960_000), "{sent}");
    assert!(sent["fleetTrace"].is_string(), "{sent}");

    let bare = farcooler_protocol::v1::TerminalList {
        items: Vec::new(),
        fleet_trace: Default::default(),
        fleet_trace_anchor: Some(5_960_000),
    };
    assert_eq!(super::fleet_trace_anchor(&bare), None, "an anchor for no trace");
    assert_eq!(super::fleet_trace(&bare), None);
    let quiet = super::fleet_json(true, serde_json::json!(0), &bare, Vec::new(), None);
    assert!(quiet["fleetTraceAnchor"].is_null(), "{quiet}");
}

/// The fleet carries the workstreams as `workspaces` from a runner that
/// has them, and no such key from one that does not.
///
/// Absent and empty are different claims: absent is a runner without
/// `workstreams`, which an app draws as one implicit workspace per
/// repository; empty would be a runner with workstreams and nothing
/// registered. The key is the envelope's, not AgentKit's persisted
/// `RunnerDirectory` `"workspaces"`, which holds worktrees.
#[test]
fn the_fleet_names_its_workstreams_only_when_the_runner_has_them() {
    let list = farcooler_protocol::v1::TerminalList::default();
    let main = serde_json::json!({ "id": "m", "name": "Main", "is_main": true });
    let with = super::fleet_json(true, serde_json::json!(0), &list, Vec::new(), Some(vec![main]));
    assert_eq!(with["workspaces"][0]["name"], "Main", "{with}");
    let empty = super::fleet_json(true, serde_json::json!(0), &list, Vec::new(), Some(Vec::new()));
    assert_eq!(empty["workspaces"], serde_json::json!([]), "{empty}");
    let old = super::fleet_json(true, serde_json::json!(0), &list, Vec::new(), None);
    assert!(old.get("workspaces").is_none(), "{old}");
}

use super::*;

/// Serializes the tests that move the tunnel library's process-wide DERP
/// map. `cargo test` runs this crate's tests on many threads in one
/// process.
///
/// `tokio`'s mutex and not `std`'s, because the section being serialized
/// contains an `.await`: a `std` guard held across one is what
/// `clippy::await_holding_lock` is about, and the runtime is free to
/// resume the task on another thread.
static DERP_MAP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A destination this connect cannot possibly complete.
///
/// The private key is not a key, so `ssh::Session::open` refuses before it
/// reaches a network — which is what makes the tests below fast, offline,
/// and about only the one thing they assert.
fn unreachable(derp_map: &str) -> ssh::Destination {
    ssh::Destination {
        reach: ssh::Reach::Direct { host: "127.0.0.1".into(), port: 1 },
        user: "u".into(),
        private_key: "not a key".into(),
        passphrase: None,
        host_key: ssh::HostKeyPolicy::Accept,
        derp_map: derp_map.to_string(),
    }
}

/// A configured DERP map reaches the tunnel library on the way into a
/// connect, not after it.
///
/// Asserted through `connect_ssh` — the function both phone apps' connect
/// actually calls — and read back out of the tunnel library rather than
/// off the struct this test just filled in. The connect fails, and it has
/// to: the rendezvous is set on the way past, so the only question left
/// is whether that happened before the dial or not at all.
#[tokio::test]
async fn a_configured_derp_map_is_applied_on_the_way_into_a_connect() {
    let _serial = DERP_MAP.lock().await;
    farcooler_tailcat::set_derp_map_url("");
    let _ = Session::connect_ssh(&unreachable("https://derp.example/derpmap.json")).await;
    assert_eq!(
        farcooler_tailcat::derp_map_url(),
        "https://derp.example/derpmap.json",
        "the configured DERP map never reached the tunnel"
    );
    farcooler_tailcat::set_derp_map_url("");
}

/// An app that configured nothing does not CLEAR what something else
/// configured.
///
/// Empty already means "the library's own default" one layer down, so
/// sending it would be a no-op for a phone and a quiet unsetting for any
/// process that reads `FARCOOLER_DERP_MAP` and then opens a client
/// session of its own — which would move that process off its own
/// rendezvous on the way past, at the moment it was dialing.
#[tokio::test]
async fn an_unset_derp_map_clears_nothing() {
    let _serial = DERP_MAP.lock().await;
    farcooler_tailcat::set_derp_map_url("https://derp.example/already-set.json");
    let _ = Session::connect_ssh(&unreachable("")).await;
    assert_eq!(
        farcooler_tailcat::derp_map_url(),
        "https://derp.example/already-set.json",
        "a connect that configured nothing unset the rendezvous anyway"
    );
    farcooler_tailcat::set_derp_map_url("");
}

/// A board write on the wire becomes news a client can act on.
///
/// Asserted through `FleetEvent::of` — the one function that turns a
/// payload into news — and not by checking that the variant exists. The
/// bug this replaces was not a missing variant; it was a present payload
/// mapped to `None`, which every other test in this crate was happy with,
/// because a dropped notice looks exactly like a quiet fleet. Put the arm
/// back to `None` and this is the test that goes red.
#[test]
fn a_board_write_becomes_news_rather_than_being_dropped() {
    use farcooler_protocol::v1::event::Payload;
    let repository = Uuid::now_v7();
    let news = FleetEvent::of(Payload::TaskChanged(farcooler_protocol::v1::TaskChanged {
        task_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        actor: "agent:0198f2c0-0000-7000-8000-000000000001".into(),
        ..Default::default()
    }));
    assert_eq!(
        news,
        Some(FleetEvent::Task {
            repository,
            workspace: None,
            from_workspace: None,
            // Verbatim, which is what lets a client compare it against the
            // word it writes with. A client that had to reconstruct this
            // from a parsed kind and id would be the second place the
            // vocabulary is spelled out.
            actor: "agent:0198f2c0-0000-7000-8000-000000000001".into(),
        }),
        "a board write reached a client as nothing at all"
    );
}

/// A board event names its board, and a move names both boards.
///
/// The half of the per-workspace re-read that lives here: the daemon sends
/// `workspace_id` on every `TaskChanged` and `from_workspace_id` on a
/// move, and a client keyed by workspace can only re-read the right board
/// if both cross. Dropping either still delivers a re-read, so nothing
/// else would notice: a board would simply stop hearing its own writes, or
/// keep a card a move took away.
#[test]
fn a_board_event_names_its_board_and_a_move_names_both() {
    use farcooler_protocol::v1::event::Payload;
    let bytes = |u: Uuid| Some(bytes::Bytes::copy_from_slice(u.as_bytes()));
    let repository = Uuid::now_v7();
    let billing = Uuid::now_v7();
    let main = Uuid::now_v7();
    let moved = FleetEvent::of(Payload::TaskChanged(farcooler_protocol::v1::TaskChanged {
        task_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        actor: "user".into(),
        workspace_id: bytes(main),
        from_workspace_id: bytes(billing),
    }));
    assert_eq!(
        moved,
        Some(FleetEvent::Task {
            repository,
            workspace: Some(main),
            from_workspace: Some(billing),
            actor: "user".into(),
        })
    );
    // A runner without `workstreams`, and bytes that are no id at all:
    // absent, never the nil uuid, which would be a board of its own.
    for workspace_id in [None, Some(bytes::Bytes::new()), bytes(Uuid::nil())] {
        let old = FleetEvent::of(Payload::TaskChanged(farcooler_protocol::v1::TaskChanged {
            repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
            workspace_id: workspace_id.clone(),
            ..Default::default()
        }));
        let Some(FleetEvent::Task { workspace, from_workspace, .. }) = old else {
            panic!("{workspace_id:?} did not arrive as board news");
        };
        assert_eq!((workspace, from_workspace), (None, None), "{workspace_id:?}");
    }
}

/// The three actor words the daemon can send all survive the crossing.
///
/// Cheap, and it guards the failure that would be invisible: an arm that
/// built the variant but dropped or normalized the actor still delivers a
/// re-read, so the board would look fine and only the "was this mine"
/// question would quietly always answer no.
#[test]
fn every_actor_word_crosses_unchanged() {
    use farcooler_protocol::v1::event::Payload;
    for word in ["user", "manager", "agent:0198f2c0-0000-7000-8000-000000000001"] {
        let news = FleetEvent::of(Payload::TaskChanged(farcooler_protocol::v1::TaskChanged {
            task_id: bytes::Bytes::new(),
            repository_id: bytes::Bytes::new(),
            actor: word.to_string(),
            ..Default::default()
        }));
        let Some(FleetEvent::Task { actor, .. }) = news else {
            panic!("{word} did not arrive as board news at all");
        };
        assert_eq!(actor, word, "the actor was rewritten on the way across");
    }
}

/// A payload with a reader keeps it, and one without stays without.
///
/// The pair is the point. Adding the board arm is one line away from
/// answering `Some` for `TerminalFrame` too, which would spend a fleet
/// re-read on every chunk of terminal output a busy runner produces.
#[test]
fn a_payload_with_no_fleet_reader_is_still_nothing() {
    use farcooler_protocol::v1::event::Payload;
    assert_eq!(
        FleetEvent::of(Payload::FleetChanged(farcooler_protocol::v1::Empty {})),
        Some(FleetEvent::Fleet)
    );
    assert_eq!(
        FleetEvent::of(Payload::TerminalFrame(
            farcooler_protocol::v1::TerminalFrame::default()
        )),
        None,
        "terminal bytes became a fleet re-read"
    );
}

#[test]
fn short_ids_use_the_random_tail_not_the_timestamp_head() {
    // Two ids minted in the same millisecond share a leading prefix, so a
    // head-based short id would collide constantly.
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    assert_ne!(short(a.as_bytes()), short(b.as_bytes()));
    assert_eq!(short(a.as_bytes()).len(), 8);
}

#[test]
fn a_wrong_result_variant_names_both_sides() {
    let error = wrong("terminal", &result::Value::Worktree(Worktree::default()));
    let message = error.to_string();
    assert!(message.contains("terminal") && message.contains("worktree"));
}

/// A scope crosses as a word, and the two directions agree.
///
/// They have to: the word this sends is the word the daemon writes into the
/// forced command, and the word it reads back out is the one this labels. A
/// pair that disagreed would enroll a device at one scope and then report
/// another.
#[test]
fn a_scope_is_the_same_word_in_both_directions() {
    for word in ["read", "control", "host_admin"] {
        let scope = scope_from_word(word).expect("a scope this build has");
        assert_eq!(scope_label(scope as i32), word);
    }
}

/// A word nobody has is refused, never rounded up.
///
/// `unspecified` included: it is a state a line can be found in, not a grant
/// a caller may ask for, and the daemon refuses it too — an unscoped line
/// already means host_admin to sshd, so accepting the word would turn a
/// forgotten field into the whole runner.
#[test]
fn a_scope_word_this_build_does_not_have_is_refused() {
    for word in ["admin", "", "READ", "unspecified", "host-admin"] {
        assert!(scope_from_word(word).is_none(), "{word} was accepted");
    }
}

/// A foreign line reports what it is: no device, no grant.
#[test]
fn a_line_far_cooler_did_not_write_grants_nothing_and_names_nobody() {
    let hand_written = farcooler_protocol::v1::EnrolledClient {
        client_id: String::new(),
        fingerprint: "SHA256:whatever".into(),
        label: "me@laptop".into(),
        scope: farcooler_protocol::v1::Scope::Unspecified as i32,
        account: "you".into(),
        enrolled_at: 0,
        foreign: true,
        // Never true for a line Far Cooler did not write: a plain line of
        // OURS is one this program rendered and manages, and a stranger's
        // key is left exactly as it was found.
        shell_access: false,
        added_by: String::new(),
    };
    let json = enrolled_client_json(&hand_written);
    assert_eq!(json["foreign"], true);
    assert_eq!(json["clientId"], "");
    assert_eq!(json["scope"], "unspecified", "reading an unscoped line as admin would lie");
    assert_eq!(json["enrolledAt"], 0);
    assert_eq!(json["shellAccess"], false);
}

/// The two lines of one Mac are distinguishable in the JSON three apps decode.
///
/// The point of the field. Both lines carry the same client id and both are
/// ours, so `shellAccess` is the only thing separating "Far Cooler access"
/// from "shell access" on screen — and the copy under the second row promises
/// that removing it takes ssh, git and Zed away. A row that could not tell
/// which it was would make that promise about the wrong key.
#[test]
fn a_mac_s_two_lines_are_told_apart_by_the_field_the_rows_need() {
    let plain = farcooler_protocol::v1::EnrolledClient {
        client_id: "mac-9".into(),
        fingerprint: "SHA256:b".into(),
        label: "farcooler-shell-macbook-air-1a2b3c4d".into(),
        // A plain line carries no forced command, so it carries no scope
        // word either — there is nowhere in the line to put one.
        scope: farcooler_protocol::v1::Scope::Unspecified as i32,
        account: "you".into(),
        enrolled_at: 0,
        foreign: false,
        shell_access: true,
        added_by: String::new(),
    };
    let restricted = farcooler_protocol::v1::EnrolledClient {
        fingerprint: "SHA256:a".into(),
        label: "farcooler-macbook-air-1a2b3c4d".into(),
        scope: farcooler_protocol::v1::Scope::Control as i32,
        shell_access: false,
        added_by: String::new(),
        ..plain.clone()
    };
    let plain = enrolled_client_json(&plain);
    let restricted = enrolled_client_json(&restricted);
    assert_eq!(plain["clientId"], restricted["clientId"], "one device, two lines");
    assert_eq!(plain["shellAccess"], true);
    assert_eq!(restricted["shellAccess"], false);
    assert_eq!(plain["foreign"], false, "a plain line of ours is managed, not a stranger's");
}

#[test]
fn silence_from_the_far_side_reads_as_a_missing_daemon() {
    // The single most common remote failure, and it must not surface as a
    // protocol error nobody can act on.
    let error: SessionError = ClientError::Closed.into();
    assert!(matches!(error, SessionError::DaemonMissing { .. }));
    assert!(error.to_string().contains("installed"));
    // It names the binary this build actually asked for. A preview client that
    // reported `farcoolerd` would send someone to check the wrong thing,
    // and finding that installed would make the message look like a lie.
    assert!(error.to_string().contains(daemon_binary()));
}

#[test]
fn a_dead_socket_is_told_apart_from_a_confused_one() {
    // The distinction the whole reconnect story rests on. An io error
    // reading a frame means the link is gone and reconnecting is the fix;
    // bytes that do not decode mean one side has a bug, and reconnecting
    // over it would just reproduce it on every attempt.
    let broken: SessionError =
        ClientError::Codec(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()).into();
    assert!(matches!(broken, SessionError::Disconnected(_)));
    assert!(broken.is_disconnect());

    let truncated: SessionError = ClientError::Codec(CodecError::Truncated).into();
    assert!(truncated.is_disconnect());

    let garbage: SessionError = ClientError::Codec(CodecError::Framing(
        farcooler_protocol::framing::FramingError::Malformed,
    ))
    .into();
    assert!(matches!(garbage, SessionError::Protocol(_)));
    assert!(!garbage.is_disconnect());
}

#[test]
fn a_refused_request_leaves_the_session_alone() {
    // Everything the daemon answers — badly, or with the wrong shape — is
    // a session still worth talking on. Treating these as drops would
    // reconnect on every typo'd method a client ever sends.
    assert!(!SessionError::Protocol("nope".into()).is_disconnect());
    assert!(!SessionError::WrongResult { expected: "host", got: "worktree" }.is_disconnect());
    assert!(!SessionError::VersionMismatch { daemon: 2, client: 1 }.is_disconnect());
}

#[test]
fn a_missing_daemon_means_two_things_depending_on_when_it_is_asked() {
    // At connect time it is a diagnosis: go install Far Cooler over there.
    // Mid-session it can only mean the pipe closed under us, so it counts
    // as a drop — and the message stays the useful one either way.
    let missing = SessionError::DaemonMissing { daemon: daemon_binary() };
    assert!(missing.is_disconnect());
    assert!(missing.to_string().contains("installed"));
}

/// The three fields the daemon computes and this used to throw away.
///
/// `head_oid` is the only thing that can prove a squash-merged branch holds
/// nothing unmerged, `merged_at` is when it landed, and `fetched_at` is what
/// `stale` is derived from — all three already crossed the wire and stopped
/// at this function. Pinned here because this is the shape both apps decode:
/// a key that stops being written is a feature that stops existing on every
/// client at once.
#[test]
fn a_pr_carries_every_field_the_daemon_computed_for_it() {
    let list = farcooler_protocol::v1::StackLinkList {
        repository_id: bytes::Bytes::new(),
        items: vec![farcooler_protocol::v1::StackLink {
            branch: "feat/x".into(),
            parent_branch: "main".into(),
            parent_source: farcooler_protocol::v1::ParentSource::Recorded as i32,
            head_commit: "aaa".into(),
            ahead: 1,
            behind: 0,
            pr: Some(farcooler_protocol::v1::PrStatus {
                number: 335,
                url: "https://github.com/o/r/pull/335".into(),
                state: farcooler_protocol::v1::PrState::Open as i32,
                checks: farcooler_protocol::v1::CheckState::Passing as i32,
                review_decision: farcooler_protocol::v1::ReviewDecision::Approved as i32,
                head_oid: "deadbeef".into(),
                merged_at: Some(1_785_925_777_000),
                fetched_at: 1_785_925_800_000,
                stale: true,
            }),
        }],
        cycle_detected: false,
        pr_known: true,
        repo_url: "https://github.com/o/r".into(),
    };

    let json = stack_json(&list);
    let pr = &json["links"][0]["pr"];
    assert_eq!(pr["headOid"], "deadbeef");
    assert_eq!(pr["mergedAt"], 1_785_925_777_000i64);
    assert_eq!(pr["fetchedAt"], 1_785_925_800_000i64);
    assert_eq!(pr["stale"], true);
}

/// "GitHub says there is no PR" and "we could not ask GitHub" are the same
/// absent `pr` on every link, and a client that could not tell them apart
/// would offer to create a pull request that already exists.
#[test]
fn a_read_says_whether_github_answered_at_all() {
    let mut list = farcooler_protocol::v1::StackLinkList {
        repository_id: bytes::Bytes::new(),
        items: Vec::new(),
        cycle_detected: false,
        pr_known: false,
        repo_url: String::new(),
    };
    let json = stack_json(&list);
    assert_eq!(json["prKnown"], false);
    // Empty is not a URL. A client must get nothing rather than a link to
    // nowhere, so the key is null and not "".
    assert!(json["repoUrl"].is_null(), "an unknown repository URL is absent, not empty");

    list.pr_known = true;
    list.repo_url = "https://github.example/o/r".into();
    let json = stack_json(&list);
    assert_eq!(json["prKnown"], true);
    assert_eq!(json["repoUrl"], "https://github.example/o/r");
}

#[test]
fn a_terminal_reports_its_pane_mode_as_a_word_a_client_can_switch_on() {
    // Numbers would make every client carry a copy of the enum and drift
    // from it. The label is the daemon's answer, not a code to look up.
    assert_eq!(pane_mode_label(farcooler_protocol::v1::PaneMode::Terminal as i32), "terminal");
    // A runner from before chat mode was retired sends 2 for a chat pane
    // (ov-377): its TUI is what a client draws.
    assert_eq!(pane_mode_label(2), "terminal");
    // An unknown value is the mode that always works, not a guess.
    assert_eq!(pane_mode_label(99), "terminal");
}
