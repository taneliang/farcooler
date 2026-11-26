use super::*;

fn inventory(scope: &str) -> Inventory {
    serde_json::from_value(json!({
        "scope": scope,
        "workspaces": [
            {"id":"main", "name":"Main", "isMain": true, "repository":"one"},
            {"id":"other", "name":"Other project", "isMain": false, "repository":"one"},
            {"id":"second", "name":"Main", "isMain": true, "repository":"two"},
        ],
        "repositories": [{"id":"one","label":"Product"}, {"id":"two","label":"Runner"}],
        "worktrees": [
            {"id":"a","label":"First worktree","repository":"one","workspace":"main"},
            {"id":"b","label":"Other worktree","repository":"one","workspace":"other"},
        ],
        "terminals": [
            {"id":"lost","title":"Build","worktree":"b","workspace":"other","agent":true,"state":"lost","columns":80,"rows":24},
            {"id":"live","title":"Shell","worktree":"a","workspace":"main","agent":false,"state":"running","columns":80,"rows":24},
        ]
    })).unwrap()
}

#[test]
fn repository_and_workspace_choose_their_own_worktrees() {
    let data = inventory("control");
    let main = view(&data, &Selection::default());
    assert_eq!(main["workspace"], "main");
    assert_eq!(main["worktree"], "a");
    assert_eq!(main["terminals"].as_array().unwrap().len(), 1);
    let second = view(&data, &Selection { repository: Some("two".into()), ..Default::default() });
    assert_eq!(second["workspace"], "second");
    assert_eq!(second["worktree"], Value::Null);
    assert!(second["create"].is_object());
    assert!(second["start"].is_null());
}

#[test]
fn terminal_sidebar_uses_the_runner_seat_and_the_checkouts_workspace() {
    let mut data = inventory("control");
    let terminal = |id: &str, tree: &str, state: &str, orchestrator| Terminal {
        id: id.into(), title: id.into(), worktree: tree.into(), workspace: None,
        state: state.into(), orchestrator, agent: orchestrator, columns: 80, rows: 24,
    };
    data.workspaces[0].orchestrator = Some("seat".into());
    data.terminals.extend([
        terminal("old", "a", "lost", true),
        terminal("seat", "a", "running", false),
        terminal("other-seat", "b", "starting", true),
        terminal("unclaimed", "missing", "running", false),
    ]);
    let main = view(&data, &Selection::default());
    assert_eq!(main["workspaces"][0]["label"], "Main · Product");
    assert_eq!(main["workspaces"][2]["label"], "Main · Runner");
    let rows = main["terminals"].as_array().unwrap();
    assert_eq!(rows.iter().map(|t| t["id"].as_str().unwrap()).collect::<Vec<_>>(), ["seat", "live", "old"]);
    assert_eq!(rows[0]["sidebarBadge"], "Orchestrator");
    assert_eq!(rows[0]["sidebarTitle"], "seat");
    assert_eq!(rows[0]["agent"], true);
    assert_eq!(rows[2]["sidebarBadge"], "Lost");
    assert_eq!(rows[2]["live"], false);

    let other = view(&data, &Selection { workspace: Some("other".into()), ..Default::default() });
    assert_eq!(other["terminals"][0]["id"], "other-seat");
    assert_eq!(other["terminals"][0]["live"], true);
    assert_eq!(other["terminals"][0]["sidebarBadge"], "Orchestrator");
}

#[test]
fn deep_links_select_lost_terminal_and_do_not_fall_through_if_gone() {
    let data = inventory("host_admin");
    let selected = view(&data, &Selection { terminal: Some("lost".into()), ..Default::default() });
    assert_eq!(selected["workspace"], "other");
    assert_eq!(selected["worktree"], "b");
    assert_eq!(selected["terminal"], "lost");
    assert_eq!(selected["terminals"][0]["recover"]["action"], "restart");
    let missing = view(&data, &Selection { terminal: Some("deleted".into()), ..Default::default() });
    assert!(missing["terminal"].is_null());
    assert_eq!(missing["notice"], "This terminal is no longer on the runner.");
    let missing_space = view(&data, &Selection { workspace: Some("deleted".into()), ..Default::default() });
    assert!(missing_space["terminals"].as_array().unwrap().is_empty());
    assert_eq!(missing_space["notice"], "This workspace is no longer on the runner.");
}

#[test]
fn a_worktree_link_opens_its_owner_and_never_substitutes_a_checkout() {
    let data = inventory("control");
    let select = |id: &str| view(&data, &Selection {
        workspace: Some("main".into()), worktree: Some(id.into()), ..Default::default()
    });
    let linked = select("b");
    assert_eq!(linked["workspace"], "other");
    assert_eq!(linked["worktree"], "b");
    let missing = select("removed");
    assert!(missing["worktree"].is_null());
    assert!(missing["start"].is_null());
    assert_eq!(missing["notice"], "This checkout is no longer available.");
}

#[test]
fn grant_controls_actions_and_stream_recovery_is_an_attach() {
    let read = view(&inventory("read"), &Selection::default());
    assert_eq!(read["terminals"][0]["access"], "Read-only");
    assert_eq!(read["terminals"][0]["attach"], false);
    assert!(read["create"].is_null());
    assert!(read["terminals"][0]["rename"].is_null());
    assert!(read["terminals"][0]["close"].is_null());
    assert!(read["terminals"][0]["recover"].is_null());
    let write = view(&inventory("control"), &Selection { stream_ended: true, ..Default::default() });
    assert_eq!(write["terminals"][0]["access"], "");
    assert_eq!(write["terminals"][0]["attach"], true);
    assert_eq!(write["terminals"][0]["recover"]["action"], "reattach");
    assert_eq!(write["start"]["presets"][0]["id"], "shell");
}
