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
        if !processes.is_empty() && !crate::effect_runner::Owner::gone(supervisor.pid)? {
            match crate::effect_runner::Owner::started_at(supervisor.pid) {
                Ok(started) if started != supervisor.started => return Ok(Vec::new()),
                Err(error) if !crate::effect_runner::Owner::gone(supervisor.pid)? => return Err(Error::Io(error)),
                _ => {}
            }
        }
        Ok(processes)
    }
}

impl Exec {
    /// End the owner's agent and close its window. The runner never signals the agent itself (ov-1330): it asks the
    /// agent's hold, which signals only the process group it created and has not reaped, and which answers `ended`
    /// only once the group is empty and nothing descended from the hold is alive. A hold that is gone removed its socket after its group was empty, so that is the
    /// same answer. The owner can end by itself while it is being ended (ov-1287); the hold gives one answer either way.
    ///
    /// An owner an earlier engine launched without a hold can only be observed: its pane
    /// dead or gone with nothing on its tty. Nothing is signalled on that path.
    pub(super) async fn quiesce_native(&self, owner: &NativeOwner) -> Result<()> {
        let tmux = server(&self.root, &owner.binding);
        if owner.terminal != terminal(&owner.binding) {
            return Err(Error::Refused("foreign native terminal"));
        }
        // A live pane must still be the retained owner's before its hold is told anything.
        let life = tmux.pane_life(&owner.pane, owner.terminal).await
            .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
        if life == farcooler_tmux::PaneLife::Live {
            let panes = tmux.list_tagged_panes().await.map_err(|_| Error::Refused("native owner inventory unavailable"))?;
            if panes.iter().any(|p| p.pane_id == owner.pane && p.terminal_id == owner.terminal
                && (p.daemon_id != owner.terminal || p.worktree_id != owner.terminal || p.tty != owner.tty)) {
                return Err(Error::Refused("native tty differs from retained owner"));
            }
        }
        if owner.hold {
            match native_hold::ask(&self.native_hold_socket(&owner.binding), "end", END_WAIT).await {
                Answer::Gone => return close_native(&tmux, owner).await,
                Answer::Said(word) if word == "ended" => return close_native(&tmux, owner).await,
                Answer::Said(word) if word == "unsettled" => return Err(Error::Refused("native agent left a process running outside its group")),
                Answer::Said(_) => return Err(Error::Refused("native hold did not end its agent")),
                Answer::Silent => return Err(Error::Refused("native hold is not answering")),
            }
        }
        for round in 0..100 {
            let life = tmux.pane_life(&owner.pane, owner.terminal).await
                .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
            let table = tokio::task::spawn_blocking(snapshot).await
                .map_err(|_| Error::Refused("native process read failed"))??;
            let busy = !owner.processes(&table)?.is_empty();
            match life {
                farcooler_tmux::PaneLife::Gone if !busy => return Ok(()),
                farcooler_tmux::PaneLife::Gone => return Err(Error::Refused("native vanished pane still has processes")),
                farcooler_tmux::PaneLife::Dead if !busy => return close_native(&tmux, owner).await,
                _ => {
                    if round % 10 == 9 { tmux.nudge_reap().await; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        Err(Error::Refused("native owner has no hold to end it"))
    }
}
