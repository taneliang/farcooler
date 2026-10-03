//! A connection's own push queue, bounded in bytes.
//!
//! What `Handler::pushes` hands `serve_connection`: the events addressed to one
//! connection alone, which today means an attached terminal's output. It used to
//! be an unbounded mpsc, on the reasoning that rule 4's watchdog was the ceiling.
//! It was not. The watchdog counts bytes already handed to `Connection::send`,
//! and a frame sitting in this queue had not been — so a phone that stopped
//! reading while a pane ran `yes` grew the daemon's memory at the pane's rate,
//! for as long as the link stayed up.
//!
//! So the queue counts its own bytes, and refuses a push that would take it past
//! its limit. What happens next is the sender's call, not this file's: the only
//! sender, `terminal.attach`, answers a refusal by throwing the queue away and,
//! once the connection is draining again, sending one fresh picture of the pane
//! in place of everything it dropped. Dropping terminal bytes is only safe
//! because of that second half — a run cut out of a byte stream is an escape
//! sequence cut in half, which the client cannot detect — and the queue provides
//! the two things that make it possible: `clear`, and `drained`, which says when
//! a picture would be read rather than queued behind a stall.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::Event;
use prost::Message;
use tokio::sync::Notify;

/// A new queue that holds at most `limit` bytes of encoded events — except that
/// an empty queue always takes one, however large, so a single picture bigger
/// than the limit can still be delivered.
pub fn push_queue(limit: usize) -> (PushSender, PushReceiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            bytes: 0,
            receiver_waiting: false,
            receiver_gone: false,
            senders: 1,
        }),
        to_receiver: Notify::new(),
        to_senders: Notify::new(),
        limit,
    });
    (PushSender { shared: shared.clone() }, PushReceiver { shared })
}

/// What became of one `PushSender::push`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushed {
    /// In the queue, behind whatever was already there.
    Queued,
    /// Not queued: it would have taken the queue past its limit. Nothing that
    /// was already queued was touched.
    Full,
    /// Not queued: nothing will ever read it.
    Closed,
}

struct Shared {
    state: Mutex<State>,
    /// Something was queued, or the last sender left.
    to_receiver: Notify,
    /// The receiver is waiting on an empty queue, or it left.
    to_senders: Notify,
    limit: usize,
}

struct State {
    queue: VecDeque<(Event, usize)>,
    /// The encoded size of everything in `queue`.
    bytes: usize,
    /// The receiver found the queue empty and is parked in `recv`. Which, from
    /// `serve_connection`, means the connection's writer is below its high-water
    /// mark: it only asks for a push when it has room to send one.
    receiver_waiting: bool,
    receiver_gone: bool,
    senders: usize,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while holding this lock leaves a queue and two counters, all
        // still consistent: every mutation below is a single step.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct PushSender {
    shared: Arc<Shared>,
}

impl Clone for PushSender {
    fn clone(&self) -> Self {
        self.shared.lock().senders += 1;
        Self { shared: self.shared.clone() }
    }
}

impl Drop for PushSender {
    fn drop(&mut self) {
        let last = {
            let mut state = self.shared.lock();
            state.senders -= 1;
            state.senders == 0
        };
        if last {
            self.shared.to_receiver.notify_one();
        }
    }
}

impl PushSender {
    /// Queue `event` if it fits. See `Pushed`.
    pub fn push(&self, event: Event) -> Pushed {
        let size = event.encoded_len();
        let mut state = self.shared.lock();
        if state.receiver_gone {
            return Pushed::Closed;
        }
        if !state.queue.is_empty() && state.bytes + size > self.shared.limit {
            return Pushed::Full;
        }
        state.bytes += size;
        state.queue.push_back((event, size));
        state.receiver_waiting = false;
        drop(state);
        self.shared.to_receiver.notify_one();
        Pushed::Queued
    }

    /// Replace whatever is queued with `events`, whatever their size.
    ///
    /// For the one sender that has just been refused and has something to put
    /// in place of what it gives up — a resync, which is worth more to the
    /// client than every byte it replaces. False when nothing will read it.
    pub fn replace(&self, events: Vec<Event>) -> bool {
        let mut state = self.shared.lock();
        if state.receiver_gone {
            return false;
        }
        state.queue.clear();
        state.bytes = 0;
        for event in events {
            let size = event.encoded_len();
            state.bytes += size;
            state.queue.push_back((event, size));
        }
        if !state.queue.is_empty() {
            state.receiver_waiting = false;
        }
        drop(state);
        self.shared.to_receiver.notify_one();
        true
    }

    /// Drop everything queued, returning what was dropped so the caller can
    /// account for it.
    pub fn clear(&self) -> Vec<Event> {
        let mut state = self.shared.lock();
        state.bytes = 0;
        state.queue.drain(..).map(|(event, _)| event).collect()
    }

    /// The encoded size of everything queued right now.
    pub fn queued_bytes(&self) -> usize {
        self.shared.lock().bytes
    }

    /// Resolves once the queue is empty AND the receiver is waiting for more —
    /// the moment something pushed would be sent rather than queued. False if
    /// the receiver is gone, and nothing pushed will ever be sent.
    ///
    /// Waiting for both, not for an empty queue alone, is the point: a queue
    /// that was just cleared is empty, and a connection whose client has
    /// stopped reading would still not send anything put in it.
    pub async fn drained(&self) -> bool {
        loop {
            let notified = self.shared.to_senders.notified();
            let mut notified = std::pin::pin!(notified);
            // Registered before the state is read, so a wake between the read
            // and the await is not lost.
            notified.as_mut().enable();
            {
                let state = self.shared.lock();
                if state.receiver_gone {
                    return false;
                }
                if state.queue.is_empty() && state.receiver_waiting {
                    return true;
                }
            }
            notified.await;
        }
    }
}

pub struct PushReceiver {
    shared: Arc<Shared>,
}

impl Drop for PushReceiver {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.receiver_gone = true;
        state.queue.clear();
        state.bytes = 0;
        drop(state);
        self.shared.to_senders.notify_waiters();
    }
}

impl PushReceiver {
    /// The next event, oldest first. `None` once every sender is gone and the
    /// queue is empty.
    pub async fn recv(&mut self) -> Option<Event> {
        loop {
            let notified = self.shared.to_receiver.notified();
            {
                let mut state = self.shared.lock();
                if let Some((event, size)) = state.queue.pop_front() {
                    state.bytes -= size;
                    return Some(event);
                }
                if state.senders == 0 {
                    return None;
                }
                if !state.receiver_waiting {
                    state.receiver_waiting = true;
                    drop(state);
                    self.shared.to_senders.notify_waiters();
                }
            }
            // `notify_one` stores a permit when nobody is waiting yet, so a push
            // that lands between the check above and this await still wakes it.
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::event::Payload;

    fn event(payload_bytes: usize) -> Event {
        Event {
            event_id: bytes::Bytes::from(vec![7u8; payload_bytes]),
            sequence: 0,
            payload: Some(Payload::FleetChanged(farcooler_protocol::v1::Empty {})),
        }
    }

    #[tokio::test]
    async fn a_push_past_the_limit_is_refused_and_the_queue_keeps_what_it_had() {
        let (tx, mut rx) = push_queue(1000);
        assert_eq!(tx.push(event(400)), Pushed::Queued);
        assert_eq!(tx.push(event(400)), Pushed::Queued);
        assert_eq!(tx.push(event(400)), Pushed::Full);
        assert!(tx.queued_bytes() <= 1000);
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn an_empty_queue_takes_one_event_of_any_size() {
        let (tx, _rx) = push_queue(10);
        assert_eq!(tx.push(event(500)), Pushed::Queued);
        assert_eq!(tx.push(event(1)), Pushed::Full);
    }

    #[tokio::test]
    async fn drained_waits_for_the_receiver_to_ask_not_just_for_an_empty_queue() {
        let (tx, mut rx) = push_queue(1000);
        tx.push(event(10));
        tx.clear();
        let early = tokio::time::timeout(std::time::Duration::from_millis(50), tx.drained()).await;
        assert!(early.is_err(), "nobody is reading yet, so a picture would only be queued");

        let reader = tokio::spawn(async move { rx.recv().await.map(|_| ()) });
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), tx.drained()).await.unwrap());
        tx.push(event(10));
        assert!(reader.await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_receiver_that_left_closes_the_queue() {
        let (tx, rx) = push_queue(1000);
        drop(rx);
        assert_eq!(tx.push(event(1)), Pushed::Closed);
        assert!(!tx.drained().await);
        assert!(!tx.replace(vec![event(1)]));
    }

    #[tokio::test]
    async fn the_last_sender_leaving_ends_the_stream() {
        let (tx, mut rx) = push_queue(1000);
        let second = tx.clone();
        tx.push(event(1));
        drop(tx);
        drop(second);
        assert!(rx.recv().await.is_some(), "what was queued still arrives");
        assert!(rx.recv().await.is_none());
    }
}
