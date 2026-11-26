//! How often the sweep looks again at an effect it could not recover (ov-1301).
//!
//! The sweep waits five seconds before its first active probe. If recovery still cannot settle the effect,
//! it waits ten seconds, then twenty and so on up to five minutes. A command can wake a receipt read early
//! without advancing the active probe's deadline.
use std::collections::{BTreeMap, BTreeSet};

/// The first wait after a failed look, and the longest.
pub(super) const FIRST_MS: i64 = 5_000;
pub(super) const MOST_MS: i64 = 5 * 60_000;

struct Wait {
    next_ms: i64,
    delay_ms: i64,
    woken: bool,
    said: Option<String>,
    leftovers: Option<crate::engine_stuck::Leftovers>,
}

/// Keyed by (workspace, effect).
#[derive(Default)]
pub(super) struct RecoveryBackoff {
    waits: BTreeMap<(String, String), Wait>,
    /// Set from the inbox, which can't take the runtime mutably: make every wait due at the next sweep.
    wake: std::sync::atomic::AtomicBool,
    /// Test counters: recoveries attempted, and warnings logged.
    #[cfg(test)]
    pub(super) looks: usize,
    #[cfg(test)]
    pub(super) warnings: usize,
}

impl RecoveryBackoff {
    /// Whether `effect` is still waiting at `now_ms`. Taken first, so a waiting effect costs no state read.
    pub(super) fn waiting(&mut self, workspace: &str, effect: &str, now_ms: i64) -> bool {
        if self.wake.swap(false, std::sync::atomic::Ordering::SeqCst) {
            for wait in self.waits.values_mut() { wait.woken = true; }
        }
        self.waits.get_mut(&(workspace.to_string(), effect.to_string()))
            .is_some_and(|w| !std::mem::take(&mut w.woken) && w.next_ms > now_ms)
    }
    /// A wake may look for a new receipt early. It cannot skip the grace period for an active probe.
    pub(super) fn automatic_due(&self, workspace: &str, effect: &str, now_ms: i64) -> bool {
        self.waits.get(&(workspace.to_string(), effect.to_string())).is_some_and(|w| now_ms >= w.next_ms)
    }
    /// A recovery is about to be tried.
    pub(super) fn looking(&mut self) {
        #[cfg(test)] { self.looks += 1; }
    }
    /// The look settled nothing: wait twice as long as last time, and say why once (again only if the reason changes).
    pub(super) fn unsettled(&mut self, workspace: &str, effect: &str, why: Option<String>, now_ms: i64) {
        let wait = self.waits.entry((workspace.to_string(), effect.to_string()))
            .or_insert(Wait { next_ms: now_ms, delay_ms: 0, woken: false, said: None, leftovers: None });
        wait.leftovers = None;
        if now_ms >= wait.next_ms {
            wait.delay_ms = wait.delay_ms.saturating_mul(2).clamp(FIRST_MS, MOST_MS);
            wait.next_ms = now_ms.saturating_add(wait.delay_ms);
        }
        let Some(why) = why else { return };
        if wait.said.as_ref() == Some(&why) {
            tracing::debug!(%workspace, %effect, error = %why, retry_in_s = wait.delay_ms / 1000, "recovery receipt still invalid");
            return;
        }
        tracing::warn!(%workspace, %effect, error = %why, retry_in_s = wait.delay_ms / 1000,
            "workflow recovery inconclusive; will check again after backoff");
        wait.said = Some(why);
        #[cfg(test)] { self.warnings += 1; }
    }
    /// A known hold wait is checked promptly without turning an admitted
    /// effect into an uncertain one or blocking the runner's inbox.
    pub(super) fn wait_for_leftovers(&mut self, workspace: &str, effect: &farcooler_workflow::Effect,
        reason: String, now_ms: i64) {
        let wait = self.waits.entry((workspace.into(), effect.id.clone()))
            .or_insert(Wait { next_ms: now_ms, delay_ms: 0, woken: false, said: None, leftovers: None });
        wait.leftovers.get_or_insert_with(|| crate::engine_stuck::Leftovers::new(workspace, effect)).report(Some(reason));
        wait.next_ms = now_ms.saturating_add(FIRST_MS);
        wait.delay_ms = 0;
    }
    /// The look settled it, or it left the outbox.
    pub(super) fn settled(&mut self, workspace: &str, effect: &str) {
        self.waits.remove(&(workspace.to_string(), effect.to_string()));
    }
    /// Drop waits for effects of `workspace` no longer uncertain, so the map holds only live ones.
    pub(super) fn keep_only(&mut self, workspace: &str, uncertain: &BTreeSet<String>) {
        self.waits.retain(|(board, effect), _| board != workspace || uncertain.contains(effect));
    }
    /// Look at every waiting effect at the next sweep.
    pub(super) fn wake(&self) { self.wake.store(true, std::sync::atomic::Ordering::SeqCst); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_failed_look_waits_longer_each_time_up_to_five_minutes_and_says_why_once() {
        let mut backoff = RecoveryBackoff::default();
        let mut now = 0;
        let mut delays = Vec::new();
        for _ in 0..12 {
            assert!(!backoff.waiting("w", "e", now));
            backoff.unsettled("w", "e", Some("missing".into()), now);
            let next = backoff.waits[&("w".into(), "e".into())].next_ms;
            assert!(backoff.waiting("w", "e", next - 1));
            delays.push(next - now);
            now = next;
        }
        assert_eq!(&delays[..4], &[5_000, 10_000, 20_000, 40_000]);
        assert_eq!(*delays.last().unwrap(), MOST_MS);
        assert_eq!(backoff.warnings, 1, "one reason, said once");
        backoff.unsettled("w", "e", Some("mismatch".into()), now);
        assert_eq!(backoff.warnings, 2, "a new reason is said");
        backoff.wake();
        assert!(!backoff.waiting("w", "e", now), "the orchestrator's word wakes every wait");
        backoff.unsettled("w", "e", None, now);
        backoff.keep_only("w", &BTreeSet::new());
        assert!(!backoff.waiting("w", "e", now));
    }
}
