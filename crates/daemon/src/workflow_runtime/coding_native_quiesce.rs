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
    pub(super) fn save_native_turn(&self, binding: &Binding, turn: &NativeSettlement) -> Result<()> {
        use std::io::Write;
        let path = self.native_path(binding, "turn");
        let parent = path.parent().ok_or(Error::Refused("missing native turn directory"))?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(&serde_json::to_vec(turn)?)?;
        staged.as_file().sync_all()?;
        staged.persist_noclobber(&path).map_err(|error| error.error)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub(super) fn saved_native_turn(&self, binding: &Binding) -> Result<Option<NativeSettlement>> {
        let raw = match self.bytes(&self.native_path(binding, "turn")) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let turn: NativeSettlement = serde_json::from_slice(&raw)?;
        turn.completion(binding)?;
        Ok(Some(turn))
    }

    pub(super) fn save_native_settlement(&self, binding: &Binding, mut turn: NativeSettlement) -> Result<Option<bool>> {
        // The last recorded verdict wins once the reviewer is quiescent (ov-1082).
        if self.recorded_verdict(binding)
            && let NativeSettlement::Launched { completed, failure, .. } = &mut turn {
            *completed = Some(true);
            *failure = None;
        }
        let completed = turn.completion(binding)?;
        durable(&self.native_path(binding, "settled"), &serde_json::to_vec(&turn)?)?;
        let _ = std::fs::remove_file(self.native_path(binding, "turn"));
        Ok(completed)
    }

    /// Resolve may accept a completed turn while detached descendants still
    /// run. Require the hold to confirm both leader exit and an empty group.
    pub(in crate::workflow_runtime) async fn resolve_native_leftovers(&self, binding: &Binding, active: bool) -> Result<bool> {
        let Some(owner) = self.retained_native_owner(binding)? else { return Ok(false); };
        if !owner.hold { return Ok(false); }
        let release = self.native_path(binding, "resolve-leftovers");
        if release.try_exists()? {
            return if active { Ok(true) } else { self.recover_native_result(binding).await };
        }
        let socket = self.native_hold_socket(binding);
        let Some(native_hold::HoldStatus::Exited { group_empty: true, leftovers, .. }) = native_hold::status(&socket).await
            else { return Ok(false); };
        let Some(reason) = native_hold::leftover_reason(&leftovers) else { return Ok(false); };
        let Some(turn @ NativeSettlement::Launched { completed: Some(_), .. }) = self.saved_native_turn(binding)?
            else { return Err(Error::NativeLeftovers(reason)); };
        // Retain the decision across a runner restart. The owned future still
        // closes its pane and drains the log before writing the settlement.
        durable(&release, b"")?;
        if !active {
            self.publish_recovered_native_turn(binding, turn)?;
        }
        Ok(true)
    }

    /// Automatic recovery waits for quiescence. Resolve can explicitly accept
    /// a completed turn while the hold continues accounting for leftovers.
    pub(in crate::workflow_runtime) async fn recover_native_result(&self, binding: &Binding) -> Result<bool> {
        let settled = self.native_path(binding, "settled");
        if settled.try_exists()? {
            let result: NativeSettlement = serde_json::from_slice(&self.bytes(&settled)?)?;
            let completed = result.completion(binding)?;
            self.write_native_fact(binding, completed, completed.is_none() && self.effect_pending(binding)?)?;
            return Ok(true);
        }
        let turn = self.saved_native_turn(binding)?;
        if self.native_path(binding, "resolve-leftovers").try_exists()? {
            let turn = turn.ok_or(Error::Refused("missing resolved native turn"))?;
            self.publish_recovered_native_turn(binding, turn)?;
            return Ok(true);
        }
        let Some(owner) = self.retained_native_owner(binding)? else { return Ok(false) };
        if !owner.hold { return Ok(false); }
        let socket = self.native_hold_socket(binding);
        if socket.try_exists()? {
            let status = native_hold::status(&socket).await;
            // The hold can close while it answers.
            if socket.try_exists()? {
                if let Some(native_hold::HoldStatus::Exited { group_empty, leftovers, .. }) = status {
                    if let Some(reason) = native_hold::leftover_reason(&leftovers) {
                        return Err(Error::NativeLeftovers(reason));
                    }
                    if group_empty {
                        return Err(Error::NativeLeftovers("Waiting for the hold to confirm the agent's processes have exited.".into()));
                    }
                }
                return Ok(false);
            }
        }
        let turn = match turn {
            Some(turn) => turn,
            None if binding.effect.kind == EffectKind::Execute(StageKind::Build) => {
                if native_hold::settled_exit(&socket) != Some(0) { return Ok(false); }
                NativeSettlement::Launched { owner, hooks: Vec::new(), completed: Some(true), failure: None }
            }
            None => return Ok(false),
        };
        self.publish_recovered_native_turn(binding, turn)?;
        Ok(true)
    }

    fn publish_recovered_native_turn(&self, binding: &Binding, turn: NativeSettlement) -> Result<()> {
        if !self.process_log(binding).try_exists()? { durable(&self.process_log(binding), b"")?; }
        let completed = self.save_native_settlement(binding, turn)?;
        self.write_native_fact(binding, completed, completed.is_none() && self.effect_pending(binding)?)?;
        self.remove_native_hook_socket(binding)?;
        Ok(())
    }

    pub(super) fn remove_native_hook_socket(&self, binding: &Binding) -> Result<()> {
        match std::fs::remove_file(self.native_socket(binding)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// End the owner's agent and close its window. The runner never signals the agent itself (ov-1330): it asks the
    /// agent's hold, which signals only the process group it created and has not reaped, and which answers `ended`
    /// only once that group is empty and nothing descended from the hold is alive; a hold that is gone removed its
    /// socket only after that, so that is the same answer. The owner can end by itself while it is being ended (ov-1287).
    ///
    /// A hold that is there but does not answer was killed before it could prove its agent ended. Its pane and tty prove
    /// nothing (a command that left the agent's session holds neither), so that is a refusal. An owner an earlier engine
    /// launched without a hold can only be observed: pane dead or gone, nothing on its tty. Nothing is ever signalled.
    pub(super) async fn quiesce_native(&self, owner: &NativeOwner) -> Result<()> {
        self.quiesce_native_owner(owner, None).await
    }

    pub(super) async fn quiesce_native_turn(&self, owner: &NativeOwner, cancelled: &tokio::sync::watch::Receiver<bool>) -> Result<()> {
        self.quiesce_native_owner(owner, Some(cancelled)).await
    }

    async fn quiesce_native_owner(&self, owner: &NativeOwner, cancelled: Option<&tokio::sync::watch::Receiver<bool>>) -> Result<()> {
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
            let socket = self.native_hold_socket(&owner.binding);
            let mut answer = native_hold::ask(&socket, if cancelled.is_some() { "end-now" } else { "end" }, END_WAIT).await;
            if answer == Answer::Said("unknown".into()) && cancelled.is_some() {
                answer = native_hold::ask(&socket, "end", END_WAIT).await;
            }
            match answer {
                Answer::Gone => {}
                Answer::Said(word) if word == "ended" => {}
                Answer::Said(word) if word == "unsettled" => {
                    if let Some(cancelled) = cancelled {
                        self.wait_for_native_leftovers(owner, &socket, cancelled).await?;
                    } else if socket.try_exists()? {
                        if let Some(native_hold::HoldStatus::Exited { leftovers, .. }) = native_hold::status(&socket).await
                            && let Some(reason) = native_hold::leftover_reason(&leftovers) {
                            return Err(Error::NativeLeftovers(reason));
                        }
                        if socket.try_exists()? {
                            return Err(Error::Refused("native agent left a process running outside its group"));
                        }
                    }
                }
                Answer::Said(_) => return Err(Error::Refused("native hold did not end its agent")),
                Answer::Silent => {
                    tracing::warn!(effect = %owner.binding.effect.id, "the native hold is not answering; refusing to settle");
                    return Err(Error::Refused("native hold is not answering, so its agent cannot be proved ended"));
                }
            }
            return close_native(&tmux, owner).await;
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

    /// The hold has already proved the group empty. Wait without asking it
    /// to signal anything again, and keep the turn's owner visible meanwhile.
    async fn wait_for_native_leftovers(&self, owner: &NativeOwner, socket: &Path,
        cancelled: &tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let binding = &owner.binding;
        let waiting = crate::engine_stuck::Leftovers::new(&binding.workspace, &binding.effect);
        let mut heard = tokio::time::Instant::now();
        let release = self.native_path(binding, "resolve-leftovers");
        while socket.try_exists()? && !release.try_exists()? {
            if *cancelled.borrow() { return Err(Error::Refused("native leftovers are still running")); }
            match native_hold::status(socket).await {
                Some(native_hold::HoldStatus::Exited { leftovers, .. }) => {
                    waiting.report(native_hold::leftover_reason(&leftovers));
                    heard = tokio::time::Instant::now();
                }
                _ if !socket.try_exists()? => break,
                _ if heard.elapsed() >= END_WAIT => return Err(Error::Refused("native hold did not report its leftovers")),
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(())
    }
}
