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
        .list_worktrees_for_repository(repo)
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
        .list_worktrees_for_repository(repo)
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
        .list_worktrees_for_repository(repo)
        .unwrap()
        .into_iter()
        .find(|w| !w.is_main_checkout)
        .unwrap();

    std::fs::remove_dir_all(&side).unwrap();

    assert!(!svc.removal_needs_confirmation(ws.id).await.unwrap());
}

/// `remove_worktree` CLOSES a live terminal rather than refusing over it.
///
/// This test asserted the opposite until a user review pointed out that
/// being told to stop four terminals by hand, after pressing Remove, is
/// being told to do the thing you just asked for. The rule changed; the
/// safety did not — see `remove_worktree`'s own comment on why killing the
/// process first is different from skipping the check.
///
/// `remove_root` still refuses, and deliberately: removing a root revokes
/// permission over a whole directory tree that may hold work in several
/// worktrees, so there the refusal is the user's cue to look at what is
/// running before they take all of it away.
///
/// Built the same way `remove_root_tests::a_starting_terminal_blocks_removal_same_as_a_running_one`
/// is: a terminal created through the store directly, with no live pane
/// behind it, derives `Starting` rather than `Running` as long as the
/// inventory itself is healthy. `Starting` is the harder case — a terminal
/// mid-launch is exactly as alive as one already confirmed.
#[tokio::test]
async fn a_starting_terminal_is_closed_by_worktree_removal_rather_than_blocking_it() {
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
        .list_worktrees_for_repository(repo)
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

    svc.remove_worktree(ws.id).await.expect("a live terminal is closed, not a refusal");

    // The terminal's record goes with it. Left behind it would point at a
    // worktree row that no longer exists, and `terminals.worktree_id` has
    // no cascade to clean that up.
    assert!(
        svc.store.get_terminal(term.id).is_err(),
        "the terminal record must go with the worktree"
    );
    assert!(
        svc.store.list_worktrees_for_repository(repo).unwrap().iter().all(|w| w.id != ws.id),
        "the worktree row is gone"
    );
}

/// The main checkout is refused by `ws.is_main_checkout`, a fact read
/// straight from `git worktree list`, not by comparing paths.
#[tokio::test]
async fn the_main_checkout_is_never_removable() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc
        .store
        .list_worktrees_for_repository(repo)
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
/// `set_worktree_identity` is used here to put a row into exactly that
/// state deliberately, standing in for that database, rather than relying
/// on `ws.is_main_checkout` ever having been right.
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
        .list_worktrees_for_repository(repo)
        .unwrap()
        .into_iter()
        .find(|w| w.is_main_checkout)
        .unwrap();

    let ws = svc
        .store
        .set_worktree_identity(ws.id, ws.resource_version, &ws.branch, false)
        .unwrap();
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
