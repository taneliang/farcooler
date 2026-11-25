//! Session-owned terminal input leases. The same per-pane lock covers the
//! lease check and delivery, so a takeover cannot overtake an admitted write.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use farcooler_core::{DomainError, Result, preconditions::check_input_lease};
use farcooler_protocol::v1::{
    Event, TerminalWriterLost, TerminalWriterState, event, terminal_writer_request::Action,
};
use farcooler_transport::PushSender;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::sessions::Session;

#[derive(Default)]
pub(crate) struct TerminalWriters {
    terminals: Mutex<HashMap<Uuid, Arc<AsyncMutex<Option<Writer>>>>>,
    next_generation: AtomicU64,
}

pub(crate) struct WriteGuard {
    lease: OwnedMutexGuard<Option<Writer>>,
}

struct Writer {
    session: Weak<Session>,
    push: PushSender,
    generation: u64,
}

impl TerminalWriters {
    fn slot(&self, id: Uuid) -> Arc<AsyncMutex<Option<Writer>>> {
        let mut terminals = self.terminals.lock().unwrap();
        if let Some(slot) = terminals.get(&id) {
            return slot.clone();
        }
        // Keep occupied locks and locks used by an in-flight call. An idle
        // slot can be replaced because generations are unique across slots.
        terminals.retain(|_, slot| {
            Arc::strong_count(slot) > 1
                || slot.try_lock().map_or(true, |writer| {
                    writer
                        .as_ref()
                        .and_then(|w| w.session.upgrade())
                        .is_some_and(|s| !s.is_closed())
                })
        });
        terminals.entry(id).or_default().clone()
    }

    async fn current(
        &self,
        id: Uuid,
        session: &Session,
    ) -> Result<OwnedMutexGuard<Option<Writer>>> {
        let mut lease = self.slot(id).lock_owned().await;
        if session.is_closed() {
            return Err(DomainError::AuthRequired);
        }
        if lease
            .as_ref()
            .is_some_and(|w| w.session.upgrade().is_none_or(|s| s.is_closed()))
        {
            *lease = None;
        }
        Ok(lease)
    }

    fn grant(&self, id: Uuid, session: &Arc<Session>, push: &PushSender) -> Writer {
        session.displaced_terminals.lock().unwrap().remove(&id);
        Writer {
            session: Arc::downgrade(session),
            push: push.clone(),
            generation: self.next_generation.fetch_add(1, Ordering::Relaxed) + 1,
        }
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
        let ours = lease.as_ref().is_some_and(|w| w.is_ours(session));
        match action {
            Action::Claim if lease.is_none() => *lease = Some(self.grant(id, session, push)),
            Action::TakeOver if !ours => {
                if let Some(previous) = lease.replace(self.grant(id, session, push)) {
                    previous.displace(id);
                }
            }
            Action::Release if ours && lease.as_ref().map(|w| w.generation) == expected => {
                *lease = None
            }
            _ => {}
        }
        let generation = lease
            .as_ref()
            .filter(|w| w.is_ours(session))
            .map(|w| w.generation);
        let reason = if generation.is_some() {
            ""
        } else if session.displaced_terminals.lock().unwrap().contains(&id) {
            "writer_taken_over"
        } else if lease.is_some() {
            "writer_busy"
        } else {
            "writer_available"
        };
        Ok(TerminalWriterState {
            generation,
            reason: reason.into(),
            occupied: lease.is_some(),
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
        if guard.lease.is_none() {
            *guard.lease = Some(self.grant(id, session, push));
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
        match lease.as_ref() {
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
    fn is_ours(&self, session: &Arc<Session>) -> bool {
        self.session.upgrade().is_some_and(|owner| {
            Arc::ptr_eq(&owner, session)
                || (session.writer_instance.get().is_some()
                    && session.writer_instance.get() == owner.writer_instance.get()
                    && session.client_id() == owner.client_id())
        })
    }

    fn displace(self, id: Uuid) {
        if let Some(session) = self.session.upgrade() {
            session.displaced_terminals.lock().unwrap().insert(id);
        }
        self.push.push(Event {
            payload: Some(event::Payload::TerminalWriterLost(TerminalWriterLost {
                terminal_id: bytes::Bytes::copy_from_slice(id.as_bytes()),
                generation: self.generation,
            })),
            ..Default::default()
        });
    }
}
