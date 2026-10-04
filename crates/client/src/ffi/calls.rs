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
//! That leaves two things this module decides.

use std::sync::{Arc, Mutex};

use farcooler_ffi_guard::locked;
use serde_json::{Value, json};

use crate::session::{Pending, Session, SessionError};

/// Queue input for the wire now, if `method` is input.
///
/// **Order.** Keys must reach the runner in the order they were sent — "l"
/// then "s" must not type "sl" — and so must a resize between them. The
/// daemon keeps requests naming one terminal in arrival order, so what is left
/// is to make arrival order the order of `farcooler_client_call`: these are
/// queued here, on the caller's thread, before it returns, rather than by
/// whichever runtime task happens to run first. They are also sent ahead of
/// ordinary calls still queued (see `deadlines`).
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
