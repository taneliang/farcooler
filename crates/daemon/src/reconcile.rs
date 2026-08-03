//! Keeping the sidebar and git in agreement about which worktrees exist.
//!
//! Git is the source of truth. The `workspaces` table is a cache keyed by
//! canonical worktree path, plus the things only Far Cooler knows: terminals,
//! layouts, agent sessions, and whether the user hid the row.
//!
//! This exists because Far Cooler only ever knew about work it started itself.
//! Someone arriving with a repository they had used for months spent their
//! first hour re-creating by hand what was already on disk, and a
//! `git worktree add` typed in a terminal never showed up at all.
//!
//! One rule governs deletion: a row whose worktree git no longer lists is
//! removed ONLY when it holds no terminals. Everything else is kept and flagged
//! missing. A `git worktree list` that raced a `mv` must not be able to destroy
//! an agent transcript.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use farcooler_core::Result;
use uuid::Uuid;

use crate::git;
use crate::service::{Service, canonical_or_raw};

/// What one pass changed. All zero means there is nothing to tell clients.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: usize,
    pub dropped: usize,
    pub missing: usize,
    /// A workspace that was flagged `worktree_missing` and just had that flag
    /// cleared because its worktree came back. Counted separately from
    /// `adopted` — nothing was created — but it still has to make
    /// `is_quiet()` false: a client showing "missing" for a workspace that
    /// just stopped being missing is stale, not merely uninformed.
    pub recovered: usize,
    /// An adopt attempt that lost a race against the unique index on
    /// `(repository_id, worktree_path)` — another writer, or a previous pass
    /// that ran concurrently, got there first. Folded into `is_quiet()` like
    /// every other field: it should be zero as long as `repo_lock` is doing
    /// its job, since the lock exists to make this path unreachable rather
    /// than merely harmless, and a nonzero count is worth a client noticing
    /// rather than the daemon quietly absorbing it.
    pub conflicts: usize,
    /// A row that already existed, for a worktree git still reports, whose
    /// `is_main_checkout` disagreed with git's own `is_main` and just got
    /// corrected. Migration 0006 added the column with `DEFAULT 0`, so every
    /// database written before this feature existed has this disagreement on
    /// its main checkout's row, permanently, until a pass notices — this is
    /// that noticing. Folded into `is_quiet()` for the same reason
    /// `recovered` is: a row that just started telling the truth about
    /// itself is news a client should see, not something to swallow.
    pub healed: usize,
}

impl Outcome {
    /// Nothing changed, so nothing needs broadcasting.
    pub fn is_quiet(&self) -> bool {
        self.adopted == 0
            && self.dropped == 0
            && self.missing == 0
            && self.recovered == 0
            && self.conflicts == 0
            && self.healed == 0
    }

    fn absorb(&mut self, other: Outcome) {
        self.adopted += other.adopted;
        self.dropped += other.dropped;
        self.missing += other.missing;
        self.recovered += other.recovered;
        self.conflicts += other.conflicts;
        self.healed += other.healed;
    }
}

/// Reconcile one repository against git.
///
/// Takes the repository's lock, so it cannot observe the gap inside
/// `Service::create_workspace` between `git worktree add` and the row insert.
pub async fn repository(svc: &Service, repository_id: Uuid) -> Result<Outcome> {
    let lock = svc.repo_lock(repository_id);
    let _guard = lock.lock().await;

    let repo = svc.store.get_repository(repository_id)?;
    let repo_path = svc.repository_worktree(&repo);

    let found = git::list_worktrees(&repo_path).await?;
    let known = svc.store.list_workspaces_for_repository(repository_id)?;

    let mut outcome = Outcome::default();

    // ---- what git has that we do not ----

    let registered: HashSet<PathBuf> =
        known.iter().map(|w| canonical_or_raw(&w.worktree_path)).collect();

    for worktree in &found {
        // A prunable record points at a directory that is gone; git will drop
        // it on the next prune. Adopting one creates a row for nothing.
        if worktree.prunable {
            continue;
        }
        let path = canonical_or_raw(&worktree.path);
        if registered.contains(&path) {
            // A row for this path already exists. Its `is_main_checkout`
            // may still be wrong: migration 0006 added the column with
            // `DEFAULT 0`, so every row written before it exists says
            // "not main" regardless of what git actually reports, and
            // nothing else ever revisits the value once the row is
            // created. Correct it here rather than leave it wrong forever.
            if let Some(ws) = known.iter().find(|w| canonical_or_raw(&w.worktree_path) == path) {
                if ws.is_main_checkout != worktree.is_main {
                    match svc.store.set_workspace_is_main_checkout(
                        ws.id,
                        ws.resource_version,
                        worktree.is_main,
                    ) {
                        Ok(_) => outcome.healed += 1,
                        Err(e) => {
                            tracing::warn!(path = %worktree.path, error = ?e, "could not heal is_main_checkout")
                        }
                    }
                }
            }
            continue;
        }
        if !Path::new(&worktree.path).is_dir() {
            continue;
        }

        let branch = worktree.branch.clone().unwrap_or_else(|| {
            // A detached worktree still has a commit, and that is the honest
            // thing to call it rather than inventing a branch name.
            format!("detached at {}", worktree.head.chars().take(8).collect::<String>())
        });

        let name = Path::new(&worktree.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| branch.clone());

        // A name git handed us, not one a user typed. If the directory is named
        // something the validator refuses, skipping it silently would make a
        // worktree permanently invisible with no way to find out why.
        if let Err(e) = farcooler_core::validate::task_name(&name) {
            tracing::warn!(path = %worktree.path, error = ?e, "worktree name is not usable");
            continue;
        }

        match svc.store.create_workspace(
            repository_id,
            &name,
            &branch,
            &worktree.path,
            worktree.is_main,
        ) {
            Ok(_) => outcome.adopted += 1,
            // The unique index rejecting this means another writer got there
            // first, which is the index doing its job rather than an error
            // worth propagating as a failure — but it is worth counting.
            Err(e) => {
                outcome.conflicts += 1;
                tracing::warn!(path = %worktree.path, error = ?e, "could not adopt worktree");
            }
        }
    }

    // ---- what we have that git does not ----

    let live: HashSet<PathBuf> = found
        .iter()
        .filter(|w| !w.prunable)
        .map(|w| canonical_or_raw(&w.path))
        .collect();

    for ws in &known {
        let path = canonical_or_raw(&ws.worktree_path);
        let gone = !live.contains(&path) || !Path::new(&ws.worktree_path).is_dir();

        if !gone {
            // Back from the dead: a worktree re-added at the same path clears
            // the flag, rather than leaving a row broken forever because it was
            // once missing for a tick.
            if ws.worktree_missing {
                match svc.store.set_workspace_flags(ws.id, ws.resource_version, ws.hidden, false) {
                    Ok(_) => outcome.recovered += 1,
                    Err(e) => tracing::warn!(error = ?e, "could not clear a recovered workspace"),
                }
            }
            continue;
        }

        // The whole test. Agent sessions hang off terminals, so a workspace with
        // no terminals has no transcript either.
        let empty = svc.store.list_terminals_for_workspace(ws.id)?.is_empty();

        if empty {
            match svc.store.delete_workspace(ws.id, ws.resource_version) {
                Ok(()) => outcome.dropped += 1,
                Err(e) => tracing::warn!(error = ?e, "could not drop a vanished workspace"),
            }
        } else if !ws.worktree_missing {
            match svc.store.set_workspace_flags(ws.id, ws.resource_version, ws.hidden, true) {
                Ok(_) => outcome.missing += 1,
                Err(e) => tracing::warn!(error = ?e, "could not flag a vanished workspace"),
            }
        }
    }

    Ok(outcome)
}

/// Reconcile every registered repository.
///
/// One repository failing does not stop the rest: a repository whose directory
/// was unmounted must not freeze the sidebar for every other project.
pub async fn all(svc: &Service) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    for repo in svc.list_repositories()? {
        match repository(svc, repo.id).await {
            Ok(one) => outcome.absorb(one),
            Err(e) => tracing::warn!(repository = %repo.id, error = ?e, "reconcile failed"),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use farcooler_protocol::v1::TerminalIntent;

    use super::*;
    use crate::git;
    use crate::service::Service;
    use crate::test_support::fixture;

    fn names(svc: &Service, repo: Uuid) -> Vec<String> {
        let mut n: Vec<String> = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .map(|w| w.task_name)
            .collect();
        n.sort();
        n
    }

    #[tokio::test]
    async fn registering_a_repository_adopts_its_main_checkout() {
        let (_dir, svc, repo) = fixture().await;
        let all = svc.store.list_workspaces_for_repository(repo).unwrap();
        assert_eq!(all.len(), 1, "the main checkout, and only it: {all:?}");
        assert!(all[0].is_main_checkout, "flagged, not inferred from its name");
    }

    #[tokio::test]
    async fn a_worktree_made_outside_far_cooler_appears() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.adopted, 1);
        assert_eq!(names(&svc, repo), vec!["repo".to_string(), "side".to_string()]);
    }

    /// Adoption is idempotent. A second pass over an unchanged repository must
    /// change nothing, or every tick would add a row.
    ///
    /// It is not enough to check the row count afterwards: `worktree_path` has
    /// a unique index per repository (`crates/store/src/migrate.rs`), so even
    /// with the `registered.contains(&path)` dedup guard deleted, a second
    /// pass's re-adopt attempt fails at the database and the row count still
    /// lands on 2. `Outcome::conflicts` is what actually distinguishes "never
    /// attempted" from "attempted and silently caught": deleting that guard
    /// makes the second pass try to re-insert both `repo` and `side`, both
    /// losing to the unique index, so `conflicts` becomes 2 instead of 0.
    /// Verified by hand, repeatedly — see the round-2 report for counts.
    ///
    /// Deliberately not a `tracing` log assertion: an earlier version of this
    /// test captured warnings through a thread-local subscriber, which was
    /// unsound under `cargo test`'s parallel default. `tracing` caches
    /// callsite interest globally per process, so a sibling test hitting the
    /// same `warn!` callsite first — with no subscriber installed on ITS
    /// thread — could disable the callsite for everyone, including this test,
    /// leaving its capture buffer empty and its negative assertion vacuously
    /// true. `conflicts` is plain in-memory state with no such hazard.
    #[tokio::test]
    async fn a_second_pass_changes_nothing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();

        repository(&svc, repo).await.unwrap();
        let second = repository(&svc, repo).await.unwrap();

        assert!(second.is_quiet(), "nothing to say: {second:?}");
        assert_eq!(names(&svc, repo).len(), 2);
        assert_eq!(
            second.conflicts, 0,
            "the dedup guard should mean the second pass never even attempts to re-adopt \
             anything, so nothing should lose a race against the unique index either: {second:?}"
        );
    }

    #[tokio::test]
    async fn a_removed_worktree_with_nothing_in_it_disappears() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.dropped, 1);
        assert_eq!(names(&svc, repo), vec!["repo".to_string()]);
    }

    #[tokio::test]
    async fn a_removed_worktree_holding_terminals_survives_as_missing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        // A record is enough. The point is that the row carries something worth
        // keeping, not that a process is alive.
        svc.store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Stopped, 80, 24)
            .unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.dropped, 0, "a row with a terminal in it is never deleted for us");
        assert_eq!(out.missing, 1);

        let after = svc.store.get_workspace(ws.id).unwrap();
        assert!(after.worktree_missing);
    }

    /// The other shape of "gone": the worktree's directory is still there, but
    /// git itself now reports the record prunable (its own `.git` link is
    /// broken — see `a_prunable_record_is_not_adopted` for how that differs
    /// from a directory that simply vanished). A row that holds a terminal
    /// must be flagged missing here exactly as it would be if the directory
    /// had been removed outright, not treated as still live because `is_dir()`
    /// alone would say yes.
    ///
    /// This is what the `live` set's own `!w.prunable` filter
    /// (`reconcile.rs`, "what we have that git does not") is for, and unlike
    /// the adopt-side filter it had no test: deleting it left `live` including
    /// the prunable path, so `gone` came out false and this workspace was
    /// never flagged. Deleting `.filter(|w| !w.prunable)` from `live` turns
    /// this red. Verified by hand, repeatedly.
    #[tokio::test]
    async fn a_workspace_whose_worktree_goes_prunable_is_flagged_missing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        svc.store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Stopped, 80, 24)
            .unwrap();

        // Only the link back to the repository's `.git/worktrees/...` entry,
        // not the directory: this is what makes git call the record prunable
        // while `side` itself still stands.
        std::fs::remove_file(side.join(".git")).unwrap();
        assert!(side.is_dir(), "the directory itself must still be here for this test to mean anything");

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.dropped, 0, "a row with a terminal in it is never deleted for us");
        assert_eq!(out.missing, 1, "prunable is still gone, even though the directory remains");

        let after = svc.store.get_workspace(ws.id).unwrap();
        assert!(after.worktree_missing);
    }

    /// A worktree that comes back clears the flag rather than staying broken.
    ///
    /// Also the only test that observes `Outcome::recovered`: `is_quiet()`
    /// folds it in specifically so that a pass which resurrects a workspace
    /// never reports quiet, since Task 5 gates broadcasting on that call.
    #[tokio::test]
    async fn a_returning_worktree_stops_being_missing() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        let repo_path = dir.path().join("repo");
        git::git(&repo_path, &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        svc.store
            .create_terminal(ws.id, "shell", "shell", TerminalIntent::Stopped, 80, 24)
            .unwrap();

        git::git(&repo_path, &["worktree", "remove", "--force", side.to_str().unwrap()])
            .await
            .unwrap();
        repository(&svc, repo).await.unwrap();

        git::git(&repo_path, &["worktree", "add", "-q", side.to_str().unwrap(), "feat/side"])
            .await
            .unwrap();
        let out = repository(&svc, repo).await.unwrap();

        assert!(!svc.store.get_workspace(ws.id).unwrap().worktree_missing);
        assert_eq!(out.recovered, 1);
        assert!(!out.is_quiet(), "a recovery is news, not nothing");
    }

    /// Hiding is the user's decision and survives a pass.
    #[tokio::test]
    async fn a_hidden_worktree_stays_hidden() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        repository(&svc, repo).await.unwrap();

        let ws = svc
            .store
            .list_workspaces_for_repository(repo)
            .unwrap()
            .into_iter()
            .find(|w| !w.is_main_checkout)
            .unwrap();
        svc.store.set_workspace_flags(ws.id, ws.resource_version, true, false).unwrap();

        repository(&svc, repo).await.unwrap();
        assert!(svc.store.get_workspace(ws.id).unwrap().hidden);
    }

    /// A prunable record can point at a directory that still exists: git marks
    /// a worktree prunable when ITS OWN administrative link is broken, not only
    /// when the whole directory is gone. Adopting one would create a row for a
    /// worktree git itself is about to drop.
    ///
    /// Deliberately removes only the worktree's own `.git` file rather than the
    /// directory. Removing the whole directory would also make `is_dir()`
    /// false, and that check alone (deliberately kept, for the ordinary case of
    /// a worktree that is simply gone) already skips adoption regardless of
    /// `prunable` — so a test built that way cannot tell the two guards apart.
    /// Deleting just the `if worktree.prunable { continue; }` check turns this
    /// one red, because the directory is still there and `is_dir()` no longer
    /// covers for it. Verified by hand.
    /// Migration 0006 added `is_main_checkout` with `DEFAULT 0`, so a
    /// database written before this feature existed has its main checkout's
    /// row saying "not main" regardless of what git reports — permanently,
    /// since nothing but this pass ever revisits the value once the row
    /// exists. Simulates that database directly rather than hand-building a
    /// pre-0006 schema: what is under test is the reconciler's correction,
    /// not the migration itself (that has its own coverage in
    /// `store::migrate::tests::archived_rows_become_hidden`).
    ///
    /// Deleting the correction in `reconcile::repository` (the
    /// `if ws.is_main_checkout != worktree.is_main` block) turns this red:
    /// `healed` stays `0`, the row's flag never changes, and `is_quiet()`
    /// comes back `true`. Verified by hand.
    #[tokio::test]
    async fn a_pre_0006_row_gets_its_main_checkout_flag_healed() {
        let (_dir, svc, repo) = fixture().await;
        let all = svc.store.list_workspaces_for_repository(repo).unwrap();
        assert_eq!(all.len(), 1, "just the main checkout: {all:?}");
        let main = &all[0];
        assert!(main.is_main_checkout, "adoption must have gotten this right to start with");

        svc.store.set_workspace_is_main_checkout(main.id, main.resource_version, false).unwrap();
        assert!(
            !svc.store.get_workspace(main.id).unwrap().is_main_checkout,
            "the test must actually start from the wrong flag to mean anything"
        );

        let out = repository(&svc, repo).await.unwrap();

        assert_eq!(out.healed, 1, "{out:?}");
        assert!(!out.is_quiet(), "a row correcting itself is news, not nothing: {out:?}");
        assert!(
            svc.store.get_workspace(main.id).unwrap().is_main_checkout,
            "the pass must have corrected the row, not merely counted the disagreement"
        );
    }

    #[tokio::test]
    async fn a_prunable_record_is_not_adopted() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("side");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/side", side.to_str().unwrap()],
        )
        .await
        .unwrap();
        // Only the link back to the repository's `.git/worktrees/...` entry,
        // not the directory: this is what git itself reports as "prunable"
        // while the worktree's own directory still stands.
        std::fs::remove_file(side.join(".git")).unwrap();
        assert!(side.is_dir(), "the directory itself must still be here for this test to mean anything");

        // Compared by canonical path, not the raw string: macOS hands back
        // `/private/var/...` for a `TempDir` under `/var/...`, and the two
        // spellings name the same directory.
        let found = git::list_worktrees(&dir.path().join("repo")).await.unwrap();
        let canonical_side = canonical_or_raw(&side.to_string_lossy());
        assert!(
            found
                .iter()
                .any(|w| !w.is_main && canonical_or_raw(&w.path) == canonical_side && w.prunable),
            "git must actually report this one prunable: {found:?}"
        );

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.adopted, 0);
        assert_eq!(names(&svc, repo), vec!["repo".to_string()]);
    }
}
