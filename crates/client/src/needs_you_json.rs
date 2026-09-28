//! What a needs-you list looks like once it leaves the wire.
//!
//! One implementation, two callers, for `changes_json`'s reason. The Mac reads
//! `farcooler needs-you --json`, and the phones read `Session::needs_you`
//! through the FFI's `needs_you`. AgentKit and Kotlin each decode it with ONE
//! decoder, pinned by `test/fixtures/needs-you.json`, which the tests below
//! check this module writes.
//!
//! Keys are snake_case, the proto's own field names, as `tasks_json`'s are.
//! Every optional field is a key that is present and null when unset, never a
//! missing key, so a decoder's "absent" and "empty" can't mean two things.
//!
//! `open_tasks` on a worktree is here too: its rows are the same `TaskRef` an
//! item's `task` is, and the two must not be two copies that merely agree.

use farcooler_protocol::v1 as pb;
use serde_json::json;

use crate::session::{pane_mode_label, some_uuid, uuid_of};
use crate::tasks_json::status_word;
use crate::workspaces_json::role_word;

/// The list: `{"items": [...]}`, in the daemon's order, which is rank order.
pub fn needs_you_json(list: &pb::NeedsYouList) -> serde_json::Value {
    json!({ "items": list.items.iter().map(item_json).collect::<Vec<_>>() })
}

/// One item.
///
/// Below Control scope the daemon leaves `detail`, `ask_id`, `actions` and
/// `worktree` out, and `question` is a fixed sentence. They arrive here as
/// null and `[]`, which is what a Read-scoped app draws without buttons.
pub fn item_json(item: &pb::NeedsYouItem) -> serde_json::Value {
    json!({
        "id": item.id,
        "kind": kind_word(item.kind),
        "also": item.also.iter().map(|k| kind_word(*k)).collect::<Vec<_>>(),
        // SMALLER sorts FIRST, on `Terminal.rank`'s scale, and it's a duration,
        // not a clock: two runners' items merge by it without comparing clocks.
        "rank": item.rank,
        // Unix milliseconds, or null from a daemon that didn't say.
        "since": item.since.as_ref().map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000),
        // Null for none, never the nil uuid: an app keys a workspace by this.
        "workspace_id": some_uuid(Some(&item.workspace_id)).map(|u| u.to_string()),
        "workspace_name": item.workspace_name,
        "repository_id": some_uuid(Some(&item.repository_id)).map(|u| u.to_string()),
        "task": item.task.as_ref().map(task_ref_json),
        "terminal": item.terminal.as_ref().map(|t| json!({
            "id": uuid_of(&t.id).to_string(),
            "worktree_id": some_uuid(Some(&t.worktree_id)).map(|u| u.to_string()),
            // The pane's current command (`claude`), falling back to its
            // title: what a push notice calls the agent.
            "label": t.label,
            "role": role_word(t.role),
            "pane_mode": pane_mode_label(t.pane_mode),
            "chat_capable": t.chat_capable,
        })),
        "worktree": item.worktree.as_ref().map(|w| json!({
            "id": uuid_of(&w.id).to_string(),
            "name": w.name,
            "branch": w.branch,
            "insertions": w.insertions,
            "deletions": w.deletions,
        })),
        "question": item.question,
        "detail": item.detail,
        // What `terminal.agent_answer` takes as its request id.
        "ask_id": item.ask_id,
        "actions": item.actions.iter().map(|a| json!({
            "id": a.id,
            "title": a.title,
            "destructive": a.destructive,
            "primary": a.primary,
        })).collect::<Vec<_>>(),
    })
}

/// A task named from somewhere else: an item's `task`, and each row of a
/// worktree's `open_tasks`.
pub fn task_ref_json(t: &pb::TaskRef) -> serde_json::Value {
    json!({
        "id": uuid_of(&t.id).to_string(),
        "key": t.key,
        "title": t.title,
        "status": status_word(t.status),
    })
}

/// A worktree's `open_tasks`: every task working in it that isn't Done or
/// Cancelled. `[]` from a runner too old to fill it, which reads as "no task",
/// and that is what such a runner can say.
pub fn open_tasks_json(tasks: &[pb::TaskRef]) -> serde_json::Value {
    json!(tasks.iter().map(task_ref_json).collect::<Vec<_>>())
}

/// A kind as a word. `unknown` for a number this build doesn't define, never a
/// guess: an app sorts an unknown kind last rather than dropping it.
pub fn kind_word(raw: i32) -> &'static str {
    match pb::NeedsYouKind::try_from(raw) {
        Ok(pb::NeedsYouKind::Ask) => "ask",
        Ok(pb::NeedsYouKind::Blocked) => "blocked",
        Ok(pb::NeedsYouKind::Decision) => "decision",
        Ok(pb::NeedsYouKind::Review) => "review",
        Ok(pb::NeedsYouKind::Unspecified) | Err(_) => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../test/fixtures/needs-you.json");
    const PROTO: &str = include_str!("../../../proto/farcooler.proto");

    fn id(n: u8) -> bytes::Bytes {
        let mut b = [0u8; 16];
        b[0] = 0x01;
        b[6] = 0x70;
        b[8] = 0x80;
        b[15] = n;
        bytes::Bytes::copy_from_slice(&b)
    }

    fn at(millis: i64) -> Option<prost_types::Timestamp> {
        Some(prost_types::Timestamp { seconds: millis / 1000, nanos: ((millis % 1000) * 1_000_000) as i32 })
    }

    fn action(id: &str, title: &str, destructive: bool, primary: bool) -> pb::NeedsYouAction {
        pb::NeedsYouAction { id: id.into(), title: title.into(), destructive, primary }
    }

    /// When the fixture's runners were read, in Unix milliseconds.
    const NOW: i64 = 1_790_000_000_000;
    /// `needs_you::TIER_SPAN` in the daemon, which `feed::rank` shares.
    const TIER_SPAN: u32 = 100_000_000;

    /// The rank the daemon gives an item of `tier` that has waited `age_secs`
    /// (`needs_you::rank`): the tier dominates, then the oldest first.
    fn rank(tier: u32, age_secs: u32) -> u32 {
        tier * TIER_SPAN + (TIER_SPAN - 1 - age_secs)
    }

    fn ago(age_secs: u32) -> Option<prost_types::Timestamp> {
        at(NOW - i64::from(age_secs) * 1000)
    }

    /// A hook ask's options exactly as the daemon offers them
    /// (`agent_core::permission::permission_options`), turned into actions as
    /// `needs_you::terminal_signal` turns them: allow first and primary, deny
    /// destructive.
    fn hook_ask_actions(allow: &str) -> Vec<pb::NeedsYouAction> {
        vec![action("allow", allow, false, true), action("deny", "Deny", true, false)]
    }

    /// `needs_you::open_action`.
    fn open() -> pb::NeedsYouAction {
        action("open", "Open", false, false)
    }

    /// The two runners the shared fixture holds, as the daemon sends them.
    ///
    /// Every value is one the daemon's builders write (`needs_you.rs`): hook
    /// ask ids, ranks on `TIER_SPAN`, an ask's question as its allow option,
    /// no detail on an ask, a review's `+N −M`, `open` never primary.
    ///
    /// The studio is read at Control. Between its items every optional field
    /// is set and every repeated one is non-empty, so the decoders' tests
    /// can't pass by reading nothing. The build box is read below Control
    /// (`redact_below_control`), and its review is on a task with no
    /// workspace.
    fn runners() -> Vec<(&'static str, pb::NeedsYouList)> {
        use pb::NeedsYouKind as K;
        let lane = pb::WorktreeRef {
            id: id(5),
            name: "fc-3-webhooks".into(),
            branch: "bil/webhooks".into(),
            insertions: 18,
            deletions: 40,
        };
        let studio = pb::NeedsYouList {
            items: vec![
                pb::NeedsYouItem {
                    id: "ask:hook-ask-01000000-0000-7000-8000-000000000029".into(),
                    kind: K::Ask as i32,
                    // Its task is also waiting on a decision.
                    also: vec![K::Decision as i32],
                    rank: rank(0, 60),
                    since: ago(60),
                    workspace_id: id(1),
                    workspace_name: "Billing".into(),
                    repository_id: id(2),
                    task: Some(pb::TaskRef {
                        id: id(3),
                        key: "bil-7".into(),
                        title: "Invoice PDF export".into(),
                        status: pb::TaskStatus::NeedsDecision as i32,
                    }),
                    terminal: Some(pb::TerminalRef {
                        id: id(4),
                        worktree_id: id(5),
                        label: "claude".into(),
                        role: pb::TerminalRole::Agent as i32,
                        pane_mode: pb::PaneMode::Terminal as i32,
                        chat_capable: true,
                    }),
                    worktree: Some(lane.clone()),
                    question: "Allow touch x".into(),
                    detail: None,
                    ask_id: Some("hook-ask-01000000-0000-7000-8000-000000000029".into()),
                    actions: hook_ask_actions("Allow touch x"),
                },
                pb::NeedsYouItem {
                    id: "blocked:01000000-0000-7000-8000-00000000000a".into(),
                    kind: K::Blocked as i32,
                    rank: rank(1, 120),
                    since: ago(120),
                    workspace_id: id(1),
                    workspace_name: "Billing".into(),
                    repository_id: id(2),
                    terminal: Some(pb::TerminalRef {
                        id: id(10),
                        worktree_id: id(11),
                        label: "codex".into(),
                        role: pb::TerminalRole::Orchestrator as i32,
                        pane_mode: pb::PaneMode::Terminal as i32,
                        chat_capable: false,
                    }),
                    worktree: Some(pb::WorktreeRef {
                        id: id(11),
                        name: "demo".into(),
                        branch: "main".into(),
                        insertions: 0,
                        deletions: 0,
                    }),
                    question: "Run the migration now?".into(),
                    actions: vec![open()],
                    ..Default::default()
                },
                pb::NeedsYouItem {
                    id: "decision:01000000-0000-7000-8000-00000000000c".into(),
                    kind: K::Decision as i32,
                    rank: rank(2, 300),
                    since: ago(300),
                    workspace_id: id(1),
                    workspace_name: "Billing".into(),
                    repository_id: id(2),
                    task: Some(pb::TaskRef {
                        id: id(12),
                        key: "bil-9".into(),
                        title: "Retry failed webhooks".into(),
                        status: pb::TaskStatus::NeedsDecision as i32,
                    }),
                    question: "Postgres or SQLite for the queue?".into(),
                    actions: vec![
                        action("Postgres", "Postgres", false, false),
                        action("SQLite", "SQLite", false, false),
                    ],
                    ..Default::default()
                },
                pb::NeedsYouItem {
                    id: "review:01000000-0000-7000-8000-00000000000d".into(),
                    kind: K::Review as i32,
                    rank: rank(3, 3600),
                    since: ago(3600),
                    workspace_id: id(1),
                    workspace_name: "Billing".into(),
                    repository_id: id(2),
                    task: Some(pb::TaskRef {
                        id: id(13),
                        key: "bil-4".into(),
                        title: "Webhook signatures".into(),
                        status: pb::TaskStatus::InReview as i32,
                    }),
                    worktree: Some(lane),
                    question: "Ready for review".into(),
                    detail: Some("+18 −40".into()),
                    actions: vec![open()],
                    ..Default::default()
                },
            ],
        };
        let build_box = pb::NeedsYouList {
            items: vec![
                pb::NeedsYouItem {
                    id: "ask:hook-ask-01000000-0000-7000-8000-00000000002a".into(),
                    kind: K::Ask as i32,
                    rank: rank(0, 10),
                    since: ago(10),
                    workspace_id: id(20),
                    workspace_name: "Main".into(),
                    repository_id: id(21),
                    terminal: Some(pb::TerminalRef {
                        id: id(22),
                        worktree_id: id(23),
                        label: "claude".into(),
                        role: pb::TerminalRole::Agent as i32,
                        pane_mode: pb::PaneMode::Terminal as i32,
                        chat_capable: false,
                    }),
                    question: "claude is asking to use a tool".into(),
                    ..Default::default()
                },
                pb::NeedsYouItem {
                    id: "review:01000000-0000-7000-8000-00000000001e".into(),
                    kind: K::Review as i32,
                    rank: rank(3, 7200),
                    since: ago(7200),
                    repository_id: id(21),
                    task: Some(pb::TaskRef {
                        id: id(30),
                        key: "ops-2".into(),
                        title: "Rotate the relay keys".into(),
                        status: pb::TaskStatus::InReview as i32,
                    }),
                    question: "Ready for review".into(),
                    ..Default::default()
                },
            ],
        };
        vec![("studio", studio), ("build-box", build_box)]
    }

    fn written() -> serde_json::Value {
        json!({
            "runners": runners().iter().map(|(name, list)| json!({
                "runner": name,
                "needs_you": needs_you_json(list),
            })).collect::<Vec<_>>(),
        })
    }

    /// The field names of `message <name>` in the proto, read from its text.
    ///
    /// The text rather than the compiled descriptor, because the descriptor
    /// is `farcooler-protocol`'s build output and this crate can't reach it.
    /// A field is a line `<type> <name> = <n>;` inside the message's braces.
    fn proto_fields(name: &str) -> Vec<String> {
        let start = PROTO
            .find(&format!("message {name} {{"))
            .unwrap_or_else(|| panic!("no message {name} in the proto"));
        let body = &PROTO[start..];
        let body = &body[body.find('{').expect("an open brace") + 1..body.find('}').expect("a close brace")];
        body.lines()
            .map(|l| l.split("//").next().unwrap_or_default().trim())
            .filter(|l| l.ends_with(';') && l.contains('='))
            .map(|l| {
                let left = l.split('=').next().unwrap_or_default().trim();
                left.rsplit(' ').next().unwrap_or_default().to_string()
            })
            .collect()
    }

    /// A field added to the item later without a key here fails, and so does
    /// one added to any message the item carries.
    #[test]
    fn every_needs_you_item_field_has_a_json_key() {
        let (_, studio) = &runners()[0];
        let item = item_json(&studio.items[0]);
        let nested = [
            ("NeedsYouItem", &item),
            ("TaskRef", &item["task"]),
            ("TerminalRef", &item["terminal"]),
            ("WorktreeRef", &item["worktree"]),
            ("NeedsYouAction", &item["actions"][0]),
        ];
        for (message, json) in nested {
            let fields = proto_fields(message);
            assert!(!fields.is_empty(), "read no fields for {message}");
            let object = json.as_object().unwrap_or_else(|| panic!("{message} is not an object"));
            for field in &fields {
                assert!(object.contains_key(field), "{message}.{field} has no key in the JSON");
            }
            assert_eq!(object.len(), fields.len(), "{message} has a key the proto doesn't: {object:?}");
        }
        // The one-line list message, so an envelope field can't go missing either.
        assert_eq!(proto_fields("NeedsYouList"), ["items"]);
    }

    #[test]
    fn the_shared_fixture_is_what_needs_you_json_writes() {
        let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("the fixture is JSON");
        assert_eq!(
            fixture,
            written(),
            "test/fixtures/needs-you.json is not what needs_you_json writes; regenerate it:\n{}",
            serde_json::to_string_pretty(&written()).unwrap_or_default()
        );
    }

    /// Every optional key is set, and every array is non-empty, somewhere in
    /// the fixture, and every kind appears, so a decode test reading it can't
    /// pass by reading nothing.
    #[test]
    fn the_shared_fixture_sets_every_optional_field() {
        let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("the fixture is JSON");
        let runners = fixture["runners"].as_array().expect("runners");
        assert_eq!(runners.len(), 2, "two runners, so a merge has something to merge");
        let items: Vec<&serde_json::Value> =
            runners.iter().flat_map(|r| r["needs_you"]["items"].as_array().expect("items")).collect();

        // Set means the daemon wrote something: not null, "", 0, false or [].
        // A field left at its zero in every item would let a decoder that
        // never reads it pass.
        let set = |v: &serde_json::Value| match v {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Number(n) => n.as_f64() != Some(0.0),
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(_) => true,
        };
        let set_somewhere =
            |pointer: &str| items.iter().any(|i| i.pointer(pointer).is_some_and(set));
        let mut paths: Vec<String> = proto_fields("NeedsYouItem").iter().map(|f| format!("/{f}")).collect();
        for (message, key) in [("TaskRef", "task"), ("TerminalRef", "terminal"), ("WorktreeRef", "worktree")] {
            paths.extend(proto_fields(message).iter().map(|f| format!("/{key}/{f}")));
        }
        for path in paths {
            assert!(set_somewhere(&path), "no item in the fixture sets {path}");
        }
        // An action's fields, in any action of any item: an ask's deny, the
        // destructive one, comes second.
        for field in proto_fields("NeedsYouAction") {
            let anywhere = items.iter().flat_map(|i| i["actions"].as_array().into_iter().flatten()).any(|a| set(&a[&field]));
            assert!(anywhere, "no action in the fixture sets {field}");
        }
        for kind in ["ask", "blocked", "decision", "review"] {
            assert!(items.iter().any(|i| i["kind"] == kind), "no {kind} item in the fixture");
        }
        assert!(items.iter().any(|i| i["workspace_id"].is_null()), "no item without a workspace");
    }

    #[test]
    fn a_kind_this_build_does_not_define_is_unknown_not_a_guess() {
        assert_eq!(kind_word(0), "unknown");
        assert_eq!(kind_word(99), "unknown");
    }

    #[test]
    fn a_worktree_s_open_tasks_are_task_refs() {
        let rows = open_tasks_json(&[pb::TaskRef {
            id: id(3),
            key: "bil-7".into(),
            title: "Invoice PDF export".into(),
            status: pb::TaskStatus::InProgress as i32,
        }]);
        assert_eq!(
            rows,
            json!([{
                "id": uuid_of(&id(3)).to_string(),
                "key": "bil-7",
                "title": "Invoice PDF export",
                "status": "in_progress",
            }])
        );
    }
}
