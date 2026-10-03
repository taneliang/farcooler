//! What a workspace looks like once it leaves the wire, and the workspace
//! fields on a worktree and a terminal.
//!
//! One implementation for both producers, for `tasks_json`'s reason. The
//! phones read the fleet through the FFI (`Session::fleet`), and the Mac reads
//! `farcooler worktree list --json` and `workspace list --json`. AgentKit
//! decodes a workspace with ONE public type, `WorkspaceSummary`, for both of
//! them, so the two producers must not be two copies that are merely meant to
//! agree. The CLI can call these directly, as it calls `tasks_json`.
//!
//! Keys are snake_case, as `tasks_json`'s are: they are the CLI's, which is
//! what an agent and a script read. The single-word keys the fleet gains
//! (`workspace`, `role`, `workspaces`) are the same in either spelling.
//!
//! **`workspaces` on the fleet is not the `"workspaces"` key in AgentKit's
//! `RunnerDirectory`.** That one is older, is on disk, and holds WORKTREES
//! under the name they had before the rename. It is frozen. This one is the
//! list of workstreams. `FleetDecodeTests` pins both.

use std::collections::HashMap;

use farcooler_protocol::v1 as pb;
use serde_json::json;
use uuid::Uuid;

use crate::session::{short, some_uuid, uuid_of};

/// One workspace: the objects under the fleet's `workspaces`, and what
/// `farcooler workspace list --json` prints.
///
/// `orchestrator` is the live orchestrator's terminal id, or null. `home` and
/// `charter` are paths, which the runner sends to `host_admin` only, so they
/// are null for any other client, never an empty string.
pub fn workspace_json(w: &pb::Workspace) -> serde_json::Value {
    json!({
        "id": uuid_of(&w.id).to_string(),
        "short": short(&w.id),
        "repository": uuid_of(&w.repository_id).to_string(),
        "name": w.name,
        "task_prefix": w.task_prefix,
        "is_main": w.is_main,
        "ordinal": w.ordinal,
        "orchestrator": some_uuid(w.orchestrator_terminal_id.as_deref()).map(|u| u.to_string()),
        "home": w.home.as_deref().filter(|p| !p.is_empty()),
        "charter": w.charter_path.as_deref().filter(|p| !p.is_empty()),
        // Null from a runner without `wake_on_answer`, which tells nobody.
        "wake_on_answer": w.wake_on_answer,
    })
}

/// A terminal's role as a word: `shell`, `agent` or `orchestrator`.
///
/// `None` for `UNSPECIFIED`, which is what a runner without `workstreams`
/// sends, and for a number this build does not define. Never a guess: an
/// unknown role read as `agent` would draw a runner's orchestrator as one more
/// worktree pane, and the Mac filters orchestrators out of worktree rows by
/// this word.
pub fn role_word(raw: i32) -> Option<&'static str> {
    match pb::TerminalRole::try_from(raw) {
        Ok(pb::TerminalRole::Shell) => Some("shell"),
        Ok(pb::TerminalRole::Agent) => Some("agent"),
        Ok(pb::TerminalRole::Orchestrator) => Some("orchestrator"),
        Ok(pb::TerminalRole::Unspecified) | Err(_) => None,
    }
}

/// The workspace that owns a worktree or a terminal, as a uuid string, or
/// null: unclaimed, or a runner without `workstreams`.
pub fn workspace_of(id: Option<&[u8]>) -> Option<String> {
    some_uuid(id).map(|u| u.to_string())
}

/// The names of the other workspaces with a live terminal in a worktree.
///
/// Names, as `worktree list --json` spells them, because this is a warning a
/// person reads: "Billing is also writing here". An id this list does not
/// know — a workspace deleted since the worktree was read — is its short id
/// rather than dropped, because dropping it would hide the one fact the field
/// exists to say.
pub fn foreign_writers(w: &pb::Worktree, workspaces: &[pb::Workspace]) -> Vec<String> {
    let names: HashMap<Uuid, &str> =
        workspaces.iter().map(|ws| (uuid_of(&ws.id), ws.name.as_str())).collect();
    w.foreign_writer_workspace_ids
        .iter()
        .map(|id| match names.get(&uuid_of(id)) {
            Some(name) => (*name).to_string(),
            None => short(id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> bytes::Bytes {
        bytes::Bytes::copy_from_slice(&[n; 16])
    }

    /// Every key AgentKit's `WorkspaceSummary` and Android's `Workspace`
    /// read, by the name they read it under. The decoders are in Swift and
    /// Kotlin, so the names are pinned here.
    #[test]
    fn a_workspace_carries_every_key_the_apps_decode() {
        let w = pb::Workspace {
            id: id(1),
            repository_id: id(2),
            name: "Billing".into(),
            task_prefix: "bil".into(),
            is_main: false,
            ordinal: 3,
            orchestrator_terminal_id: Some(id(4)),
            home: None,
            charter_path: Some(String::new()),
            ..Default::default()
        };
        let json = workspace_json(&w);
        assert_eq!(json["id"], uuid_of(&id(1)).to_string());
        assert_eq!(json["repository"], uuid_of(&id(2)).to_string());
        assert_eq!(json["name"], "Billing");
        assert_eq!(json["task_prefix"], "bil");
        assert_eq!(json["is_main"], false);
        assert_eq!(json["ordinal"], 3);
        assert_eq!(json["orchestrator"], uuid_of(&id(4)).to_string());
        // Paths withheld from this client are null, not "".
        assert!(json["home"].is_null(), "{json}");
        assert!(json["charter"].is_null(), "{json}");
        // An older runner's says nothing, which is null rather than off.
        assert!(json["wake_on_answer"].is_null(), "{json}");

        let none = workspace_json(&pb::Workspace { is_main: true, wake_on_answer: Some(false), ..w });
        assert_eq!(none["is_main"], true);
        assert_eq!(none["wake_on_answer"], false);
    }

    #[test]
    fn a_role_is_a_word_or_nothing() {
        assert_eq!(role_word(pb::TerminalRole::Shell as i32), Some("shell"));
        assert_eq!(role_word(pb::TerminalRole::Agent as i32), Some("agent"));
        assert_eq!(role_word(pb::TerminalRole::Orchestrator as i32), Some("orchestrator"));
        assert_eq!(role_word(pb::TerminalRole::Unspecified as i32), None);
        assert_eq!(role_word(9_999), None);
    }

    /// Unclaimed is null, never the nil uuid, which a client would group as a
    /// workspace of its own.
    #[test]
    fn an_unclaimed_worktree_has_no_workspace() {
        assert_eq!(workspace_of(None), None);
        assert_eq!(workspace_of(Some(&[])), None);
        assert_eq!(workspace_of(Some(Uuid::nil().as_bytes())), None);
        assert_eq!(workspace_of(Some(&id(7))), Some(uuid_of(&id(7)).to_string()));
    }

    #[test]
    fn a_foreign_writer_is_named_and_an_unknown_one_is_not_dropped() {
        let billing = pb::Workspace { id: id(1), name: "Billing".into(), ..Default::default() };
        let w = pb::Worktree { foreign_writer_workspace_ids: vec![id(1), id(9)], ..Default::default() };
        assert_eq!(foreign_writers(&w, &[billing]), vec!["Billing".to_string(), short(&id(9))]);
    }
}
