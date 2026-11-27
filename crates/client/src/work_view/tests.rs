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
        state: state.into(), orchestrator, agent: orchestrator, columns: 80, rows: 24, restart_refusal: None,
    };
    data.workspaces[0].orchestrator = Some("seat".into());
    data.terminals.extend([
        terminal("old", "a", "lost", true),
        terminal("seat", "a", "running", false),
        terminal("other-seat", "b", "starting", true),
        terminal("unclaimed", "missing", "running", false),
    ]);
    let main = view(&data, &Selection::default());
    assert_eq!(main["orchestrator"], "seat");
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
    assert_eq!(other["orchestrator"], "other-seat");
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
    assert_eq!(selected["terminals"][0]["lost"], true);
    assert_eq!(selected["terminals"][0]["recover"]["sidebarLabel"], "Recover");
    let title = selected["terminals"][0]["title"].as_str().unwrap();
    assert_eq!(selected["terminals"][0]["recover"]["sidebarName"], format!("Recover {title}"));
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
    assert_eq!(read["create"]["disabled"], READ_ONLY);
    assert!(read["terminals"][0]["rename"].is_null());
    assert!(read["terminals"][0]["close"].is_null());
    assert!(read["terminals"][0]["recover"].is_null());
    let write = view(&inventory("control"), &Selection { stream_ended: true, ..Default::default() });
    assert_eq!(write["terminals"][0]["access"], "");
    assert_eq!(write["terminals"][0]["attach"], true);
    assert_eq!(write["terminals"][0]["recover"]["action"], "reattach");
    assert_eq!(write["start"]["presets"][0]["id"], "shell");
}

fn row<'a>(view: &'a Value, id: &str) -> &'a Value {
    view["terminals"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap()
}

fn pane(id: &str, state: &str, orchestrator: bool) -> Terminal {
    Terminal {
        id: id.into(), title: id.into(), worktree: "a".into(), workspace: Some("main".into()),
        agent: orchestrator, orchestrator, state: state.into(), columns: 80, rows: 24,
        restart_refusal: None,
    }
}

#[test]
fn a_new_terminal_offers_the_macs_agents_and_each_agents_models() {
    let mut data = inventory("control");
    let shown = view(&data, &Selection::default());
    assert_eq!(shown["labels"]["open"], "New Terminal");
    assert_eq!(shown["start"]["model"], "Model");
    assert!(shown["start"]["disabled"].is_null());
    let presets = shown["start"]["presets"].as_array().unwrap();
    let ids = |list: &Value| list.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(ids(&shown["start"]["presets"]), ["shell", "claude", "codex", "cursor"]);
    assert_eq!(ids(&presets[0]["models"]), Vec::<String>::new());
    assert_eq!(ids(&presets[1]["models"]), ["", "opus", "sonnet", "haiku"]);
    assert_eq!(presets[1]["models"][0]["label"], "Default Model");
    assert_eq!(ids(&presets[2]["models"]), ["", "gpt-5.6-sol", "gpt-5.6-sol-high"]);
    assert_eq!(ids(&presets[3]["models"]), ["", "auto", "sonnet-4.5", "gpt-5"]);
    assert!(presets.iter().all(|p| p["unavailable"] == false));

    // A runner that says what it has: the rest are listed, and said.
    data.agents = Some(vec!["codex".into(), "cursor-agent".into()]);
    let shown = view(&data, &Selection::default());
    let presets = shown["start"]["presets"].as_array().unwrap();
    assert_eq!(presets[1]["label"], "Claude Code · Not Installed");
    assert_eq!(presets[1]["unavailable"], true);
    assert_eq!(presets[0]["unavailable"], false);
    assert_eq!(presets[3]["unavailable"], false, "cursor's program is cursor-agent");
}

/// The Mac's list is `Agents.all` in Swift. Until the Mac reads this one,
/// the two must name the same agents and models.
#[test]
fn the_agents_and_models_are_the_mac_apps() {
    let swift = include_str!("../../../../apps/macos/Sources/FarCooler/Composer.swift");
    let listed: Vec<_> = swift.lines().filter_map(|l| l.trim().strip_prefix("Agent(id: ")).collect();
    let ours: Vec<_> = [Preset::Claude, Preset::Codex, Preset::Cursor].map(|p| {
        let models = p.models().iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(", ");
        format!("\"{}\", name: \"{}\", models: [{models}]),", p.command(), p.label())
    }).to_vec();
    assert_eq!(listed, ours);
}

#[test]
fn a_model_is_part_of_the_preset_and_only_a_listed_one() {
    assert_eq!(Preset::Claude.launch("sonnet").as_deref(), Some("claude:sonnet"));
    assert_eq!(Preset::Codex.launch("").as_deref(), Some("codex"));
    assert_eq!(Preset::Shell.launch("").as_deref(), Some("shell"));
    assert_eq!(Preset::Shell.launch("opus"), None);
    assert_eq!(Preset::Codex.launch("opus"), None);
}

#[test]
fn a_stopped_terminal_offers_restart_in_place() {
    let mut data = inventory("control");
    data.terminals = vec![pane("done", "exited", false), pane("broke", "error", false), pane("gone", "lost", false), pane("live", "running", false)];
    let shown = view(&data, &Selection::default());
    for id in ["done", "broke"] {
        assert_eq!(row(&shown, id)["recover"]["action"], "restart", "{id}");
        assert_eq!(row(&shown, id)["recover"]["label"], "Restart Terminal", "{id}");
        assert!(row(&shown, id)["recover"]["disabled"].is_null(), "{id}");
    }
    assert_eq!(row(&shown, "gone")["recover"]["label"], "Recover Terminal");
    assert!(row(&shown, "live")["recover"].is_null(), "only the orchestrator restarts while running");
    assert_eq!(row(&shown, "done")["attachNote"], "This terminal’s process ended. Restart it to continue.");
}

#[test]
fn the_workspaces_orchestrator_can_be_restarted() {
    let mut data = inventory("control");
    data.workspaces[0].orchestrator = Some("seat".into());
    data.terminals = vec![pane("seat", "running", true), pane("live", "running", false)];
    let shown = view(&data, &Selection::default());
    let restart = &row(&shown, "seat")["recover"];
    assert_eq!(restart["action"], "restart_orchestrator");
    assert_eq!(restart["label"], "Restart Orchestrator");
    assert!(restart["confirm"].as_str().unwrap().contains("stops"), "{restart}");
    assert!(restart["disabled"].is_null());

    data.terminals = vec![pane("seat", "exited", true)];
    let shown = view(&data, &Selection::default());
    assert_eq!(row(&shown, "seat")["recover"]["action"], "restart");
    assert_eq!(row(&shown, "seat")["recover"]["label"], "Restart Orchestrator");
}

#[test]
fn a_reader_sees_new_terminal_and_restart_disabled_with_the_reason() {
    let mut data = inventory("read");
    data.workspaces[0].orchestrator = Some("seat".into());
    data.terminals = vec![pane("seat", "running", true), pane("done", "exited", false), pane("gone", "lost", false)];
    let shown = view(&data, &Selection::default());
    assert_eq!(shown["start"]["disabled"], READ_ONLY);
    assert_eq!(shown["create"]["disabled"], READ_ONLY);
    for id in ["seat", "done", "gone"] {
        assert_eq!(row(&shown, id)["recover"]["disabled"], READ_ONLY, "{id}");
    }
}

#[test]
fn a_member_the_runner_would_refuse_sees_restart_disabled_with_its_reason() {
    let mut data = inventory("control");
    data.workspaces[0].orchestrator = Some("seat".into());
    let mut seat = pane("seat", "running", true);
    seat.restart_refusal = Some("not_lead".into());
    let mut theirs = pane("theirs", "exited", false);
    theirs.restart_refusal = Some("not_yours".into());
    data.terminals = vec![seat, theirs];
    let shown = view(&data, &Selection::default());
    assert_eq!(row(&shown, "seat")["recover"]["disabled"],
        "Only this workspace’s lead or the runner’s owner can restart its orchestrator.");
    assert_eq!(row(&shown, "theirs")["recover"]["disabled"], "Your account can’t manage this terminal on the runner.");
    assert!(shown["start"]["disabled"].is_null(), "any member may open a terminal");
}

#[test]
fn a_viewer_who_is_not_the_writer_cannot_restart_what_the_writer_is_using() {
    let mut data = inventory("control");
    data.workspaces[0].orchestrator = Some("seat".into());
    data.terminals = vec![pane("seat", "running", true)];
    let viewing = |terminal: &str| view(&data, &Selection {
        viewing: Some(Viewing { terminal: terminal.into(), reason: "Another client is writing to this terminal.".into() }),
        ..Default::default()
    });
    assert_eq!(row(&viewing("seat"), "seat")["recover"]["disabled"],
        "Another client is writing to this terminal. Only the writer can restart it.");
    assert!(row(&viewing("other"), "seat")["recover"]["disabled"].is_null());
}

#[test]
fn the_runners_refusal_travels_with_the_terminal() {
    let listed = wire::Terminal {
        id: Uuid::nil().as_bytes().to_vec().into(), restart_refusal: Some("not_lead".into()), ..Default::default()
    };
    assert_eq!(terminal(listed).unwrap().restart_refusal.as_deref(), Some("not_lead"));
}
