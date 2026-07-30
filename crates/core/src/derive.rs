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

use overnight_protocol::v1::{TerminalIntent, TerminalState, WorkspaceState};
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
    /// The user acknowledged a loss. Stops it holding the workspace in `error`
    /// without ever relabelling it `exited`.
    pub loss_dismissed: bool,
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

/// Workspace state is derived from its terminals on every read.
pub fn derive_workspace(
    archived: bool,
    creation_failed: bool,
    terminals: &[(TerminalRecord, DerivedTerminal)],
) -> WorkspaceState {
    if archived {
        return WorkspaceState::Archived;
    }
    if creation_failed {
        return WorkspaceState::Error;
    }

    let unresolved_loss = terminals
        .iter()
        .any(|(rec, d)| d.state == TerminalState::Lost && !rec.loss_dismissed);
    if unresolved_loss {
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
            loss_dismissed: false,
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
        let s = derive_workspace(false, false, &[(r, derived(TerminalState::Running))]);
        assert_eq!(s, WorkspaceState::Active);
    }

    #[test]
    fn workspace_is_ready_with_no_live_terminal() {
        let r = record(TerminalIntent::Stopped, true);
        let s = derive_workspace(false, false, &[(r, derived(TerminalState::Exited))]);
        assert_eq!(s, WorkspaceState::Ready);
    }

    #[test]
    fn unresolved_loss_holds_workspace_in_error() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(false, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::Error);
    }

    #[test]
    fn dismissing_the_loss_clears_the_error_without_claiming_an_exit() {
        let mut r = record(TerminalIntent::Running, true);
        r.loss_dismissed = true;
        let s = derive_workspace(false, false, &[(r.clone(), derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::Ready, "dismissal resolves the workspace");
        // and the terminal itself stays truthfully lost
        assert_eq!(r.intent, TerminalIntent::Running);
    }

    #[test]
    fn archived_wins_over_everything() {
        let r = record(TerminalIntent::Running, true);
        let s = derive_workspace(true, false, &[(r, derived(TerminalState::Lost))]);
        assert_eq!(s, WorkspaceState::Archived);
    }
}
