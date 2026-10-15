//! How a terminal stream ends: its client hangs up, or its pane goes.
//!
//! Not when the pane's pipe ends (ov-554). Before tmux 3.8 a pane's pipe
//! lasted as long as the pane did: through its program dying, the pane kept
//! dead by `remain-on-exit`, and a restart or a chat/terminal switch
//! respawning it. A stream watching it saw the last output, then nothing,
//! then the new program, all through one pipe and one fanout. tmux 3.8 ends
//! the pipe when the program dies (`server_destroy_pane`), so the fanout ends
//! with it, and every stream on that pane ended there. No client opens a
//! stream again for a pane that is still the one it is showing, so a restart
//! left the view on the dead program's last screen, live underneath.
//!
//! So an ended pipe is asked about. A pane that is gone ends the stream, as it
//! always did. A dead pane is waited on until it is respawned or goes (see
//! `Revival` for how often it is looked at, and what that costs). A live one,
//! a respawned pane or one whose fanout dropped this watcher for falling
//! behind, gets a new pipe, and the stream goes on with the pane's picture as
//! it is now, because whatever the new program wrote before the new pipe
//! existed went nowhere.

use std::time::Duration;

use farcooler_core::inventory::{RuntimeInventory, TaggedPane};
use farcooler_tmux::PaneLife;
use tokio::time::Instant;

use crate::runtime::Runtime;

/// A stream's wait for its pane to have a pipe again.
///
/// **What a dead pane costs while it is watched.** Looks start `FIRST` apart
/// and double to `LONGEST`; once the pane has been dead for `LONG_DEAD`, they
/// are `DEAD_PAUSE` apart, for as long as somebody keeps it on screen. Over the
/// wire (`reading_inventory`), a look is the daemon's own inventory, which its
/// watcher refreshes every tick anyway, and tmux is asked only when that says
/// the pane is live or does not list it: nothing is spawned while it stays
/// dead. A `farcoolerd stream` (`asking_tmux`) is its own process with no
/// inventory that moves, so each of its looks is one `display-message`: five
/// in the first second and a quarter, then two a second until the pane has
/// been dead for `LONG_DEAD`, then one every `DEAD_PAUSE`. A restart is seen within the pause in force when it happens.
pub(crate) struct Revival {
    pause: Duration,
    /// When the looks first found the pane dead, since output last flowed.
    dead_since: Option<Instant>,
    inventory: bool,
}

impl Revival {
    /// The first look after a pipe ends. Not none: a fanout that ends again
    /// at once would otherwise be a tmux process and a capture in a loop.
    const FIRST: Duration = Duration::from_millis(50);

    /// The longest between looks while a pane may be about to be restarted.
    const LONGEST: Duration = Duration::from_millis(500);

    /// How long dead before nobody is likely to be restarting it right now.
    const LONG_DEAD: Duration = Duration::from_secs(30);

    /// Between looks at a pane dead for `LONG_DEAD`.
    const DEAD_PAUSE: Duration = Duration::from_secs(3);

    /// Each look asks tmux.
    pub(crate) fn asking_tmux() -> Self {
        Self { pause: Self::FIRST, dead_since: None, inventory: false }
    }

    /// Each look reads `Runtime::inventory`, which the daemon keeps fresh.
    pub(crate) fn reading_inventory() -> Self {
        Self { inventory: true, ..Self::asking_tmux() }
    }

    /// Output is flowing again: the next end starts the waits afresh.
    pub(crate) fn flowing(&mut self) {
        self.pause = Self::FIRST;
        self.dead_since = None;
    }

    /// The pause after a look at `now` that found the pane `life`.
    fn looked(&mut self, life: Option<PaneLife>, now: Instant) {
        if life == Some(PaneLife::Dead) {
            let since = *self.dead_since.get_or_insert(now);
            if now.duration_since(since) >= Self::LONG_DEAD {
                self.pause = Self::DEAD_PAUSE;
                return;
            }
        }
        self.pause = (self.pause * 2).min(Self::LONGEST);
    }

    /// A new connection to `pane`'s fanout and the pane's picture as it is
    /// now, once the pane is running a program; `None` once it has gone.
    ///
    /// The picture is `Runtime::opening_replay`'s, taken after the new pipe is
    /// up, so nothing written after it is missed. The seam is the other way
    /// round from an opening's, which captures first: bytes the program writes
    /// during the captures are in the picture and then again in the live
    /// stream, so a fresh shell's prompt can show twice until its next
    /// repaint. A resync has the same seam. A tmux that cannot be asked is
    /// asked again, rather than ending a stream the next look might keep.
    pub(crate) async fn revive(&mut self, runtime: &Runtime, pane: &TaggedPane) -> Option<(tokio::net::UnixStream, Vec<u8>)> {
        loop {
            tokio::time::sleep(self.pause).await;
            let life = self.look(runtime, pane).await;
            self.looked(life, Instant::now());
            match life {
                Some(PaneLife::Gone) => return None,
                Some(PaneLife::Dead) | None => continue,
                Some(PaneLife::Live) => {}
            }
            match runtime.attach_to_fanout(&pane.pane_id).await {
                Ok(socket) => return Some((socket, runtime.opening_replay(&pane.pane_id).await)),
                Err(error) => tracing::debug!(%error, pane = %pane.pane_id, "a respawned pane's pipe did not start"),
            }
        }
    }

    /// What the pane is now; `None` when tmux could not be asked.
    ///
    /// The inventory is believed only when it says dead. Live is confirmed
    /// with tmux, because the inventory may not have caught up with the death
    /// that ended the pipe; a pane it does not list is left to tmux too, so a
    /// read that failed never ends a stream.
    async fn look(&self, runtime: &Runtime, pane: &TaggedPane) -> Option<PaneLife> {
        if self.inventory {
            let snapshot = runtime.inventory.snapshot();
            let listed = snapshot
                .panes
                .iter()
                .find(|p| p.pane_id == pane.pane_id && p.terminal_id == pane.terminal_id);
            if snapshot.inventory_healthy && listed.is_some_and(|p| p.dead) {
                return Some(PaneLife::Dead);
            }
        }
        runtime.tmux.pane_life(&pane.pane_id, pane.terminal_id).await.ok()
    }
}

/// Resolves when stdin says the far end of `Runtime::stream` has gone.
pub(crate) async fn stdin_hangup() {
    use tokio::io::AsyncReadExt;
    if !stdin_can_hang_up() {
        // Never resolves, so the select in `stream_to` is left with one arm.
        std::future::pending::<()>().await;
    }
    let mut ignored = [0u8; 64];
    let mut stdin = tokio::io::stdin();
    loop {
        match stdin.read(&mut ignored).await {
            Ok(0) | Err(_) => return,
            Ok(_) => continue,
        }
    }
}

/// Whether closed stdin would mean anything here.
///
/// True for a pipe or a socket, which is what an ssh channel and a parent
/// process's pipe both are, and where end of stream genuinely means the
/// other end is gone. False for everything else — a terminal, where reading
/// would take a person's keystrokes, and `/dev/null`, which is at end of
/// stream before it is ever read.
fn stdin_can_hang_up() -> bool {
    // SAFETY: a zeroed stat is a valid out parameter, and fd 0 is always a
    // valid descriptor number to ask about — fstat reports if it is closed.
    unsafe {
        let mut info: libc::stat = std::mem::zeroed();
        if libc::fstat(0, &mut info) != 0 {
            return false;
        }
        let kind = info.st_mode & libc::S_IFMT;
        kind == libc::S_IFIFO || kind == libc::S_IFSOCK
    }
}

#[cfg(test)]
#[path = "stream_end_tests.rs"]
mod tests;
