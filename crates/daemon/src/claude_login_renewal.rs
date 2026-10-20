//! A Claude login that stops working, and a member with none (ov-587, phase 5
//! of `reports/ov-581/design.md`; the hooks are `reports/ov-584/design.md`
//! section 3).
//!
//! **Detection.** A turn that ends in `FailureKind::Auth` (claude's
//! `authentication_failed` `StopFailure`) reaches `note_auth_failed` through
//! the supervisor's listener. The pane is mapped to a login by the one rule
//! launches use (`recorded_login`):
//!
//! - the runner's own login does nothing here. Its failed turn is what it was
//!   before: the pane's "Its last turn didn’t finish" block, for everyone;
//! - a member's login is noted `stopped_at` on their `member_logins` row, once,
//!   and only for their current generation (a removed member's old pane says
//!   nothing about whoever holds the handle now).
//!
//! **Only from signed in.** A failure is noted only while the member's
//! directory still names an account. A deliberate Sign Out empties it and
//! clears the note, and the panes still running on the signed-out login fail
//! their next turn: those failures are what the member asked for, so they
//! note nothing and never nag. (A login that expires or is revoked keeps its
//! account, so that case is noted. A `claude auth logout` run by hand in the
//! directory is indistinguishable from a Sign Out, and reads as `none`.)
//!
//! **The note is the state.** `member_claude_login` reports a signed-in member
//! with a note as `stopped_working`; signing in again, or out, clears it, and
//! with it the Needs You item (`needs_you::login`). Which devices see the item
//! is decided there.
//!
//! **No login, no rep.** `rep.start` for a member who isn't signed in is
//! refused `claude_sign_in_needed`, and notes `asked_at`, which raises the
//! sign-in ask until they sign in. So is a wake or relaunch of a stopped rep
//! whose member has since signed out (`resume_rep`, `relaunch_dir`). It never
//! starts the rep on the owner's login: `launch_dir` has no path from a
//! member to the runner's own, and ov-581's owner-fallback switch was dropped
//! by the owner (Oct 16), so there is no setting for one.
//!
//! **Nor on the owner's credential.** A member's claude is launched under
//! `env -u ANTHROPIC_API_KEY -u CLAUDE_CODE_OAUTH_TOKEN -u
//! ANTHROPIC_AUTH_TOKEN`, inside the login shell's `-ilc` payload, so the
//! owner's rc files, which run first, cannot put one back; and so are the
//! sign-in, status and logout processes
//! (`claude_logins::env_without_owner_credentials`). A credential in the
//! environment outranks the config directory's login. A member's plain shell
//! (`env -u … {shell} -il`) is covered only against the daemon's own
//! environment: its rc files run after the unset and may export one again,
//! which a claude typed there would use. Nothing detects that yet.
//!
//! **A stale failure is not news.** A turn that began on the old token can
//! fail just after Sign In Again, so a failure within `SIGN_IN_GRACE_MS` of
//! the sign-in is ignored (the daemon is not told when a turn began).
//!
//! **Who reads the list.** A caller inside a pane is read by that pane's
//! login (`needs_you_viewer`), not by the local socket's owner.

use farcooler_core::{DomainError, Result};
use uuid::Uuid;

use crate::claude_logins::Login;
use crate::service::Service;

/// Why a member's Claude login is on the Needs You list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Their agents' turns fail to authenticate.
    Stopped,
    /// They have no login and asked for a rep.
    Asked,
}

/// One member's login that someone has to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginNeed {
    pub handle: String,
    /// The name people read; never their email.
    pub name: String,
    pub since_ms: i64,
    pub why: Why,
}

impl Service {
    /// Whether `handle`'s directory names an account: a login exists, not
    /// that it still works.
    pub(crate) fn member_signed_in(&self, handle: &str) -> bool {
        self.member_claude_dir(handle).and_then(|d| crate::people::account_in(&d.join(".claude.json"))).is_some()
    }

    /// A turn in `terminal` failed to authenticate. `true` when that is news:
    /// it just made a member's login stopped working.
    pub(crate) fn note_auth_failed(&self, terminal: Uuid) -> Result<bool> {
        let term = self.store.get_terminal(terminal)?;
        let (handle, login_id) = match self.recorded_login(&term)? {
            Login::Runner => return Ok(false),
            Login::Member { handle, recorded: Some(recorded) } => (handle, recorded.login_id),
            Login::Member { handle, recorded: None } => match self.store.member_login(&handle)? {
                Some(id) => (handle, id),
                None => return Ok(false),
            },
        };
        if self.store.member_login(&handle)?.as_deref() != Some(login_id.as_str()) || !self.member_signed_in(&handle) {
            return Ok(false);
        }
        let news = self.store.mark_login_stopped(&handle, &login_id, crate::review::now_millis())?;
        if news {
            tracing::info!("a member's Claude login stopped working");
        }
        Ok(news)
    }

    /// Refuse `claude_sign_in_needed` a rep for a member who isn't signed in,
    /// and note that they were asked to.
    pub(crate) fn refuse_rep_until_signed_in(&self, handle: &str) -> Result<()> {
        self.store.member(handle)?;
        if self.member_signed_in(handle) {
            return Ok(());
        }
        self.store.ensure_member_login(handle).map_err(|e| match e {
            DomainError::NotFound => DomainError::Conflict { what: "login_gone" },
            other => other,
        })?;
        self.store.mark_sign_in_asked(handle, crate::review::now_millis())?;
        Err(DomainError::Conflict { what: "claude_sign_in_needed" })
    }

    /// Who is reading Needs You: the caller's person, except that a caller
    /// inside a pane reads as that pane's login, as `login_for_new_pane`
    /// decides whose a pane it opens is. Bob's rep, or `farcooler needs-you`
    /// typed in it, comes in on the local socket as the owner, and must not
    /// be told Alice's login stopped working. A pane on the runner's own
    /// login is the owner's; one on a login that is gone reads as nobody.
    pub(crate) fn needs_you_viewer(&self, peer: &farcooler_transport::Peer, caller_pid: Option<i32>) -> Result<crate::people::Person> {
        use crate::people::Person;
        if let Some(pane) = caller_pid.and_then(|pid| crate::people::pane_of(self, pid)) {
            let term = self.store.get_terminal(pane)?;
            return Ok(match self.recorded_login(&term)? {
                Login::Runner => Person::Owner,
                login @ Login::Member { .. } if self.refuse_a_gone_generation(&login).is_err() => Person::Released,
                Login::Member { handle, .. } => Person::Member(handle),
            });
        }
        crate::people::person_of(&self.store, peer)
    }

    /// Every login that needs its member, oldest first.
    pub(crate) fn login_needs(&self) -> Vec<LoginNeed> {
        let mut needs: Vec<LoginNeed> = Vec::new();
        for flags in self.store.login_flags().unwrap_or_default() {
            let Ok(Some(name)) = self.store.member_name(&flags.handle) else { continue };
            let signed_in = self.member_signed_in(&flags.handle);
            let (since_ms, why) = match (flags.stopped_at, flags.asked_at) {
                (Some(at), _) if signed_in => (at, Why::Stopped),
                (_, Some(at)) if !signed_in => (at, Why::Asked),
                _ => continue,
            };
            needs.push(LoginNeed { handle: flags.handle, name, since_ms, why });
        }
        needs.sort_by(|a, b| a.since_ms.cmp(&b.since_ms).then_with(|| a.handle.cmp(&b.handle)));
        needs
    }
}

/// Hear every turn that fails to authenticate and note whose login it was.
/// Weak, as the other listeners are: the service outlives no watcher it owns.
/// A note that is news moves the Needs You list.
pub(crate) fn listen_for_auth_failures(service: &std::sync::Arc<Service>, watcher: &std::sync::Arc<crate::watch::Watcher>) {
    let (service_weak, watcher_weak) = (std::sync::Arc::downgrade(service), std::sync::Arc::downgrade(watcher));
    service.agents().on_auth_failed(move |terminal| {
        let (Some(service), Some(watcher)) = (service_weak.upgrade(), watcher_weak.upgrade()) else { return };
        if service.note_auth_failed(terminal).unwrap_or(false) {
            watcher.announce_needs_you();
            watcher.announce_fleet_changed();
        }
    });
}

#[cfg(test)]
#[path = "claude_login_renewal_tests.rs"]
mod tests;
