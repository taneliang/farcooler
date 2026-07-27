//! `core` owns the derivation rule but never depends on the `tmux` crate.
//!
//! Dependencies point one way: `tmux` implements this trait, `core` defines it.
//! The trait returns the WHOLE tagged inventory in one call rather than
//! answering per-terminal lookups, because derivation runs on the fleet-render
//! path and a per-terminal trait would reintroduce the round-trip problem the
//! inventory view exists to avoid.

use uuid::Uuid;

/// One pane carrying Overnight's exact tags.
///
/// Names, indexes, and PID values are display or diagnostic data only and never
/// establish identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedPane {
    pub daemon_id: Uuid,
    pub workspace_id: Uuid,
    pub terminal_id: Uuid,
    pub schema_version: u32,
    /// Stable tmux pane id (`%12`). Diagnostic, never identity.
    pub pane_id: String,
    /// Stable tmux window id (`@7`). Diagnostic, never identity.
    pub window_id: String,
    pub columns: u32,
    pub rows: u32,
    /// The command exited but the pane is retained by `remain-on-exit`.
    ///
    /// This distinction is load-bearing. Without a retained pane, tmux destroys
    /// the window the instant the command exits, and a clean exit becomes
    /// indistinguishable from a terminal that was lost. `exited` is defined as
    /// "the daemon observed an exit code or signal", which is only observable
    /// because the dead pane stays long enough to be read.
    pub dead: bool,
    /// Exit code reported by tmux for a dead pane, when it gave one.
    pub dead_status: Option<i32>,
}

impl TaggedPane {
    /// A dead pane proves an exit. It does not prove life.
    pub fn proves_life(&self) -> bool {
        !self.dead
    }
}

/// A snapshot of what is alive right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub panes: Vec<TaggedPane>,
    /// False when the private tmux server could not be inventoried safely. The
    /// daemon then serves durable state with every terminal derived as `lost`
    /// and shows a visible degraded state rather than guessing.
    pub inventory_healthy: bool,
}

impl RuntimeSnapshot {
    pub fn healthy(panes: Vec<TaggedPane>) -> Self {
        Self { panes, inventory_healthy: true }
    }

    pub fn unavailable() -> Self {
        Self { panes: Vec::new(), inventory_healthy: false }
    }

    /// All panes claiming a given terminal id. More than one is not proof.
    pub fn claimants(&self, terminal_id: Uuid) -> Vec<&TaggedPane> {
        self.panes.iter().filter(|p| p.terminal_id == terminal_id).collect()
    }
}

/// Implemented by the `tmux` crate over its live control-mode view.
pub trait RuntimeInventory: Send + Sync {
    fn snapshot(&self) -> RuntimeSnapshot;
}

/// Test double. Lets the derivation rule be unit-tested with no tmux present.
#[derive(Debug, Default)]
pub struct FakeInventory {
    pub snapshot: RuntimeSnapshot,
}

impl FakeInventory {
    pub fn with_panes(panes: Vec<TaggedPane>) -> Self {
        Self { snapshot: RuntimeSnapshot::healthy(panes) }
    }

    pub fn unavailable() -> Self {
        Self { snapshot: RuntimeSnapshot::unavailable() }
    }
}

impl RuntimeInventory for FakeInventory {
    fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.clone()
    }
}
