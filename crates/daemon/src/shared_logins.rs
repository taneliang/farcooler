//! Members may share the runner's own logins (ov-652, the owner's decision
//! of Oct 21): "for all of these harnesses we should optionally allow
//! sharing of a single account. no need to enforce separation unless the
//! admin on the runner requires it".
//!
//! **The switch.** Per harness (Claude, Codex, Cursor), this runner's admins
//! (its owner's socket and any `host_admin` device, the scope `member.set`
//! takes) say whether members sign in with their own account
//! (`farcooler_store::member_login_policy`). Off by default.
//!
//! **Each launch of a member's pane decides afresh** (`launch_dir`):
//!
//! - **Claude** (and a shell, whose claude is a Claude login): their own
//!   login when they have signed in, or when the switch is on, as before
//!   (ov-583); otherwise the runner's own. Their own login wins: shared means
//!   allowed, not forced.
//! - **Codex and Cursor**: refused until they connect their own when the
//!   switch is on (Lane A); otherwise the runner's own. Neither has a member
//!   login yet.
//!
//! A pane that changes sides at a relaunch starts a new conversation: its
//! transcript is under the other login's directory.
//!
//! **Whose pane it is doesn't change.** A shared pane still records its
//! member and their generation (`TerminalLogin.shared`), so it speaks as
//! them, opens panes on their login, is stopped when they're removed, and
//! asks the Needs You of theirs; only the credential is the runner's.
//!
//! **What a shared pane carries** (the security reasoning):
//!
//! - nothing of the member's: no `CLAUDE_CONFIG_DIR`, so not their Claude
//!   login, and no Codex or Cursor credential (they have none here). So a
//!   member's Cursor key can never reach cursor-agent's default store and
//!   overwrite the owner's Keychain login (design §1.2);
//! - what any owner's pane carries: the daemon's environment and the
//!   owner's rc files, the owner's account, quota and history (Claude
//!   memory, Codex sessions, Cursor chats), and whatever key those export.
//!   That is what sharing is, and the switch's copy says so;
//! - and, for a rep, no claude.ai connector: those are the owner's on the
//!   owner's login, and a member's list names their own (`reps`).
//!
//! A member's own login is unchanged: `env -u` the owner's credentials, then
//! their directory (`claude_logins`).

use farcooler_core::Result;
use farcooler_protocol::v1 as pb;

use crate::service::Service;

/// The agent whose login a member's pane needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginHarness {
    Claude,
    Codex,
    Cursor,
}

impl LoginHarness {
    /// Whose login `preset` runs on: `codex…` and `cursor…` whatever their
    /// case (Lane A's rule), everything else Claude's. A shell's only login
    /// is a claude typed in it.
    pub fn of(preset: &str) -> LoginHarness {
        let base = preset.split(':').next().unwrap_or_default().to_ascii_lowercase();
        if base.starts_with("codex") {
            LoginHarness::Codex
        } else if base.starts_with("cursor") {
            LoginHarness::Cursor
        } else {
            LoginHarness::Claude
        }
    }

    /// As the store and the wire name it.
    pub fn name(self) -> &'static str {
        match self {
            LoginHarness::Claude => "claude",
            LoginHarness::Codex => "codex",
            LoginHarness::Cursor => "cursor",
        }
    }
}

impl Service {
    /// Whether this runner's admins require members to sign in to `harness`
    /// with their own account. A policy that can't be read requires it: a
    /// store error never puts a member on the owner's login.
    pub(crate) fn members_sign_in_own(&self, harness: LoginHarness) -> bool {
        match self.store.member_login_policy() {
            Ok(policy) => policy.members_own(harness.name()).unwrap_or(true),
            Err(e) => {
                tracing::warn!(error = %e.what(), "couldn't read whether members share the runner's logins; they don't");
                true
            }
        }
    }

    /// Whether `handle`'s pane of `harness` runs on their own login now:
    /// required, or, for Claude, they have one.
    pub(crate) fn on_own_login(&self, handle: &str, harness: LoginHarness) -> bool {
        self.members_sign_in_own(harness) || (harness == LoginHarness::Claude && self.member_signed_in(handle))
    }

    /// The Claude account `handle`'s Claude panes run on now: their own when
    /// they're on it, else the runner's.
    pub(crate) fn claude_account_of(&self, handle: &str) -> Option<String> {
        if self.on_own_login(handle, LoginHarness::Claude) {
            return crate::people::account_in(&self.member_claude_dir(handle)?.join(".claude.json"));
        }
        self.claude_account()
    }

    /// The switches as `member.list` tells them: `may_change` for a caller
    /// with `host_admin`, which `member.set_own_login` needs.
    pub(crate) fn own_logins_wire(&self, scope: pb::Scope) -> Result<pb::MemberOwnLogins> {
        let policy = self.store.member_login_policy()?;
        Ok(pb::MemberOwnLogins { claude: policy.claude, codex: policy.codex, cursor: policy.cursor, may_change: scope == pb::Scope::HostAdmin })
    }

    /// `member.set_own_login`: one harness's switch, as `by` (a device id,
    /// `-` for the local socket). `host_admin` is checked before this, by
    /// the method's scope. Logged with who changed it, as an enrollment is.
    pub(crate) fn set_own_login(&self, p: &pb::MemberOwnLoginSet, by: &str) -> Result<bool> {
        let harness = p.harness.trim();
        let changed = self.store.set_member_login_policy(harness, p.on, by, crate::review::now_millis())?;
        if changed {
            tracing::info!(by, harness, members_own = p.on, "changed whether members sign in to an agent with their own account");
        }
        Ok(changed)
    }
}

#[cfg(test)]
#[path = "shared_logins_tests.rs"]
mod tests;
