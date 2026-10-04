//! The worktree rows `worktree list --json` writes, and the envelope around them.

use super::*;

/// The workstreams ride beside the worktrees, as `workspace list
/// --json`'s objects, so the Mac reads the whole fleet in one call — and
/// an empty list is still a list, where a runner without workspaces
/// sends no key at all.
#[test]
fn the_envelope_carries_the_workspaces_beside_the_worktrees() {
    let billing = farcooler_protocol::v1::Workspace {
        id: bytes::Bytes::copy_from_slice(&[1; 16]),
        repository_id: bytes::Bytes::copy_from_slice(&[2; 16]),
        name: "Billing".into(),
        task_prefix: "bil".into(),
        ..Default::default()
    };
    let v = worktree_list_envelope(true, 0, String::new(), vec![], Some(vec![workspace_json(&billing)]));
    assert_eq!(v["workspaces"], serde_json::json!([workspace_json(&billing)]), "{v}");
    assert_eq!(v["workspaces"][0]["task_prefix"], "bil");
    assert_eq!(v["workspaces"][0]["repository"], uuid_of(&[2; 16]).to_string());
    assert_eq!(v["worktrees"], serde_json::json!([]));
    let none = worktree_list_envelope(true, 0, String::new(), vec![], Some(vec![]));
    assert_eq!(none["workspaces"], serde_json::json!([]));
}

/// A worktree row names its repository by id as well as by name, which
/// is how the Mac places a worktree nobody has claimed; and its owner,
/// how it was claimed, and who else is writing in it, by name. Each of
/// its panes says its workspace and its role.
#[test]
fn a_worktree_row_names_its_repository_its_workspace_and_its_panes_roles() {
    use farcooler_protocol::v1 as pb;
    let id = |n: u8| bytes::Bytes::copy_from_slice(&[n; 16]);
    let repositories = [pb::Repository { id: id(2), display_name: "api".into(), ..Default::default() }];
    let workspaces = [
        pb::Workspace { id: id(3), repository_id: id(2), name: "Main".into(), ..Default::default() },
        pb::Workspace { id: id(4), repository_id: id(2), name: "Billing".into(), ..Default::default() },
    ];
    let w = Worktree {
        id: id(1),
        repository_id: id(2),
        workspace_id: Some(id(3)),
        claim_source: Some("hook".into()),
        foreign_writer_workspace_ids: vec![id(4)],
        lfs_pointers: 2,
        open_tasks: vec![pb::TaskRef {
            id: id(8),
            key: "bil-9".into(),
            title: "Invoice PDF export".into(),
            status: pb::TaskStatus::InProgress as i32,
        }],
        ..Default::default()
    };
    let pane = |n: u8, role: pb::TerminalRole, workspace: Option<bytes::Bytes>| Terminal {
        id: id(n),
        worktree_id: id(1),
        role: role as i32,
        workspace_id: workspace,
        ..Default::default()
    };
    let terminals = [
        pane(5, pb::TerminalRole::Orchestrator, Some(id(3))),
        pane(6, pb::TerminalRole::Agent, Some(id(4))),
        Terminal { worktree_id: id(9), ..pane(7, pb::TerminalRole::Shell, None) },
    ];
    let row = worktree_list_row(&w, None, &repositories, &terminals, &workspaces);
    assert_eq!(row["repository"], "api", "the name the rows have always carried");
    assert_eq!(row["repository_id"], uuid_of(&id(2)).to_string());
    assert_eq!(row["workspace"], uuid_of(&id(3)).to_string());
    assert_eq!(row["claim_source"], "hook");
    assert_eq!(row["lfs_pointers"], 2, "how many large files weren't downloaded (ov-199)");
    assert_eq!(row["foreign_writers"], serde_json::json!(["Billing"]));
    assert_eq!(
        row["open_tasks"],
        serde_json::json!([{
            "id": uuid_of(&id(8)).to_string(),
            "key": "bil-9",
            "title": "Invoice PDF export",
            "status": "in_progress",
        }])
    );
    let panes = row["terminals"].as_array().expect("terminals");
    assert_eq!(panes.len(), 2, "only this worktree's panes");
    assert_eq!((&panes[0]["role"], &panes[0]["workspace"]), (&serde_json::json!("orchestrator"), &serde_json::json!(uuid_of(&id(3)).to_string())));
    assert_eq!((&panes[1]["role"], &panes[1]["workspace"]), (&serde_json::json!("agent"), &serde_json::json!(uuid_of(&id(4)).to_string())));

    let unclaimed =
        Worktree { workspace_id: None, claim_source: None, foreign_writer_workspace_ids: vec![], open_tasks: vec![], ..w };
    let row = worktree_list_row(&unclaimed, None, &repositories, &[], &workspaces);
    assert_eq!(row["workspace"], serde_json::json!(null));
    assert_eq!(row["claim_source"], serde_json::json!(null));
    assert_eq!(row["foreign_writers"], serde_json::json!([]));
    assert_eq!(row["open_tasks"], serde_json::json!([]), "an empty list, not a missing key");
    assert_eq!(row["repository_id"], uuid_of(&id(2)).to_string(), "unclaimed still says where it is");
}
