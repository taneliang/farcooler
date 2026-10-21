//! A rep's claude.ai connectors (ov-586): what its launch keeps out, what its
//! gate file says, and who may change the list. On the fixture's real tmux
//! server with the test stub in place of claude.

use std::path::Path;
use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::rep_gate::{Gate, Verdict, decide};
use crate::test_support::ScratchDir;

/// A runner whose reps run on their members' own Claude logins, which is
/// where a member's connectors are (ov-652: a rep sharing the runner's gets
/// none, `shared_logins_tests`).
async fn runner() -> (ScratchDir, Arc<Service>, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member_login_policy("claude", true, "-", 0).unwrap();
    (dir, svc, main)
}

async fn launched(svc: &Service, terminal: Uuid) -> String {
    let snapshot = svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = svc.tmux.run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"]).await.unwrap();
    out.stdout.trim().to_string()
}

fn settings_of(svc: &Service, rep: Uuid) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(svc.root_dir().join(format!("rep-{rep}.json"))).unwrap()).unwrap()
}

fn connectors_file(svc: &Service, rep: Uuid) -> ConnectorsFile {
    serde_json::from_str(&std::fs::read_to_string(svc.rep_connectors_path(rep)).unwrap()).unwrap()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

/// No local MCP server starts in a rep, with or without connectors (R-66,
/// acceptance 2), as its launch line and settings file say:
///
/// - no settings source but the runner's own file (`--setting-sources=`,
///   which also keeps out the user's, project's and local scope's servers),
///   and no `--mcp-config`;
/// - `allowedMcpServers` whose only command is one no server has, so no
///   stdio server passes, and no `serverName` entry a local server could
///   take; the built-ins, which skip it, denied by name;
/// - with none on, `--strict-mcp-config` and `disableClaudeAiConnectors`;
///   with one on, neither, since strict mode drops the connectors too.
///
/// And the gate is told where its member's connectors are.
#[tokio::test]
async fn no_local_mcp_starts_in_a_rep_and_its_connectors_load_only_when_on() {
    let (_dir, svc, main) = runner().await;
    svc.store.set_member_connectors("alice", &names(&["Slack"])).unwrap();
    let alices = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal.id;
    let bobs = svc.start_rep(main, "bob", "").await.expect("Bob's rep").terminal.id;

    for (rep, on) in [(alices, true), (bobs, false)] {
        let command = launched(&svc, rep).await;
        // The exact argument, an empty list, closed by its quote: no source.
        // tmux shows the pane's quoting doubled: '\\'' around each word.
        assert!(command.contains(r"'\\''--setting-sources='\\'' "), "{command}");
        let sources: Vec<String> = permission_args("/x/farcooler", on).into_iter().filter(|a| a.contains("setting-sources")).collect();
        assert_eq!(sources, ["--setting-sources="], "one argument, empty");
        assert!(!command.contains("--mcp-config"), "{command}");
        assert_eq!(command.contains("--strict-mcp-config"), !on, "strict only with none on: {command}");

        let settings = settings_of(&svc, rep);
        let allowed = settings["allowedMcpServers"].as_array().expect("an allowlist");
        let commands: Vec<_> = allowed.iter().filter_map(|e| e.get("serverCommand")).collect();
        assert_eq!(commands, vec![&serde_json::json!([NO_LOCAL_SERVER])], "no command a server has");
        assert!(allowed.iter().all(|e| e.get("serverName").is_none()), "no name a local server could take");
        assert!(allowed.iter().all(|e| e.as_object().is_some_and(|o| o.len() == 1)), "{allowed:?}");
        let denied: Vec<&str> = settings["deniedMcpServers"].as_array().unwrap().iter().filter_map(|e| e["serverName"].as_str()).collect();
        for built_in in BUILT_IN_SERVERS {
            assert!(denied.contains(built_in), "{built_in} isn't denied");
        }
        assert_eq!(settings["disableClaudeAiConnectors"].as_bool(), (!on).then_some(true), "{settings}");
        assert!(settings.get("enableAllProjectMcpServers").is_none() && settings.get("mcpServers").is_none(), "{settings}");

        let gate = settings["hooks"]["PreToolUse"].as_array().unwrap().iter()
            .filter_map(|e| e["hooks"][0]["command"].as_str()).find(|c| c.contains(" rep-gate ")).unwrap().to_string();
        let path = svc.rep_connectors_path(rep).display().to_string();
        assert!(gate.contains(&format!(" --connectors '{path}' ")), "{gate}");
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the runner's alone");
    }
    assert_eq!(connectors_file(&svc, alices).connectors, names(&["Slack"]));
    assert!(connectors_file(&svc, bobs).connectors.is_empty(), "Bob turned none on");

    // The file goes with its rep.
    svc.stop_terminal(bobs).await.unwrap();
    svc.remove_terminal(bobs).await.unwrap();
    assert!(!svc.rep_connectors_path(bobs).exists());
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(factory: &crate::rpc::RpcFactory, req: Request) -> std::result::Result<Vec<String>, String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => match r.value {
            Some(pb::result::Value::MemberConnectorList(list)) => Ok(list.servers),
            other => panic!("not a list: {other:?}"),
        },
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn connectors(handle: &str, set: Option<&[&str]>) -> Request {
    Request {
        method: "member.connectors".into(),
        payload: Some(request::Payload::MemberConnectors(pb::MemberConnectors {
            handle: handle.into(),
            servers: set.map(names).unwrap_or_default(),
            set: set.is_some(),
        })),
        ..Default::default()
    }
}

/// Only the member, from their own device, and the owner, read or change a
/// member's connectors. Never another member (a runner admin included), a
/// released device, or a process in an agent's or a rep's pane, however it's
/// scoped. And a change reaches a running rep's gate at once: one turned off
/// is refused from its next call.
#[tokio::test]
async fn only_the_member_and_the_owner_set_a_reps_connectors() {
    let (dir, svc, main) = runner().await;
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    let agent = svc.create_terminal(rep.worktree_id, "Agent 2", "claude").await.expect("an agent");
    let snapshot = svc.inventory.refresh().await;
    let pid_of = |id: Uuid| snapshot.claimants(id).into_iter().next().expect("a pane").stamp.pid as i32;

    let person = crate::test_support::Outside::a_person(dir.path(), 6590);
    let owner = factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    let alice = factory(&svc, Some("phone-a"), Scope::Control);
    assert_eq!(call(&alice, connectors("alice", Some(&["Slack", "Linear"]))).await, Ok(names(&["Linear", "Slack"])));
    assert_eq!(connectors_file(&svc, rep.id).connectors, names(&["Linear", "Slack"]), "her running rep's gate");
    assert_eq!(call(&alice, connectors("alice", None)).await, Ok(names(&["Linear", "Slack"])), "she reads her own");

    let refusals = [
        (factory(&svc, Some("phone-b"), Scope::Control), "not-yours:not_yours", "another member"),
        (factory(&svc, Some("phone-b"), Scope::HostAdmin), "not-yours:not_yours", "another member who's a runner admin"),
        (factory(&svc, Some("phone-c"), Scope::HostAdmin), "not-yours:not_yours", "a released device"),
        (factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(pid_of(agent.id))), "resource-conflict:person_only", "an agent's pane"),
        (factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(pid_of(rep.id))), "resource-conflict:rep_messenger", "the rep's own pane"),
        (factory(&svc, Some("phone-a"), Scope::Read), "scope-denied:", "her device below control"),
    ];
    for (who, word, what) in &refusals {
        let refused = call(who, connectors("alice", Some(&["Slack", "Linear", "Asana"]))).await;
        assert_eq!(refused.as_ref().err().map(String::as_str), Some(*word), "{what}: {refused:?}");
    }
    let bob = &refusals[0].0;
    assert_eq!(call(bob, connectors("alice", None)).await.err().as_deref(), Some("not-yours:not_yours"), "nor reads hers");
    assert_eq!(svc.store.member_connectors("alice").unwrap(), names(&["Linear", "Slack"]), "nothing changed");

    // The owner sets anyone's; Slack is refused from the rep's next call.
    assert_eq!(call(&owner, connectors("alice", Some(&["Linear"]))).await, Ok(names(&["Linear"])));
    let on = connectors_file(&svc, rep.id).connectors;
    let gate = Gate { checkout: Path::new("/w"), cli: "/x/farcooler", is_file: &|_| true, tracked: &|_| Some(true), connectors: &on };
    let ask = |tool: &str| decide(&gate, tool, &serde_json::json!({}), &|p: &Path| Some(p.to_path_buf()));
    assert!(matches!(ask("mcp__claude_ai_Slack__send_message"), Verdict::Deny(_)));
    assert_eq!(ask("mcp__claude_ai_Linear__create_issue"), Verdict::Allow);
    assert_eq!(call(&owner, connectors("nobody", Some(&["Linear"]))).await.err().as_deref(), Some("not-found:"));
}

/// Turning a connector off reaches every running rep (review M1): a rep whose
/// gate file can't be written has it removed, which refuses it every MCP
/// tool, the others are still written, and the change is reported failed.
#[tokio::test]
async fn turning_a_connector_off_reaches_every_rep_whatever_one_write_does() {
    let (_dir, svc, main) = runner().await;
    let repo = svc.store.get_workspace(main).unwrap().repository_id;
    let second = svc.store.create_workspace(repo, "Second", "sec").unwrap().id;
    svc.store.set_member_connectors("alice", &names(&["Slack"])).unwrap();
    svc.start_rep(main, "alice", "").await.expect("a rep on Main");
    svc.start_rep(second, "alice", "").await.expect("a rep on Second");
    let reps: Vec<Uuid> = svc.store.reps_of("alice").unwrap().iter().map(|r| r.terminal).collect();
    assert_eq!(reps.len(), 2);

    // The first one refresh comes to can't be written.
    hooks::fail_writes_of(reps[0]);
    svc.store.set_member_connectors("alice", &names(&["Linear"])).unwrap();
    assert!(svc.refresh_rep_connectors("alice").is_err(), "the failure is reported");
    assert!(!svc.rep_connectors_path(reps[0]).exists(), "its stale file is gone: no connector at all");
    assert_eq!(connectors_file(&svc, reps[1]).connectors, names(&["Linear"]), "the other rep heard");
}

/// A change that lands while a launch writes its rep's gate file isn't
/// overwritten by the list the launch read first (review M1).
#[tokio::test]
async fn a_change_during_a_launchs_write_is_not_overwritten() {
    let (_dir, svc, main) = runner().await;
    svc.store.set_member_connectors("alice", &names(&["Slack", "Linear"])).unwrap();
    let rep = svc.start_rep(main, "alice", "").await.expect("a rep").terminal.id;
    let changer = svc.clone();
    hooks::midway(move || {
        changer.store.set_member_connectors("alice", &names(&["Linear"])).unwrap();
        changer.refresh_rep_connectors("alice").unwrap();
    });
    // The launch's write (`rep_launch`), read before the change, written after.
    svc.write_rep_connectors(rep, "alice").unwrap();
    assert_eq!(connectors_file(&svc, rep).connectors, names(&["Linear"]), "Slack, turned off mid-launch, stays off");
}
