//! How often the sweep looks again at an effect it could not recover (ov-1301).
//!
//! An Uncertain effect whose receipt is missing or doesn't match stays Uncertain: the item waits in Needs You for the
//! orchestrator, who can resolve, send back or cancel it. Looking at it again every second changes nothing and cost the
//! Canary daemon 70 percent of a core and about 5,000 warnings an hour with eight such items, which starved the builds
//! it was running. So each one is looked at again after 5 seconds, then 10, 20 and so on up to 5 minutes, and its
//! reason is logged once. Anything the orchestrator does (a command, a resolve, a settle) wakes them all, so a
//! receipt it just wrote is read on the next sweep, not minutes later.
use std::collections::{BTreeMap, BTreeSet};

/// The first wait after a failed look, and the longest.
pub(super) const FIRST_MS: i64 = 5_000;
pub(super) const MOST_MS: i64 = 5 * 60_000;

struct Wait { next_ms: i64, delay_ms: i64, said: Option<String> }

/// Keyed by (workspace, effect).
#[derive(Default)]
pub(super) struct RecoveryBackoff {
    waits: BTreeMap<(String, String), Wait>,
    /// Set from the inbox, which can't take the runtime mutably: forget every wait at the next sweep.
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
        if self.wake.swap(false, std::sync::atomic::Ordering::SeqCst) { self.waits.clear(); }
        self.waits.get(&(workspace.to_string(), effect.to_string())).is_some_and(|w| w.next_ms > now_ms)
    }
    /// A recovery is about to be tried.
    pub(super) fn looking(&mut self) {
        #[cfg(test)] { self.looks += 1; }
    }
    /// The look settled nothing: wait twice as long as last time, and say why once (again only if the reason changes).
    pub(super) fn unsettled(&mut self, workspace: &str, effect: &str, why: Option<String>, now_ms: i64) {
        let wait = self.waits.entry((workspace.to_string(), effect.to_string()))
            .or_insert(Wait { next_ms: now_ms, delay_ms: 0, said: None });
        wait.delay_ms = wait.delay_ms.saturating_mul(2).clamp(FIRST_MS, MOST_MS);
        wait.next_ms = now_ms.saturating_add(wait.delay_ms);
        let Some(why) = why else { return };
        if wait.said.as_ref() == Some(&why) {
            tracing::debug!(%workspace, %effect, error = %why, retry_in_s = wait.delay_ms / 1000, "recovery receipt still invalid");
            return;
        }
        tracing::warn!(%workspace, %effect, error = %why, retry_in_s = wait.delay_ms / 1000,
            "invalid recovery receipt; the effect stays Uncertain for the orchestrator to resolve, send back or cancel, and is looked at less often");
        wait.said = Some(why);
        #[cfg(test)] { self.warnings += 1; }
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
