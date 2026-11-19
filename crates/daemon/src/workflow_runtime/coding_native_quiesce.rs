//! Quiescence for a retained native session.
use super::*;

impl NativeOwner {
    pub(super) fn processes(&self, table: &crate::foreground::Foreground) -> Result<Vec<i32>> {
        let tty = self.tty.trim_start_matches("/dev/");
        let Some(supervisor) = &self.supervisor else { return Ok(table.under_tty(tty)) };
        let session = i32::try_from(supervisor.pid).ok().filter(|&pid| pid > 1)
            .ok_or(Error::Refused("invalid native supervisor pid"))?;
        let processes = table.under_tty_in_session(tty, session);
        // A session keeps its ID while any member survives. If the PID was
        // recycled as a new session leader, its start identity has changed.
        if !processes.is_empty() && !gone(session) {
            match crate::effect_runner::Owner::started_at(supervisor.pid) {
                Ok(started) if started != supervisor.started => return Ok(Vec::new()),
                Err(error) if !gone(session) => return Err(Error::Io(error)),
                _ => {}
            }
        }
        Ok(processes)
    }
}

impl Exec {
    /// End the owner's process tree and close its window. The owner can end by itself while it is being ended: a
    /// cancelled reviewer whose Stop hook loses its answer exits (ov-1287). Whichever is seen first, the proof is the
    /// same, nothing left on the owner's tty, and the answer is the same. A pane tmux still calls live with nothing
    /// on its tty is an exit tmux hasn't noticed yet, so it is read again until tmux agrees.
    pub(super) async fn quiesce_native(&self, owner: &NativeOwner) -> Result<()> {
        let tmux = server(&self.root, &owner.binding);
        if owner.terminal != terminal(&owner.binding) {
            return Err(Error::Refused("foreign native terminal"));
        }
        let mut owned_sample = None;
        for round in 0..100 {
            let life = tmux.pane_life(&owner.pane, owner.terminal).await
                .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
            let supervisor = if life == farcooler_tmux::PaneLife::Live {
                let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native owner inventory unavailable"))?;
                let Some(pane) = panes.iter().find(|p| !p.dead && p.pane_id == owner.pane && p.terminal_id == owner.terminal) else {
                    // Closed since tmux called it live: read its life again.
                    tokio::time::sleep(Duration::from_millis(10)).await; continue;
                };
                if pane.daemon_id != owner.terminal || pane.worktree_id != owner.terminal || pane.tty != owner.tty {
                    return Err(Error::Refused("native tty differs from retained owner"));
                }
                if owner.supervisor.as_ref().is_some_and(|record| record.pid != pane.stamp.pid) {
                    return Err(Error::Refused("native supervisor differs from retained owner"));
                }
                #[cfg(test)]
                if let Some(hook) = AFTER_LIVE.with(|h| h.borrow_mut().take()) { hook(); }
                Some(pane.stamp.pid as i32)
            } else { None };
            let table = tokio::task::spawn_blocking(snapshot).await
                .map_err(|_| Error::Refused("native process read failed"))??;
            let current = owner.processes(&table)?;
            match life {
                // A retained dead pane can still name a tty the kernel has handed to another pane.
                farcooler_tmux::PaneLife::Gone if current.is_empty() => return Ok(()),
                farcooler_tmux::PaneLife::Gone => return Err(Error::Refused("native vanished pane still has processes")),
                farcooler_tmux::PaneLife::Dead if current.is_empty() => return close_native(&tmux, owner).await,
                farcooler_tmux::PaneLife::Dead if owner.supervisor.is_some() => {
                    owned_sample = Some((None, current));
                    break;
                }
                farcooler_tmux::PaneLife::Dead => return Err(Error::Refused("native dead pane still has processes")),
                farcooler_tmux::PaneLife::Live => {}
            }
            if let Some(pid) = supervisor.filter(|pid| current.contains(pid)) { owned_sample = Some((Some(pid), current)); break; }
            if round % 10 == 9 { tmux.nudge_reap().await; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let Some((supervisor, mut current)) = owned_sample else { return Err(Error::Refused("native owner exit never observed")) };
        // Stop the tty roots and all descendants before terminating them so an
        // interactive shell's separate job groups cannot escape a leader kill.
        let mut owned = BTreeSet::new();
        let mut stable = false;
        for _ in 0..8 {
            let old = owned.len();
            for pid in current {
                if pid <= 1 || pid == std::process::id() as i32 { return Err(Error::Refused("invalid owned native pid")); }
                owned.insert(pid);
                // Only current live descendants of this exact tagged PTY.
                unsafe { libc::kill(pid, libc::SIGSTOP); }
            }
            if old == owned.len() { stable = true; break; }
            let table = tokio::task::spawn_blocking(snapshot).await
                .map_err(|_| Error::Refused("native process read failed"))??;
            current = owner.processes(&table)?;
            // A legacy owner has no session pin. Stop collecting processes if its supervisor exits.
            if owner.supervisor.is_none() && supervisor.is_some_and(|pid| !current.contains(&pid)) { stable = true; break; }
        }
        if !stable { return Err(Error::Refused("native process tree did not settle")); }
        for &pid in owned.iter().rev() { unsafe { libc::kill(pid, libc::SIGKILL); } }
        // tmux is the wait/reap owner of the PTY child, not the workflow daemon.
        // A killed process stays findable by `kill(pid, 0)` until its parent reaps it, and a
        // pane's process is tmux's child: tmux 3.4 can miss the exit and leave it a zombie.
        // A zombie is dead, and the nudge makes tmux reap it.
        for round in 0..100 {
            if owned.iter().all(|&pid| gone(pid)) { return close_native(&tmux, owner).await; }
            if round % 10 == 9 { tmux.nudge_reap().await; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(Error::Refused("native descendants not quiescent"))
    }
}
