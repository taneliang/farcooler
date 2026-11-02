//! The dedicated hourly check-in pass. It never enters the ordinary wake
//! queue, and a submitted occurrence is only observed, never sent again.

use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::watch::Watcher;

impl Watcher {
    /// Run only under the exclusive daemon owner, after stale attempt recovery.
    #[cfg(unix)]
    pub async fn run_check_in_scheduler(self: Arc<Self>, daemon: Uuid, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        {
            let _gate = self.check_in_cleanup_gate.lock().await;
            let armed = self.check_in_arm.load(std::sync::atomic::Ordering::SeqCst) == 0
                && matches!(self.service.store.arm_check_in_instance(daemon), Ok(true));
            self.check_in_arm.store(if armed { 1 } else { 2 }, std::sync::atomic::Ordering::SeqCst);
            if !armed { tracing::warn!("scheduled check-ins unavailable: unclean or unreadable instance"); }
        }
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break }
                    continue;
                }
                _ = ticker.tick() => {}
            }
            let now = super::now_millis();
            match self.service.store.submitted_check_ins() {
                Ok(submitted) => for (occurrence, attempt) in submitted {
                    if let Err(error) = self.observe_check_in(occurrence, attempt).await {
                        tracing::warn!(%occurrence, error = %error, "check-in receipt observation failed");
                    }
                },
                Err(error) => tracing::warn!(error = %error, "could not read submitted check-ins"),
            }
            if let Err(error) = self.service.store.expire_check_in_submissions(now) {
                tracing::warn!(error = %error, "could not expire unconfirmed check-ins");
            }
            match self.service.store.due_check_in_workspaces(now) {
                Ok(workspaces) => for workspace in workspaces {
                    if let Err(error) = self.service.store.tick_check_in(workspace, now) {
                        tracing::warn!(%workspace, error = %error, "could not create due check-in");
                    }
                },
                Err(error) => tracing::warn!(error = %error, "could not read due check-ins"),
            }
            match self.service.store.pending_check_ins() {
                Ok(pending) => for occurrence in pending {
                    if *shutdown.borrow() || self.check_in_arm.load(std::sync::atomic::Ordering::SeqCst) != 1 { break }
                    if let Err(error) = self.send_check_in(&occurrence, daemon).await {
                        tracing::warn!(id = %occurrence.id, error = %error, "check-in did not complete delivery");
                    }
                },
                Err(error) => tracing::warn!(error = %error, "could not read pending check-ins"),
            }
        }
        let _gate = self.check_in_cleanup_gate.lock().await;
        if self.check_in_arm.swap(2, std::sync::atomic::Ordering::SeqCst) == 1
            && !matches!(self.service.store.close_check_in_instance(daemon), Ok(true))
        { tracing::warn!("scheduled check-in instance remains unclean"); }
    }

    #[cfg(unix)]
    pub(super) async fn observe_check_in(&self, occurrence: Uuid, attempt: Uuid) -> farcooler_core::Result<()> {
        use std::os::unix::fs::MetadataExt;
        use crate::claude_registry::{Kernel, Processes};
        use farcooler_core::session_log::codex_prompts::exact_prompt_in_pinned_file;

        let Some(observation) = self.service.store.check_in_observation(occurrence, attempt)? else { return Ok(()) };
        let Some(to) = self.service.live_orchestrator(observation.workspace)? else { return Ok(()) };
        if to.id != observation.terminal || to.agent_session_id.as_deref() != Some(observation.thread.as_str())
            || i64::try_from(to.lease_generation).ok() != Some(observation.terminal_lease)
            || i64::try_from(to.epoch).ok() != Some(observation.terminal_epoch)
        { return Ok(()) }
        let Ok((preset, _, pid)) = self.proven_agent(&to).await else { return Ok(()) };
        if preset != "codex" || pid != observation.process_id || Kernel.started(pid) != Some(observation.process_started) { return Ok(()) }
        if self.service.runtime().pane_id_of(to.id).ok().as_deref() != Some(observation.pane.as_str()) { return Ok(()) }
        let path = std::path::Path::new(&observation.path);
        if super::codex_turn::rollout_now(pid).await.as_deref() != Some(path)
            || crate::log_join::codex_thread_of(path).is_none_or(|(id, _)| id != observation.thread)
        { return Ok(()) }
        let Ok(mut file) = std::fs::File::open(path) else { return Ok(()) };
        if farcooler_core::session_log::codex_prompts::main_thread_in(&mut file).as_deref() != Some(observation.thread.as_str()) { return Ok(()) }
        let (Ok(device), Ok(inode), Ok(from)) = (u64::try_from(observation.device), u64::try_from(observation.inode), u64::try_from(observation.from)) else { return Ok(()) };
        let Some(matched) = exact_prompt_in_pinned_file(&mut file, device, inode, from, &observation.message, observation.admitted_at) else { return Ok(()) };
        // The path still names the same opened file after the read. A rename
        // or replacement cannot silently redirect the durable evidence.
        let Ok(path_meta) = std::fs::metadata(path) else { return Ok(()) };
        if path_meta.dev() != device || path_meta.ino() != inode { return Ok(()) }
        let (Some(message_at), Ok(line)) = (matched.prompt.at_ms, i64::try_from(matched.offset)) else { return Ok(()) };
        self.service.store.confirm_check_in_receipt(
            occurrence, attempt, to.id, &observation.thread, observation.device, observation.inode,
            line, message_at, super::now_millis(),
        )?;
        Ok(())
    }
}
