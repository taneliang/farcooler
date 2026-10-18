//! The event channel a new session opens for fleet news.

use std::sync::{Arc, Mutex};

use super::{EventQueue, event_line, locked};
use crate::session::Session;

/// Start this session pushing fleet news into `events`.
///
/// Best effort, and deliberately not a reason to fail the connection. A runner
/// too old, an sshd out of channels, or a transport that is not ssh at all —
/// each of those is a runner the client polls the way it always did, and
/// refusing to connect over it would turn a slower app into no app.
///
/// `generation` is the one `EventQueue::opening` handed out when the connect
/// was answered, which is before this runs (ov-613).
pub(super) async fn subscribe(open: &Session, events: &Arc<Mutex<EventQueue>>, generation: u64) {
    // The sink. It runs on the runtime thread reading the daemon's socket, so
    // everything it does is under one uncontended lock and bounded: see
    // `EventQueue`.
    let queue = Arc::clone(events);
    let sink: crate::session::EventSink = Arc::new(move |what| {
        locked(&queue).push(event_line(&what));
    });

    match open.subscribe(sink).await {
        Ok(subscription) => {
            locked(events).opened(generation);
            // One re-read for whatever changed between the connect's answer,
            // which the first reads went out on, and this channel opening
            // (ov-613): nothing was there to push it, and a poller that now
            // sees a live channel waits 15 s before asking.
            locked(events).push(event_line(&crate::session::FleetEvent::Resync));
            let queue = Arc::clone(events);
            // Awaited rather than dropped, so `live` is an answer about now.
            // The task ends when the ssh channel does, which is also when the
            // session it belongs to was dropped — hence the generation check
            // inside `closed`.
            tokio::spawn(async move {
                let _ = subscription.await;
                locked(&queue).closed(generation);
            });
        }
        Err(e) => {
            tracing::warn!(error = %e, "no dedicated event channel; this client will poll");
        }
    }
}
