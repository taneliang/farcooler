//! Which connections this daemon is serving, and how to end the ones that
//! belong to a device.
//!
//! Only revocation needs this, and revocation is the reason it exists: sshd
//! reads `authorized_keys` at authentication and never again, so deleting a
//! device's line stops the NEXT login and does nothing whatsoever to the
//! session that device is holding right now. Something has to be able to find
//! that session, and a connection could not be attributed to a device at all
//! until `Peer` carried the forced command's `--client` the whole way here.
//!
//! Indexed by nothing. A `HashMap<client_id, …>` is the obvious shape and the
//! wrong one: a device may hold several connections at once (the control
//! channel, a stream, a relayed session), most connections belong to no device,
//! and revocation is a rare, human-driven ceremony. A linear sweep of a handful
//! of live connections costs nothing and cannot get its index out of step with
//! what is actually open.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::Scope;
use farcooler_transport::{Peer, SessionPreamble};

use farcooler_fence::scope_from_word;

/// What one connection is, as this runner's `authorized_keys` describes it.
///
/// `None` for a preamble is not "unknown": it is a caller that named no device,
/// which is every local socket client. See the call site in `main.rs` for why
/// those hold `host_admin`, and `Peer::client_id` for why they can never be the
/// target of a revocation.
///
/// `None` for the whole answer refuses the connection: a preamble that IS a
/// preamble but names a scope word this daemon does not have is a mistake — a
/// typo in a forced command — and a mistake must never be rounded up to a scope.
pub fn peer_from_preamble(preamble: Option<&SessionPreamble>) -> Option<Peer> {
    match preamble {
        None => Some(Peer { client_id: None, scope: Scope::HostAdmin }),
        Some(said) => Some(Peer {
            client_id: said.client.clone(),
            scope: scope_from_word(&said.scope)?,
        }),
    }
}

/// Every connection this daemon is currently serving.
///
/// One per `Service`, because that is the unit a runner's live state belongs
/// to. A `farcoolerd --stdio` process that could not reach a running daemon
/// opens its own `Service` and therefore its own registry, which is honest: it
/// can only close what it is serving, and it is serving exactly one session.
#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<u64, Live>>,
    /// Handed out in order and never reused, so a session that ends while a
    /// sweep is deciding cannot have its slot taken by a new one.
    next: AtomicU64,
}

struct Live {
    client_id: Option<String>,
    gate: Arc<tokio::sync::Semaphore>,
}

impl Sessions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a connection for as long as the returned handle is held.
    pub fn open(self: &Arc<Self>, client_id: Option<String>) -> Arc<Session> {
        // Zero permits: `acquire` on it can only ever finish by the semaphore
        // being CLOSED, which makes this a one-way switch rather than a counter.
        // Chosen over a `Notify` because it has no lost-wakeup edge — a waiter
        // that arrives after the close is failed immediately rather than parked
        // forever, so a connection cannot survive by being registered a moment
        // too late.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.live.lock().unwrap().insert(id, Live { client_id, gate: gate.clone() });
        Arc::new(Session { sessions: self.clone(), id, gate })
    }

    /// Close every connection this device is holding. Answers with how many.
    ///
    /// Synchronous, and finished when it returns: closing a gate is what makes
    /// the connection's next poll return, and no caller has to await anything
    /// for that to have been decided. Revocation depends on exactly this — it
    /// answers a person only after this call has come back.
    ///
    /// A connection with no client id is never matched, whatever is asked for.
    /// Those are the local callers, including the one doing the revoking.
    pub fn close(&self, client_id: &str) -> usize {
        let live = self.live.lock().unwrap();
        let mut closed = 0;
        for session in live.values() {
            if session.client_id.as_deref() == Some(client_id) {
                session.gate.close();
                closed += 1;
            }
        }
        closed
    }
}

/// One connection's registration, alive as long as the connection is.
///
/// Held by the connection's `Handler` and dropped with it, so nothing has to
/// remember to deregister on each of the ways a connection can end — a clean
/// close, a codec error, a peer that vanished, or this.
pub struct Session {
    sessions: Arc<Sessions>,
    id: u64,
    gate: Arc<tokio::sync::Semaphore>,
}

impl Session {
    /// Resolves once this session has been closed, and never otherwise.
    pub async fn closed(&self) {
        // The error IS the signal: `acquire` on a zero-permit semaphore can
        // finish no other way.
        let _ = self.gate.acquire().await;
    }

    /// Whether this session has already been closed.
    ///
    /// Read synchronously, which is what lets a test assert the ORDER of a
    /// revocation rather than merely its effect.
    pub fn is_closed(&self) -> bool {
        self.gate.is_closed()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // A poisoned lock would mean a panic inside `close`, which does nothing
        // that can panic. Ignored rather than propagated so a connection ending
        // never becomes a second panic during unwind.
        if let Ok(mut live) = self.sessions.live.lock() {
            live.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_a_device_leaves_a_local_session_open() {
        // The over-broad close, at the smallest scale it can be shown: a local
        // caller carries no client id, and the connection asking for the
        // revocation is usually one of them. Matching `None` would close the
        // Mac app's own connection every time somebody revoked a phone.
        let sessions = Sessions::new();
        let local = sessions.open(None);
        let phone = sessions.open(Some("phone".into()));

        assert_eq!(sessions.close("phone"), 1);
        assert!(phone.is_closed());
        assert!(!local.is_closed(), "a connection naming no device was closed by a device's name");
    }

    #[tokio::test]
    async fn every_connection_a_device_holds_is_closed() {
        // Not one per device: a phone holds a control connection and can hold a
        // relayed session beside it, and revoking a device that keeps one of
        // them is a revocation that did not happen.
        let sessions = Sessions::new();
        let first = sessions.open(Some("phone".into()));
        let second = sessions.open(Some("phone".into()));
        let other = sessions.open(Some("laptop".into()));

        assert_eq!(sessions.close("phone"), 2);
        assert!(first.is_closed() && second.is_closed());
        assert!(!other.is_closed(), "another device's session was closed");
    }

    #[tokio::test]
    async fn a_session_that_ended_is_no_longer_there_to_close() {
        let sessions = Sessions::new();
        drop(sessions.open(Some("phone".into())));
        assert_eq!(sessions.close("phone"), 0, "a connection that ended was counted as closed");
    }

    #[tokio::test]
    async fn closed_resolves_for_a_waiter_that_arrives_late() {
        // The lost-wakeup edge, asserted rather than argued: a connection may
        // start awaiting this after the close has already happened, and it must
        // not park forever waiting for an event that is over.
        let sessions = Sessions::new();
        let phone = sessions.open(Some("phone".into()));
        sessions.close("phone");
        tokio::time::timeout(std::time::Duration::from_secs(1), phone.closed())
            .await
            .expect("a waiter that arrived after the close never woke");
    }

    #[test]
    fn a_caller_with_no_preamble_is_a_local_host_admin() {
        let peer = peer_from_preamble(None).expect("a local caller is never refused");
        assert_eq!(peer.scope, Scope::HostAdmin);
        assert!(peer.client_id.is_none(), "a local caller named a device");
    }

    #[test]
    fn a_preamble_carries_its_device_all_the_way_through() {
        let peer = peer_from_preamble(Some(&SessionPreamble {
            scope: "read".into(),
            client: Some("phone".into()),
        }))
        .expect("a well-formed preamble");
        assert_eq!(peer.scope, Scope::Read);
        assert_eq!(peer.client_id.as_deref(), Some("phone"));
    }

    #[test]
    fn a_scope_word_this_daemon_does_not_have_refuses_the_connection() {
        // A typo in a forced command must not become privilege escalation on
        // the quietest possible path.
        assert!(
            peer_from_preamble(Some(&SessionPreamble {
                scope: "admin".into(),
                client: Some("phone".into()),
            }))
            .is_none()
        );
    }
}
