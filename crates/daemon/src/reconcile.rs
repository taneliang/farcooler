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
    /// A row that already existed, for a worktree git still reports, which
    /// disagreed with git about something git owns — the branch checked out
    /// there, whether it is the main checkout, or (for the main checkout) what
    /// it is called — and just got rewritten to match. One per row, not per
    /// column: a row with two things wrong is still one row that started
    /// telling the truth.
    ///
    /// Every one of these is permanent without a pass like this one, because
    /// nothing else revisits those columns after the row is created. `branch`
    /// goes stale the moment someone types `git checkout` in the worktree;
    /// `is_main_checkout` is wrong from birth on every database written before
    /// migration 0006 added the column with `DEFAULT 0`; and a main checkout
    /// adopted by an older Far Cooler is called `main` rather than named after
    /// its directory. Folded into `is_quiet()` for the same reason `recovered`
    /// is: a row that just started telling the truth about itself is news a
    /// client should see, not something to swallow.
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

/// What to call the branch a worktree is on.
///
/// Shared by adoption and healing on purpose. Two spellings of the detached
/// case would put those two out of step, and a row healing to a string the
/// next pass disagrees with is not a one-off correction — it is every pass
/// finding a disagreement, rewriting the row, and bumping `resource_version`
/// on a timer for as long as the daemon runs.
fn branch_of(worktree: &git::WorktreeInfo) -> String {
    worktree.branch.clone().unwrap_or_else(|| {
        // A detached worktree still has a commit, and that is the honest
        // thing to call it rather than inventing a branch name.
        format!("detached at {}", worktree.head.chars().take(8).collect::<String>())
    })
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
        let branch = branch_of(worktree);

        if registered.contains(&path) {
            // A row for this path already exists, and git has just re-read the
            // worktree it caches. Everything git owns about that row can have
            // drifted since it was written, because nothing else ever revisits
            // any of it: `branch` goes stale the moment someone types
            // `git checkout` here, and `is_main_checkout` is wrong from birth
            // on any database written before migration 0006 added the column
            // with `DEFAULT 0`. Correct it here rather than leave it wrong
            // forever.
            //
            // The name is not among them, and there used to be a careful
            // carve-out here explaining which workspaces could have theirs
            // re-derived from git and which could not. A workspace is named by
            // its worktree directory now, read on every access, so there is no
            // stored name to go stale and nothing to decide.
            if let Some(ws) = known.iter().find(|w| canonical_or_raw(&w.worktree_path) == path) {
                if ws.branch != branch || ws.is_main_checkout != worktree.is_main {
                    match svc.store.set_workspace_identity(
                        ws.id,
                        ws.resource_version,
                        &branch,
                        worktree.is_main,
                    ) {
                        Ok(_) => outcome.healed += 1,
                        Err(e) => {
                            tracing::warn!(path = %worktree.path, error = ?e, "could not heal a workspace against git")
                        }
                    }
                }
            }
            continue;
        }
        if !Path::new(&worktree.path).is_dir() {
            continue;
        }

        // A worktree another install made is not ours to adopt.
        //
        // git reports every worktree of a repository regardless of which daemon
        // created it, so without this two installs sharing a host each adopt
        // the other's, show it in their own fleet, and can start agents in the
        // same directory at the same time on separate tmux servers. This is the
        // rule `@farcooler_daemon_id` already applies to panes, extended to the
        // thing that outlives them.
        //
        // Unmarked is adoptable: that is a worktree a person made by hand, and
        // picking those up is what adoption is for.
        if let Some(owner) = git::owner_of(Path::new(&worktree.path)).await
            && owner != svc.install_id()
        {
            tracing::debug!(path = %worktree.path, %owner, "worktree belongs to another install");
            continue;
        }

        match svc.store.create_workspace(
            repository_id,
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
            .map(|w| w.name())
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

        svc.store
            .set_workspace_identity(main.id, main.resource_version, &main.branch, false)
            .unwrap();
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

    /// git owns which branch a worktree is on; the row is a cache of that.
    ///
    /// Nothing but this pass ever revisits `branch` once the row exists —
    /// `Store::update_workspace` has no production caller at all — so a
    /// `git checkout` typed by hand in the main checkout used to leave the
    /// sidebar naming whatever branch happened to be current when the
    /// repository was registered, permanently.
    ///
    /// Deleting the `branch` comparison in `reconcile::repository` turns this
    /// red: `healed` stays `0` and the row keeps saying `main`.
    #[tokio::test]
    async fn a_branch_switched_by_hand_is_picked_up() {
        let (dir, svc, repo) = fixture().await;
        let all = svc.store.list_workspaces_for_repository(repo).unwrap();
        assert_eq!(all.len(), 1, "just the main checkout: {all:?}");
        let main = &all[0];
        assert_eq!(main.branch, "main", "the fixture starts on main");

        git::git(&dir.path().join("repo"), &["checkout", "-q", "-b", "feat/switched"])
            .await
            .unwrap();

        let out = repository(&svc, repo).await.unwrap();

        assert_eq!(out.healed, 1, "{out:?}");
        assert!(!out.is_quiet(), "a row correcting itself is news, not nothing: {out:?}");
        assert_eq!(
            svc.store.get_workspace(main.id).unwrap().branch,
            "feat/switched",
            "the pass must have corrected the row, not merely counted the disagreement"
        );
    }

    /// A detached main checkout heals to the same string adoption would have
    /// written, rather than to a branch name git never reported.
    ///
    /// Worth its own test because the two derivations live in different
    /// places: get them out of step and every pass would see a disagreement,
    /// heal it to the other spelling, and bump `resource_version` forever —
    /// a row that is never quiet, on a timer.
    #[tokio::test]
    async fn a_detached_main_checkout_heals_to_its_commit() {
        let (dir, svc, repo) = fixture().await;
        let repo_path = dir.path().join("repo");
        let main = svc.store.list_workspaces_for_repository(repo).unwrap().remove(0);

        git::git(&repo_path, &["checkout", "-q", "--detach"]).await.unwrap();
        let head = git::git(&repo_path, &["rev-parse", "HEAD"]).await.unwrap();
        let expected = format!("detached at {}", head.stdout.trim().chars().take(8).collect::<String>());

        let out = repository(&svc, repo).await.unwrap();
        assert_eq!(out.healed, 1, "{out:?}");
        assert_eq!(svc.store.get_workspace(main.id).unwrap().branch, expected);

        // And then stays put: a second pass over an unchanged detached HEAD
        // must find nothing left to correct.
        let again = repository(&svc, repo).await.unwrap();
        assert!(again.is_quiet(), "healing must converge, not oscillate: {again:?}");
    }

    /// A worktree keeps its name while the branch inside it moves.
    ///
    /// This is the whole reason a workspace is named by its directory. One
    /// worktree hosts a stack of commits over its life: you branch, you branch
    /// again off that, you rebase, and the branch checked out in the directory
    /// changes each time. A name taken from the branch would rename the
    /// workspace on every one of those, so the sidebar row you were watching an
    /// agent work in would keep becoming a different row.
    ///
    /// The branch column still follows git — that is what `healed` counts here.
    /// The name does not move with it.
    #[tokio::test]
    async fn a_worktree_keeps_its_name_as_the_branch_under_it_moves() {
        let (dir, svc, repo) = fixture().await;
        let side = dir.path().join("rate-limiting");
        git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-q", "-b", "feat/rate-limiting", side.to_str().unwrap()],
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
            .expect("the adopted worktree");
        assert_eq!(ws.name(), "rate limiting", "adoption names it after its directory");

        // The next branch in the stack, checked out in the same worktree.
        git::git(&side, &["checkout", "-q", "-b", "feat/rate-limiting-tests"]).await.unwrap();
        let out = repository(&svc, repo).await.unwrap();

        assert_eq!(out.healed, 1, "the branch column follows git: {out:?}");
        let after = svc.store.get_workspace(ws.id).unwrap();
        assert_eq!(after.branch, "feat/rate-limiting-tests");
        assert_eq!(after.name(), "rate limiting", "the name is the directory, which did not move");
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

/// Two installs on one host do not adopt each other's worktrees.
///
/// The property the whole channel design rests on, and the reason a worktree
/// needs an owner at all: `list_worktrees` reports what GIT knows, and git knows
/// every worktree of a repository regardless of which daemon made it.
#[cfg(test)]
mod isolation_tests {
    use crate::test_support::two_daemons;

    #[tokio::test]
    async fn a_worktree_one_install_made_is_not_adopted_by_another() {
        let (_dir, a, b, repo_a, repo_b) = two_daemons().await;

        let ws = a.create_workspace(repo_a, "rate limiting", "feat/rate", "HEAD").await.unwrap();

        let outcome = super::repository(&b, repo_b).await.unwrap();
        assert_eq!(
            outcome.adopted, 0,
            "the other install's worktree must not be adopted: two daemons in one \
             directory means two agents writing the same files"
        );

        let seen = b.store.list_workspaces_for_repository(repo_b).unwrap();
        assert!(
            !seen.iter().any(|w| w.worktree_path == ws.worktree_path),
            "it must not appear in the other install's fleet either"
        );
    }

    #[tokio::test]
    async fn two_installs_have_different_identities() {
        let (_dir, a, b, _, _) = two_daemons().await;
        assert_ne!(
            a.install_id(),
            b.install_id(),
            "the tmux server is `tmux -L farcooler-<install-id>`, so equal ids \
             would mean one tmux server and therefore shared panes"
        );
        assert_ne!(a.host_id, b.host_id);
    }

    #[tokio::test]
    async fn a_hand_made_worktree_is_still_adopted() {
        // Adoption exists so that a worktree someone made by hand gets picked
        // up. Ownership must not cost that: an UNMARKED worktree belongs to
        // nobody and is fair game, and only a DIFFERENT install's mark refuses.
        let (dir, _a, b, _, repo_b) = two_daemons().await;
        let by_hand = dir.path().join("by-hand");
        crate::git::git(
            &dir.path().join("repo"),
            &["worktree", "add", "-b", "manual", &by_hand.to_string_lossy(), "HEAD"],
        )
        .await
        .unwrap();

        let outcome = super::repository(&b, repo_b).await.unwrap();
        assert_eq!(outcome.adopted, 1, "an unmarked worktree is still adopted");
    }
}
