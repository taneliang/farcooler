//! A member's device changes none of a workspace's settings and starts no
//! orchestrator it doesn't lead (ov-637, the owner's "read-only" ruling):
//! `gate`'s rules, asked directly, as the handler would only answer after.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope};

use super::*;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Bob and Dana, members whose phones are `phone-b` and
/// `phone-d`, and Carol, who was one and whose `phone-c` is nobody's now.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("dana", "Dana", &["phone-d".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.remove_member("carol").unwrap();
    Runner { _dir: dir, svc, workspace }
}

const BOB: Option<&str> = Some("phone-b");
const DANA: Option<&str> = Some("phone-d");
const RELEASED: Option<&str> = Some("phone-c");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const LOCAL: Option<&str> = None;

fn peer(device: Option<&str>, scope: Scope) -> Peer {
    Peer { client_id: device.map(str::to_string), scope }
}

fn settings(r: &Runner, change: pb::WorkspaceSetSettings) -> Request {
    Request {
        method: "workspace.set_settings".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceSetSettings(change)),
        ..Default::default()
    }
}

fn refused(r: &Runner, device: Option<&str>, req: &Request) -> Option<&'static str> {
    match gate(&r.svc, &peer(device, Scope::HostAdmin), None, req) {
        Ok(()) => None,
        Err(DomainError::NotYours { what }) => Some(what),
        Err(other) => panic!("{device:?} on {}: {other:?}", req.method),
    }
}

/// Every setting is the owner's, and the four that act as nobody a
/// Maintainer's too (ov-784, `people_maintainer_tests`): how the workspace
/// lands (straight to the base, with the owner's credentials, or by pull
/// request), onto what, the pull requests' size and cost line, the nudge and
/// how soon it posts, waking on an answer, and who leads. A member's device
/// and a released one are refused each, at any scope; the owner's phone and
/// the local socket pass. Turning the nudge off is anyone's, as before.
#[tokio::test]
async fn a_member_changes_no_workspace_setting() {
    let r = runner().await;
    let s = pb::WorkspaceSetSettings::default;
    let changes = [
        ("landing", pb::WorkspaceSetSettings { landing: Some(pb::LandingMode::Direct as i32), ..s() }),
        ("base", pb::WorkspaceSetSettings { base: Some("release".into()), ..s() }),
        ("pr_max_lines", pb::WorkspaceSetSettings { pr_max_lines: Some(4_000), ..s() }),
        ("pr_cost_line", pb::WorkspaceSetSettings { pr_cost_line: Some(false), ..s() }),
        ("pr_nudge", pb::WorkspaceSetSettings { pr_nudge: Some(true), ..s() }),
        ("pr_nudge_hours", pb::WorkspaceSetSettings { pr_nudge_hours: Some(1), ..s() }),
        ("wake_on_answer", pb::WorkspaceSetSettings { wake_on_answer: Some(false), ..s() }),
        ("lead", pb::WorkspaceSetSettings { lead: Some("bob".into()), ..s() }),
        ("nudge off with another", pb::WorkspaceSetSettings { pr_nudge: Some(false), base: Some("release".into()), ..s() }),
    ];
    for (field, change) in changes {
        let req = settings(&r, change);
        for device in [BOB, RELEASED] {
            assert_eq!(refused(&r, device, &req), Some("not_yours"), "{device:?} changes {field}");
        }
        for device in [OWNERS_PHONE, LOCAL] {
            assert_eq!(refused(&r, device, &req), None, "{device:?} changes {field}");
        }
    }
    for off in [
        pb::WorkspaceSetSettings { pr_nudge: Some(false), ..s() },
        pb::WorkspaceSetSettings { pr_nudge: Some(false), expected_version: Some(3), ..s() },
    ] {
        assert_eq!(refused(&r, BOB, &settings(&r, off)), None, "Bob turns the nudge off");
    }
}

fn start(r: &Runner) -> Request {
    Request {
        method: "workspace.start_orchestrator".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceStartOrchestrator(pb::WorkspaceStartOrchestrator {
            harness: "claude".into(),
            replace: true,
            handoff_task: "Read ov-9 first".into(),
        })),
        ..Default::default()
    }
}

/// Starting a workspace's orchestrator, or replacing it with one that reads
/// a handoff card first, is its lead's and the owner's (R-54), as talking to
/// it is: a member who doesn't lead it is refused `not_lead`, and a released
/// device, which writes nothing, `not_yours` (ov-712). The owner's phone and
/// the local socket pass, and so does Bob once he leads it.
#[tokio::test]
async fn only_the_lead_starts_the_orchestrator() {
    let r = runner().await;
    let req = start(&r);
    for device in [BOB, DANA] {
        assert_eq!(refused(&r, device, &req), Some("not_lead"), "{device:?} while the owner leads");
    }
    assert_eq!(refused(&r, RELEASED, &req), Some("not_yours"), "a released device while the owner leads");
    for device in [OWNERS_PHONE, LOCAL] {
        assert_eq!(refused(&r, device, &req), None, "{device:?} while the owner leads");
    }
    r.svc.store.set_workspace_lead(r.workspace, Some("bob")).unwrap();
    assert_eq!(refused(&r, BOB, &req), None, "Bob, its lead");
    assert_eq!(refused(&r, DANA, &req), Some("not_lead"), "Dana while Bob leads");
    assert_eq!(refused(&r, RELEASED, &req), Some("not_yours"), "a released device while Bob leads");
    for device in [OWNERS_PHONE, LOCAL] {
        assert_eq!(refused(&r, device, &req), None, "{device:?} while Bob leads");
    }
}

/// Deleting, renaming or re-prefixing a workspace, rewriting its charter, and
/// the runner's own settings (where repositories are found, the branch
/// prefix, the projector, the terminal themes) are the owner's, a rename and
/// a prefix a Maintainer's too (ov-784): refused a member's device and a
/// released one `not_yours`, at any scope. The owner's phone and the local
/// socket pass the gate, to be answered by the handler.
#[tokio::test]
async fn a_member_reshapes_no_workspace_or_runner_setting() {
    let r = runner().await;
    let methods = [
        "workspace.delete",
        "workspace.rename",
        "workspace.set_prefix",
        "workspace.write_charter",
        "repository_root.add",
        "repository_root.remove",
        "settings.set_branch_prefix",
        "settings.set_projector",
        "theme.upsert",
        "theme.delete",
    ];
    for method in methods {
        let req = Request { method: method.into(), target_resource_id: Some(id_bytes(r.workspace)), ..Default::default() };
        for device in [BOB, RELEASED] {
            assert_eq!(refused(&r, device, &req), Some("not_yours"), "{device:?} on {method}");
        }
        for device in [OWNERS_PHONE, LOCAL] {
            assert_eq!(refused(&r, device, &req), None, "{device:?} on {method}");
        }
    }
}
