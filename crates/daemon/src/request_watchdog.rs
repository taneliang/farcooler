//! The watchdog's thread and clock do not depend on any Tokio worker.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant};

const STALLED_AFTER: Duration = Duration::from_secs(10);
static ACTIVE: LazyLock<Mutex<HashMap<u64, Call>>> = LazyLock::new(Mutex::default);
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Call {
    method: &'static str,
    started: Instant,
    reported: bool,
}

pub(crate) struct Request(u64);

impl Request {
    pub(crate) fn begin(method: &'static str) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        ACTIVE.lock().unwrap().insert(id, Call { method, started: Instant::now(), reported: false });
        Self(id)
    }
}

impl Drop for Request {
    fn drop(&mut self) { ACTIVE.lock().unwrap().remove(&self.0); }
}

fn overdue(calls: &mut HashMap<u64, Call>, now: Instant) -> Vec<(&'static str, Duration)> {
    calls.values_mut().filter_map(|call| {
        let elapsed = now.saturating_duration_since(call.started);
        if elapsed < STALLED_AFTER || call.reported { return None; }
        call.reported = true;
        Some((call.method, elapsed))
    }).collect()
}

pub fn start() {
    let beat = Arc::new(AtomicU64::new(0));
    let heartbeat = beat.clone();
    tokio::spawn(async move {
        loop {
            heartbeat.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    if let Err(error) = std::thread::Builder::new().name("request-watchdog".into()).spawn(move || {
        let mut last = (0, Instant::now());
        let mut reported = false;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = Instant::now();
            let tick = beat.load(Ordering::Relaxed);
            if tick != last.0 {
                last = (tick, now);
                reported = false;
            }
            let stalled_for = now.saturating_duration_since(last.1);
            if stalled_for >= STALLED_AFTER && !reported {
                tracing::error!(elapsed_ms = stalled_for.as_millis(), "runner request executor stalled");
                reported = true;
            }
            let calls = overdue(&mut ACTIVE.lock().unwrap(), now);
            for (method, elapsed) in calls {
                tracing::warn!(method, elapsed_ms = elapsed.as_millis(), "runner request still in flight");
            }
        }
    }) {
        tracing::error!(%error, "cannot start runner request watchdog");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_calls_are_reported_once_with_their_method_and_elapsed_time() {
        let now = Instant::now();
        let mut calls = HashMap::from([(1, Call { method: "workflow.get", started: now, reported: false })]);
        assert!(overdue(&mut calls, now + Duration::from_secs(9)).is_empty());
        assert_eq!(overdue(&mut calls, now + Duration::from_secs(12)), [("workflow.get", Duration::from_secs(12))]);
        assert!(overdue(&mut calls, now + Duration::from_secs(13)).is_empty());
    }
}
