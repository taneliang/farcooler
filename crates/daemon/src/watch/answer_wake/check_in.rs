//! The dedicated hourly check-in pass. It never enters the ordinary wake
//! queue, and a submitted occurrence is only observed, never sent again.

use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::watch::Watcher;

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Destination {
    pub pane: String,
    pub tty: String,
    pub pid: i32,
    pub incarnation: i64,
    pub path: std::path::PathBuf,
}

#[cfg(unix)]
async fn scheduled_foreground(tty: &str) -> Option<(&'static str, i32)> {
    let name = tty.strip_prefix("/dev/").unwrap_or(tty);
    let listing = super::codex_turn::discovery_output("ps", &["-t", name, "-o", "pid=,ppid=,stat=,comm="]).await?;
    let (pid, comm) = super::in_front(&listing)?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or(comm);
    let args = super::codex_turn::discovery_output("ps", &["-o", "args=", "-p", &pid.to_string()]).await?;
    super::agent_of_process(&exe, args.trim()).map(|agent| (agent, pid))
}

#[cfg(unix)]
fn unique_pane(snapshot: &farcooler_core::inventory::RuntimeSnapshot, terminal: Uuid) -> Option<&farcooler_core::inventory::TaggedPane> {
    if !snapshot.inventory_healthy { return None }
    let claimants = snapshot.claimants(terminal);
    let [pane] = claimants.as_slice() else { return None };
    pane.proves_life().then_some(*pane)
}

/// Unexpected scheduler termination must never leave its volatile arm set.
struct SchedulerArmGuard<'a>(&'a std::sync::atomic::AtomicU8);
impl Drop for SchedulerArmGuard<'_> {
    fn drop(&mut self) { self.0.store(2, std::sync::atomic::Ordering::SeqCst); }
}

impl Watcher {
    /// A snapshot proof only: callers recheck it at each side-effect boundary.
    #[cfg(unix)]
    pub(super) async fn check_in_destination(&self, to: &farcooler_store::models::Terminal) -> Option<Destination> {
        use std::os::unix::fs::MetadataExt;
        use crate::claude_registry::{Kernel, Processes};
        if super::codex_turn::discovery_reap_failed() { return None }
        let inventory = self.service.inventory_snapshot();
        let pane = unique_pane(&inventory, to.id)?;
        let (preset, pid) = scheduled_foreground(&pane.tty).await?;
        if preset != "codex" || Kernel.tty(pid)? != std::fs::metadata(&pane.tty).ok()?.rdev() { return None }
        let incarnation = crate::claude_registry::process_incarnation(pid)?;
        let path = super::codex_turn::unique_rollout_now(pid).await?;
        // Discovery itself may have crossed a process replacement.
        if crate::claude_registry::process_incarnation(pid) != Some(incarnation)
            || scheduled_foreground(&pane.tty).await != Some(("codex", pid)) { return None }
        let fresh = self.service.inventory_snapshot();
        let fresh_pane = unique_pane(&fresh, to.id)?;
        if fresh_pane.pane_id != pane.pane_id || fresh_pane.tty != pane.tty { return None }
        if crate::claude_registry::process_incarnation(pid) != Some(incarnation) { return None }
        Some(Destination { pane: pane.pane_id.clone(), tty: pane.tty.clone(), pid, incarnation, path })
    }
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
        let _arm_guard = SchedulerArmGuard(&self.check_in_arm);
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
            if super::codex_turn::discovery_reap_failed() {
                self.check_in_arm.store(2, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _gate = self.check_in_cleanup_gate.lock().await;
        if super::codex_turn::discovery_reap_failed() {
            self.check_in_arm.store(2, std::sync::atomic::Ordering::SeqCst);
        }
        if self.check_in_arm.swap(2, std::sync::atomic::Ordering::SeqCst) == 1
            && !matches!(self.service.store.close_check_in_instance(daemon), Ok(true))
        { tracing::warn!("scheduled check-in instance remains unclean"); }
    }

    #[cfg(unix)]
    pub(super) async fn observe_check_in(&self, occurrence: Uuid, attempt: Uuid) -> farcooler_core::Result<()> {
        use std::os::unix::fs::MetadataExt;
        use farcooler_core::session_log::codex_prompts::exact_prompt_in_pinned_file;

        let Some(observation) = self.service.store.check_in_observation(occurrence, attempt)? else { return Ok(()) };
        let Some(to) = self.service.live_orchestrator(observation.workspace)? else { return Ok(()) };
        if to.id != observation.terminal || to.agent_session_id.as_deref() != Some(observation.thread.as_str())
            || i64::try_from(to.lease_generation).ok() != Some(observation.terminal_lease)
            || i64::try_from(to.epoch).ok() != Some(observation.terminal_epoch)
        { return Ok(()) }
        let Some(destination) = self.check_in_destination(&to).await else { return Ok(()) };
        if destination.pid != observation.process_id || destination.incarnation != observation.process_started
            || destination.pane != observation.pane || destination.tty != observation.tty { return Ok(()) }
        let path = std::path::Path::new(&observation.path);
        if destination.path != path
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
        if self.check_in_destination(&to).await.as_ref() != Some(&destination) { return Ok(()) }
        let Ok(final_meta) = std::fs::metadata(path) else { return Ok(()) };
        if final_meta.dev() != device || final_meta.ino() != inode { return Ok(()) }
        let (Some(message_at), Ok(line)) = (matched.prompt.at_ms, i64::try_from(matched.offset)) else { return Ok(()) };
        self.service.store.confirm_check_in_receipt(
            occurrence, attempt, to.id, &observation.thread, observation.device, observation.inode,
            line, message_at, super::now_millis(),
        )?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod ownership_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn scheduler_rejects_duplicate_dead_or_unhealthy_pane_claims() {
        use farcooler_core::inventory::{RuntimeSnapshot, ScreenStamp, TaggedPane};
        let id = Uuid::now_v7();
        let pane = TaggedPane {
            daemon_id: Uuid::nil(), worktree_id: Uuid::nil(), terminal_id: id, schema_version: 1,
            pane_id: "%1".into(), window_id: "@1".into(), columns: 80, rows: 24, left: 0, top: 0,
            window_active: true, pane_active: true, zoomed: false, tty: "/dev/tty1".into(),
            dead: false, dead_status: None, dead_signal: None, command: "codex".into(), title: String::new(),
            stamp: ScreenStamp::default(),
        };
        let one = RuntimeSnapshot::healthy(vec![pane.clone()]);
        assert_eq!(unique_pane(&one, id).unwrap().pane_id, "%1");
        let mut duplicate = pane.clone();
        duplicate.pane_id = "%2".into();
        assert!(unique_pane(&RuntimeSnapshot::healthy(vec![pane.clone(), duplicate]), id).is_none());
        let mut dead = pane;
        dead.dead = true;
        assert!(unique_pane(&RuntimeSnapshot::healthy(vec![dead]), id).is_none());
        assert!(unique_pane(&RuntimeSnapshot::unavailable(), id).is_none());
    }

    #[test]
    fn destination_binding_changes_for_each_incarnation_pane_tty_and_thread() {
        let original = Destination { pane: "%1".into(), tty: "/dev/tty1".into(), pid: 42,
            incarnation: 10_000_001, path: "/sessions/rollout-a.jsonl".into() };
        let mut changed = original.clone();
        changed.incarnation += 1; // Same second, distinct kernel start.
        assert_ne!(original, changed);
        changed = original.clone(); changed.pane = "%2".into(); assert_ne!(original, changed);
        changed = original.clone(); changed.tty = "/dev/tty2".into(); assert_ne!(original, changed);
        changed = original.clone(); changed.path = "/sessions/rollout-b.jsonl".into(); assert_ne!(original, changed);
        changed = original.clone(); changed.pid += 1; assert_ne!(original, changed);
    }

    #[tokio::test]
    async fn scheduler_arms_only_after_cleanup_gate_and_joins_on_shutdown() {
        let (_dir, service, _) = crate::test_support::fixture().await;
        let watcher = Watcher::new(service.clone());
        let gate = watcher.check_in_cleanup_gate.lock().await;
        let instance = Uuid::now_v7();
        let (stop, rx) = tokio::sync::watch::channel(false);
        let owner = tokio::spawn(watcher.clone().run_check_in_scheduler(instance, rx));
        tokio::task::yield_now().await;
        assert_eq!(watcher.check_in_arm.load(Ordering::SeqCst), 0);
        // If the owner had already armed, this competing arm would fail.
        // Do not actually arm here: the gate is the ordering under test.
        drop(gate);
        tokio::time::timeout(Duration::from_secs(5), async {
            while watcher.check_in_arm.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(watcher.check_in_arm.load(Ordering::SeqCst), 1);
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), owner).await.unwrap().unwrap();
        assert_eq!(watcher.check_in_arm.load(Ordering::SeqCst), 2);
        assert!(service.store.arm_check_in_instance(Uuid::now_v7()).unwrap(), "joined clean instance permits the next arm");
    }
}
