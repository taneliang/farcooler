//! Removing a member wipes their Claude login from the runner (ov-588,
//! phase 6 of `reports/ov-581/design.md`).
//!
//! `member.remove` (`rpc_people`) stops their reps, revokes their devices,
//! removes the member, and starts the orchestrators they led afresh on the
//! owner's login (`lead_change`). Then `wipe_login`, here:
//!
//! 1. **Every pane on their login stops**: each pane recorded on it
//!    (`terminal_logins`), not just their reps.
//! 2. **It's signed out** with `claude auth logout` in its directory
//!    (ov-584), which also asks Anthropic to revoke it: deleting the Keychain
//!    entry alone leaves it valid there (ov-582 spike 2). What's confirmed is
//!    only that it's signed out here (ov-584 review M3). A sign-out that
//!    can't be confirmed is `revoke` in the answer, and only then is the
//!    admin told to have them sign out on claude.ai.
//! 3. **Its Keychain entry is deleted**, by exact service. claude names it
//!    `Claude Code-credentials-<first 8 hex of the sha256 of the
//!    CLAUDE_CONFIG_DIR string>` (ov-582), filed under the pane's `$USER` or
//!    `claude-code-user`. The daemon's `$USER` may not be the pane's (review
//!    1, M2), so it deletes by service alone, every entry, until there's
//!    none (`MAX_ENTRIES`): the service is this login's alone, whatever the
//!    account. A missing entry is no failure. The secret is never read.
//! 4. **Its directory is deleted**, `.credentials.json` (claude's fallback
//!    when the Keychain refuses) and all.
//!
//! **A launch that raced the removal** (review 1, M1): one that passed its
//! member check before the row went can record and spawn its pane after
//! the wipe looked. So the wipe runs in rounds (`MAX_ROUNDS`): stop every
//! running pane on the login, delete, read the panes again, and go round
//! once more if one is running. `panes` is reported from every pane seen.
//! No lock per handle: it would have to be held across every launch
//! (`open_terminal`, a relaunch) and taken inside a workspace's
//! orchestrator lock there, while removal takes that lock inside it
//! (`follow_leads`), so the order would invert.
//!
//! **Why after the member row goes, not before** (the brief had it last):
//! once the row is gone, nothing can launch on the login again
//! (`login_gone`), so no pane can start, re-make the directory or sign in
//! again between the stop and the wipe. The login id is read first, and
//! `terminal_logins` rows outlive the member on purpose (`farcooler_store::
//! claude_logins`), so nothing is lost by the order. A step that fails is
//! named in the answer (`MemberRemoval.failed`) and logged by its kind only;
//! the member is removed either way, and nothing relaunches on the login.
//!
//! **Threat model.** Any pane (the runner's one Unix user: it can write any
//! file or link under the runtime directory and set any variable), the
//! repository's files, a remote member (every request field; `member.remove`
//! is `host_admin` and takes only a handle) and other members may be hostile.
//! Removal must never delete the owner's login (`Claude Code-credentials`
//! with no suffix, or the runner's own `$CLAUDE_CONFIG_DIR`'s entry), nor
//! another member's directory or entry. So:
//!
//! - the login id is the store's (`member_logins`), a uuid only the daemon
//!   makes, never the handle or anything a request or pane says;
//! - the directory deleted is `<runtime>/claude/<login id>`, built from it,
//!   never globbed; a root or directory that's a link or isn't a directory
//!   is refused, and `remove_dir_all` never follows a link inside;
//! - a Keychain service is the hash of a string ending in
//!   `/claude/<login id>`: the canonical one, and each string recorded on
//!   that login's panes. One that the runner's own directory or another
//!   login's recorded or present directory hashes to as well is skipped.
//!   It always has a hash suffix, so it's never the owner's unsuffixed
//!   `Claude Code-credentials`. A member root that isn't a real directory
//!   (a link) refuses the Keychain step (review 1, L2);
//! - nothing depends on a pane's environment or a directory's contents.
//!
//! A guardrail, not a boundary, as `claude_logins` says: a same-user process
//! can delete any of this by hand. It stops mistakes and a pane tricking the
//! daemon, not someone determined.

use std::collections::BTreeSet;
use std::path::Path;

use farcooler_protocol::v1::{self as pb, TerminalIntent};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::claude_logins::{MEMBERS_DIR, is_login_id, member_root};
use crate::service::Service;

/// The Keychain service claude names for the runner's own login, with no
/// `CLAUDE_CONFIG_DIR`, and the prefix of every directory's.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// How many times a removal stops, deletes and looks again, while a pane
/// that raced it is found running on the login.
const MAX_ROUNDS: usize = 3;

/// The Keychain service claude keys a login in `config_dir` by: the string
/// exactly as passed, hashed (ov-582).
pub fn keychain_service(config_dir: &str) -> String {
    let hash = Sha256::digest(config_dir.as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{KEYCHAIN_SERVICE}-{hex}")
}

/// Whether `dir` is shaped like login `login_id`'s directory: absolute, and
/// ending `/claude/<login id>`.
fn names_login(dir: &str, login_id: &str) -> bool {
    let path = Path::new(dir);
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).map(str::to_string);
    is_login_id(login_id)
        && path.is_absolute()
        && name(path).as_deref() == Some(login_id)
        && path.parent().and_then(name).as_deref() == Some(MEMBERS_DIR)
}

/// The Keychain services to delete for login `login_id`, from the
/// directory strings it was launched with (`mine`): only strings shaped like
/// its directory, and never one a directory in `theirs` (the runner's own,
/// every other login's) hashes to as well. Never the unsuffixed service:
/// `keychain_service` always adds the hash.
pub fn keychain_targets(login_id: &str, mine: &[String], theirs: &[String]) -> Vec<String> {
    let taken: BTreeSet<String> = theirs.iter().map(|d| keychain_service(d)).collect();
    let mut services = Vec::new();
    for dir in mine.iter().filter(|d| names_login(d, login_id)) {
        let service = keychain_service(dir);
        if taken.contains(&service) {
            tracing::warn!("a removed member's Keychain entry shares its name with another login's; it's left");
            continue;
        }
        if !services.contains(&service) {
            services.push(service);
        }
    }
    services
}

/// `security`'s arguments to delete one generic password, the first with
/// exactly this service, whatever its account. Never `-w` or `-g`: the
/// secret is never read.
pub fn delete_args(service: &str) -> [&str; 3] {
    ["delete-generic-password", "-s", service]
}

/// The most entries one service's deletion removes before it's reported
/// failed: claude files one per account, and there are two it uses.
pub const MAX_ENTRIES: usize = 8;

/// What one `security` deletion found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deleted {
    /// An entry was there and is gone; there may be another.
    One,
    /// None was there.
    None,
}

/// How the runner deletes a Keychain entry. In the daemon's own tests it
/// keeps a pretend Keychain, records each call, and never runs `security`.
#[derive(Default)]
pub struct Keychain {
    /// Each service a deletion was asked for, in order.
    #[cfg(test)]
    pub(crate) deleted: std::sync::Mutex<Vec<String>>,
    /// How many entries each service has in the pretend Keychain.
    #[cfg(test)]
    pub(crate) entries: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    /// Make every deletion fail, for a test of the answer.
    #[cfg(test)]
    pub(crate) failing: std::sync::atomic::AtomicBool,
    /// Run once, at the first deletion: a test's launch that raced the
    /// removal (review 1, M1).
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    pub(crate) at_first_delete: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Keychain {
    /// Every entry with exactly `service` deleted, whatever its account:
    /// `Ok` once none is left, `Err` when one couldn't be, or when there
    /// were more than `MAX_ENTRIES`.
    async fn delete_all(&self, service: &str) -> Result<(), ()> {
        for _ in 0..MAX_ENTRIES {
            if self.delete(service).await? == Deleted::None {
                return Ok(());
            }
        }
        tracing::warn!(max = MAX_ENTRIES, "a removed member's Keychain service still had entries");
        Err(())
    }

    /// Delete the first generic password with `service`.
    async fn delete(&self, service: &str) -> Result<Deleted, ()> {
        #[cfg(test)]
        {
            let hook = self.at_first_delete.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(hook) = hook {
                hook();
            }
            self.deleted.lock().unwrap_or_else(|e| e.into_inner()).push(service.to_string());
            if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(());
            }
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            Ok(match entries.get_mut(service) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    Deleted::One
                }
                _ => Deleted::None,
            })
        }
        #[cfg(not(test))]
        {
            // A harness's daemon (stub agents, a stand-in) made no real
            // login, and must never touch the Keychain of whoever runs it.
            if crate::service::test_stub_agents() || crate::service::stand_in_agent().is_some() {
                return Ok(Deleted::None);
            }
            security_delete(service).await
        }
    }
}

/// `security delete-generic-password`, its output discarded: it prints the
/// item's attributes, never the secret, and nothing of it is logged.
#[cfg(not(test))]
async fn security_delete(service: &str) -> Result<Deleted, ()> {
    /// `security`'s exit status for an item that isn't there.
    const NOT_FOUND: i32 = 44;
    let mut command = tokio::process::Command::new("/usr/bin/security");
    command
        .args(delete_args(service))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(30), command.status()).await {
        Ok(Ok(status)) if status.success() => Ok(Deleted::One),
        Ok(Ok(status)) if status.code() == Some(NOT_FOUND) => Ok(Deleted::None),
        Ok(Ok(status)) => {
            tracing::warn!(code = ?status.code(), "couldn't delete a removed member's Keychain entry");
            Err(())
        }
        Ok(Err(e)) => {
            tracing::warn!(kind = ?e.kind(), "couldn't run security to delete a removed member's Keychain entry");
            Err(())
        }
        Err(_) => {
            tracing::warn!("security timed out deleting a removed member's Keychain entry");
            Err(())
        }
    }
}

/// Delete login `login_id`'s directory under `runtime`: `true` when there
/// was one. Refused for anything but a login id, and when the member root or
/// the directory is a link or isn't a directory.
pub fn delete_login_dir(runtime: &Path, login_id: &str) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind};
    if !is_login_id(login_id) {
        return Err(Error::from(ErrorKind::InvalidInput));
    }
    let root = member_root(runtime);
    match std::fs::symlink_metadata(&root) {
        Ok(m) if m.file_type().is_dir() => {}
        Ok(_) => return Err(Error::other("not a directory")),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    let dir = root.join(login_id);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_dir() => std::fs::remove_dir_all(&dir).map(|()| true),
        Ok(_) => Err(Error::other("not a directory")),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

impl Service {
    /// The panes running on login `login_id` now, and `handle`'s reps: what
    /// a removal stops, read before it starts.
    pub(crate) fn running_on_login(&self, handle: &str, login_id: Option<&str>) -> farcooler_core::Result<Vec<Uuid>> {
        let mut panes: Vec<Uuid> = self.store.reps_of(handle)?.into_iter().map(|r| r.terminal).collect();
        if let Some(login_id) = login_id {
            panes.extend(self.store.terminal_logins()?.into_iter().filter(|(_, l)| l.login_id == login_id).map(|(id, _)| id));
        }
        panes.sort();
        panes.dedup();
        Ok(panes.into_iter().filter(|id| self.on_running_intent(*id)).collect())
    }

    fn on_running_intent(&self, id: Uuid) -> bool {
        self.store.get_terminal(id).is_ok_and(|t| t.intent == TerminalIntent::Running)
    }

    /// Wipe a removed member's Claude login `login_id` (`None`: they never
    /// had one), after their row is gone: stop every pane still running on
    /// it, delete its Keychain entry, then its directory, and go round again
    /// while a pane that raced the removal is found running on it
    /// (`MAX_ROUNDS`). `before` is what `running_on_login` read before the
    /// removal began. `panes` is failed when any pane seen running on the
    /// login, then or since, still is.
    pub(crate) async fn wipe_login(&self, login_id: Option<&str>, before: &[Uuid]) -> pb::MemberRemoval {
        let mut seen: BTreeSet<Uuid> = before.iter().copied().collect();
        let mut failed = Vec::new();
        if let Some(login_id) = login_id {
            for _ in 0..MAX_ROUNDS {
                let recorded = self.recorded_logins();
                let running: Vec<Uuid> = recorded
                    .iter()
                    .filter(|(_, l)| l.login_id == login_id)
                    .map(|(id, _)| *id)
                    .chain(seen.iter().copied())
                    .filter(|p| self.on_running_intent(*p))
                    .collect();
                for pane in running {
                    seen.insert(pane);
                    if let Err(e) = self.stop_terminal(pane).await {
                        tracing::warn!(terminal = %pane, error = %e.what(), "couldn't stop a pane on a removed member's login");
                    }
                }
                failed = self.delete_login(login_id, &recorded).await;
                let raced = self.recorded_logins().iter().any(|(id, l)| l.login_id == login_id && self.on_running_intent(*id));
                if !raced {
                    break;
                }
                tracing::warn!("a pane started on a removed member's login during the removal; wiping again");
                seen.extend(self.recorded_logins().iter().filter(|(_, l)| l.login_id == login_id).map(|(id, _)| *id));
            }
        }
        if seen.iter().any(|p| self.on_running_intent(*p)) {
            failed.insert(0, "panes".to_string());
        }
        let panes_stopped = seen.iter().filter(|p| !self.on_running_intent(**p)).count();
        pb::MemberRemoval { panes_stopped: panes_stopped as u32, had_login: login_id.is_some(), failed }
    }

    /// Every pane's recorded login, or none when they can't be read (only
    /// the panes known before are stopped then).
    fn recorded_logins(&self) -> Vec<(Uuid, farcooler_store::claude_logins::TerminalLogin)> {
        self.store.terminal_logins().unwrap_or_else(|e| {
            tracing::warn!(error = %e.what(), "couldn't read the panes' logins");
            Vec::new()
        })
    }

    /// `member.remove` again for `handle`, who's no longer a member (review
    /// 1, L1: after a pane that couldn't be stopped is): each of their logins
    /// still on record, by a pane, wiped as the first removal would. `None`
    /// when there's none, and the removal is refused `NotFound` as before.
    /// A login is a generation's uuid, and none any member has now is wiped
    /// (review 2, Low 2: a row naming this handle can't reach another
    /// member's login), so this reaches nobody else's.
    pub(crate) async fn rewipe_gone(&self, handle: &str) -> farcooler_core::Result<Option<pb::MemberRemoval>> {
        let current: BTreeSet<String> = self.store.member_login_ids()?.into_iter().collect();
        let gone: BTreeSet<String> = self
            .store
            .terminal_logins()?
            .into_iter()
            .filter(|(_, l)| l.handle == handle && !current.contains(&l.login_id))
            .map(|(_, l)| l.login_id)
            .collect();
        if gone.is_empty() {
            return Ok(None);
        }
        let mut all = pb::MemberRemoval { had_login: true, ..Default::default() };
        for login_id in &gone {
            let one = self.wipe_login(Some(login_id), &[]).await;
            all.panes_stopped += one.panes_stopped;
            for step in one.failed {
                if !all.failed.contains(&step) {
                    all.failed.push(step);
                }
            }
        }
        Ok(Some(all))
    }

    /// Wipe what a finish signed in after its member was removed (ov-584
    /// review M2): signed out, its Keychain entry and its directory deleted,
    /// as a removal does. The steps that failed, as `MemberRemoval.failed`
    /// words.
    pub(crate) async fn wipe_sign_in(&self, login_id: &str) -> Vec<String> {
        let recorded = self.recorded_logins();
        self.delete_login(login_id, &recorded).await
    }

    /// Delete login `login_id`'s Keychain entries, then its directory: the
    /// steps that failed, as `MemberRemoval.failed` words.
    async fn delete_login(&self, login_id: &str, recorded: &[(Uuid, farcooler_store::claude_logins::TerminalLogin)]) -> Vec<String> {
        let mut failed = Vec::new();
        match self.login_dir_strings(login_id, recorded) {
            Some(mine) => {
                let theirs = self.other_login_dirs(login_id, recorded);
                if !self.revoke_login(login_id, &mine, &theirs).await {
                    failed.push("revoke".to_string());
                }
                let mut keychain_ok = true;
                for service in keychain_targets(login_id, &mine, &theirs) {
                    keychain_ok &= self.claude_logins.keychain.delete_all(&service).await.is_ok();
                }
                if !keychain_ok {
                    failed.push("keychain".to_string());
                }
            }
            None => {
                tracing::warn!("the member root isn't a real directory; a removed member's Keychain entry is left");
                failed.push("keychain".to_string());
            }
        }
        if let Err(e) = delete_login_dir(self.root_dir(), login_id) {
            tracing::warn!(kind = ?e.kind(), "couldn't delete a removed member's Claude directory");
            failed.push("login_dir".to_string());
        }
        // Their empty Codex home too (ov-652), which a codex they signed in
        // to from their own shell may have filled.
        if let Err(e) = crate::shared_logins::delete_codex_home(self.root_dir(), login_id) {
            tracing::warn!(kind = ?e.kind(), "couldn't delete a removed member's Codex home");
            failed.push("login_dir".to_string());
        }
        failed
    }

    /// Sign login `login_id` out with `claude auth logout` in each of its
    /// directory strings that's still a directory (ov-584), before its
    /// Keychain entry goes: that also asks Anthropic to revoke it, which
    /// deleting the entry alone never does (ov-582 spike 2). Only a
    /// string shaped like its own directory, and never one another login's
    /// or the runner's own hashes to as well (`keychain_targets`' rule):
    /// `false` when one couldn't be confirmed signed out. A runner that may
    /// run no claude (a harness, the daemon's own tests without a stand-in)
    /// revokes nothing and says so in its log.
    async fn revoke_login(&self, login_id: &str, mine: &[String], theirs: &[String]) -> bool {
        let Ok(program) = self.sign_in_program() else {
            tracing::info!("no claude to sign a removed member out with; their Keychain entry is still deleted");
            return true;
        };
        let taken: BTreeSet<String> = theirs.iter().map(|d| keychain_service(d)).collect();
        let mut dirs: Vec<&String> = mine
            .iter()
            .filter(|d| names_login(d, login_id) && !taken.contains(&keychain_service(d)))
            .filter(|d| std::fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_dir()))
            .collect();
        dirs.sort();
        dirs.dedup();
        let mut all = true;
        for dir in dirs {
            all &= self.claude_logout(&program, dir).await;
        }
        if !all {
            tracing::warn!("a removed member's Claude login couldn't be confirmed signed out");
        }
        all
    }

    /// Every string login `login_id`'s directory was or would be handed to
    /// claude as: the canonical one now, and each its panes recorded. `None`
    /// when the member root is there but isn't a real directory, before or
    /// after it's resolved (review 1, L2): a root swapped for a link names
    /// some other directory.
    fn login_dir_strings(&self, login_id: &str, recorded: &[(Uuid, farcooler_store::claude_logins::TerminalLogin)]) -> Option<Vec<String>> {
        let root = member_root(self.root_dir());
        let real = |p: &Path| match std::fs::symlink_metadata(p) {
            Ok(m) => Some(m.file_type().is_dir()),
            Err(_) => None,
        };
        let canonical = match real(&root) {
            Some(false) => return None,
            None => None,
            Some(true) => {
                let resolved = std::fs::canonicalize(&root).ok();
                if real(&root) != Some(true) {
                    return None;
                }
                resolved.and_then(|r| r.join(login_id).into_os_string().into_string().ok())
            }
        };
        Some(canonical.into_iter().chain(recorded.iter().filter(|(_, l)| l.login_id == login_id).map(|(_, l)| l.config_dir.clone())).collect())
    }

    /// Every directory string that's someone else's: the runner's own, each
    /// other login's recorded on a pane, and each other member directory
    /// there is now.
    fn other_login_dirs(&self, login_id: &str, recorded: &[(Uuid, farcooler_store::claude_logins::TerminalLogin)]) -> Vec<String> {
        let runner = self.claude_logins.runner.iter().filter_map(|p| p.to_str().map(str::to_string));
        let others = recorded.iter().filter(|(_, l)| l.login_id != login_id).map(|(_, l)| l.config_dir.clone());
        let present = std::fs::canonicalize(member_root(self.root_dir())).ok().map(|root| {
            crate::claude_logins::member_dirs(self.root_dir())
                .into_iter()
                .filter_map(|d| d.file_name().and_then(|n| n.to_str()).map(str::to_string))
                .filter(|name| name != login_id)
                .filter_map(|name| root.join(name).into_os_string().into_string().ok())
                .collect::<Vec<_>>()
        });
        runner.chain(others).chain(present.into_iter().flatten()).collect()
    }
}

#[cfg(test)]
#[path = "member_removal_tests.rs"]
mod tests;
