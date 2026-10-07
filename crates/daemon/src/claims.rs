//! Which workspace a worktree belongs to, when nobody said.
//!
//! A worktree is claimed by the first of three signals to name it, and the
//! claim sticks (`Store::claim_worktree`); only `worktree assign` moves one,
//! and `reconcile`, which gives a main checkout another workspace holds back
//! to Main. A main checkout is Main's alone: no signal here claims it for
//! any other workspace.
//!
//! 1. **Explicit**: `worktree create` or dispatch names the workspace. That
//!    happens in `Service::create_worktree_with` and `adopt_branch`, and the
//!    main checkout is Main's from `reconcile`. Nothing here.
//! 2. **Hook**: a Claude Code hook reports the `cwd` its session is in. On the
//!    events Far Cooler registers (session start, prompt submit, stop, and
//!    `MessageDisplay`), that is where the turn is working, by the end of the
//!    turn at the latest. `hook_ingress` hands it to `observe_in`. Codex and
//!    Cursor get nothing from this signal: the spike (spec, "Spike findings
//!    (2026-09-27)") found Codex's `cwd` is always its launch directory and
//!    Cursor's registered events carry no `cwd` at all, only
//!    `workspace_roots`, which doesn't move either.
//! 3. **Process**: every tick, `scan` walks each pane's processes by parent
//!    pid, reads each one's working directory from the kernel (`proc_cwd`),
//!    and observes it. By parent and not by tty, because Codex and Cursor run
//!    each command as a child with no tty. It sees a worktree only while
//!    something is running there, so it's the weakest of the three.
//!
//! Rules the signals share, in `observe_in` (and `scan`, which checks the
//! terminal from the row it already has):
//! - A `cwd` belongs to the worktree with the LONGEST path containing it.
//!   Worktrees nest inside the main checkout (`.worktrees/x`), and a shorter
//!   match would hand every nested worktree's activity to Main.
//! - Orchestrators never claim: a Codex orchestrator runs in the main
//!   checkout, and would otherwise claim it for its own workspace.
//! - No stealing. A terminal of workspace B working in a worktree A owns is
//!   recorded as a foreign writer in the `Ledger`, and ownership stays put.
//!   `Service::foreign_writers` and `Worktree.foreign_writer_workspace_ids`
//!   report it.
//! - A report lasts until the same signal next sees that terminal working in
//!   a worktree: each observation replaces what that signal said before. So
//!   one read-only visit (a `git -C` into Main's checkout) is reported while
//!   it's seen, and clears once the terminal is seen in its own worktree or
//!   an unclaimed one, rather than for the rest of the terminal's life.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use farcooler_core::inventory::RuntimeSnapshot;
use farcooler_core::{DomainError, Result};
use farcooler_store::Store;
use farcooler_store::models::{ClaimSource, TerminalRole, Worktree};
use uuid::Uuid;

use crate::foreground::Foreground;
use crate::service::{Service, WorktreeView};

/// The worktree whose path contains `cwd`, choosing the longest such path.
///
/// Both sides are resolved first, since Cursor reports a path as the model
/// typed it (`/tmp/…`) and git and the kernel report `/private/tmp/…`. A
/// path that won't resolve is compared as given. Containment is by whole
/// components, so `/r/.worktrees/xy` isn't inside `/r/.worktrees/x`.
pub fn containing_worktree<'a>(worktrees: &'a [Worktree], cwd: &Path) -> Option<&'a Worktree> {
    let resolved: Vec<(PathBuf, &Worktree)> =
        worktrees.iter().map(|w| (canonical(Path::new(&w.worktree_path)), w)).collect();
    longest_containing(&resolved, &canonical(cwd)).map(|(_, w)| w)
}

/// `containing_worktree` over paths already resolved, so one tick of `scan`
/// resolves each worktree once rather than once per process. The resolved
/// path comes back with the row, for the walk's nested-checkout check.
fn longest_containing<'r, 'a>(
    resolved: &'r [(PathBuf, &'a Worktree)],
    cwd: &Path,
) -> Option<(&'r Path, &'a Worktree)> {
    resolved
        .iter()
        .filter(|(path, _)| cwd.starts_with(path))
        .max_by_key(|(path, _)| path.components().count())
        .map(|(path, w)| (path.as_path(), *w))
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `cwd` is in a checkout nested inside `worktree` that has no row
/// yet: an ancestor of `cwd` strictly below `worktree` holds a `.git`.
///
/// An agent that runs `git worktree add .worktrees/x` and works there is
/// ahead of `reconcile` for a moment, and until then the longest known path
/// containing it is the main checkout. Acting on that would claim the wrong
/// worktree, or report a writer in Main's checkout that was never there.
/// Nothing is decided instead; the next observation, after adoption, finds
/// the new row.
fn inside_an_unknown_checkout(cwd: &Path, worktree: &Path) -> bool {
    cwd.ancestors()
        .take_while(|dir| *dir != worktree && dir.starts_with(worktree))
        .any(|dir| dir.join(".git").exists())
}

/// What one observation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// Nothing to decide: an orchestrator, a terminal with no workspace, a
    /// `cwd` in no known worktree, or one ahead of `reconcile`.
    Nothing,
    /// The worktree had no owner and is now the terminal's workspace's.
    Claimed(Uuid),
    /// The worktree is already the terminal's workspace's.
    Owned(Uuid),
    /// Another workspace owns the worktree; the terminal is recorded as a
    /// foreign writer there.
    Foreign(Uuid),
}

/// Which signal saw a terminal. Each keeps its own report, because they see
/// different things: a Claude hook's `cwd` follows a `cd` in its Bash tool
/// while the Claude process itself stays where it started, so the walk
/// reading that process mustn't clear what the hook said, nor the other way
/// round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Signal {
    Hook,
    Process,
}

impl Signal {
    fn of(source: ClaimSource) -> Signal {
        match source {
            ClaimSource::Process => Signal::Process,
            _ => Signal::Hook,
        }
    }
}

/// Terminals seen working in a worktree another workspace owns, and whether
/// anything about claims changed since the watcher last asked.
///
/// In memory only, like the rest of the daemon's runtime truth: a restarted
/// daemon sees them again the next time they work there.
#[derive(Debug, Default)]
pub struct Ledger {
    /// Per terminal and signal, the worktrees another workspace owns that the
    /// signal last saw it working in. Replaced each time that signal sees the
    /// terminal in any worktree (`settle`), so a report clears once the
    /// terminal is seen working somewhere else. A terminal that has ended is
    /// left out when read (`Service::foreign_writers`), and its entries go
    /// with its record (`forget_terminal`).
    seen: Mutex<HashMap<(Uuid, Signal), HashSet<Uuid>>>,
    /// Set by a claim, or by a foreign writer arriving or leaving; taken by
    /// the watcher, which announces `fleet_changed`. A flag and not a call,
    /// because the hook path has no watcher to call.
    changed: AtomicBool,
}

impl Ledger {
    /// Say where `signal` now sees `terminal` writing as a foreigner: exactly
    /// `foreign`, which may be nowhere. Marked as news if that differs from
    /// what the signal said before.
    fn settle(&self, terminal: Uuid, signal: Signal, foreign: HashSet<Uuid>) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let same = match seen.get(&(terminal, signal)) {
            Some(before) => *before == foreign,
            None => foreign.is_empty(),
        };
        if same {
            return;
        }
        if foreign.is_empty() {
            seen.remove(&(terminal, signal));
        } else {
            seen.insert((terminal, signal), foreign);
        }
        drop(seen);
        self.mark_changed();
    }

    /// The terminals recorded as writing in `worktree`, by either signal.
    pub fn terminals_in(&self, worktree: Uuid) -> Vec<Uuid> {
        let mut found: Vec<Uuid> = Vec::new();
        for ((terminal, _), w) in self.seen.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            if w.contains(&worktree) && !found.contains(terminal) {
                found.push(*terminal);
            }
        }
        found
    }

    /// Drop a terminal whose record is gone.
    pub fn forget_terminal(&self, terminal: Uuid) {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).retain(|(t, _), _| *t != terminal);
    }

    fn mark_changed(&self) {
        self.changed.store(true, Ordering::Relaxed);
    }

    /// Hold this ledger's lock until the guard drops, so a test can make
    /// every claim check wait on it.
    #[cfg(test)]
    pub(crate) fn lock_for_test(&self) -> impl Sized + '_ {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a claim, or a foreign writer arriving or leaving, happened
    /// since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }
}

/// One observation of a terminal working in `cwd`, through a `Service`.
pub fn observe(svc: &Service, terminal: Uuid, cwd: &Path, source: ClaimSource) -> Result<Observed> {
    observe_in(&svc.store, svc.claims(), terminal, cwd, source)
}

/// One observation of a terminal working in `cwd`.
///
/// In order: a terminal that's gone or is an orchestrator, or has no
/// workspace, decides nothing. Then the longest worktree containing `cwd`:
/// none, or one `cwd` is only inside because a nested checkout isn't adopted
/// yet, decides nothing. An unclaimed one is claimed for the terminal's
/// workspace with `source`. One owned by another workspace is where this
/// signal now reports the terminal as a foreign writer. One the terminal's
/// workspace owns, or one it has just claimed, clears what this signal
/// reported before: the terminal is working somewhere else now.
pub fn observe_in(
    store: &Store,
    ledger: &Ledger,
    terminal: Uuid,
    cwd: &Path,
    source: ClaimSource,
) -> Result<Observed> {
    let term = match store.get_terminal(terminal) {
        Ok(t) => t,
        Err(DomainError::NotFound) => return Ok(Observed::Nothing),
        Err(e) => return Err(e),
    };
    if term.role == TerminalRole::Orchestrator {
        return Ok(Observed::Nothing);
    }
    let Some(workspace) = term.workspace_id else { return Ok(Observed::Nothing) };
    let seen = judge(store, ledger, terminal, workspace, cwd, source)?;
    match seen {
        Observed::Nothing => {}
        Observed::Claimed(_) | Observed::Owned(_) => ledger.settle(terminal, Signal::of(source), HashSet::new()),
        Observed::Foreign(w) => ledger.settle(terminal, Signal::of(source), HashSet::from([w])),
    }
    Ok(seen)
}

/// `observe_in` past its terminal checks, for a terminal of `workspace`
/// already known not to be an orchestrator: `scan` has the row in hand.
/// Claims, but leaves the ledger's reports to its caller, which knows
/// everything the signal saw at once.
fn judge(
    store: &Store,
    ledger: &Ledger,
    terminal: Uuid,
    workspace: Uuid,
    cwd: &Path,
    source: ClaimSource,
) -> Result<Observed> {
    // Hidden rows included: hiding never stops what's running in a worktree.
    let worktrees: Vec<Worktree> = store
        .list_worktrees_in_order()?
        .into_iter()
        .filter(|w| !w.worktree_missing && !w.creation_failed)
        .collect();
    let cwd = canonical(cwd);
    let Some(worktree) = containing_worktree(&worktrees, &cwd) else { return Ok(Observed::Nothing) };
    if inside_an_unknown_checkout(&cwd, &canonical(Path::new(&worktree.worktree_path))) {
        return Ok(Observed::Nothing);
    }

    match worktree.workspace_id {
        None => {
            // A terminal of one repository in another's worktree can't claim
            // it; the store would refuse with `other_repository`.
            let repository = store.get_workspace(workspace)?.repository_id;
            if repository != worktree.repository_id {
                return Ok(Observed::Nothing);
            }
            match store.claim_worktree(worktree.id, workspace, source)? {
                Some(_) => {
                    tracing::info!(
                        worktree = %worktree.id,
                        %workspace,
                        %terminal,
                        source = source.as_str(),
                        "a worktree is claimed by the workspace working in it"
                    );
                    ledger.mark_changed();
                    Ok(Observed::Claimed(worktree.id))
                }
                // Claimed by someone else between the read and the write,
                // or a main checkout, which only Main may claim. Judged
                // again at the next observation: the main checkout is
                // Main's by the next reconcile pass, and this terminal
                // then reads as a foreign writer there.
                None => Ok(Observed::Nothing),
            }
        }
        Some(owner) if owner == workspace => Ok(Observed::Owned(worktree.id)),
        Some(_) => Ok(Observed::Foreign(worktree.id)),
    }
}

/// One tick of the process walk, over the fleet, the panes and the process
/// table the watcher already read this tick.
pub fn scan(svc: &Service, fleet: &[WorktreeView], snapshot: &RuntimeSnapshot, table: &Foreground) {
    scan_with(&svc.store, svc.claims(), fleet, snapshot, table, crate::proc_cwd::cwd_of);
}

/// `scan`, with the kernel's answer replaceable, so a test can say where a
/// pid is working.
///
/// Every terminal with a workspace that isn't an orchestrator, and whose
/// pane is alive, has every process under its tty read. A pane whose
/// process has exited is dead to tmux, so an ended terminal is skipped by
/// that.
///
/// Each terminal is judged on everything it was seen doing this tick,
/// together: the worktrees another workspace owns that any of its processes
/// is working in become what the walk reports for it, replacing last tick's.
/// So a command that ran in Main's checkout is reported while it runs, and
/// no longer once the tick after it finds the terminal only in its own
/// worktree. A terminal none of whose processes is in a known worktree
/// keeps what was reported. Only a `cwd` in an unclaimed worktree reaches
/// `judge` and its store reads; the rest is decided from this tick's rows.
pub(crate) fn scan_with(
    store: &Store,
    ledger: &Ledger,
    fleet: &[WorktreeView],
    snapshot: &RuntimeSnapshot,
    table: &Foreground,
    cwd_of: impl Fn(i32) -> Option<PathBuf>,
) {
    let mut seen: Vec<(Uuid, Uuid, HashSet<PathBuf>)> = Vec::new();
    for view in fleet {
        for t in view.terminals.iter().map(|v| &v.terminal) {
            let Some(workspace) = t.workspace_id else { continue };
            if t.role == TerminalRole::Orchestrator {
                continue;
            }
            let Some(pane) = snapshot.panes.iter().find(|p| p.terminal_id == t.id && !p.dead) else {
                continue;
            };
            let cwds: HashSet<PathBuf> =
                table.under_tty(pane.tty.trim_start_matches("/dev/")).into_iter().filter_map(&cwd_of).collect();
            if !cwds.is_empty() {
                seen.push((t.id, workspace, cwds));
            }
        }
    }
    if seen.is_empty() {
        return;
    }

    let worktrees = match store.list_worktrees_in_order() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the worktrees; this tick claims nothing");
            return;
        }
    };
    let resolved: Vec<(PathBuf, &Worktree)> = worktrees
        .iter()
        .filter(|w| !w.worktree_missing && !w.creation_failed)
        .map(|w| (canonical(Path::new(&w.worktree_path)), w))
        .collect();
    for (terminal, workspace, cwds) in seen {
        let mut placed = false;
        let mut foreign: HashSet<Uuid> = HashSet::new();
        for cwd in cwds {
            let Some((path, worktree)) = longest_containing(&resolved, &cwd) else { continue };
            match worktree.workspace_id {
                Some(owner) if owner == workspace => placed = true,
                Some(_) => {
                    if !inside_an_unknown_checkout(&cwd, path) {
                        placed = true;
                        foreign.insert(worktree.id);
                    }
                }
                None => match judge(store, ledger, terminal, workspace, &cwd, ClaimSource::Process) {
                    Ok(Observed::Nothing) => {}
                    Ok(Observed::Claimed(_) | Observed::Owned(_)) => placed = true,
                    Ok(Observed::Foreign(w)) => {
                        placed = true;
                        foreign.insert(w);
                    }
                    Err(e) => tracing::debug!(%terminal, error = %e, "a process observation could not be judged"),
                },
            }
        }
        if placed {
            ledger.settle(terminal, Signal::Process, foreign);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_core::inventory::TaggedPane;

    fn wt(path: &str) -> Worktree {
        Worktree {
            id: Uuid::now_v7(),
            repository_id: Uuid::nil(),
            branch: String::new(),
            worktree_path: path.into(),
            hidden: false,
            creation_failed: false,
            is_main_checkout: false,
            worktree_missing: false,
            ordinal: 0,
            resource_version: 1,
            workspace_id: None,
            claim_source: None,
        }
    }

    /// The fixture repository's main checkout row, which `fixture()` registers
    /// and gives to Main (F18).
    fn main_worktree(svc: &Service, repo: Uuid) -> Uuid {
        svc.store
            .list_worktrees_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| w.is_main_checkout)
            .expect("fixture registers its main checkout")
            .id
    }

    /// A real directory inside the fixture's repository, with an unclaimed
    /// row for it, the way `reconcile` leaves a worktree it adopted.
    fn nested_worktree(svc: &Service, repo: Uuid, name: &str) -> (Uuid, PathBuf) {
        let main = svc.store.get_worktree(main_worktree(svc, repo)).unwrap();
        let path = canonical(Path::new(&main.worktree_path)).join(".worktrees").join(name);
        std::fs::create_dir_all(&path).unwrap();
        let id = svc.store.create_unclaimed_worktree_for_test(repo, &path.to_string_lossy());
        (id, path)
    }

    #[test]
    fn the_longest_worktree_path_wins() {
        let rows = [wt("/r"), wt("/r/.worktrees/x"), wt("/r/.worktrees/xy")];
        let hit = containing_worktree(&rows, Path::new("/r/.worktrees/x/src/lib")).unwrap();
        assert_eq!(hit.worktree_path, "/r/.worktrees/x");
        let hit = containing_worktree(&rows, Path::new("/r/.worktrees/xy")).unwrap();
        assert_eq!(hit.worktree_path, "/r/.worktrees/xy", "a path prefix is not a directory prefix");
        let hit = containing_worktree(&rows, Path::new("/r/src")).unwrap();
        assert_eq!(hit.worktree_path, "/r");
        // Without `xy` among the rows, only a string prefix would put `xy`
        // inside `x`, however the rows are ordered.
        let rows = [wt("/r"), wt("/r/.worktrees/x")];
        let hit = containing_worktree(&rows, Path::new("/r/.worktrees/xy/src")).unwrap();
        assert_eq!(hit.worktree_path, "/r", "a sibling whose name starts the same is not inside");
        assert_eq!(containing_worktree(&rows, Path::new("/elsewhere")), None);
    }

    /// Cursor reports `/tmp/…` as typed; git and the kernel say
    /// `/private/tmp/…` on macOS. Both must name one worktree.
    #[test]
    fn a_worktree_is_found_through_a_symlink_in_either_path() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("src")).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let rows = [wt(&real.to_string_lossy())];
        assert!(containing_worktree(&rows, &link.join("src")).is_some());
        let rows = [wt(&link.to_string_lossy())];
        assert!(containing_worktree(&rows, &real.join("src")).is_some());
    }

    #[tokio::test]
    async fn an_agent_working_in_an_unclaimed_worktree_claims_it_for_its_workspace() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let (wt, path) = nested_worktree(&svc, repo, "x");
        let term = svc.store.create_terminal_for_test(main_worktree(&svc, repo), billing.id);
        std::fs::create_dir_all(path.join("src")).unwrap();
        let seen = observe(&svc, term, &path.join("src"), ClaimSource::Hook).unwrap();
        assert_eq!(seen, Observed::Claimed(wt));
        let row = svc.store.get_worktree(wt).unwrap();
        assert_eq!((row.workspace_id, row.claim_source), (Some(billing.id), Some(ClaimSource::Hook)));
        assert!(svc.claims().take_changed(), "a claim is news for every client");
        assert!(!svc.claims().take_changed(), "and it's told once");
    }

    #[tokio::test]
    async fn an_orchestrator_never_claims() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let (wt, path) = nested_worktree(&svc, repo, "x");
        let term = svc.store.create_terminal_for_test(main_worktree(&svc, repo), billing.id);
        svc.store.set_terminal_role(term, TerminalRole::Orchestrator).unwrap();
        assert_eq!(observe(&svc, term, &path, ClaimSource::Hook).unwrap(), Observed::Nothing);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None);
    }

    /// The first claim sticks: a second workspace working there later is a
    /// foreign writer, reported, and ownership stays where it was.
    #[tokio::test]
    async fn working_in_another_workspaces_worktree_is_reported_not_stolen() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let main = svc.store.ensure_main_workspace(repo).unwrap();
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let main_wt = main_worktree(&svc, repo);
        // Billing's terminal lives in Billing's own worktree, so nothing about
        // where it sits says it's working in Main's.
        let (home_wt, _) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty());

        let path = svc.store.get_worktree(main_wt).unwrap().worktree_path;
        let seen = observe(&svc, term, Path::new(&path), ClaimSource::Hook).unwrap();
        assert_eq!(seen, Observed::Foreign(main_wt));
        let row = svc.store.get_worktree(main_wt).unwrap();
        assert_eq!(row.workspace_id, Some(main.id));
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id]);
        assert!(svc.claims().take_changed(), "a new foreign writer is news");
        observe(&svc, term, Path::new(&path), ClaimSource::Hook).unwrap();
        assert!(!svc.claims().take_changed(), "the same one again isn't");
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id], "reported once");

        // A later claim of another kind doesn't move it either.
        observe(&svc, term, Path::new(&path), ClaimSource::Process).unwrap();
        assert_eq!(svc.store.get_worktree(main_wt).unwrap().workspace_id, Some(main.id));
    }

    /// A foreign writer stops being reported once its terminal has ended, and
    /// is dropped when its record goes.
    #[tokio::test]
    async fn a_foreign_writer_leaves_with_its_terminal() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let main_wt = main_worktree(&svc, repo);
        let (home_wt, _) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);
        let path = svc.store.get_worktree(main_wt).unwrap().worktree_path;
        observe(&svc, term, Path::new(&path), ClaimSource::Hook).unwrap();
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id]);

        // Made its workspace's orchestrator afterwards, it's no longer a
        // second writer, by the rule that says so for one sitting there.
        svc.store.set_terminal_role(term, TerminalRole::Orchestrator).unwrap();
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "an orchestrator isn't a writer");
        svc.store.set_terminal_role(term, TerminalRole::Agent).unwrap();
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id]);

        // Its process exited on its own: the exit is on the row.
        let row = svc.store.get_terminal(term).unwrap();
        svc.store
            .update_terminal(
                term,
                row.resource_version,
                farcooler_store::models::TerminalUpdate {
                    title: row.title.clone(),
                    command_preset: row.command_preset.clone(),
                    intent: farcooler_protocol::v1::TerminalIntent::Stopped,
                    runtime_confirmed: row.runtime_confirmed,
                    exit_code: Some(0),
                    exit_signal: None,
                    lease_generation: row.lease_generation,
                    epoch: row.epoch,
                    columns: row.columns,
                    rows: row.rows,
                },
            )
            .unwrap();
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "an ended terminal writes nothing");
        assert_eq!(svc.claims().terminals_in(main_wt), vec![term], "though it's remembered until it goes");

        svc.remove_terminal(term).await.unwrap();
        assert!(svc.claims().terminals_in(main_wt).is_empty(), "a removed terminal is forgotten");
    }

    /// An agent that has just made a worktree inside the main checkout is
    /// ahead of `reconcile`. Until the row exists, the longest known path is
    /// the main checkout, and neither claiming nor reporting against it is
    /// right.
    #[tokio::test]
    async fn a_checkout_not_yet_adopted_is_nobodys() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let main_wt = main_worktree(&svc, repo);
        let (home_wt, _) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);

        let main_path = canonical(Path::new(&svc.store.get_worktree(main_wt).unwrap().worktree_path));
        let fresh = main_path.join(".worktrees").join("fresh");
        std::fs::create_dir_all(fresh.join("src")).unwrap();
        std::fs::write(fresh.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(observe(&svc, term, &fresh.join("src"), ClaimSource::Hook).unwrap(), Observed::Nothing);
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty());

        // An ordinary directory of the main checkout is the main checkout.
        std::fs::create_dir_all(main_path.join("docs")).unwrap();
        assert_eq!(
            observe(&svc, term, &main_path.join("docs"), ClaimSource::Hook).unwrap(),
            Observed::Foreign(main_wt)
        );
    }

    /// A terminal working in another repository's worktree can't claim it:
    /// ownership is per repository, and the store would refuse it anyway.
    #[tokio::test]
    async fn a_worktree_of_another_repository_is_not_claimed() {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let root = svc.store.get_repository(repo).unwrap().repository_root_id;
        let second = svc.store.create_repository(svc.host_id, root, "second", "/nowhere/.git", "").unwrap();
        let path = canonical(dir.path()).join("second");
        std::fs::create_dir_all(&path).unwrap();
        let wt = svc.store.create_unclaimed_worktree_for_test(second.id, &path.to_string_lossy());
        let term = svc.store.create_terminal_for_test(main_worktree(&svc, repo), billing.id);
        assert_eq!(observe(&svc, term, &path, ClaimSource::Hook).unwrap(), Observed::Nothing);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None);
    }

    /// The daemon's own hook listener feeds the daemon's own ledger and
    /// store: a Claude hook sent to the socket `resume_agent_listeners` binds
    /// claims the worktree its `cwd` is in.
    #[tokio::test]
    async fn a_hook_to_the_daemons_socket_claims() {
        use farcooler_agent_hooks::Agent;
        use farcooler_agent_hooks::wire::{HookLine, encode_line};
        use tokio::io::AsyncWriteExt;

        let (dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let (wt, path) = nested_worktree(&svc, repo, "x");
        let (home_wt, _) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);
        let row = svc.store.get_terminal(term).unwrap();
        svc.store
            .set_pane_mode(
                term,
                row.resource_version,
                farcooler_store::models::PaneMode::Terminal,
                Some("a-session".to_string()),
                false,
            )
            .unwrap();
        let main_path = svc.store.get_worktree(main_worktree(&svc, repo)).unwrap().worktree_path;

        svc.resume_agent_listeners();
        let socket = crate::hook_ingress::HookIngress::socket_path(&dir.path().join("state"));
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        // The unclaimed worktree first, then Main's checkout: the other order
        // would clear the report the second hook makes (see
        // `a_report_clears_when_the_terminal_is_seen_working_elsewhere`).
        let lines = [&path.to_string_lossy().into_owned(), &main_path].map(|cwd| HookLine {
            agent: Agent::Claude,
            event: "Stop".to_string(),
            payload: serde_json::json!({
                "session_id": "a-session",
                "cwd": cwd,
                "hook_event_name": "Stop",
                "last_assistant_message": "done",
            }),
        });
        let mut stream = tokio::net::UnixStream::connect(&socket).await.expect("the daemon listens");
        for line in &lines {
            stream.write_all(encode_line(line).unwrap().as_bytes()).await.unwrap();
        }
        stream.shutdown().await.unwrap();

        let mut claimed = None;
        for _ in 0..200 {
            claimed = svc.store.get_worktree(wt).unwrap().workspace_id;
            if claimed.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(claimed, Some(billing.id));
        let main_wt = main_worktree(&svc, repo);
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id], "and Main's checkout reports it");
    }

    fn pane(terminal: Uuid, tty: &str) -> TaggedPane {
        TaggedPane {
            daemon_id: Uuid::nil(),
            worktree_id: Uuid::nil(),
            terminal_id: terminal,
            schema_version: 1,
            pane_id: "%1".into(),
            window_id: "@1".into(),
            columns: 80,
            rows: 24,
            left: 0,
            top: 0,
            window_active: true,
            pane_active: true,
            zoomed: false,
            tty: format!("/dev/{tty}"),
            dead: false,
            dead_status: None,
            dead_signal: None,
            command: "codex".into(),
            title: String::new(),
            stamp: Default::default(),
        }
    }

    /// A codex pane: the shell and codex on the tty, and the command codex
    /// runs as a child with none, working in the worktree it moved to.
    const CODEX: &str = "\
900 800 900 ttys020  Ss   /opt/homebrew/bin/fish -il
901 900 901 ttys020  S+   codex
902 901 902 ??       Ss   /bin/zsh -lc ls
";

    /// Every live pane's processes are walked, a child with no tty included,
    /// and a worktree one of them is working in is claimed by `Process`.
    #[tokio::test]
    async fn a_command_running_in_an_unclaimed_worktree_claims_it() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let (wt, path) = nested_worktree(&svc, repo, "x");
        let main_wt = main_worktree(&svc, repo);
        let main_path = PathBuf::from(svc.store.get_worktree(main_wt).unwrap().worktree_path);
        let term = svc.store.create_terminal_for_test(main_wt, billing.id);
        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "ttys020")]);
        let table = crate::foreground::parse(CODEX);
        let cwd_of = |pid: i32| match pid {
            902 => Some(path.clone()),
            900 | 901 => Some(main_path.clone()),
            _ => None,
        };

        scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        let row = svc.store.get_worktree(wt).unwrap();
        assert_eq!((row.workspace_id, row.claim_source), (Some(billing.id), Some(ClaimSource::Process)));
        assert!(svc.claims().take_changed());
    }

    #[tokio::test]
    async fn the_walk_skips_orchestrators_dead_panes_and_other_ttys() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let (wt, path) = nested_worktree(&svc, repo, "x");
        let main_wt = main_worktree(&svc, repo);
        let table = crate::foreground::parse(CODEX);
        let cwd_of = |_: i32| Some(path.clone());

        let term = svc.store.create_terminal_for_test(main_wt, billing.id);
        svc.store.set_terminal_role(term, TerminalRole::Orchestrator).unwrap();
        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "ttys020")]);
        scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None, "an orchestrator");
        svc.store.set_terminal_role(term, TerminalRole::Agent).unwrap();

        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![TaggedPane { dead: true, ..pane(term, "ttys020") }]);
        scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None, "a dead pane");

        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "ttys021")]);
        scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, None, "another tty's processes");

        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "ttys020")]);
        scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        assert_eq!(svc.store.get_worktree(wt).unwrap().workspace_id, Some(billing.id), "and then it does");
    }

    /// A foreign-writer report isn't for the terminal's whole life. Once the
    /// same signal sees it working in its own worktree, or in an unclaimed
    /// one it then claims, the report clears, and that is news.
    #[tokio::test]
    async fn a_report_clears_when_the_terminal_is_seen_working_elsewhere() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let main_wt = main_worktree(&svc, repo);
        let main_path = PathBuf::from(svc.store.get_worktree(main_wt).unwrap().worktree_path);
        let (home_wt, home) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let (free_wt, free) = nested_worktree(&svc, repo, "free");
        let ops = svc.store.create_workspace(repo, "Ops", "ops").unwrap();
        let (ops_wt, ops_path) = nested_worktree(&svc, repo, "ops");
        svc.store.assign_worktree(ops_wt, ops.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);

        observe(&svc, term, &main_path, ClaimSource::Hook).unwrap();
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id]);
        assert!(svc.claims().take_changed());
        observe(&svc, term, &ops_path, ClaimSource::Hook).unwrap();
        assert_eq!(svc.foreign_writers(ops_wt).await.unwrap(), vec![billing.id]);
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "moved on to Ops's");
        assert!(svc.claims().take_changed());

        assert_eq!(observe(&svc, term, &home, ClaimSource::Hook).unwrap(), Observed::Owned(home_wt));
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "back in its own worktree");
        assert!(svc.claims().take_changed(), "a writer leaving is news too");
        observe(&svc, term, &home, ClaimSource::Hook).unwrap();
        assert!(!svc.claims().take_changed(), "and staying home isn't");

        observe(&svc, term, &main_path, ClaimSource::Hook).unwrap();
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id], "a second visit");
        assert_eq!(observe(&svc, term, &free, ClaimSource::Hook).unwrap(), Observed::Claimed(free_wt));
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "working in the one it claimed");

        // Somewhere that's no worktree at all says nothing about where it
        // works, so a report stands.
        observe(&svc, term, &main_path, ClaimSource::Hook).unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(observe(&svc, term, outside.path(), ClaimSource::Hook).unwrap(), Observed::Nothing);
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id]);
    }

    /// The walk judges a terminal on all it saw in one tick: a command in
    /// Main's checkout beside the shell at home is a report, and the tick
    /// that finds only the shell clears it. What a hook reported is its own:
    /// the walk seeing the Claude process where it started doesn't clear it.
    #[tokio::test]
    async fn a_command_that_visited_is_reported_only_while_it_runs() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap();
        let main_wt = main_worktree(&svc, repo);
        let main_path = PathBuf::from(svc.store.get_worktree(main_wt).unwrap().worktree_path);
        let (home_wt, home) = nested_worktree(&svc, repo, "billing");
        svc.store.assign_worktree(home_wt, billing.id).unwrap();
        let term = svc.store.create_terminal_for_test(home_wt, billing.id);
        let fleet = svc.fleet().await.unwrap();
        let snapshot = RuntimeSnapshot::healthy(vec![pane(term, "ttys020")]);
        let table = crate::foreground::parse(CODEX);
        let tick = |child: Option<&PathBuf>| {
            let cwd_of = |pid: i32| match pid {
                902 => child.cloned(),
                900 | 901 => Some(home.clone()),
                _ => None,
            };
            scan_with(&svc.store, svc.claims(), &fleet, &snapshot, &table, cwd_of);
        };

        tick(Some(&main_path));
        assert_eq!(svc.foreign_writers(main_wt).await.unwrap(), vec![billing.id], "while `git -C` runs");
        assert!(svc.claims().take_changed());
        tick(Some(&main_path));
        assert!(!svc.claims().take_changed(), "still there is not news");
        tick(None);
        assert!(svc.foreign_writers(main_wt).await.unwrap().is_empty(), "once it's done");
        assert!(svc.claims().take_changed());

        observe(&svc, term, &main_path, ClaimSource::Hook).unwrap();
        tick(None);
        assert_eq!(
            svc.foreign_writers(main_wt).await.unwrap(),
            vec![billing.id],
            "the walk can't see a Bash `cd`, so it doesn't undo the hook that did"
        );
    }
}
