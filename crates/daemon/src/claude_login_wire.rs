//! Whose Claude login a pane's agent runs on, said on the wire (ov-597).
//!
//! `claude_logins` decides it at a pane's first launch and records it. This
//! reads that record back for the apps, which show it in a pane's info
//! ("Uses Alex's Claude login").
//!
//! **Only where there is a choice.** A runner where no member has a login of
//! its own (ov-583 makes one the first time a member's pane launches) has
//! the owner's login alone, so every pane says nothing and a single-user
//! setup sees nothing new. Once any member has one, every pane says whose it
//! is, the owner's included, because "no label" would then be ambiguous.
//!
//! **The record's answer, never a guess.** A pane with no recorded login
//! runs on the runner's own, and that is what a reparented pane is (a
//! pane started afresh under the owner, as an orchestrator is when its lead
//! is removed): it reports the owner. A pane whose member has been removed
//! says nothing; its pane is stopped and the name is gone.
//!
//! No directory, login id or other part of the record is sent: only a
//! handle and the name people read.

use std::collections::HashMap;

use farcooler_protocol::v1 as wire;
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::{self, PaneMode};
use uuid::Uuid;

use crate::service::Service;

/// Who has a login on this runner, read once for a list or an event so a
/// hundred terminals do not each ask the store.
pub(crate) struct LoginsSeen {
    /// Every pane's recorded login.
    rows: HashMap<Uuid, TerminalLogin>,
    /// Each member by handle: their name, and the login id they have now.
    members: HashMap<String, (String, Option<String>)>,
    /// Each rep's member, for a rep from before logins were recorded.
    reps: HashMap<Uuid, String>,
}

impl Service {
    /// The runner's logins as they are now; `None` when no member has one,
    /// which is the owner's alone, where every pane says nothing.
    pub(crate) fn claude_logins_seen(&self) -> Option<LoginsSeen> {
        if self.store.member_login_ids().ok()?.is_empty() {
            return None;
        }
        let mut members = HashMap::new();
        let mut reps = HashMap::new();
        for member in self.store.members().ok()? {
            for rep in self.store.reps_of(&member.handle).ok()? {
                reps.insert(rep.terminal, member.handle.clone());
            }
            let login = self.store.member_login(&member.handle).ok()?;
            members.insert(member.handle, (member.name, login));
        }
        Some(LoginsSeen { rows: self.store.terminal_logins().ok()?.into_iter().collect(), members, reps })
    }

    /// Set `message.claude_login`; called by both places a terminal message
    /// is finished, so a list and an event cannot disagree.
    pub(crate) fn stamp_claude_login(&self, seen: &Option<LoginsSeen>, term: &models::Terminal, message: &mut wire::Terminal) {
        message.claude_login = seen.as_ref().and_then(|seen| seen.label(term));
    }
}

impl LoginsSeen {
    /// `term`'s login as the apps are told it. `None` for a pane that has
    /// none to show: a codex, web or changes pane (nothing in it reads a
    /// Claude login; a shell does, since `claude` typed there runs on it),
    /// and a pane whose member was removed or whose handle someone else holds
    /// now (the pane's login id is not theirs).
    fn label(&self, term: &models::Terminal) -> Option<wire::ClaudeLogin> {
        let runs_claude = term.command_preset == "shell" || term.command_preset.starts_with("claude");
        if !runs_claude || !matches!(term.pane_mode, PaneMode::Terminal | PaneMode::Agent) {
            return None;
        }
        let handle = match self.rows.get(&term.id) {
            Some(row) => {
                let (_, now) = self.members.get(&row.handle)?;
                if now.as_deref() != Some(row.login_id.as_str()) {
                    return None;
                }
                &row.handle
            }
            None => match self.reps.get(&term.id) {
                Some(handle) => handle,
                None => return Some(wire::ClaudeLogin { handle: String::new(), name: String::new(), owner: true }),
            },
        };
        let (name, _) = self.members.get(handle)?;
        Some(wire::ClaudeLogin { handle: handle.clone(), name: name.clone(), owner: false })
    }
}

#[cfg(test)]
#[path = "claude_login_wire_tests.rs"]
mod tests;
