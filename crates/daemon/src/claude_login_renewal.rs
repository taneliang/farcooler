//! A Claude login that stops working, and a member with none (ov-587, phase 5
//! of `reports/ov-581/design.md`; the hooks are `reports/ov-584/design.md`
//! section 3).
//!
//! **Detection.** A turn that ends in `FailureKind::Auth` (claude's
//! `authentication_failed` `StopFailure`) reaches `note_auth_failed` through
//! the supervisor's listener. The pane is mapped to a login by the one rule
//! launches use (`recorded_login`):
//!
//! - the runner's own login does nothing here, a member's pane shared onto it
//!   (ov-652) included. Its failed turn is what it was before: the pane's
//!   "Its last turn didn’t finish" block, for everyone;
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
//! **No login, no rep, when the runner requires one.** While this runner's
//! admins require members' own Claude logins (`shared_logins`, ov-652; off
//! by default), `rep.start` for a member who isn't signed in is refused
//! `claude_sign_in_needed`, and notes `asked_at`, which raises the sign-in
//! ask until they sign in. So is a wake or relaunch of a stopped rep whose
//! member has since signed out (`resume_rep`, `relaunch_dir`). With it off,
//! their rep runs on the runner's login instead, and an old ask is not shown.
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
//! which a claude typed there would use. `claude_key_exports` asks the login
//! shell which are set and warns the owner (ov-651).
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
use crate::shared_logins::LoginHarness;
use farcooler_store::claude_logins::TerminalLogin;
use crate::service::Service;

/// Why a member's Claude login is on the Needs You list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Their agents' turns fail to authenticate.
    Stopped,
    /// They have no login and asked for a rep.
    Asked,
    /// A Codex pane for them was refused: Codex has no login of theirs (ov-652).
    CodexAsked,
    /// A Cursor pane for them was refused: Cursor has no login of theirs (ov-652).
    CursorAsked,
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

/// A member's login that has just stopped working (ov-646).
pub(crate) struct Stopped {
    pub handle: String,
    pub login_id: String,
    pub at: i64,
}

impl Service {
    /// Whether `handle`'s directory names an account: a login exists, not
    /// that it still works.
    pub(crate) fn member_signed_in(&self, handle: &str) -> bool {
        self.member_claude_dir(handle).and_then(|d| crate::people::account_in(&d.join(".claude.json"))).is_some()
    }

    /// A turn in `terminal` failed to authenticate. `true` when that is news:
    /// it just made a member's login stopped working.
    #[cfg(test)]
    pub(crate) fn note_auth_failed(&self, terminal: Uuid) -> Result<bool> {
        Ok(self.note_auth_stop(terminal)?.is_some())
    }

    /// `note_auth_failed`, naming the stoppage when it is news.
    pub(crate) fn note_auth_stop(&self, terminal: Uuid) -> Result<Option<Stopped>> {
        let term = self.store.get_terminal(terminal)?;
        let (handle, login_id) = match self.recorded_login(&term)? {
            // A member's pane on the runner's own login fails as the owner's
            // does (ov-652): that says nothing about the member's.
            Login::Runner | Login::Member { recorded: Some(TerminalLogin { shared: true, .. }), .. } => return Ok(None),
            Login::Member { handle, recorded: Some(recorded) } => (handle, recorded.login_id),
            Login::Member { handle, recorded: None } => match self.store.member_login(&handle)? {
                Some(id) => (handle, id),
                None => return Ok(None),
            },
        };
        if self.store.member_login(&handle)?.as_deref() != Some(login_id.as_str()) || !self.member_signed_in(&handle) {
            return Ok(None);
        }
        let at = crate::review::now_millis();
        if !self.store.mark_login_stopped(&handle, &login_id, at)? {
            return Ok(None);
        }
        tracing::info!("a member's Claude login stopped working");
        Ok(Some(Stopped { handle, login_id, at }))
    }

    /// The relay account to tell that `stopped` happened to their login: the
    /// one the team directory gave the member when they were added by QR
    /// (ov-525). `None` for a member added by hand, who gets no push.
    pub(crate) fn stopped_member_account(&self, stopped: &Stopped) -> Option<String> {
        self.store.member(&stopped.handle).ok()?.account
    }

    /// Refuse `claude_sign_in_needed` a rep for a member who isn't signed in
    /// while the runner's admins require their own Claude login (ov-652),
    /// and note that they were asked to. Otherwise their rep may run on the
    /// runner's.
    pub(crate) fn refuse_rep_until_signed_in(&self, handle: &str) -> Result<()> {
        self.store.member(handle)?;
        if self.member_signed_in(handle) || !self.members_sign_in_own(LoginHarness::Claude) {
            return Ok(());
        }
        self.store.ensure_member_login(handle).map_err(|e| match e {
            DomainError::NotFound => DomainError::Conflict { what: "login_gone" },
            other => other,
        })?;
        self.store.mark_sign_in_asked(handle, crate::review::now_millis())?;
        Err(DomainError::Conflict { what: "claude_sign_in_needed" })
    }

    /// Refuse `codex_sign_in_needed` or `cursor_sign_in_needed` a pane of
    /// `preset` for a member, and note that they were asked to connect it
    /// (ov-652). Their Codex and Cursor have no login of their own yet, so
    /// the pane would run on the runner's: never. Any other preset passes.
    pub(crate) fn refuse_harness_until_connected(&self, handle: &str, preset: &str) -> Result<()> {
        let base = preset.split(':').next().unwrap_or_default().to_ascii_lowercase();
        let (harness, what) = if base.starts_with("codex") {
            ("codex", "codex_sign_in_needed")
        } else if base.starts_with("cursor") {
            ("cursor", "cursor_sign_in_needed")
        } else {
            return Ok(());
        };
        self.store.ensure_member_login(handle).map_err(|e| match e {
            DomainError::NotFound => DomainError::Conflict { what: "login_gone" },
            other => other,
        })?;
        self.store.mark_harness_asked(handle, harness, crate::review::now_millis())?;
        tracing::info!(harness, "refused a member's pane: that agent has no login of theirs yet");
        Err(DomainError::Conflict { what })
    }

    /// Refuse `unsupported` a member a preset Far Cooler doesn't name. The
    /// launch builder runs any plain identifier as a program (`agent`,
    /// Cursor's alias, among them), and nothing in it sets a member's login.
    pub(crate) fn refuse_unknown_preset(preset: &str) -> Result<()> {
        let base = preset.split(':').next().unwrap_or_default();
        let known = ["claude", "shell", "codex", "cursor", crate::service::CHANGES_PRESET, crate::web_pane::WEB_PRESET];
        if known.contains(&base) || base.to_ascii_lowercase().starts_with("codex") || base.to_ascii_lowercase().starts_with("cursor") {
            return Ok(());
        }
        Err(DomainError::Conflict { what: "unsupported" })
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
        self.login_needs_at(crate::review::now_millis())
    }

    /// `login_needs` as of `now_ms`: a Codex or Cursor ask older than
    /// `HARNESS_ASK_TTL_MS` is gone.
    pub(crate) fn login_needs_at(&self, now_ms: i64) -> Vec<LoginNeed> {
        let mut needs: Vec<LoginNeed> = Vec::new();
        let policy = self.login_policy();
        for flags in self.store.login_flags().unwrap_or_default() {
            let Ok(Some(name)) = self.store.member_name(&flags.handle) else { continue };
            let signed_in = self.member_signed_in(&flags.handle);
            // An ask stands only while its harness needs the member's own
            // login (ov-652): shared again, there's nothing to ask.
            let own = |h| policy.members_own(LoginHarness::name(h)).unwrap_or(true);
            let claude = match (flags.stopped_at, flags.asked_at) {
                (Some(at), _) if signed_in => Some((at, Why::Stopped)),
                (_, Some(at)) if !signed_in && own(LoginHarness::Claude) => Some((at, Why::Asked)),
                _ => None,
            };
            let others = [(flags.codex_asked_at, Why::CodexAsked, LoginHarness::Codex), (flags.cursor_asked_at, Why::CursorAsked, LoginHarness::Cursor)]
                .into_iter()
                .filter(|(_, _, h)| own(*h))
                .map(|(at, why, _)| (at, why));
            let found = claude.into_iter().chain(others.filter_map(|(at, why)| Some((at?, why))).filter(|(at, _)| now_ms - at <= farcooler_store::claude_logins::HARNESS_ASK_TTL_MS));
            for (since_ms, why) in found {
                needs.push(LoginNeed { handle: flags.handle.clone(), name: name.clone(), since_ms, why });
            }
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
    // One client with a deadline, for every push: the callback must not hang
    // on a relay that never answers.
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap_or_default();
    service.agents().on_auth_failed(move |terminal| {
        let (Some(service), Some(watcher)) = (service_weak.upgrade(), watcher_weak.upgrade()) else { return };
        let Some(stopped) = service.note_auth_stop(terminal).unwrap_or(None) else { return };
        watcher.announce_needs_you();
        watcher.announce_fleet_changed();
        tell_member(&service, &client, &stopped);
    });
}

/// Push the member's own phone that their login stopped working (ov-646):
/// once per stoppage, since `note_auth_stop` is news only once, and only for
/// a member who has a relay account. Off the caller's path: the relay's
/// answer, or its silence, never holds up the renewal.
fn tell_member(service: &Service, client: &reqwest::Client, stopped: &Stopped) {
    let Some(account) = service.stopped_member_account(stopped) else { return };
    let Some(pairing) = crate::push::Pairing::load_in(service.root_dir()) else { return };
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let event = crate::push::member_event_id(&stopped.login_id, stopped.at);
    let client = client.clone();
    let runner = crate::service::stable_host_id(service.install_id()).to_string();
    runtime.spawn(async move {
        crate::push::notify_member(&client, &pairing, &account, crate::push::MemberKind::Login, &event, Some(&runner)).await;
    });
}

#[cfg(test)]
#[path = "claude_login_renewal_tests.rs"]
mod tests;
