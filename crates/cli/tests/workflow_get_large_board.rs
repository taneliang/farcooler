//! The real `farcooler workflow get --json` on a large board (ov-1519): 350 failed and 350
//! landed groups, 350 closed tickets and 200 tickets at rest, over a frame-limited RPC socket.
//! It's the shape of the daemon's `engine_reads_a_large_board_with_the_shipped_request`.
#[path = "../../daemon/tests/support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::Scope;
use farcooler_workflow::{Archived, ArchivedGroup, Definition, GroupOf, Item, Phase, RunRecord, State, digest};
use in_process::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const GROUPS: i64 = 350;
const TICKETS: usize = 200;
/// How many of the newest groups, failed or landed, a `get` reply previews.
const PREVIEWED_GROUPS: i64 = 12;

fn large_board(workspace: Uuid) -> State {
    let fixture: Value = serde_json::from_str(include_str!("../../client/src/testdata/workflow-presentation.json")).unwrap();
    let template: Item = serde_json::from_value(fixture[0]["request"]["snapshot"]["items"]["item"].clone()).unwrap();
    let workspace = workspace.to_string();
    let mut state = State::empty(workspace.clone(), 3);
    state.revision = 4_217;
    state.source_sequences = [("controller", 88), ("wire:owner", 41), ("wire:orchestrator", 1_900)]
        .map(|(source, sequence)| (source.to_string(), sequence)).into();

    let tracker = Definition::ticket_at("todo").unwrap();
    let mut ticket = template.clone();
    ticket.pin = tracker.pin();
    ticket.definition = tracker;
    ticket.phase = Phase::WaitingApproval;
    ticket.pending = None;
    for n in 0..TICKETS {
        let id = format!("ticket-{n:03}");
        let mut ticket = ticket.clone();
        ticket.run = digest(&(&workspace, &id));
        ticket.resource = format!("item:{workspace}:{id}");
        state.items.insert(id, ticket);
    }

    let coding = Definition::coding();
    coding.validate_group().unwrap();
    let mut group = ticket;
    group.pin = coding.pin();
    group.definition = coding;
    group.phase = Phase::Failed;
    group.group_of = Some(GroupOf { name: "landing".into(), pin: group.pin.clone() });
    group.brief = Some("a failed landing's brief ".repeat(160));
    group.handed = vec!["ticket-000".into()];
    let last_run = RunRecord { phase: Phase::Done, run: "earlier-run".into(), stop_run: None,
        pin: "earlier-pin".into(), fence: 1, candidate: Some("a".repeat(40)), tree: None,
        models: Default::default(), reason: Some("Prior result. ".repeat(30)), rework_history: Vec::new(),
        sent_back: Vec::new(), back_edges: Default::default() };
    for n in 0..GROUPS {
        let id = format!("group-{n}-failed");
        let mut failed = group.clone();
        failed.since_ms = 2 * n;
        failed.run = digest(&(&workspace, &id));
        failed.resource = format!("item:{workspace}:{id}");
        state.items.insert(id, failed);
        state.archive.insert(format!("group-{n}-landed"), Archived {
            phase: Phase::Done,
            group: Some(ArchivedGroup {
                name: "landing".into(), tickets: vec!["ticket-000".into()],
                pushed: last_run.candidate.clone(), since_ms: 2 * n + 1, landing_ci: None,
            }),
            candidate: last_run.candidate.clone(), start_load: None, closed: None,
            last_run: Some(last_run.clone()), history: Vec::new(),
        });
        state.archive.insert(format!("closed-{n:04}"), Archived {
            phase: Phase::Done, group: None, candidate: None, start_load: None, closed: None,
            last_run: Some(last_run.clone()), history: Vec::new(),
        });
    }
    state
}

#[tokio::test]
async fn workflow_get_json_keeps_every_live_ticket_on_a_large_board() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let state = large_board(repo.workspace);
    let encoded = serde_json::to_string(&state).unwrap();
    assert!(encoded.len() > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES,
        "the unbounded board must not fit one control frame");
    farcooler_store::testing::exec_for_tests(&h.service.store, &format!(
        "INSERT INTO workflow_states(workspace,revision,epoch,state) VALUES('{}',{},{},'{}')",
        state.workspace, state.revision, state.epoch, encoded.replace('\'', "''")
    ));
    h.service.start_workflows().unwrap();

    // Every ticket is live. Of the groups, the newest are previewed, failed and landed alike.
    let newest_failed = GROUPS - PREVIEWED_GROUPS / 2..GROUPS;
    let expected: BTreeMap<&str, &Item> = state.items.iter()
        .filter(|(id, item)| item.group_of.is_none()
            || newest_failed.clone().any(|n| **id == format!("group-{n}-failed")))
        .map(|(id, item)| (id.as_str(), item))
        .collect();
    assert_eq!(expected.len(), TICKETS + 6);

    for flags in [&["--json"][..], &["--full", "--json"][..]] {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .env("FARCOOLER_HOME", h.socket.parent().unwrap())
            .env_remove("FARCOOLER_WORKSPACE")
            .env_remove("FARCOOLER_ACTOR")
            .args(["workflow", "get", &state.workspace])
            .args(flags)
            .kill_on_drop(true)
            .output().await.unwrap();
        assert!(output.status.success(), "{flags:?}: {}", String::from_utf8_lossy(&output.stderr));
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();

        assert_eq!(reply["state"]["revision"], state.revision, "{flags:?}");
        assert_eq!(reply["state"]["epoch"], state.epoch, "{flags:?}");
        assert_eq!(reply["state"]["source_sequences"], json!(state.source_sequences), "{flags:?}");
        // The groups were really trimmed, so this isn't the small-board path.
        assert_eq!(reply["landing_groups_omitted"], 2 * GROUPS - PREVIEWED_GROUPS, "{flags:?}");

        let items = reply["state"]["items"].as_object().unwrap();
        assert_eq!(items.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected.keys().copied().collect(), "{flags:?}");
        for (id, item) in &expected {
            assert_eq!(items[*id]["run"], item.run, "{flags:?} {id}");
            assert_eq!(items[*id]["pin"], item.pin, "{flags:?} {id}");
            assert_eq!(items[*id]["phase"], json!(item.phase), "{flags:?} {id}");
        }
    }
    h.service.stop_workflows().await.unwrap();
}
