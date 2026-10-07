//! `core` owns the derivation rule but never depends on the `tmux` crate.
//!
//! Dependencies point one way: `tmux` implements this trait, `core` defines it.
//! The trait returns the WHOLE tagged inventory in one call rather than
//! answering per-terminal lookups, because derivation runs on the fleet-render
//! path and a per-terminal trait would reintroduce the round-trip problem the
//! inventory view exists to avoid.

use uuid::Uuid;

/// One pane carrying Far Cooler's exact tags.
///
/// Names, indexes, and PID values are display or diagnostic data only and never
/// establish identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedPane {
    pub daemon_id: Uuid,
    pub worktree_id: Uuid,
    pub terminal_id: Uuid,
    pub schema_version: u32,
    /// Stable tmux pane id (`%12`). Diagnostic, never identity.
    pub pane_id: String,
    /// Stable tmux window id (`@7`). Diagnostic, never identity.
    pub window_id: String,
    pub columns: u32,
    pub rows: u32,
    /// Where the pane sits in its window, in cells, as tmux placed it.
    ///
    /// Read rather than computed. tmux already owns pane arrangement — it has
    /// split trees, five named layouts, resizable dividers and a zoom flag — and
    /// a second implementation of that in the daemon, plus a third in each client
    /// to draw it, was three chances to disagree about where a pane is. Asking is
    /// one.
    pub left: u32,
    pub top: u32,
    /// The window this pane shares with the rest of its layout.
    pub window_active: bool,
    pub pane_active: bool,
    /// tmux's own zoom, `resize-pane -Z`.
    pub zoomed: bool,
    /// The pane's terminal device, e.g. `/dev/ttys162`.
    ///
    /// The handle for finding what is actually running: `pane_current_command`
    /// gives a process NAME, so `pnpm dev` reads as `node` and `cargo build` as
    /// `cargo`. The foreground process group on this tty has the argv.
    pub tty: String,
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
    /// The signal that killed a dead pane's command, when one did, as tmux
    /// names it.
    ///
    /// A name and not a number because that is what tmux gives: `sig2name`
    /// renders `kill` where the platform has `sys_signame` (macOS) and `9`
    /// where it does not (Linux).
    ///
    /// A dead pane carries exactly one of this and `dead_status` once tmux has
    /// reaped its process. Neither means the exit is not settled yet — see
    /// `exit_unsettled`.
    pub dead_signal: Option<String>,
    /// The pane's foreground process, as tmux reports it.
    ///
    /// This is how Far Cooler knows an agent is running. A terminal is created
    /// as a plain shell and the user types `claude` into it — so what the
    /// terminal was launched as says nothing about what is running in it now,
    /// and only the live process does.
    pub command: String,
    /// The pane's OSC title, as its program set it.
    ///
    /// Empty for most programs. The three coding agents all set one, and it
    /// carries both what they are doing and what they are doing it to — see
    /// `crate::title`.
    pub title: String,
    /// What tmux says about this pane's screen without drawing it. See
    /// `ScreenStamp`.
    pub stamp: ScreenStamp,
}

/// What a pane's screen was doing when tmux was last asked, in numbers that
/// come from the same `list-panes` that finds the pane.
///
/// Reading a screen is a `capture-pane`, which is a process; reading these is
/// free, because the inventory already ran. A screen that was captured under
/// one stamp and is asked for again under the SAME stamp, some seconds later,
/// has nothing new to say: tmux bumps `window_activity` whenever any pane in
/// the window receives output, moves the cursor and the scrollback with it,
/// and a respawned program has a new pid. The sampling loop uses that to read
/// only the screens that moved (`watch::screen_cache`).
///
/// `activity` is a window's, in whole seconds, so it is blunt: it also moves
/// for a sibling pane in the same window, and it cannot say which of two writes
/// in the same second came last. The cache allows for both. It is zero when
/// the number is unknown, and a zero stamp is never trusted.
///
/// Compares EQUAL to every other stamp, on purpose. A pane's output moves its
/// stamp on every tick of a busy agent, and `TaggedPane`'s equality is what the
/// backstop reconcile uses to report a missed notification as a defect: a
/// stamp that took part would turn every busy pane into one. Whether a stamp
/// moved is `unchanged_since`.
#[derive(Debug, Clone, Copy, Default, Eq)]
pub struct ScreenStamp {
    /// The window's last activity, Unix seconds.
    pub activity: u64,
    /// Lines of scrollback.
    pub history: u32,
    pub cursor: (u32, u32),
    /// The program's pid: a respawned pane has the old pane's id.
    pub pid: u32,
}

impl PartialEq for ScreenStamp {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl ScreenStamp {
    /// Whether nothing tmux can see about the screen moved since `then`.
    /// False for a stamp whose activity is unknown, which proves nothing.
    pub fn unchanged_since(&self, then: &ScreenStamp) -> bool {
        self.activity != 0
            && self.activity == then.activity
            && self.history == then.history
            && self.cursor == then.cursor
            && self.pid == then.pid
    }
}

impl TaggedPane {
    /// A dead pane proves an exit. It does not prove life.
    pub fn proves_life(&self) -> bool {
        !self.dead
    }

    /// Dead, but tmux does not yet know how it died.
    ///
    /// tmux marks a pane dead when its pty reports end of file, and records
    /// the exit status when SIGCHLD is handled. Those are two separate events
    /// in its loop, in either order, so for a moment a pane can read as dead
    /// with neither an exit code nor a signal.
    pub fn exit_unsettled(&self) -> bool {
        self.dead && self.dead_status.is_none() && self.dead_signal.is_none()
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

    pub fn unavailable() -> Self {
        Self { snapshot: RuntimeSnapshot::unavailable() }
    }
}

impl RuntimeInventory for FakeInventory {
    fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.clone()
    }
}
