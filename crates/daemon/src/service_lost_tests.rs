//! A lost terminal, opened from the Mac's worktree page, the iPhone's pane or
//! Android's: Restart and Dismiss, end to end against a real tmux (ov-191).
//!
//! Restart runs the preset recorded at creation, in the terminal's own
//! worktree. A shell has no command recorded beyond `shell` (what was typed
//! into it never was), so it comes back as a bare shell, which the clients
//! say before Restart is pressed (`LostPane.restartNote`).

use super::*;
use super::restart_wiring_tests::{a_worktree, pane_start_command};

/// A terminal that once proved a pane and now has none: what `lost` means.
fn a_lost(svc: &Service, ws: &models::Worktree, preset: &str) -> models::Terminal {
    let term = svc.store.create_terminal(ws.id, "lost", preset, TerminalIntent::Running, 80, 24).unwrap();
    let term = svc
        .store
        .update_terminal(term.id, term.resource_version, terminal_update(&term, |u| u.runtime_confirmed = true))
        .unwrap();
    assert_eq!(svc.derive_one(&term).state, TerminalState::Lost, "needs a healthy inventory to read as lost");
    term
}

/// Where the pane runs, as tmux has it.
async fn pane_path(svc: &Service, terminal: Uuid) -> String {
    let snapshot = svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = svc
        .tmux
        .run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_current_path}"])
        .await
        .expect("tmux answered");
    out.stdout.trim().to_string()
}

/// **Restart with a recorded command.** The preset is run again, in the
/// terminal's own worktree, and the terminal is running again.
#[tokio::test]
async fn a_lost_terminal_restarts_its_recorded_command_in_its_worktree() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_lost(&svc, &ws, "sleepnomore");

    let back = svc.restart_terminal(term.id).await.expect("restart");

    let command = pane_start_command(&svc, term.id).await;
    assert!(command.contains("-ilc") && command.contains("sleepnomore"), "runs it again: {command}");
    assert_eq!(svc.derive_one(&back).state, TerminalState::Running);
    assert_eq!(back.epoch, term.epoch + 1, "a new run, so clients replace rather than append");
    let _ = svc.stop_terminal(term.id).await;
}

/// **Restart without a recorded command.** A shell comes back as a shell, in
/// its worktree, and runs nothing else: what was typed into it was never
/// recorded, and guessing at it from `ps` would re-run a truncated label.
#[tokio::test]
async fn a_lost_shell_restarts_as_a_bare_shell_in_its_worktree() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_lost(&svc, &ws, "shell");

    let back = svc.restart_terminal(term.id).await.expect("restart");

    let command = pane_start_command(&svc, term.id).await;
    // tmux hands it back quoted, so it's read for the flags.
    assert!(command.contains(" -il") && !command.contains("-ilc"), "a login shell and nothing else: {command}");
    assert_eq!(
        canonical_or_raw(&pane_path(&svc, term.id).await),
        canonical_or_raw(&ws.worktree_path),
        "in its own worktree"
    );
    assert_eq!(svc.derive_one(&back).state, TerminalState::Running);
    let _ = svc.stop_terminal(term.id).await;
}

/// **Dismiss.** The row goes, and nothing is left for a client to list.
#[tokio::test]
async fn a_dismissed_lost_terminal_leaves_the_list() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_lost(&svc, &ws, "shell");

    svc.dismiss_lost(term.id).await.expect("a lost terminal is dismissable");

    let listed = svc.store.list_terminals_for_worktree(ws.id).unwrap();
    assert!(listed.iter().all(|t| t.id != term.id), "still listed after Dismiss");
    assert!(svc.inventory.refresh().await.claimants(term.id).is_empty(), "and no pane was made for it");
}
