//! The people on a runner as an app reads them (ov-508): `member.list`'s
//! answer, which `farcooler member list --json` prints and the phones' bridge
//! answers, so the Mac, which reads the CLI, and the phones decode one model
//! (AgentKit's `RunnerPeople`).
//!
//! Keys are snake_case, as the CLI's are, rather than this bridge's usual
//! camelCase: one shape on both roads is worth more than the house style.

use farcooler_protocol::v1 as pb;
use serde_json::{Value, json};

/// `{members: [{handle, name, devices, reps, rep_panes: [{terminal,
/// workspace}]}], claude_account, you}`. A rep's `workspace` is "" from a
/// runner before ov-508 sent it.
pub fn members_json(list: &pb::MemberList) -> Value {
    let id = |b: &[u8]| uuid::Uuid::from_slice(b).map(|u| u.to_string()).unwrap_or_default();
    let members: Vec<Value> = list
        .members
        .iter()
        .map(|m| {
            let panes: Vec<Value> = m
                .rep_terminal_ids
                .iter()
                .enumerate()
                .map(|(i, t)| json!({ "terminal": id(t), "workspace": m.rep_workspace_ids.get(i).map(|w| id(w)).unwrap_or_default() }))
                .collect();
            json!({ "handle": m.handle, "name": m.name, "devices": m.client_ids, "reps": m.rep_terminal_ids.len(), "rep_panes": panes })
        })
        .collect();
    json!({ "members": members, "claude_account": list.claude_account, "you": list.you })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field the apps read, a rep's board beside its pane, and none
    /// for a rep from a runner that didn't send one.
    #[test]
    fn the_list_in_the_shape_the_apps_decode() {
        let pane = uuid::Uuid::from_u128(7);
        let board = uuid::Uuid::from_u128(9);
        let list = pb::MemberList {
            members: vec![pb::Member {
                handle: "alice".into(),
                name: "Alice Chen".into(),
                client_ids: vec!["phone-a".into()],
                rep_terminal_ids: vec![pane.as_bytes().to_vec().into(), pane.as_bytes().to_vec().into()],
                rep_workspace_ids: vec![board.as_bytes().to_vec().into()],
            }],
            claude_account: "team@example.com".into(),
            you: "alice".into(),
        };
        let v = members_json(&list);
        assert_eq!(v["claude_account"], "team@example.com");
        assert_eq!(v["you"], "alice");
        assert_eq!(v["members"][0]["name"], "Alice Chen");
        assert_eq!(v["members"][0]["devices"], json!(["phone-a"]));
        assert_eq!(v["members"][0]["reps"], 2);
        assert_eq!(v["members"][0]["rep_panes"][0], json!({ "terminal": pane.to_string(), "workspace": board.to_string() }));
        assert_eq!(v["members"][0]["rep_panes"][1]["workspace"], "");
    }
}
