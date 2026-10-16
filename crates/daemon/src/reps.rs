//! A member's representative (ov-494): a Claude Code pane the runner binds to
//! one person in one workspace, which relays that person's asks to the
//! orchestrator and the orchestrator's questions back, and never answers for
//! them (ov-479 D1 and D3; `people` has the rules).
//!
//! It runs in the repository's main checkout, as the orchestrator does, with
//! the plugin's `rep` skill as its first message, told who it's for and whose
//! Claude account the runner uses (ov-479 D2). Its messages are filed on the
//! member's desk card in that workspace, made the first time and kept across
//! reps, so the card's thread is that person's conversation with the
//! orchestrator.
//!
//! Claude Code only: the account it runs on is the point of D2, and the skill
//! is the plugin's.

use farcooler_core::{DomainError, Result};
use farcooler_store::models::{self, Actor, TaskUpdate};
use uuid::Uuid;

use crate::service::Service;

/// Where a new pane sits before its first launch (`Service::open_terminal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    /// Its workspace's orchestrator.
    Orchestrator(Uuid),
    /// A member's rep in a workspace, by handle.
    Rep(Uuid, String),
}

/// The label every desk card carries, which the manager skill reads as "a
/// person's thread, never work to dispatch".
pub const DESK_LABEL: &str = "rep";

/// A rep that started: its pane, and its desk card.
#[derive(Debug, Clone)]
pub struct Started {
    pub terminal: models::Terminal,
    pub desk: models::Task,
}

/// The first message a rep starts on: the skill, by name, then who it's for,
/// and the two accounts it tells its person about first (ov-583): its own,
/// which is its person's login, and the lead's, which the orchestrator's work
/// for them runs on. `None` for an account whose email can't be read.
pub fn first_prompt(name: &str, handle: &str, workspace: &str, own: Option<&str>, lead: Option<&str>) -> String {
    let own = own.map_or_else(|| "their own Claude login on this runner".to_string(), |e| format!("their own Claude login, {e}"));
    let lead = lead.map_or_else(|| "the workspace lead's Claude account".to_string(), |e| format!("the workspace lead's Claude account, {e}"));
    format!("/farcooler:rep You represent {name} ({handle}) in the {workspace} workspace. You run on {own}. Work the orchestrator does for them runs on {lead}.")
}

/// `FARCOOLER_REP` for a rep's pane: its member's handle, which a handle's
/// own rule keeps plain (`valid_handle`).
pub(crate) fn env(store: &farcooler_store::Store, term: &models::Terminal) -> Option<(String, String)> {
    let rep = store.rep_of_terminal(term.id).ok().flatten()?;
    Some((farcooler_core::pane_env::REP.to_string(), rep.handle))
}

/// The tools a rep's Claude Code is never given (R-66): it writes no file and
/// fetches no page.
pub const DENIED_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit", "WebFetch"];

/// The Bash a rep runs without a prompt, after the CLI's absolute path. Only
/// so the gate's allow isn't followed by an ask: the boundary is the gate
/// (`rep_gate`), which refuses everything its grammar doesn't name whatever
/// these say, and the runner's allow-list (`people::rep_may`) behind it.
const CLI_COMMANDS: &[&str] = &["plan", "plan --json", "task show:*", "task list", "task list --json", "message orchestrator:*", "task note:*", "search:*"];

/// A rep's Claude Code launch arguments (R-66, ov-527):
///
/// - `--permission-mode dontAsk`: nothing is asked, so what isn't allowed is
///   refused.
/// - `--setting-sources=`: none of the user's, the project's or the local
///   settings, so neither the runner's user nor the repository adds an allow
///   rule. The `--settings` file, with the gate in it, still loads.
/// - `--strict-mcp-config`, with no `--mcp-config`: no MCP server starts,
///   not the checkout's `.mcp.json` nor the user's, so nothing runs at launch
///   that the gate never sees.
/// - The file and web tools denied by name, and the CLI's own commands
///   allowed without a prompt. **No allow rule for Read, Grep or Glob**: a
///   gate that answers nothing, or times out, leaves a read to `dontAsk`,
///   never to a rule, so only the gate's own allow lets one through.
///
/// Each list is one `--flag=value` word, because these flags take any number
/// of values and would otherwise swallow the prompt after them. `cli` is the
/// CLI's absolute path (`Service::rep_cli`), as the rep skill names it.
pub fn permission_args(cli: &str) -> Vec<String> {
    let quoted = crate::service::shell_quote(cli);
    let mut allowed = Vec::new();
    for spelling in [quoted.as_str(), cli] {
        allowed.extend(CLI_COMMANDS.iter().map(|c| format!("Bash({spelling} {c})")));
    }
    vec![
        "--permission-mode".to_string(),
        "dontAsk".to_string(),
        "--setting-sources=".to_string(),
        "--strict-mcp-config".to_string(),
        format!("--allowedTools={}", allowed.join(",")),
        format!("--disallowedTools={}", DENIED_TOOLS.join(",")),
    ]
}

/// One lock per rep pane, held across each of its launches: a resume
/// (`Service::resume_rep`) and a restart from a client alike (`relaunch`).
#[derive(Debug, Default)]
pub struct Resumes(
    std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    /// How many launches have begun, for a test to see that one is waiting.
    #[cfg(test)]
    std::sync::atomic::AtomicUsize,
);

impl Resumes {
    /// `pane`'s lock, made the first time it's asked for.
    pub(crate) fn lock_for(&self, pane: Uuid) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.0.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(pane).or_default().clone()
    }

    /// Drop `pane`'s lock once the pane is gone, so the map holds only live
    /// reps.
    pub(crate) fn forget(&self, pane: Uuid) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&pane);
    }

    /// Note that a launch has begun, for a test.
    #[cfg(test)]
    pub(crate) fn began(&self) {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// How many launches have begun, for a test.
    #[cfg(test)]
    pub(crate) fn begun(&self) -> usize {
        self.1.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether `pane` has a lock, for a test.
    #[cfg(test)]
    pub(crate) fn holds(&self, pane: Uuid) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&pane)
    }
}

/// `cli` as a rep may run it: by an absolute path, or not at all.
pub fn absolute_cli(cli: Option<&std::path::Path>) -> Option<String> {
    cli.filter(|c| c.is_absolute()).map(|c| c.display().to_string())
}

/// How long Claude Code waits on a rep's gate, in seconds. It answers from
/// memory and a lexical check, so this is generous; a gate that takes longer
/// is a gate that said nothing, which `permission_args` leaves refused.
pub const GATE_TIMEOUT_S: u64 = 10;

/// What a rep's launch carries beyond any agent's (`LaunchExtras::rep`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepLaunch {
    /// `permission_args`.
    pub args: Vec<String>,
    /// Its settings file, with the gate hook in it (`rep_settings`).
    pub settings: std::path::PathBuf,
    /// Its first message, sent again by a restart that has no conversation
    /// to resume, so it still starts on its skill.
    pub opening: String,
}

/// A rep's claude settings: the hooks every pane reports with
/// (`hook_install::claude_settings`), plus the rep gate as one more
/// `PreToolUse` hook on every tool, with `GATE_TIMEOUT_S`. `gate` is
/// `rep_gate_command`. Refused, never quietly without the gate, when there's
/// no `PreToolUse` list to add it to.
pub fn rep_settings(socket: &std::path::Path, gate: &str) -> Result<serde_json::Value> {
    with_gate(crate::hook_install::claude_settings(socket), gate)
}

/// `settings` with the gate added to its `PreToolUse` list, or a refusal
/// when it has none.
fn with_gate(mut settings: serde_json::Value, gate: &str) -> Result<serde_json::Value> {
    let entries = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or(DomainError::OperationFailed)?;
    entries.push(serde_json::json!({
        "matcher": "*",
        "hooks": [ { "type": "command", "command": gate, "timeout": GATE_TIMEOUT_S } ]
    }));
    Ok(settings)
}

/// Write `text` to `path` readable by this user only, through a temporary
/// file beside it renamed into place, so a Claude Code reading the file as
/// it's rewritten sees the old one or the new one, never half of either.
fn write_settings(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let temp = path.with_extension(format!("json.{}.tmp", Uuid::now_v7()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// The command a rep's gate hook runs: the CLI's `rep-gate`, naming the
/// checkout reads stay in and the CLI the rep may run. It **fails closed**:
/// `|| exit 2` turns a CLI that can't be run, or any failure of its own, into
/// Claude Code's blocking exit, which no allow rule overrides.
///
/// `git` is the runner's own, by its absolute path: the gate asks it whether a
/// file is tracked, and must not find one through the pane's `PATH`.
pub fn rep_gate_command(cli: &str, checkout: &std::path::Path, git: Option<&str>) -> String {
    let quoted = crate::service::shell_quote(cli);
    let checkout = crate::service::shell_quote(&checkout.display().to_string());
    // With no usable git the gate is told so, and refuses every tracked check;
    // it never goes looking on the pane's `PATH`.
    let git = git.map_or(" --no-git".to_string(), |g| format!(" --git {}", crate::service::shell_quote(g)));
    format!("{quoted} rep-gate --checkout {checkout} --cli {quoted}{git} || exit 2")
}

impl Service {
    /// What a launch of `term` in `ws` carries as a rep (`RepLaunch`): for a
    /// rep's Claude Code pane, on its first launch and every relaunch, with
    /// its settings file written fresh in the runtime directory; `None` for
    /// every other pane. A rep whose gate can't be written isn't launched
    /// (`rep_gate`): it would run without its boundary.
    pub(crate) fn rep_launch(&self, term: &models::Terminal, ws: &models::Worktree) -> Result<Option<RepLaunch>> {
        if crate::orchestrator::harness_of(&term.command_preset) != Some(crate::orchestrator::Harness::Claude) {
            return Ok(None);
        }
        let Some(rep) = self.store.rep_of_terminal(term.id)? else { return Ok(None) };
        // By its absolute path, never a name found through `PATH`, which the
        // checkout can influence: no rep runs without one.
        let Some(cli) = absolute_cli(self.rep_cli()) else {
            tracing::warn!(terminal = %term.id, "no CLI by an absolute path for a rep's gate; it isn't launched");
            return Err(DomainError::OperationFailed);
        };
        let git = crate::git::absolute_git().ok().and_then(|g| g.into_string().ok());
        let socket = crate::hook_ingress::HookIngress::socket_path(self.root_dir());
        let settings = rep_settings(&socket, &rep_gate_command(&cli, std::path::Path::new(&ws.worktree_path), git.as_deref()))?;
        let path = self.root_dir().join(format!("rep-{}.json", term.id));
        let written = serde_json::to_string_pretty(&settings).map_err(std::io::Error::other).and_then(|t| write_settings(&path, &t));
        if let Err(e) = written {
            tracing::warn!(terminal = %term.id, error = %e, "couldn't write a rep's gate; it isn't launched");
            return Err(DomainError::OperationFailed);
        }
        let member = self.store.member(&rep.handle)?;
        let board = self.store.get_workspace(rep.workspace)?;
        let (own, lead) = self.rep_accounts(rep.workspace, &rep.handle)?;
        Ok(Some(RepLaunch {
            args: permission_args(&cli),
            settings: path,
            opening: first_prompt(&member.name, &rep.handle, &board.name, own.as_deref(), lead.as_deref()),
        }))
    }

    /// Open `handle`'s rep in `workspace`, running `harness` (`claude`, or
    /// `claude:<model>`). One per member per workspace: a running one is
    /// refused (`rep_running`), and a stopped one is resumed (`resume_rep`),
    /// never replaced: its conversation is its person's thread, so it comes
    /// back on the harness it had.
    pub async fn start_rep(&self, workspace: Uuid, handle: &str, harness: &str) -> Result<Started> {
        let harness = match harness.trim() {
            "" => "claude",
            h => h,
        };
        if crate::orchestrator::harness_of(harness) != Some(crate::orchestrator::Harness::Claude) {
            return Err(DomainError::InvalidArgument { what: "command_preset" });
        }
        let member = self.store.member(handle)?;
        let board = self.store.get_workspace(workspace)?;
        if let Some(rep) = self.store.rep_for(workspace, handle)? {
            let pane = self.store.get_terminal(rep.terminal)?;
            if self.is_running(&pane) {
                return Err(DomainError::Conflict { what: "rep_running" });
            }
            let terminal = self.resume_rep(pane.id).await?;
            return Ok(Started { terminal, desk: self.desk(&board, &member)? });
        }
        let main = self
            .store
            .list_worktrees_for_repository(board.repository_id)?
            .into_iter()
            .find(|w| w.is_main_checkout)
            .ok_or(DomainError::NotFound)?;
        let desk = self.desk(&board, &member)?;
        let (own, lead) = self.rep_accounts(workspace, handle)?;
        let prompt = first_prompt(&member.name, handle, &board.name, own.as_deref(), lead.as_deref());
        let title = format!("{}'s rep", member.name);
        let seat = Some(Seat::Rep(workspace, handle.to_string()));
        let terminal = self.open_terminal(main.id, &title, harness, Some(&prompt), None, seat, None).await?;
        Ok(Started { terminal, desk })
    }

    /// The emails of `handle`'s own Claude login and of `workspace`'s lead's
    /// (the runner's own while the owner leads), each when it can be read.
    fn rep_accounts(&self, workspace: Uuid, handle: &str) -> Result<(Option<String>, Option<String>)> {
        let of = |h: &str| crate::people::account_in(&self.member_claude_dir(h)?.join(".claude.json"));
        let lead = match self.store.workspace_lead(workspace)? {
            Some(lead) => of(&lead),
            None => self.claude_account(),
        };
        Ok((of(handle), lead))
    }

    /// Bring a stopped rep's pane back on the conversation it had: a restart
    /// resumes the session its record holds (`agent_session_id`, through
    /// `respawn_command`), with its permission profile (`rep_launch`), and
    /// sends no first message again.
    ///
    /// **Single-flight** (`Resumes`): a second resume of the same pane, from
    /// another message or a `rep.start`, waits for the first and then finds
    /// the rep running, rather than restarting it twice.
    pub(crate) async fn resume_rep(&self, pane: Uuid) -> Result<models::Terminal> {
        let lock = self.rep_resumes.lock_for(pane);
        let _only_one = lock.lock().await;
        let row = self.store.get_terminal(pane)?;
        self.inventory.refresh().await;
        if self.is_running(&row) {
            return Ok(row);
        }
        tracing::info!(terminal = %pane, "resuming a stopped rep");
        self.relaunch_held(pane, false).await
    }

    /// `member`'s desk card on `board`: the one made before, or a new one,
    /// labeled `DESK_LABEL` and saying what it's for.
    fn desk(&self, board: &models::Workspace, member: &farcooler_store::people::Member) -> Result<models::Task> {
        if let Some(id) = self.store.rep_desk(board.id, &member.handle)? {
            if let Ok(task) = self.store.get_task(id) {
                return Ok(task);
            }
        }
        let made = self.store.create_task(board.id, &format!("{}'s desk", member.name), Actor::Runner)?;
        let update = TaskUpdate {
            title: made.title.clone(),
            intent: format!(
                "Where {}'s rep files their asks and answers for the orchestrator, and where the orchestrator's \
                 questions for them are kept. A person's thread, not work to dispatch.",
                member.name
            ),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            labels: vec![DESK_LABEL.to_string()],
            worktree_id: None,
        };
        let task = self.store.update_task(made.id, made.resource_version, &update)?;
        self.store.set_rep_desk(board.id, &member.handle, task.id)?;
        Ok(task)
    }

    /// Stop and remove every rep `handle` has, before they're removed.
    pub async fn stop_reps_of(&self, handle: &str) -> Result<()> {
        for rep in self.store.reps_of(handle)? {
            if self.store.get_terminal(rep.terminal).is_ok() {
                self.stop_terminal(rep.terminal).await?;
                self.remove_terminal(rep.terminal).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "reps_tests.rs"]
mod tests;
