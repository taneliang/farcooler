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
use crate::{git, paths};

/// Launch presets. Coding agents run through the user's configured shell so
/// startup files, version managers, direnv, and aliases behave like a
/// hand-launched terminal. The default mode is an interactive login shell.
pub fn preset_command(preset: &str) -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    match preset {
        "shell" => format!("{shell} -il"),
        "claude" => format!("{shell} -ilc claude"),
        "codex" => format!("{shell} -ilc codex"),
        "cursor" => format!("{shell} -ilc cursor-agent"),
        other => format!("{shell} -ilc {other}"),
    }
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

        Ok(Self { store, tmux, inventory, host_id, root })
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
        let _ = self.tmux.kill_terminal_window(id).await;
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

        // 2. Create and tag the window.
        let command = preset_command(command_preset);
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

    /// Stop a terminal. Signals the daemon-owned window, then records intent.
    pub async fn stop_terminal(&self, id: Uuid) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        self.tmux.kill_terminal_window(id).await?;
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

        let command = preset_command(&term.command_preset);
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
        assert!(preset_command("claude").contains("-ilc claude"));
        assert!(preset_command("shell").ends_with("-il"));
        assert!(preset_command("cursor").contains("cursor-agent"));
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
}

/// Create a fifo. tmux writes into it, the daemon reads and forwards.
pub(crate) fn make_fifo(path: &str) -> Result<()> {
    let c = std::ffi::CString::new(path).map_err(|_| DomainError::OperationFailed)?;
    // SAFETY: c is a valid NUL terminated path for the duration of the call.
    let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
    if rc != 0 {
        tracing::warn!(path, "mkfifo failed");
        return Err(DomainError::OperationFailed);
    }
    Ok(())
}
