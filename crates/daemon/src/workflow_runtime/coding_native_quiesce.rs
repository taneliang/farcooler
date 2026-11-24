//! Quiescence for a retained native session.
use super::*;

impl NativeOwner {
    pub(super) fn processes(&self, table: &crate::foreground::Foreground) -> Result<Vec<i32>> {
        let tty = self.tty.trim_start_matches("/dev/");
        let Some(supervisor) = &self.supervisor else { return Ok(table.under_tty(tty)) };
        let session = i32::try_from(supervisor.pid).ok().filter(|&pid| pid > 1)
            .ok_or(Error::Refused("invalid native supervisor pid"))?;
        let processes = table.under_tty_in_session(tty, session)
            .ok_or(Error::Refused("native process sessions unavailable"))?;
        // A session keeps its ID while any member survives. If the PID was
        // recycled as a new session leader, its start identity has changed.
        let owns_session = || crate::process_reads::DETAILS.read(move || unsafe { libc::getsid(session) } == session)
            .ok_or(Error::Refused("native process session query timed out"));
        if !processes.is_empty() && owns_session()? {
            match crate::effect_runner::Owner::matches_started(supervisor.pid, &supervisor.started) {
                Ok(false) => return Ok(Vec::new()),
                Err(error) if owns_session()? => return Err(Error::Io(error)),
                _ => {}
            }
        }
        Ok(processes)
    }
}

impl Exec {
    /// End the owner's agent and close its window. The runner never signals the agent itself (ov-1330): it asks the
    /// agent's hold, which signals only the process group it created and has not reaped, and which answers `ended`
    /// only once that group is empty and nothing descended from the hold is alive; a hold that is gone removed its
    /// socket only after that, so that is the same answer. The owner can end by itself while it is being ended (ov-1287).
    ///
    /// A hold that is there but does not answer was killed before it could prove its agent ended. Its pane and tty prove
    /// nothing (a command that left the agent's session holds neither), so that is a refusal. An owner an earlier engine
    /// launched without a hold can only be observed: pane dead or gone, nothing on its tty. Nothing is ever signalled.
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
                // Its group is empty, but a process the agent started outside it is still alive: never settle over
                // work that may still be writing.
                Answer::Said(word) if word == "unsettled" => return Err(Error::Refused("native agent left a process running outside its group")),
                Answer::Said(_) => return Err(Error::Refused("native hold did not end its agent")),
                Answer::Silent => {
                    tracing::warn!(effect = %owner.binding.effect.id, "the native hold is not answering; refusing to settle");
                    return Err(Error::Refused("native hold is not answering, so its agent cannot be proved ended"));
                }
            }
        }
        for round in 0..100 {
            let life = tmux.pane_life(&owner.pane, owner.terminal).await
                .map_err(|_| Error::Refused("native pane ownership unavailable"))?;
            let table = snapshot().await?;
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
