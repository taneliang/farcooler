//! A Maintainer keeps the workspaces (ov-784, slice 2 of ov-753): they
//! rename one, set its prefix, and set the four settings that don't act as
//! the owner (its pull requests' size and cost line, waking on an answer,
//! the nudge's hours). A member who isn't one is refused each, a workspace's
//! lead included, and so is a removed member's device; nobody but the owner
//! touches the settings that act as the owner. Renames, prefixes and lead
//! changes go to the people log. Through `Rpc`, where a client reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response, result};
use farcooler_store::member_roles::Role;

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Main and Bob, Carol and Erin, members whose phones are
/// `phone-b`, `phone-c` and `phone-e`, and Dan, who was one and whose
/// `phone-d` is nobody's now. Carol is a Maintainer; Erin leads Main.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    for (handle, name, phone) in [("bob", "Bob", "phone-b"), ("carol", "Carol", "phone-c"), ("erin", "Erin", "phone-e"), ("dan", "Dan", "phone-d")] {
        svc.store.set_member(handle, name, &[phone.to_string()]).unwrap();
    }
    svc.store.remove_member("dan").unwrap();
    svc.store.set_member_role("carol", Role::Maintainer, "user").unwrap();
    svc.store.set_workspace_lead(workspace, Some("erin")).unwrap();
    Runner { _dir: dir, svc, workspace }
}

const BOB: Option<&str> = Some("phone-b");
const CAROL: Option<&str> = Some("phone-c");
const ERIN: Option<&str> = Some("phone-e");
const RELEASED: Option<&str> = Some("phone-d");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const NOT_YOURS: &str = "not-yours:not_yours";

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

/// The answer, or the refusal as `<code word>:<what>`.
async fn call(r: &Runner, device: Option<&str>, req: Request) -> std::result::Result<Option<result::Value>, String> {
    use farcooler_transport::Handler as _;
    match factory(&r.svc, device, Scope::Control).handle(req).await.outcome {
        Some(response::Outcome::Result(res)) => Ok(res.value),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn on(r: &Runner, method: &str, payload: request::Payload) -> Request {
    Request { method: method.into(), target_resource_id: Some(id_bytes(r.workspace)), payload: Some(payload), ..Default::default() }
}

fn rename(r: &Runner, name: &str) -> Request {
    on(r, "workspace.rename", request::Payload::WorkspaceRename(pb::WorkspaceRename { name: name.into(), expected_version: None }))
}

fn prefix(r: &Runner, prefix: &str) -> Request {
    on(r, "workspace.set_prefix", request::Payload::WorkspaceSetPrefix(pb::WorkspaceSetPrefix { task_prefix: prefix.into(), expected_version: None }))
}

fn settings(r: &Runner, change: pb::WorkspaceSetSettings) -> Request {
    on(r, "workspace.set_settings", request::Payload::WorkspaceSetSettings(change))
}

/// The four settings that act as nobody, each set to something other than
/// its default, named.
fn the_four() -> [(&'static str, pb::WorkspaceSetSettings); 4] {
    let s = pb::WorkspaceSetSettings::default;
    [
        ("pr_max_lines", pb::WorkspaceSetSettings { pr_max_lines: Some(4_000), ..s() }),
        ("pr_cost_line", pb::WorkspaceSetSettings { pr_cost_line: Some(false), ..s() }),
        ("wake_on_answer", pb::WorkspaceSetSettings { wake_on_answer: Some(false), ..s() }),
        ("pr_nudge_hours", pb::WorkspaceSetSettings { pr_nudge_hours: Some(7), ..s() }),
    ]
}

fn workspace(r: &Runner) -> farcooler_store::models::Workspace {
    r.svc.store.get_workspace(r.workspace).unwrap()
}

/// Carol, a Maintainer, renames Main from her phone at Control, gives it a
/// new prefix, and sets each of the four settings, and each is written.
#[tokio::test]
async fn a_maintainer_renames_and_sets_the_four_settings() {
    let r = runner().await;
    call(&r, CAROL, rename(&r, "Platform")).await.expect("Carol renames it");
    assert_eq!(workspace(&r).name, "Platform");
    call(&r, CAROL, prefix(&r, "plt")).await.expect("Carol sets its prefix");
    assert_eq!(workspace(&r).task_prefix, "plt");
    for (field, change) in the_four() {
        let Ok(Some(result::Value::Workspace(w))) = call(&r, CAROL, settings(&r, change)).await else {
            panic!("Carol sets {field}")
        };
        let set = match field {
            "pr_max_lines" => w.pr_max_lines == Some(4_000),
            "pr_cost_line" => w.pr_cost_line == Some(false),
            "wake_on_answer" => w.wake_on_answer == Some(false),
            _ => w.pr_nudge_hours == Some(7),
        };
        assert!(set, "{field} written: {w:?}");
    }
}

/// Bob, a member, and Erin, who leads Main but isn't a Maintainer, are
/// refused a rename, a prefix and each of the four `not_yours`, at Control
/// and as runner admins; so is a removed member's phone. Nothing changes,
/// and nothing is logged.
#[tokio::test]
async fn a_member_who_isnt_a_maintainer_is_refused() {
    let r = runner().await;
    let before = workspace(&r);
    let mut asks = vec![("rename", rename(&r, "Mine")), ("prefix", prefix(&r, "MINE"))];
    asks.extend(the_four().map(|(field, change)| (field, settings(&r, change))));
    for (what, req) in asks {
        for (who, device) in [("Bob, a member", BOB), ("Erin, the lead", ERIN), ("a removed member's phone", RELEASED)] {
            for scope in [Scope::Control, Scope::HostAdmin] {
                use farcooler_transport::Handler as _;
                let outcome = factory(&r.svc, device, scope).handle(req.clone()).await.outcome;
                let Some(response::Outcome::Error(e)) = outcome else { panic!("{who} at {scope:?}: {what} went through") };
                assert_eq!(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what), NOT_YOURS, "{who} at {scope:?}: {what}");
            }
        }
    }
    let after = workspace(&r);
    assert_eq!((after.name, after.task_prefix, after.resource_version), (before.name, before.task_prefix, before.resource_version));
    assert!(r.svc.store.people_events(10).unwrap().iter().all(|e| e.kind == "role"), "only Carol's role, given in the fixture");
}

/// A Maintainer's role reaches no further than the four: how Main lands,
/// onto what, turning the nudge on (it posts as the owner) and who leads it
/// stay the owner's, refused Carol `not_yours`. Turning the nudge off is
/// still anyone's.
#[tokio::test]
async fn a_maintainer_sets_nothing_that_acts_as_the_owner() {
    let r = runner().await;
    let s = pb::WorkspaceSetSettings::default;
    for (field, change) in [
        ("landing", pb::WorkspaceSetSettings { landing: Some(pb::LandingMode::Direct as i32), ..s() }),
        ("base", pb::WorkspaceSetSettings { base: Some("release".into()), ..s() }),
        ("pr_nudge", pb::WorkspaceSetSettings { pr_nudge: Some(true), ..s() }),
        ("lead", pb::WorkspaceSetSettings { lead: Some("carol".into()), ..s() }),
        ("one of the four with landing", pb::WorkspaceSetSettings { pr_max_lines: Some(9), landing: Some(pb::LandingMode::Direct as i32), ..s() }),
    ] {
        assert_eq!(call(&r, CAROL, settings(&r, change)).await.err().as_deref(), Some(NOT_YOURS), "Carol sets {field}");
    }
    assert_eq!(r.svc.store.workspace_lead(r.workspace).unwrap().as_deref(), Some("erin"));
    call(&r, BOB, settings(&r, pb::WorkspaceSetSettings { pr_nudge: Some(false), ..s() })).await.expect("Bob turns the nudge off");
}

/// The owner's phone renames, re-prefixes and sets the four, as before.
#[tokio::test]
async fn the_owner_still_does_all_of_it() {
    let r = runner().await;
    call(&r, OWNERS_PHONE, rename(&r, "Owned")).await.expect("a rename");
    call(&r, OWNERS_PHONE, prefix(&r, "OWN")).await.expect("a prefix");
    for (field, change) in the_four() {
        call(&r, OWNERS_PHONE, settings(&r, change)).await.unwrap_or_else(|e| panic!("{field}: {e}"));
    }
}

/// A rename, a new prefix and a lead handed on each go to the people log,
/// under who made them and naming the workspace, with what it was and what
/// it is. One that changes nothing logs nothing.
#[tokio::test]
async fn renames_and_lead_changes_are_in_the_people_log() {
    let r = runner().await;
    let was = workspace(&r);
    call(&r, CAROL, rename(&r, "Platform")).await.unwrap();
    call(&r, CAROL, rename(&r, "Platform")).await.unwrap();
    call(&r, CAROL, prefix(&r, "plt")).await.unwrap();
    let lead = |to: &str| settings(&r, pb::WorkspaceSetSettings { lead: Some(to.into()), ..Default::default() });
    let admin = |req| async { use farcooler_transport::Handler as _; factory(&r.svc, OWNERS_PHONE, Scope::HostAdmin).handle(req).await };
    admin(lead("bob")).await;
    admin(lead("bob")).await;
    admin(lead("owner")).await;
    let log: Vec<(String, String, String, String)> = r
        .svc
        .store
        .people_events(10)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind != "role")
        .inspect(|e| assert_eq!(e.workspace, Some(r.workspace), "{e:?}"))
        .map(|e| (e.actor, e.handle, e.kind, e.detail))
        .collect();
    let line = |actor: &str, handle: &str, kind: &str, detail: &str| (actor.to_string(), handle.to_string(), kind.to_string(), detail.to_string());
    assert_eq!(
        log,
        [
            line("user", "", "lead", r#"{"from":"bob","to":"owner"}"#),
            line("user", "bob", "lead", r#"{"from":"erin","to":"bob"}"#),
            line("member:carol", "", "prefix", &format!(r#"{{"from":"{}","to":"plt"}}"#, was.task_prefix)),
            line("member:carol", "", "rename", &format!(r#"{{"from":"{}","to":"Platform"}}"#, was.name)),
        ],
        "newest first"
    );
}
