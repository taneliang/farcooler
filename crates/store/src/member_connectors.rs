//! The claude.ai connectors each member lets their rep use (ov-586, phase 4
//! of ov-581, R-66 as amended on Oct 16).
//!
//! A rep runs on its member's own Claude login (ov-583), so it could reach
//! every connector that member has in claude.ai. It reaches only the ones
//! named here: the rep gate (`farcooler_daemon::rep_gate`) refuses every
//! other MCP tool. **Empty by default**, so a member who turns on nothing
//! has a rep with no connector at all, as before.
//!
//! A connector is kept by the name claude.ai shows for it ("Slack", "Google
//! Drive"), and matched by the key claude derives from it ([`connector_key`]),
//! so two spellings of one key are one connector.
//!
//! Who may change a list is the daemon's (`rpc_people`): the member
//! themself, from their own device, and the owner. Never a process in an
//! agent pane, a rep's included.
//!
//! **A guardrail, not a boundary** (review L3), the model ov-580 and ov-583
//! accepted: a process in a shell-role pane, one that escaped its pane's
//! process tree, and a caller whose preamble claims are taken at its word
//! all still count as the owner, and may set anyone's list.

use rusqlite::params;

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// One new table, its rows going with their member by cascade, whichever
/// build removes them. A build from before it never names it: its reps run
/// with no connector, as every rep did then.
pub(crate) fn migration_0054_member_connectors(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE member_connectors (
             handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             server TEXT NOT NULL,
             PRIMARY KEY (handle, server)
         );",
    )
}

/// The most connectors one member turns on: far more than anyone has, and a
/// bound on the file the gate reads on every call.
pub const MOST_CONNECTORS: usize = 32;

/// The longest connector name kept, in characters.
const LONGEST_NAME: usize = 64;

/// What claude.ai prefixes a connector's server name with in Claude Code,
/// which a name may be typed with and is kept without.
const SERVER_PREFIX: &str = "claude.ai ";

/// The key Claude Code builds a connector's tool names from: `Slack` is the
/// server `claude.ai Slack`, whose tools are `mcp__claude_ai_Slack__<tool>`.
///
/// Exactly as claude 2.1.301 does it (its bundle's server-name normalizer,
/// read with grep): every character outside `A-Z`, `a-z`, `0-9`, `_` and `-`
/// becomes `_`, and for a name starting `claude.ai ` each run of `_` is one,
/// and none leads or trails. So `Google Drive` is `claude_ai_Google_Drive`,
/// `Slack (2)` (claude's name for a second connector called Slack) is
/// `claude_ai_Slack_2`, and a key never holds `__`, which is what keeps a
/// tool name's server segment whole.
pub fn connector_key(name: &str) -> String {
    let server = format!("{SERVER_PREFIX}{}", name.strip_prefix(SERVER_PREFIX).unwrap_or(name));
    let mut key = String::with_capacity(server.len());
    for c in server.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' };
        if !(c == '_' && key.ends_with('_')) {
            key.push(c);
        }
    }
    key.trim_matches('_').to_string()
}

/// `name` as kept: trimmed, without the `claude.ai ` prefix claude shows it
/// with, or refused as `connector` when it's empty, too long, holds a
/// control character, or has no letter or digit to make a key from.
fn kept_name(name: &str) -> Result<String> {
    let name = name.trim_start();
    let name = name.strip_prefix(SERVER_PREFIX).unwrap_or(name).trim();
    if name.is_empty()
        || name.chars().count() > LONGEST_NAME
        || name.chars().any(char::is_control)
        || !name.chars().any(|c| c.is_ascii_alphanumeric())
    {
        return Err(DomainError::InvalidArgument { what: "connector" });
    }
    Ok(name.to_string())
}

impl Store {
    /// The connectors `handle` lets their rep use, by name, sorted. Empty
    /// for a member who turned none on, and for nobody by that handle.
    pub fn member_connectors(&self, handle: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT server FROM member_connectors WHERE handle = ?1 ORDER BY server").map_err(map_err)?;
        let rows = stmt.query_map([handle], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }

    /// Replace `handle`'s connectors with exactly `names`, answering the list
    /// as kept. Two names with one key are one connector, the first kept.
    /// Refused, writing nothing: `NotFound` for no such member, `connector`
    /// for a name `kept_name` refuses, `connectors` for more than
    /// [`MOST_CONNECTORS`].
    pub fn set_member_connectors(&self, handle: &str, names: &[String]) -> Result<Vec<String>> {
        let mut kept: Vec<String> = Vec::new();
        for name in names {
            let name = kept_name(name)?;
            if !kept.iter().any(|k| connector_key(k) == connector_key(&name)) {
                kept.push(name);
            }
        }
        if kept.len() > MOST_CONNECTORS {
            return Err(DomainError::InvalidArgument { what: "connectors" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let known: bool = tx
            .query_row("SELECT EXISTS(SELECT 1 FROM members WHERE handle = ?1)", [handle], |r| r.get(0))
            .map_err(map_err)?;
        if !known {
            return Err(DomainError::NotFound);
        }
        tx.execute("DELETE FROM member_connectors WHERE handle = ?1", [handle]).map_err(map_err)?;
        for name in &kept {
            tx.execute("INSERT INTO member_connectors (handle, server) VALUES (?1, ?2)", params![handle, name])
                .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        drop(conn);
        self.member_connectors(handle)
    }
}

#[cfg(test)]
#[path = "member_connectors_tests.rs"]
mod tests;
