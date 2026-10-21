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

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;

use crate::service::Service;

/// The agent whose login a member's pane needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginHarness {
    Claude,
    Codex,
    Cursor,
    /// A shell, whose login is whatever is typed in it: any of the three.
    Shell,
}

impl LoginHarness {
    /// Whose login `preset` runs on: `codex…` and `cursor…` whatever their
    /// case (Lane A's rule), a shell any agent's (review 1, item 2),
    /// everything else Claude's.
    pub fn of(preset: &str) -> LoginHarness {
        let base = preset.split(':').next().unwrap_or_default().to_ascii_lowercase();
        if base == "shell" {
            LoginHarness::Shell
        } else if base.starts_with("codex") {
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
            LoginHarness::Shell => "shell",
        }
    }
}

/// Every switch on: what an unreadable policy reads as, so a store error
/// never puts a member on the owner's login (review 1, item 5).
const ALL_ON: farcooler_store::member_login_policy::MemberLoginPolicy =
    farcooler_store::member_login_policy::MemberLoginPolicy { claude: true, codex: true, cursor: true };

/// What a member's pane on the runner's own login is kept from (review 1,
/// item 1): the owner's connected apps and MCP servers, which aren't the
/// owner's account but things it reaches in the owner's name. Claude:
/// `--strict-mcp-config` with no `--mcp-config`, which drops the user's and
/// project's servers and claude.ai's connectors (claude 2.1.301, as `reps`
/// measured). Codex: its connected apps off (`features.apps`), and each MCP
/// server the owner's or the checkout's config.toml names turned off by
/// name, since `-c mcp_servers={}` merges rather than replaces (codex
/// 0.153.4, measured in a scratch `CODEX_HOME`). Cursor has no such switch:
/// its MCP servers stay, and the admins' copy says so. Hooks, plugins and
/// CLAUDE.md stay: Far Cooler's own hooks need them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    /// The codex MCP servers to turn off, by name.
    pub codex_servers: Vec<String>,
}

/// The claude flags a shared pane adds. A rep has its own (`reps`).
pub(crate) fn claude_flags(extras: &crate::service::LaunchExtras) -> &'static str {
    if extras.shared.is_some() && extras.rep.is_none() { " --strict-mcp-config" } else { "" }
}

/// The codex flags a shared pane adds, each with a space in front.
pub(crate) fn codex_flags(extras: &crate::service::LaunchExtras) -> String {
    let Some(kept) = &extras.shared else { return String::new() };
    let servers: String = kept.codex_servers.iter().map(|s| format!(" -c mcp_servers.{s}.enabled=false")).collect();
    format!(" -c features.apps=false{servers}")
}

/// The MCP servers a codex config.toml at `path` names, each by a name a
/// `-c` key can carry (letters, digits, `_`, `-`). `Err` when one can't be,
/// so its server can't be turned off: the launch is refused rather than left
/// with it (review 2, item 3). No file, or one that doesn't parse, names
/// none, as codex would refuse to start on it.
pub(crate) fn codex_servers_in(path: &std::path::Path) -> std::result::Result<Vec<String>, ()> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(Vec::new()) };
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else { return Ok(Vec::new()) };
    let Some(servers) = doc.get("mcp_servers").and_then(|t| t.as_table_like()) else { return Ok(Vec::new()) };
    let names: Vec<String> = servers.iter().map(|(name, _)| name.to_string()).collect();
    let plain = |n: &String| !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !names.iter().all(plain) {
        tracing::warn!("a codex MCP server's name can't be turned off for a member's shared pane; it isn't launched");
        return Err(());
    }
    Ok(names)
}

/// Login `login_id`'s Codex home: an empty directory under the runtime's
/// `codex/`, 0700, made if missing. Far Cooler's, never the owner's.
pub(crate) fn ensure_codex_home(runtime: &std::path::Path, login_id: &str) -> Result<String> {
    if !crate::claude_logins::is_login_id(login_id) {
        return Err(DomainError::InvalidArgument { what: "login" });
    }
    let root = runtime.join("codex");
    let dir = root.join(login_id);
    crate::claude_logins::private_dir(&root)
        .and_then(|()| crate::claude_logins::private_dir(&dir))
        .and_then(|()| std::fs::canonicalize(&dir))
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .ok_or_else(|| {
            tracing::warn!("couldn't make a member's Codex home; the launch is refused");
            DomainError::OperationFailed
        })
}

/// Delete login `login_id`'s Codex home, if it has one: a real directory
/// under a real `codex/`, never a link followed. `true` when one went.
pub(crate) fn delete_codex_home(runtime: &std::path::Path, login_id: &str) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind};
    if !crate::claude_logins::is_login_id(login_id) {
        return Err(Error::from(ErrorKind::InvalidInput));
    }
    let root = runtime.join("codex");
    for path in [root.clone(), root.join(login_id)] {
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => return Err(Error::other("not a directory")),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        }
    }
    std::fs::remove_dir_all(root.join(login_id)).map(|()| true)
}

impl Service {
    /// Whether this runner's admins require members to sign in to `harness`
    /// with their own account. A policy that can't be read requires it: a
    /// store error never puts a member on the owner's login.
    /// A shell shares the runner's logins only while every switch is off
    /// (review 1, item 2): anything can be typed in it.
    pub(crate) fn members_sign_in_own(&self, harness: LoginHarness) -> bool {
        let policy = self.login_policy();
        match harness {
            LoginHarness::Shell => policy.claude || policy.codex || policy.cursor,
            other => policy.members_own(other.name()).unwrap_or(true),
        }
    }

    /// The policy now, every switch on when it can't be read.
    pub(crate) fn login_policy(&self) -> farcooler_store::member_login_policy::MemberLoginPolicy {
        self.store.member_login_policy().unwrap_or_else(|e| {
            tracing::warn!(error = %e.what(), "couldn't read whether members share the runner's logins; they don't");
            ALL_ON
        })
    }

    /// What a member's pane is kept from, set on `extras` for its launch in
    /// `cwd`, from the row its launch recorded; nothing for anyone else's.
    ///
    /// - **Codex, in their shell and Claude panes** (review 2, item 1): while
    ///   the runner requires members' own Codex login, `CODEX_HOME` is an
    ///   empty directory of theirs, so a codex they or their agent run there
    ///   finds no login, not the owner's.
    /// - **On the runner's own login** (`Kept`): a Codex pane has the owner's
    ///   and the checkout's MCP servers turned off by name, and is refused
    ///   `codex_server_name` when one's name can't be (review 2, item 3); a
    ///   Claude pane's `--settings` carries a rep's MCP policy
    ///   (`reps::mcp_policy`), which denies the built-in servers that skip
    ///   `--strict-mcp-config` and turns claude.ai's connectors off (item 2).
    pub(crate) fn keep_from_members(&self, term: &farcooler_store::models::Terminal, cwd: &std::path::Path, extras: &mut crate::service::LaunchExtras) -> Result<()> {
        extras.shared = None;
        extras.codex_home = None;
        let Some(login) = self.store.terminal_login(term.id)? else { return Ok(()) };
        let harness = LoginHarness::of(&term.command_preset);
        if matches!(harness, LoginHarness::Claude | LoginHarness::Shell) && self.members_sign_in_own(LoginHarness::Codex) {
            extras.codex_home = Some(ensure_codex_home(self.root_dir(), &login.login_id)?);
        }
        if !login.shared {
            return Ok(());
        }
        let mut kept = Kept::default();
        if harness == LoginHarness::Codex {
            let user = crate::codex_trust::codex_home().map(|h| h.join("config.toml"));
            for path in user.into_iter().chain([cwd.join(".codex").join("config.toml")]) {
                kept.codex_servers.extend(codex_servers_in(&path).map_err(|()| DomainError::Conflict { what: "codex_server_name" })?);
            }
            kept.codex_servers.sort();
            kept.codex_servers.dedup();
        } else if crate::orchestrator::harness_of(&term.command_preset) == Some(crate::orchestrator::Harness::Claude) && extras.rep.is_none() {
            extras.settings = Some(self.shared_settings(term.id, extras.settings.as_deref())?);
        }
        extras.shared = Some(kept);
        Ok(())
    }

    /// The settings file a shared Claude pane launches with: the one it
    /// would have had, hooks and all, with a rep's MCP policy over it.
    /// Refused when it can't be written: the pane would start with the
    /// owner's built-in servers.
    fn shared_settings(&self, pane: uuid::Uuid, base: Option<&std::path::Path>) -> Result<std::path::PathBuf> {
        let mut settings = match base.map(std::fs::read_to_string) {
            Some(Ok(text)) => serde_json::from_str(&text).map_err(|_| DomainError::OperationFailed)?,
            Some(Err(_)) => return Err(DomainError::OperationFailed),
            None => serde_json::Value::Object(serde_json::Map::new()),
        };
        settings.as_object_mut().ok_or(DomainError::OperationFailed)?.extend(crate::reps::mcp_policy(false));
        let path = self.root_dir().join(format!("shared-{pane}.json"));
        let text = serde_json::to_string_pretty(&settings).map_err(|_| DomainError::OperationFailed)?;
        crate::reps::write_settings(&path, &text).map_err(|e| {
            tracing::warn!(terminal = %pane, error = %e, "couldn't write a shared pane's settings; it isn't launched");
            DomainError::OperationFailed
        })?;
        Ok(path)
    }

    /// Whether `handle`'s pane of `harness` runs on their own login now:
    /// required, or, for Claude and a shell, they have one.
    pub(crate) fn on_own_login(&self, handle: &str, harness: LoginHarness) -> bool {
        self.members_sign_in_own(harness) || (matches!(harness, LoginHarness::Claude | LoginHarness::Shell) && self.member_signed_in(handle))
    }

    /// The Claude account `handle`'s Claude panes run on now: their own when
    /// they're on it, else the runner's.
    pub(crate) fn claude_account_of(&self, handle: &str) -> Option<String> {
        if self.on_own_login(handle, LoginHarness::Claude) {
            return crate::people::account_in(&self.member_claude_dir(handle)?.join(".claude.json"));
        }
        self.claude_account()
    }

    /// Every switch on when the policy can't be read, as launches read it.
    pub(crate) fn own_logins_wire(&self, scope: pb::Scope) -> pb::MemberOwnLogins {
        let policy = self.login_policy();
        pb::MemberOwnLogins { claude: policy.claude, codex: policy.codex, cursor: policy.cursor, may_change: scope == pb::Scope::HostAdmin }
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
