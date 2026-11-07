//! Admission and join for the daemon's own writer owners. Every spawned writer
//! (landing job, line job, delivery pass, hook callback, scheduler pass) holds a
//! `Ticket` for its whole life, through its final effect and durable
//! disposition. `close_and_join` closes admission first, then waits for every
//! ticket; a ticket dropped without `finish` (panic, cancelled future) makes the
//! join fail, so a dropped future can never become quiescence. The state lock is
//! never held across an await, and nothing here touches the Store mutex.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct State { closed: bool, active: BTreeMap<&'static str, usize>, abandoned: BTreeMap<&'static str, usize> }
#[derive(Default)]
pub struct WriterGate { state: Mutex<State>, changed: Notify }

#[derive(Debug, PartialEq, Eq)]
pub struct Closed(pub &'static str);

/// Held by one admitted writer. Not Clone; moves into the spawned task.
pub struct Ticket { gate: Arc<WriterGate>, identity: &'static str, finished: bool }
impl Ticket {
    /// The writer reached its final durable disposition.
    pub fn finish(mut self) { self.finished = true; }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        let mut s = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = s.active.get_mut(self.identity) { *n -= 1; if *n == 0 { s.active.remove(self.identity); } }
        if !self.finished { *s.abandoned.entry(self.identity).or_default() += 1; }
        drop(s);
        self.gate.changed.notify_waiters();
    }
}

impl WriterGate {
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }
    pub fn admit(self: &Arc<Self>, identity: &'static str) -> Result<Ticket, Closed> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.closed { return Err(Closed(identity)); }
        *s.active.entry(identity).or_default() += 1;
        Ok(Ticket { gate: self.clone(), identity, finished: false })
    }
    pub fn is_closed(&self) -> bool { self.state.lock().unwrap_or_else(|e| e.into_inner()).closed }
    /// Close admission, then wait up to `limit` for every ticket. Ok carries a
    /// receipt per identity that was ever admitted-and-finished or idle; Err
    /// names what is still running or was abandoned.
    pub async fn close_and_join(&self, limit: Duration) -> Result<(), String> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((who, _)) = s.abandoned.iter().next() { return Err(format!("{who} writer dropped without finishing")); }
                if s.active.is_empty() { return Ok(()); }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!("still running: {}", s.active.keys().copied().collect::<Vec<_>>().join(",")));
                }
            }
            let _ = tokio::time::timeout_at(deadline, notified).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn join_waits_for_a_finished_ticket_and_refuses_new_admission() {
        let gate = WriterGate::new();
        let ticket = gate.admit("landing-jobs").unwrap();
        let g = gate.clone();
        let join = tokio::spawn(async move { g.close_and_join(Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(gate.admit("landing-jobs").err(), Some(Closed("landing-jobs")));
        assert!(!join.is_finished(), "an admitted writer holds the join open");
        ticket.finish();
        assert!(join.await.unwrap().is_ok());
    }
    #[tokio::test]
    async fn a_dropped_or_stuck_writer_never_becomes_quiescence() {
        let gate = WriterGate::new();
        drop(gate.admit("delivery").unwrap());
        assert!(gate.close_and_join(Duration::from_secs(1)).await.unwrap_err().contains("dropped"));
        let gate = WriterGate::new();
        let _held = gate.admit("line-jobs").unwrap();
        assert!(gate.close_and_join(Duration::from_millis(50)).await.unwrap_err().contains("line-jobs"));
    }
}
