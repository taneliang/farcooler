//! Runtime state is DERIVED, never stored.
//!
//! SQLite holds only what must outlive tmux: identity and intent. There is no
//! row that could say `running` after the process died, so the daemon cannot
//! serve a stale claim. The proof-based rule is structural instead of a
//! discipline someone has to remember.
//!
//! ```text
//!   durable intent  ×  live exactly-tagged panes   ->   derived state
//!   ─────────────────────────────────────────────────────────────────
//!   RUNNING (confirmed)   exactly one              ->   running
//!   RUNNING (confirmed)   none                     ->   lost
//!   RUNNING (confirmed)   more than one            ->   lost + orphans
//!   RUNNING (unconfirmed) none                     ->   starting
//!   STOPPED               any                      ->   exited
//!   FAILED                any                      ->   error
//! ```

use farcooler_protocol::v1::{TerminalIntent, TerminalState, WorkspaceState};
use uuid::Uuid;

use crate::inventory::{RuntimeSnapshot, TaggedPane};

/// The durable half of a terminal. Note the absence of any runtime state field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRecord {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub intent: TerminalIntent,
    /// True once creation verified exact tags on a live pane. Until then a
    /// terminal with no pane is still coming up, not lost.
    pub runtime_confirmed: bool,
    /// Set only when an exit was actually observed.
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedTerminal {
    pub state: TerminalState,
    /// Panes that claim this terminal but cannot prove it (duplicates).
    pub orphan_candidates: Vec<TaggedPane>,
}

/// The rule. Pure, total, and unit-testable against a fake inventory.
pub fn derive_terminal(record: &TerminalRecord, snapshot: &RuntimeSnapshot) -> DerivedTerminal {
    match record.intent {
        TerminalIntent::Stopped => {
            DerivedTerminal { state: TerminalState::Exited, orphan_candidates: Vec::new() }
        }
        TerminalIntent::Failed => {
            DerivedTerminal { state: TerminalState::Error, orphan_candidates: Vec::new() }
        }
        TerminalIntent::Running | TerminalIntent::Unspecified => {
            // An unusable inventory is not proof of life. Everything expected to
            // run derives `lost` rather than being guessed as alive.
            if !snapshot.inventory_healthy {
                return DerivedTerminal {
                    state: TerminalState::Lost,
                    orphan_candidates: Vec::new(),
                };
            }

            let claimants: Vec<TaggedPane> =
                snapshot.claimants(record.id).into_iter().cloned().collect();

            // Duplicate claimants are simply not proof of identity, so the
            // terminal is `lost` and every claimant becomes an orphan candidate.
            // There is no `conflict` state, because there is no comparison to
            // arbitrate.
            if claimants.len() > 1 {
                return DerivedTerminal {
                    state: TerminalState::Lost,
                    orphan_candidates: claimants,
                };
            }

            match claimants.first() {
                // A live pane proves the terminal is running.
                Some(p) if p.proves_life() => {
                    DerivedTerminal { state: TerminalState::Running, orphan_candidates: Vec::new() }
                }
                // A retained dead pane is an OBSERVED exit, not a loss. This is
                // the whole reason managed windows set `remain-on-exit`.
                Some(_) => {
                    DerivedTerminal { state: TerminalState::Exited, orphan_candidates: Vec::new() }
                }
                None if !record.runtime_confirmed => {
                    DerivedTerminal { state: TerminalState::Starting, orphan_candidates: Vec::new() }
                }
                None => DerivedTerminal { state: TerminalState::Lost, orphan_candidates: Vec::new() },
            }
        }
    }
}

/// Panes tagged with this daemon's identity that no durable record claims.
/// Never auto-adopted, never killed.
pub fn orphaned_panes<'a>(
    records: &[TerminalRecord],
    snapshot: &'a RuntimeSnapshot,
    daemon_id: Uuid,
) -> Vec<&'a TaggedPane> {
    snapshot
        .panes
        .iter()
        .filter(|p| p.daemon_id == daemon_id)
        .filter(|p| !records.iter().any(|r| r.id == p.terminal_id))
        .collect()
}

/// A workspace's state, from the durable facts plus its terminals.
///
/// Ordered by what the user must act on. Hidden first because it is the user's
/// own decision and outranks anything the machine noticed; a missing worktree
/// next because every terminal in it is lost as a consequence, and reporting
/// the consequence would send someone to restart a process in a directory that
/// is not there.
pub fn derive_workspace(
    hidden: bool,
    worktree_missing: bool,
    creation_failed: bool,
    terminals: &[(TerminalRecord, DerivedTerminal)],
) -> WorkspaceState {
    if hidden {
        return WorkspaceState::Hidden;
    }
    if worktree_missing {
        return WorkspaceState::WorktreeMissing;
    }
    if creation_failed {
        return WorkspaceState::Error;
    }

    // A loss is unresolved for exactly as long as the record exists: dismissing
    // one deletes it, and restarting one replaces it. There is no acknowledged
    // -but-still-listed state, because a row that can never say anything again
    // is not evidence, it is clutter.
    if terminals.iter().any(|(_, d)| d.state == TerminalState::Lost) {
        return WorkspaceState::Error;
    }

    let any_live = terminals
        .iter()
        .any(|(_, d)| matches!(d.state, TerminalState::Running | TerminalState::Starting));
    if any_live { WorkspaceState::Active } else { WorkspaceState::Ready }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::RuntimeSnapshot;

    fn daemon() -> Uuid {
        Uuid::from_u128(1)
    }

    fn pane(terminal_id: Uuid) -> TaggedPane {
        TaggedPane {
            daemon_id: daemon(),
            workspace_id: Uuid::from_u128(50),
            terminal_id,
            schema_version: 1,
            pane_id: "%1".into(),
            window_id: "@1".into(),
            columns: 80,
            rows: 24,
            dead: false,
            dead_status: None,
            command: "zsh".into(),
            left: 0,
            top: 0,
            window_active: true,
            pane_active: true,
            zoomed: false,
            tty: String::new(),
        }
    }

    fn dead_pane(terminal_id: Uuid, status: i32) -> TaggedPane {
        TaggedPane { dead: true, dead_status: Some(status), ..pane(terminal_id) }
    }

    fn record(intent: TerminalIntent, confirmed: bool) -> TerminalRecord {
        TerminalRecord {
            id: Uuid::from_u128(100),
            workspace_id: Uuid::from_u128(50),
            intent,
            runtime_confirmed: confirmed,
            exit_code: None,
            exit_signal: None,
        }
    }

    #[test]
    fn running_with_exactly_one_pane_is_running() {
        let r = record(TerminalIntent::Running, true);
        let s = RuntimeSnapshot::healthy(vec![pane(r.id)]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Running);
    }

    #[test]
    fn a_retained_dead_pane_is_an_observed_exit_not_a_loss() {
        // Without remain-on-exit tmux would destroy the window and this would be
        // indistinguishable from `lost`. That is the bug the retained pane fixes.
        let r = record(TerminalIntent::Running, true);
        let s = RuntimeSnapshot::healthy(vec![dead_pane(r.id, 0)]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Exited);
    }

    #[test]
    fn a_dead_pane_does_not_prove_life() {
        let p = dead_pane(Uuid::from_u128(100), 1);
        assert!(!p.proves_life());
    }

    #[test]
    fn running_with_no_pane_is_lost() {
        let r = record(TerminalIntent::Running, true);
        let s = RuntimeSnapshot::healthy(vec![]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Lost);
    }

    #[test]
    fn unconfirmed_with_no_pane_is_starting_not_lost() {
        let r = record(TerminalIntent::Running, false);
        let s = RuntimeSnapshot::healthy(vec![]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Starting);
    }

    #[test]
    fn duplicate_claimants_are_lost_and_surface_orphans() {
        let r = record(TerminalIntent::Running, true);
        let mut a = pane(r.id);
        a.pane_id = "%1".into();
        let mut b = pane(r.id);
        b.pane_id = "%2".into();
        let s = RuntimeSnapshot::healthy(vec![a, b]);

        let d = derive_terminal(&r, &s);
        assert_eq!(d.state, TerminalState::Lost, "duplicates are not proof");
        assert_eq!(d.orphan_candidates.len(), 2);
    }

    #[test]
    fn stopped_is_exited_regardless_of_panes() {
        let r = record(TerminalIntent::Stopped, true);
        let s = RuntimeSnapshot::healthy(vec![pane(r.id)]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Exited);
    }

    #[test]
    fn failed_is_error_regardless_of_panes() {
        let r = record(TerminalIntent::Failed, false);
        let s = RuntimeSnapshot::healthy(vec![pane(r.id)]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Error);
    }

    #[test]
    fn unusable_inventory_never_claims_life() {
        let r = record(TerminalIntent::Running, true);
        let s = RuntimeSnapshot::unavailable();
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Lost);
    }

    #[test]
    fn panes_for_other_terminals_do_not_count() {
        let r = record(TerminalIntent::Running, true);
        let s = RuntimeSnapshot::healthy(vec![pane(Uuid::from_u128(999))]);
        assert_eq!(derive_terminal(&r, &s).state, TerminalState::Lost);
    }

    #[test]
    fn untracked_tagged_pane_is_an_orphan_candidate() {
        let r = record(TerminalIntent::Running, true);
        let stray = pane(Uuid::from_u128(777));
        let s = RuntimeSnapshot::healthy(vec![pane(r.id), stray]);
        let orphans = orphaned_panes(&[r], &s, daemon());
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].terminal_id, Uuid::from_u128(777));
    }

    #[test]
    fn other_daemons_panes_are_ignored_completely() {
        let r = record(TerminalIntent::Running, true);
        let mut foreign = pane(Uuid::from_u128(777));
        foreign.daemon_id = Uuid::from_u128(2);
        let s = RuntimeSnapshot::healthy(vec![foreign]);
        assert!(orphaned_panes(&[r], &s, daemon()).is_empty());
    }

    // ---- workspace derivation ----

    fn derived(state: TerminalState) -> DerivedTerminal {
        DerivedTerminal { state, orphan_candidates: vec![] }
    }

    #[test]
    fn workspace_is_active_when_a_terminal_runs() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(false, false, false, &[(r, derived(TerminalState::Running))]);
        assert_eq!(s, WorkspaceState::Active);
    }

    #[test]
    fn workspace_is_ready_with_no_live_terminal() {
        let r = record(TerminalIntent::Stopped, true);
        let s = derive_workspace(false, false, false, &[(r, derived(TerminalState::Exited))]);
        assert_eq!(s, WorkspaceState::Ready);
    }

    #[test]
    fn a_lost_terminal_holds_workspace_in_error() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(false, false, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::Error);
    }

    #[test]
    fn dismissing_the_loss_clears_the_error_by_removing_the_record() {
        // What dismissal does is delete the row — see `Service::dismiss_lost` —
        // so from here it is simply a workspace with one terminal fewer.
        let s = derive_workspace(false, false, false, &[]);
        assert_eq!(s, WorkspaceState::Ready);
    }

    #[test]
    fn hidden_beats_everything_else() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(true, false, false, &[(r, derived(TerminalState::Running))]);
        assert_eq!(s, WorkspaceState::Hidden, "hiding is the user's decision, not a symptom");
    }

    #[test]
    fn hidden_wins_over_a_lost_terminal_too() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(true, false, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::Hidden);
    }

    /// A missing worktree outranks a lost terminal.
    ///
    /// Both are errors, and the terminal is lost BECAUSE the directory went
    /// away. Reporting the symptom would send someone to restart a terminal in
    /// a directory that no longer exists.
    #[test]
    fn a_missing_worktree_outranks_a_lost_terminal() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(false, true, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::WorktreeMissing);
    }
}
