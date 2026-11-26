//! Browser and native renderers can show the same runner connection state.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Connection {
    Up,
    Reconnecting { tunnel: Tunnel },
    Connecting,
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tunnel {
    Dialing,
    Ready,
    Failed,
}

#[derive(Serialize)]
pub struct Health {
    pub state: &'static str,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub action: Option<&'static str>,
}

pub fn health(runner: &str, connection: Connection) -> Health {
    let retry = Some("Reconnect");
    let reconnecting = || Some(format!("Reconnecting to {runner}…"));
    let (state, title, detail, action) = match connection {
        Connection::Up => ("Connected", None, None, None),
        Connection::Reconnecting { tunnel } => {
            let detail = match tunnel {
                Tunnel::Failed => format!("Far Cooler couldn’t reach {runner} yet. Select Reconnect to try again, then touch your passkey."),
                _ => "The connection dropped. Select Reconnect, then touch your passkey. The terminal comes back as you left it.".into(),
            };
            ("Disconnected", reconnecting(), Some(detail), retry)
        }
        Connection::Connecting => (
            "Connecting",
            reconnecting(),
            Some("Complete the passkey prompt to sign in.".into()),
            None,
        ),
        Connection::Failed => (
            "Unavailable",
            Some(format!("Couldn’t reconnect to {runner}")),
            None,
            retry,
        ),
    };
    Health {
        state,
        title,
        detail,
        action,
    }
}

pub fn runtime_health(host: &farcooler_protocol::v1::Host) -> String {
    use farcooler_protocol::v1::SelfHealth;
    match SelfHealth::try_from(host.self_health) {
        Ok(SelfHealth::Healthy) => "Runner healthy".into(),
        Ok(SelfHealth::Degraded) => "Runner degraded. Check tmux on this runner.".into(),
        _ => "Runner health unavailable".into(),
    }
}

pub fn attention(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 item needs your attention".into()),
        n => Some(format!("{n} items need your attention")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_and_a_failed_reconnect_keep_recovery_within_the_client() {
        assert_eq!(health("build", Connection::Up).state, "Connected");
        let lost = health(
            "build",
            Connection::Reconnecting {
                tunnel: Tunnel::Ready,
            },
        );
        assert_eq!(lost.state, "Disconnected");
        assert_eq!(lost.action, Some("Reconnect"));
        assert_eq!(health("build", Connection::Failed).state, "Unavailable");
        assert_eq!(health("build", Connection::Connecting).action, None);
    }

    #[test]
    fn attention_has_an_accessible_label_only_while_work_is_waiting() {
        assert_eq!(attention(0), None);
        assert_eq!(attention(1).as_deref(), Some("1 item needs your attention"));
        assert_eq!(attention(2).as_deref(), Some("2 items need your attention"));
    }
}

/// The branch diff review surface uses the shared Changes RPCs, like native.
pub fn review_files(changes: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "title": "Review Changes",
        "pick": "Changed File",
        "empty": "There are no committed changes on this branch.",
        "files": changes["files"],
    })
}

pub fn review_patch(diff: &serde_json::Value) -> String {
    if diff["unsupported"].is_string() {
        return "This file cannot be shown as a text diff.".into();
    }
    let mut patch = String::new();
    if let Some(hunks) = diff["hunks"].as_array() {
        for hunk in hunks {
            patch.push_str(hunk["header"].as_str().unwrap_or_default());
            if !patch.ends_with('\n') {
                patch.push('\n');
            }
            if let Some(lines) = hunk["lines"].as_array() {
                for line in lines {
                    patch.push(match line["kind"].as_str() {
                        Some("added") => '+',
                        Some("removed") => '-',
                        _ => ' ',
                    });
                    patch.push_str(line["text"].as_str().unwrap_or_default());
                    patch.push('\n');
                }
            }
        }
    }
    if diff["truncated"] == true {
        patch.push_str("The runner shortened this diff.\n");
    }
    if patch.is_empty() {
        "No text changes.".into()
    } else {
        patch
    }
}
