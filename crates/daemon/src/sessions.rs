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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Devices revoked, until each is enrolled again (ov-820 review B1),
    /// kept in the store across a restart when [`Sessions::persisted`]. A connection one of them opens later, a new
    /// channel on an ssh transport the revoke hasn't severed yet, say while a
    /// tunneled device's farewell is let through, starts closed: it is told
    /// its farewell and served nothing.
    revoked: Mutex<std::collections::HashSet<String>>,
    /// How many times each device has been marked revoked since this daemon
    /// started, never lowered, not even by `readmit` (ov-826). A session
    /// keeps the count it opened under, so a request it was already running
    /// can tell that its device was revoked after it began, even once the
    /// device is enrolled again. One number per revoked id.
    revocations: Mutex<HashMap<String, u64>>,
    /// Handed out in order and never reused, so a session that ends while a
    /// sweep is deciding cannot have its slot taken by a new one.
    next: AtomicU64,
    /// Where `revoked` is kept across a restart (ov-832, the ov-826 review's
    /// S2), or nowhere for a bare registry in a test.
    kept: Option<Arc<farcooler_store::Store>>,
}

struct Live {
    client_id: Option<String>,
    gate: Arc<tokio::sync::Semaphore>,
    /// Closed once the connection has said its farewell (ov-820), or ended.
    said: Arc<tokio::sync::Semaphore>,
    /// Closed because its person's access changed, not because the device was
    /// revoked: it says no `device_released` and just reconnects (ov-856).
    quiet: Arc<AtomicBool>,
}

impl Sessions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A registry whose revoked marks outlive the daemon (ov-832): loaded
    /// from `store` now, each new one written there, and each lifted one
    /// taken out. Without this a restart forgot every mark, so a revoked
    /// Mac's still-open ssh master could open a fresh, unmarked session and
    /// enroll its own line back. A store that can't be read refuses to open.
    pub fn persisted(store: Arc<farcooler_store::Store>) -> farcooler_core::Result<Arc<Self>> {
        let marked = store.revoked_marks()?;
        Ok(Arc::new(Self { revoked: Mutex::new(marked.into_iter().collect()), kept: Some(store), ..Self::default() }))
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
        let said = Arc::new(tokio::sync::Semaphore::new(0));
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        // Checked and registered under `live`'s lock, the one `closing` holds
        // while it records the device, so no connection slips between them.
        let mut live = self.live.lock().unwrap();
        if client_id.as_ref().is_some_and(|id| self.revoked.lock().unwrap().contains(id)) {
            gate.close();
        }
        let revocations_at_open = client_id.as_deref().map_or(0, |id| self.revocations_of(id));
        let quiet = Arc::new(AtomicBool::new(false));
        live.insert(id, Live { client_id: client_id.clone(), gate: gate.clone(), said: said.clone(), quiet: quiet.clone() });
        drop(live);
        Arc::new(Session { sessions: self.clone(), id, client_id, revocations_at_open, gate, said, quiet })
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
        self.closing(client_id).count()
    }

    /// Close every connection this device is holding because its person's
    /// access changed (ov-856, `access_changed`): no farewell, since the device
    /// isn't released, and it reconnects at the scope its line now carries.
    /// Answers with how many. Marks nothing revoked.
    pub fn reconnect(&self, client_id: &str) -> usize {
        let live = self.live.lock().unwrap();
        let mut closed = 0;
        for session in live.values().filter(|s| s.client_id.as_deref() == Some(client_id)) {
            session.quiet.store(true, Ordering::SeqCst);
            session.said.close();
            session.gate.close();
            closed += 1;
        }
        closed
    }

    /// [`close`](Self::close), keeping what it takes to wait for each closed
    /// connection's farewell (ov-820). A revocation that is about to tear
    /// down a tunneled device's route waits on it first, so the
    /// `device_released` frame isn't cut off on its way.
    ///
    /// It closes and marks nothing (ov-832, the ov-826 review's N2): every
    /// caller has already marked the device with
    /// [`mark_revoked`](Self::mark_revoked), so a connection it opens from
    /// then on starts closed (review B1). Marking here too, after the
    /// revoke's write and outside the fence lock, could land just after a
    /// real re-enrollment and shut that device out, now past a restart.
    pub fn closing(&self, client_id: &str) -> Closing {
        let live = self.live.lock().unwrap();
        let mut said = Vec::new();
        for session in live.values() {
            if session.client_id.as_deref() == Some(client_id) {
                session.gate.close();
                said.push(session.said.clone());
            }
        }
        Closing { said }
    }
}

impl Sessions {
    /// This device is revoked: every connection it opens from now on starts
    /// closed, until [`readmit`](Self::readmit). Called by a revoke under the
    /// fence lock, before its write lands (ov-820 review S4).
    ///
    /// Kept in the store too (ov-832), so a restart doesn't forget it. A
    /// store that can't be written is logged and the revoke goes on: taking
    /// the line out matters more than remembering the mark.
    pub fn mark_revoked(&self, client_id: &str) {
        {
            let _live = self.live.lock().unwrap();
            self.mark(client_id);
        }
        if let Some(store) = &self.kept
            && let Err(e) = store.keep_revoked_mark(client_id)
        {
            tracing::error!(client = %client_id, error = %e.what(), "a revoked mark wasn't kept; a restart would forget it");
        }
    }

    /// The mark, and one more revocation counted. With `live` held, so the
    /// locks are always taken `live`, `revoked`, `revocations`.
    fn mark(&self, client_id: &str) {
        self.revoked.lock().unwrap().insert(client_id.to_string());
        *self.revocations.lock().unwrap().entry(client_id.to_string()).or_default() += 1;
    }

    /// Whether `client_id` is marked revoked now, and not enrolled again since.
    pub fn is_marked(&self, client_id: &str) -> bool {
        self.revoked.lock().unwrap().contains(client_id)
    }

    fn revocations_of(&self, client_id: &str) -> u64 {
        self.revocations.lock().unwrap().get(client_id).copied().unwrap_or(0)
    }

    /// This device was enrolled again: its connections are served again.
    /// Only for a real enrollment write of this id, under the fence lock.
    ///
    /// The kept mark goes first (ov-832): when the store can't take it out,
    /// this fails and the device stays marked, and the enroll writes nothing.
    pub fn readmit(&self, client_id: &str) -> farcooler_core::Result<()> {
        if let Some(store) = &self.kept {
            store.lift_revoked_mark(client_id)?;
        }
        self.revoked.lock().unwrap().remove(client_id);
        Ok(())
    }
}

/// What a closed connection says last (ov-820): `device_released`.
///
/// A session is closed only by revocation ([`Sessions::close`]), so this is
/// said only to a device whose access was just taken away, on its own
/// connection, which already knows who it is: it tells nobody anything they
/// couldn't read before.
pub fn farewell() -> farcooler_protocol::v1::Event {
    farcooler_protocol::v1::Event {
        event_id: bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()),
        sequence: 0,
        payload: Some(farcooler_protocol::v1::event::Payload::DeviceReleased(farcooler_protocol::v1::Empty {})),
    }
}

/// Connections [`Sessions::closing`] closed, whose farewells can be waited on.
#[must_use = "a closing that nobody waits on is `Sessions::close`"]
pub struct Closing {
    said: Vec<Arc<tokio::sync::Semaphore>>,
}

impl Closing {
    /// How many connections were closed.
    pub fn count(&self) -> usize {
        self.said.len()
    }

    /// Wait until every closed connection has said its farewell or ended,
    /// for `within` at most. Answers whether they all had.
    pub async fn said_within(self, within: std::time::Duration) -> bool {
        let all = async {
            for said in &self.said {
                // Closed is the only way it finishes, as with `gate`.
                let _ = said.acquire().await;
            }
        };
        tokio::time::timeout(within, all).await.is_ok()
    }

    /// Several devices' closings as one, waited on together.
    pub fn and(mut self, other: Closing) -> Closing {
        self.said.extend(other.said);
        self
    }

    /// Nothing closed yet: what [`and`](Self::and) starts from.
    pub fn none() -> Closing {
        Closing { said: Vec::new() }
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
    client_id: Option<String>,
    /// [`Sessions::revocations`] for this device when the session opened.
    revocations_at_open: u64,
    gate: Arc<tokio::sync::Semaphore>,
    said: Arc<tokio::sync::Semaphore>,
    quiet: Arc<AtomicBool>,
}

impl Session {
    /// Whether this session was closed for an access change, so it says no
    /// farewell: the device is still welcome and reconnects at its new scope.
    pub fn closed_quietly(&self) -> bool {
        self.quiet.load(Ordering::SeqCst)
    }

    /// Resolves once this session has been closed, and never otherwise.
    pub async fn closed(&self) {
        // The error IS the signal: `acquire` on a zero-permit semaphore can
        // finish no other way.
        let _ = self.gate.acquire().await;
    }

    /// The device this session's line names, or `None` for a local caller.
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    /// Whether this session has already been closed.
    ///
    /// Read synchronously, which is what lets a test assert the ORDER of a
    /// revocation rather than merely its effect.
    pub fn is_closed(&self) -> bool {
        self.gate.is_closed()
    }

    /// This connection has said its farewell, or couldn't (ov-820): what
    /// [`Closing::said_within`] waits for.
    pub fn said_farewell(&self) {
        self.said.close();
    }

    /// Whether this session's device has been revoked since the session
    /// opened, or is marked revoked now (ov-826). Never for a session that
    /// names no device.
    ///
    /// What a request the session was already running asks before it writes:
    /// a revoke closes the session only after its own write lands, so the
    /// closed gate alone can't stop an enroll that was waiting on the fence
    /// lock. The mark and the count are both set under that lock. The count
    /// is there for a device revoked and then enrolled again before the
    /// request gets its turn: `readmit` lifts the mark, never the count.
    pub fn revoked_since_open(&self) -> bool {
        let Some(id) = self.client_id.as_deref() else { return false };
        self.sessions.revoked.lock().unwrap().contains(id) || self.sessions.revocations_of(id) != self.revocations_at_open
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // An ended connection has nothing left to say, so nobody waits on it.
        self.said.close();
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

    #[tokio::test]
    async fn a_closing_waits_for_each_farewell_or_end() {
        // What a revocation waits on before it tears down a tunneled
        // device's route (ov-820): every closed connection's farewell, or
        // its end, and no longer than it is told.
        let sessions = Sessions::new();
        let control = sessions.open(Some("phone".into()));
        let events = sessions.open(Some("phone".into()));
        let short = std::time::Duration::from_millis(50);

        let closing = sessions.closing("phone");
        assert_eq!(closing.count(), 2);
        control.said_farewell();
        assert!(!closing.said_within(short).await, "one connection had said nothing and it didn't wait");

        let closing = sessions.closing("phone");
        drop(events);
        assert!(closing.said_within(short).await, "an ended connection and a said farewell were waited on still");
        drop(control);
        assert_eq!(sessions.closing("phone").count(), 0);
    }

    #[tokio::test]
    async fn a_marked_device_opens_closed_until_it_is_readmitted() {
        // What a revoke records under the fence lock, and a written
        // enrollment lifts (ov-820 review S4).
        let sessions = Sessions::new();
        sessions.mark_revoked("phone");
        assert!(sessions.open(Some("phone".into())).is_closed(), "a revoked device's connection was served");
        assert!(!sessions.open(Some("laptop".into())).is_closed(), "another device was shut out");
        sessions.readmit("phone").unwrap();
        assert!(!sessions.open(Some("phone".into())).is_closed(), "a readmitted device stayed shut out");
    }

    #[tokio::test]
    async fn a_session_knows_its_device_was_revoked_after_it_opened() {
        // What an enroll in flight asks under the fence lock (ov-826): the
        // mark while it stands, and the count once a readmit has lifted it.
        let sessions = Sessions::new();
        let before = sessions.open(Some("phone".into()));
        let local = sessions.open(None);
        assert!(!before.revoked_since_open(), "a device nobody revoked reads as revoked");
        sessions.mark_revoked("phone");
        assert!(before.revoked_since_open(), "the mark went unseen");
        sessions.readmit("phone").unwrap();
        assert!(before.revoked_since_open(), "a readmit made an older session's revoke unhappen");
        assert!(!sessions.open(Some("phone".into())).revoked_since_open(), "a session opened after the readmit reads as revoked");
        assert!(!local.revoked_since_open(), "a local session was matched by a device's revoke");
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
