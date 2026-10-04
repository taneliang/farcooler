//! How a call reaches the session, now that calls run alongside each other
//! (ov-147).
//!
//! The slot used to be an async lock held for the whole of each call, which
//! made every call on a handle wait for the one before it: a keystroke behind
//! a diff, for as long as the diff took, or for the ssh keepalive's ninety
//! seconds on a link that had gone quiet. Now a call takes the session out of
//! the slot (a clone of an `Arc`, under a lock held for no longer than that)
//! and the transport carries any number of calls at once.
//!
//! That leaves three things this module decides: the order of a terminal's
//! input, paste included; when a dead link empties the slot; and how input is
//! answered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use farcooler_ffi_guard::locked;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::session::{Pending, Session, SessionError};

/// Queue input for the wire now, if `method` is input.
///
/// **Order.** Keys must reach the runner in the order they were sent — "l"
/// then "s" must not type "sl" — and so must a resize between them. The
/// daemon keeps requests naming one terminal in arrival order, so what is left
/// is to make arrival order the order of `farcooler_client_call`: these are
/// queued by their terminal's lane (`Lanes`), which takes them in the order
/// that function was called, rather than by whichever runtime task happens to
/// run first. They are also sent ahead of ordinary calls still queued (see
/// `deadlines`).
///
/// Input, as hex. A key is not always a character — arrows, Ctrl-C and a
/// bracketed paste are byte sequences — so nothing here re-encodes.
///
/// `None` for every other method, which `dispatch` makes as it always did.
pub(super) fn queue(
    session: &Session,
    method: &str,
    args: &Value,
) -> Option<Result<Pending, SessionError>> {
    let terminal = || {
        args.get("terminal")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .ok_or_else(|| SessionError::Protocol(format!("{method} needs a terminal")))
    };
    let number = |key: &str, default: u64| {
        args.get(key).and_then(|v| v.as_u64()).unwrap_or(default) as u32
    };
    match method {
        "terminal.write" => Some((|| {
            let hex = args.get("hex").and_then(|v| v.as_str()).unwrap_or_default();
            let bytes = super::decode_hex(hex)
                .ok_or_else(|| SessionError::Protocol("input must be hex".into()))?;
            session.start_write(terminal()?, bytes)
        })()),
        "terminal.resize" => {
            Some(terminal().and_then(|t| session.start_resize(t, number("columns", 80), number("rows", 24))))
        }
        _ => None,
    }
}

/// Each terminal's input in the order the app sent it, a paste included.
///
/// Keys alone keep their order by being queued for the wire synchronously
/// (`queue`). A paste is several requests — chunks, then the path the runner
/// types after the last — and the runner orders one pane's requests only as
/// they arrive, so a key sent mid-upload would land between two chunks and be
/// typed before the path. Before ov-147 the session lock held keys back for
/// the whole paste, along with everything else.
///
/// So each terminal has a lane: one task taking that terminal's input jobs in
/// the order `farcooler_client_call` and `farcooler_client_paste_file` were
/// called. A key is queued for the wire when its turn comes, which is at once
/// unless a paste is ahead of it; a paste holds the lane until it finishes.
/// Only this terminal's input waits: other panes and other calls do not.
///
/// One task per terminal a handle has sent input to, ended when the handle
/// is freed.
#[derive(Default)]
pub(super) struct Lanes(Mutex<HashMap<uuid::Uuid, mpsc::UnboundedSender<Job>>>);

pub(super) enum Job {
    /// Queue a request for the wire, now that it is its turn.
    Now(Box<dyn FnOnce() + Send>),
    /// Hand over the lane, and wait until the sender given back is dropped.
    Hold(oneshot::Sender<oneshot::Sender<()>>),
}

impl Lanes {
    fn send(&self, runtime: &tokio::runtime::Handle, terminal: uuid::Uuid, job: Job) {
        let mut lanes = locked(&self.0);
        let lane = lanes.entry(terminal).or_insert_with(|| {
            let (tx, mut jobs) = mpsc::unbounded_channel::<Job>();
            runtime.spawn(async move {
                while let Some(job) = jobs.recv().await {
                    match job {
                        Job::Now(queue) => queue(),
                        Job::Hold(turn) => {
                            let (done, finished) = oneshot::channel();
                            if turn.send(done).is_ok() {
                                // Ends when the holder drops `done`, however
                                // its paste ended.
                                let _ = finished.await;
                            }
                        }
                    }
                }
            });
            tx
        });
        // The task outlives every sender, so this cannot fail.
        let _ = lane.send(job);
    }

    /// The lane for `terminal`, once the input sent before this has gone.
    /// Hold what it resolves to for as long as the lane must wait.
    pub(super) fn hold(
        &self,
        runtime: &tokio::runtime::Handle,
        terminal: uuid::Uuid,
    ) -> oneshot::Receiver<oneshot::Sender<()>> {
        let (turn, held) = oneshot::channel();
        self.send(runtime, terminal, Job::Hold(turn));
        held
    }
}

/// `queue`, in `terminal`'s turn: see `Lanes`. `None` if `method` is not
/// input; otherwise what will carry the queued call once its turn comes.
pub(super) fn queue_in_turn(
    lanes: &Lanes,
    runtime: &tokio::runtime::Handle,
    session: &Arc<Session>,
    method: &str,
    args: &Value,
) -> Option<oneshot::Receiver<Result<Pending, SessionError>>> {
    if !matches!(method, "terminal.write" | "terminal.resize") {
        return None;
    }
    let (tx, rx) = oneshot::channel();
    let terminal = args.get("terminal").and_then(|v| v.as_str()).and_then(|s| s.parse::<uuid::Uuid>().ok());
    let (session, method, args) = (Arc::clone(session), method.to_string(), args.clone());
    let job = move || {
        let queued = queue(&session, &method, &args)
            .unwrap_or_else(|| Err(SessionError::Protocol(format!("{method} is not input"))));
        let _ = tx.send(queued);
    };
    match terminal {
        Some(terminal) => lanes.send(runtime, terminal, Job::Now(Box::new(job))),
        // No terminal: `queue` refuses it at once, and there is no lane to wait in.
        None => job(),
    }
    Some(rx)
}

/// The answer to input `queue_in_turn` sent, as the boundary reports it.
pub(super) async fn answered_in_turn(
    queued: oneshot::Receiver<Result<Pending, SessionError>>,
) -> Result<Value, SessionError> {
    // The lane went without queueing it: the handle is being freed.
    let queued = queued.await.map_err(|_| SessionError::Protocol("the client was closed".into()))?;
    answered(queued).await
}

/// The answer to input `queue` sent, as the boundary reports it.
pub(super) async fn answered(queued: Result<Pending, SessionError>) -> Result<Value, SessionError> {
    queued?.value().await?;
    Ok(json!({}))
}

/// **A dead link.** Empty the slot, but only if it still holds the session
/// that failed: with calls running at once, a reconnect may have filled it
/// with a new one between this call starting and failing, and emptying that
/// would throw away a working session on a stale report.
pub(super) fn forget(slot: &Mutex<Option<Arc<Session>>>, dead: Option<&Arc<Session>>) {
    let mut held = locked(slot);
    let same = matches!((held.as_ref(), dead), (Some(now), Some(dead)) if Arc::ptr_eq(now, dead));
    if same {
        *held = None;
    }
}
