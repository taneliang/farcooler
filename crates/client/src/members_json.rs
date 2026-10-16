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
/// workspace}], account, email}], claude_account, you}`. A rep's `workspace`
/// is "" from a runner before ov-508 sent it, and `account` and `email` are ""
/// for a member added by hand or from a runner before ov-525.
///
/// `member.remove`'s answer adds `removed` (ov-588): `{panes_stopped,
/// had_login, login_deleted, failed: [panes|keychain|login_dir],
/// sign_out_on_claude_ai}`. `login_deleted` is whether their Claude login is
/// gone from the runner, Keychain entry and directory both.
/// `sign_out_on_claude_ai` is whether to tell the admin to have them sign out
/// of that login's session on claude.ai, which a runner can't end.
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
            json!({
                "handle": m.handle, "name": m.name, "devices": m.client_ids, "reps": m.rep_terminal_ids.len(),
                "rep_panes": panes, "account": m.account, "email": m.email,
            })
        })
        .collect();
    let mut answer = json!({ "members": members, "claude_account": list.claude_account, "you": list.you });
    if let Some(removed) = &list.removed {
        answer["removed"] = removal_json(removed);
    }
    answer
}

/// What `member.remove` did to the member's Claude login.
fn removal_json(removed: &pb::MemberRemoval) -> Value {
    let wiped = removed.had_login && !removed.failed.iter().any(|f| f == "keychain" || f == "login_dir");
    json!({
        "panes_stopped": removed.panes_stopped, "had_login": removed.had_login, "login_deleted": wiped,
        "failed": removed.failed, "sign_out_on_claude_ai": removed.had_login,
    })
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
                account: "user_alice".into(),
                email: "alice@example.com".into(),
            }],
            claude_account: "team@example.com".into(),
            you: "alice".into(),
            removed: None,
        };
        let v = members_json(&list);
        assert!(v.get("removed").is_none(), "only a removal's answer says what it removed");
        assert_eq!(v["claude_account"], "team@example.com");
        assert_eq!(v["you"], "alice");
        assert_eq!(v["members"][0]["name"], "Alice Chen");
        assert_eq!(v["members"][0]["devices"], json!(["phone-a"]));
        assert_eq!(v["members"][0]["reps"], 2);
        assert_eq!(v["members"][0]["rep_panes"][0], json!({ "terminal": pane.to_string(), "workspace": board.to_string() }));
        assert_eq!(v["members"][0]["rep_panes"][1]["workspace"], "");
        assert_eq!((&v["members"][0]["account"], &v["members"][0]["email"]), (&json!("user_alice"), &json!("alice@example.com")));
    }

    /// A removal's answer says what was wiped of the member's Claude login,
    /// what wasn't, and whether to have them sign out on claude.ai (ov-588).
    #[test]
    fn a_removal_says_what_it_wiped() {
        let removed = |had_login, failed: &[&str]| pb::MemberList {
            removed: Some(pb::MemberRemoval { panes_stopped: 3, had_login, failed: failed.iter().map(|f| f.to_string()).collect() }),
            ..Default::default()
        };
        let all = members_json(&removed(true, &[]));
        assert_eq!(
            all["removed"],
            json!({ "panes_stopped": 3, "had_login": true, "login_deleted": true, "failed": [], "sign_out_on_claude_ai": true })
        );
        let left = members_json(&removed(true, &["panes", "keychain"]));
        assert_eq!((&left["removed"]["login_deleted"], &left["removed"]["failed"]), (&json!(false), &json!(["panes", "keychain"])));
        let none = members_json(&removed(false, &[]));
        assert_eq!((&none["removed"]["login_deleted"], &none["removed"]["sign_out_on_claude_ai"]), (&json!(false), &json!(false)));
    }
}
