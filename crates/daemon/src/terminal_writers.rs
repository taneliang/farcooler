//! Terminal input leases shared by connections from one app instance.
//! The same per-pane lock covers the lease check and delivery, so a takeover
//! cannot overtake an admitted write.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use farcooler_core::{DomainError, Result, preconditions::check_input_lease};
use farcooler_protocol::v1::{
    Event, TerminalWriterChanged, TerminalWriterLost, TerminalWriterState, event, terminal_writer_request::Action,
};
use farcooler_transport::PushSender;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::sessions::Session;

#[derive(Default)]
pub(crate) struct TerminalWriters {
    terminals: Mutex<HashMap<Uuid, Arc<AsyncMutex<Slot>>>>,
    next_generation: AtomicU64,
}

pub(crate) struct WriteGuard {
    lease: OwnedMutexGuard<Slot>,
}

#[derive(Default)]
struct Slot {
    writer: Option<Writer>,
    watching: Vec<WriterConnection>,
}

impl Slot {
    fn prune(&mut self, id: Uuid) {
        self.watching.retain(|watcher| watcher.live_session().is_some());
        if self.writer.as_mut().is_some_and(|writer| !writer.keep_live_connections()) {
            self.writer = None;
            self.changed(id, None);
        }
    }

    fn changed(&self, id: Uuid, except: Option<&Arc<Session>>) {
        let event = Event {
            payload: Some(event::Payload::TerminalWriterChanged(TerminalWriterChanged {
                terminal_id: bytes::Bytes::copy_from_slice(id.as_bytes()),
            })),
            ..Default::default()
        };
        for watcher in &self.watching {
            if let Some(session) = watcher.live_session()
                && !except.is_some_and(|except| Arc::ptr_eq(except, &session))
            {
                watcher.push.push(event.clone());
            }
        }
    }
}

struct Writer {
    connections: Vec<WriterConnection>,
    generation: u64,
}

struct WriterConnection {
    session: Weak<Session>,
    push: PushSender,
}

impl WriterConnection {
    fn live_session(&self) -> Option<Arc<Session>> {
        self.session
            .upgrade()
            .filter(|session| !session.is_closed())
    }
}

impl TerminalWriters {
    fn slot(&self, id: Uuid) -> Arc<AsyncMutex<Slot>> {
        let mut terminals = self.terminals.lock().unwrap();
        if let Some(slot) = terminals.get(&id) {
            return slot.clone();
        }
        // Keep occupied locks and locks used by an in-flight call. An idle
        // slot can be replaced because generations are unique across slots.
        terminals.retain(|_, slot| {
            Arc::strong_count(slot) > 1
                || slot.try_lock().map_or(true, |mut slot| {
                    !slot.watching.is_empty() || slot.writer.as_mut().is_some_and(Writer::keep_live_connections)
                })
        });
        terminals.entry(id).or_default().clone()
    }

    /// Run when a participating connection closes, even if nobody reads or
    /// writes the pane again. Each pane is reconciled under its input lock.
    pub(crate) async fn prune_closed(&self, ids: std::collections::HashSet<Uuid>) {
        let slots: Vec<_> = {
            let terminals = self.terminals.lock().unwrap();
            ids.into_iter().filter_map(|id| Some((id, terminals.get(&id)?.clone()))).collect()
        };
        for (id, slot) in slots {
            slot.lock().await.prune(id);
        }
    }

    async fn current(
        &self,
        id: Uuid,
        session: &Session,
    ) -> Result<OwnedMutexGuard<Slot>> {
        let mut lease = self.slot(id).lock_owned().await;
        if session.is_closed() {
            return Err(DomainError::AuthRequired);
        }
        lease.prune(id);
        Ok(lease)
    }

    fn grant(&self, id: Uuid, session: &Arc<Session>, push: &PushSender) -> Writer {
        let mut writer = Writer {
            connections: Vec::new(),
            generation: self.next_generation.fetch_add(1, Ordering::Relaxed) + 1,
        };
        writer.join(id, session, push);
        writer
    }

    pub(crate) async fn access(
        &self,
        id: Uuid,
        session: &Arc<Session>,
        push: &PushSender,
        action: Action,
        expected: Option<u64>,
    ) -> Result<TerminalWriterState> {
        let mut lease = self.current(id, session).await?;
        if matches!(action, Action::Watch | Action::Release) {
            let weak = Arc::downgrade(session);
            lease.watching.retain(|watcher| !watcher.session.ptr_eq(&weak));
            if action == Action::Watch {
                session.track_terminal_writer(id);
                lease.watching.push(WriterConnection { session: weak, push: push.clone() });
            }
        }
        let ours = lease.writer.as_ref().is_some_and(|w| w.is_ours(session));
        let changed = match action {
            Action::Claim | Action::Watch if lease.writer.is_none() => {
                lease.writer = Some(self.grant(id, session, push));
                true
            }
            Action::TakeOver if !ours => {
                if let Some(previous) = lease.writer.replace(self.grant(id, session, push)) {
                    previous.displace(id);
                }
                true
            }
            Action::Release if ours && lease.writer.as_ref().map(|w| w.generation) == expected => {
                lease.writer = None;
                true
            }
            _ => false,
        };
        if changed {
            lease.changed(id, Some(session));
        }
        let generation = lease.writer.as_mut().filter(|w| w.is_ours(session)).map(|w| {
            w.join(id, session, push);
            w.generation
        });
        let reason = if generation.is_some() {
            ""
        } else if session.displaced_terminals.lock().unwrap().contains(&id) {
            "writer_taken_over"
        } else if lease.writer.is_some() {
            "writer_busy"
        } else {
            "writer_available"
        };
        Ok(TerminalWriterState {
            generation,
            reason: reason.into(),
            occupied: lease.writer.is_some(),
        })
    }

    pub(crate) async fn write(
        &self,
        id: Uuid,
        session: &Arc<Session>,
        push: &PushSender,
        generation: Option<u64>,
    ) -> Result<WriteGuard> {
        let mut guard = self.check(id, session, generation).await?;
        if let Some(writer) = guard.lease.writer.as_mut() {
            writer.join(id, session, push);
        } else {
            guard.lease.writer = Some(self.grant(id, session, push));
            guard.lease.changed(id, Some(session));
        }
        Ok(guard)
    }

    /// Geometry and draft reads respect the current writer without claiming
    /// an empty lease.
    pub(crate) async fn check(
        &self,
        id: Uuid,
        session: &Arc<Session>,
        generation: Option<u64>,
    ) -> Result<WriteGuard> {
        let lease = self.current(id, session).await?;
        match lease.writer.as_ref() {
            Some(w) if w.is_ours(session) => {
                if let Some(generation) = generation {
                    check_input_lease(w.generation, generation).map_err(|_| {
                        DomainError::Conflict {
                            what: "writer_taken_over",
                        }
                    })?;
                }
            }
            Some(_) => {
                let displaced = session.displaced_terminals.lock().unwrap().contains(&id);
                return Err(DomainError::Conflict {
                    what: if displaced {
                        "writer_taken_over"
                    } else {
                        "writer_busy"
                    },
                });
            }
            None if generation.is_some() => {
                return Err(DomainError::Conflict {
                    what: "writer_taken_over",
                });
            }
            None => {}
        }
        Ok(WriteGuard { lease })
    }
}

impl Writer {
    fn keep_live_connections(&mut self) -> bool {
        self.connections
            .retain(|connection| connection.live_session().is_some());
        !self.connections.is_empty()
    }

    fn is_ours(&self, session: &Arc<Session>) -> bool {
        self.connections
            .iter()
            .filter_map(WriterConnection::live_session)
            .any(|owner| {
                Arc::ptr_eq(&owner, session)
                    || (session.writer_instance.get().is_some()
                        && session.writer_instance.get() == owner.writer_instance.get()
                        && session.client_id() == owner.client_id())
            })
    }

    // A native action may claim the lease before the terminal input connection
    // learns its generation. Both connections participate in the same lease.
    fn join(&mut self, id: Uuid, session: &Arc<Session>, push: &PushSender) {
        let weak = Arc::downgrade(session);
        if self
            .connections
            .iter()
            .any(|connection| connection.session.ptr_eq(&weak))
        {
            return;
        }
        session.track_terminal_writer(id);
        session.displaced_terminals.lock().unwrap().remove(&id);
        self.connections.push(WriterConnection {
            session: weak,
            push: push.clone(),
        });
    }

    fn displace(self, id: Uuid) {
        let lost = Event {
            payload: Some(event::Payload::TerminalWriterLost(TerminalWriterLost {
                terminal_id: bytes::Bytes::copy_from_slice(id.as_bytes()),
                generation: self.generation,
            })),
            ..Default::default()
        };
        for connection in self.connections {
            if let Some(session) = connection.live_session() {
                session.displaced_terminals.lock().unwrap().insert(id);
                connection.push.push(lost.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Uuid, event};
    use crate::sessions::Sessions;
    use farcooler_transport::{PushReceiver, push_queue};
    use std::time::Duration;

    async fn changed(events: &mut PushReceiver, id: Uuid) {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await.expect("a lease change was pushed").unwrap();
        assert!(matches!(event.payload, Some(event::Payload::TerminalWriterChanged(change))
            if change.terminal_id.as_ref() == id.as_bytes()));
    }

    #[tokio::test]
    async fn ov_1499_viewers_hear_release_claim_takeover_and_disconnect() {
        let sessions = Sessions::new();
        let leases = &sessions.terminal_writers;
        let owner = sessions.open(None);
        let viewer = sessions.open(None);
        let next = sessions.open(None);
        let (owner_push, _owner_events) = push_queue(1024);
        let (viewer_push, mut viewer_events) = push_queue(1024);
        let (next_push, _next_events) = push_queue(1024);
        let id = Uuid::now_v7();

        let grant = leases.access(id, &owner, &owner_push, Action::Claim, None).await.unwrap();
        let state = leases.access(id, &viewer, &viewer_push, Action::Watch, None).await.unwrap();
        assert_eq!((state.generation, state.reason.as_str()), (None, "writer_busy"));

        leases.access(id, &owner, &owner_push, Action::Release, grant.generation).await.unwrap();
        changed(&mut viewer_events, id).await;
        let idle = leases.access(id, &viewer, &viewer_push, Action::Status, None).await.unwrap();
        assert_eq!((idle.generation, idle.occupied), (None, false), "status stays read-only");

        drop(leases.write(id, &owner, &owner_push, None).await.unwrap());
        changed(&mut viewer_events, id).await;
        leases.access(id, &next, &next_push, Action::TakeOver, None).await.unwrap();
        changed(&mut viewer_events, id).await;
        drop(next);
        changed(&mut viewer_events, id).await;
        assert!(!leases.access(id, &viewer, &viewer_push, Action::Status, None).await.unwrap().occupied);

        leases.access(id, &viewer, &viewer_push, Action::Release, None).await.unwrap();
        leases.access(id, &owner, &owner_push, Action::Claim, None).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(20), viewer_events.recv()).await.is_err(),
            "a hidden pane no longer watches this lease");
    }
}
