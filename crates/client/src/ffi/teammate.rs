//! The ceremony's teammate door, over the C boundary (ov-525).
//!
//! Two entry points beside the four in `ffi.rs`, with the same buffer
//! contract and the same rule that a refusal is a word, never a sentence. The
//! rules are `crate::ceremony::accept_teammate_offer`'s; an app hands over the
//! team directory's own answers and shows what comes back.

use std::ffi::c_char;

use farcooler_ffi_guard::guarded;
use serde_json::{Value, json};

use super::{read_str, refusal, spill};

/// Read a scanned offer from another account, accepted only when the team
/// directory says it shares a team with the account signed in here.
///
/// `expecting_account` is the account signed in on THIS device. Call
/// `farcooler_client_ceremony_scan` first: an offer from your own account
/// goes through that, unchanged, and this is only for its `wrong_account`.
///
/// `directory_json` is `{"me": …, "rosters": […]}`: the relay's answer to
/// `directory/me`, and its answer to `directory/teams/members` for each team
/// that `me` lists, each exactly as it arrived. NULL means the directory
/// couldn't be asked, and is `{"error":"directory_unreachable"}` — never a
/// pass. So is an answer that can't decide.
///
/// `held_ms` is how long ago this device scanned, judged first, as in
/// `farcooler_client_ceremony_scan`.
///
/// Writes `{"offer": "<the offer, re-encoded>", "teammate": {"account",
/// "email", "team"}}`. The offer string is what to pass to
/// `farcooler_client_ceremony_teammate_reply`, and the email is the
/// directory's, for the confirmation: never anything the code said.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_ceremony_scan_teammate(
    encoded: *const c_char,
    expecting_account: *const c_char,
    directory_json: *const c_char,
    held_ms: u64,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        use crate::ceremony::{CeremonyError, accept_teammate_offer, encode_offer, still_fresh};

        let Some(encoded) = (unsafe { read_str(encoded) }) else {
            return spill(&refusal(&CeremonyError::Malformed("nothing scanned".into())), out, capacity);
        };
        let account = unsafe { read_str(expecting_account) }.unwrap_or_default();
        // Unparseable is the same as absent: an answer nobody can read can't
        // vouch for anybody.
        let directory = unsafe { read_str(directory_json) }.and_then(|d| serde_json::from_str::<Value>(&d).ok());

        let answer = still_fresh(std::time::Duration::from_millis(held_ms))
            .and_then(|()| accept_teammate_offer(&encoded, &account, directory.as_ref()))
            .map(|accepted| {
                json!({
                    "offer": encode_offer(&accepted.offer),
                    "teammate": {
                        "account": accepted.teammate.account,
                        "email": accepted.teammate.email,
                        "team": accepted.teammate.team,
                    },
                })
                .to_string()
            });
        spill(&answer.unwrap_or_else(|e| refusal(&e)), out, capacity)
    })
}

/// Build the reply for a teammate's device: `farcooler_client_ceremony_reply`
/// with `granted_by`, the email of the account granting, written into it so
/// the teammate's device can ask "Join …'s runners?" before adding them.
///
/// `granted_by` is shown on the other device, never trusted there: the reply
/// leg proves nothing about who made it. NULL or empty is a same-account
/// reply, exactly as `farcooler_client_ceremony_reply` builds it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_ceremony_teammate_reply(
    offer_json: *const c_char,
    runners_json: *const c_char,
    granted_by: *const c_char,
    budget_bytes: usize,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        use crate::ceremony::{
            BUDGET, CeremonyError, RunnerEntry, decode_offer, encode_manifest, fit_manifest_from,
        };

        let malformed = |what: &str| refusal(&CeremonyError::Malformed(what.to_string()));
        let Some(offer_json) = (unsafe { read_str(offer_json) }) else {
            return spill(&malformed("no offer"), out, capacity);
        };
        let runners_json = unsafe { read_str(runners_json) }.unwrap_or_else(|| "[]".into());
        let granted_by = unsafe { read_str(granted_by) }.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());

        let offer = match decode_offer(&offer_json) {
            Ok(offer) => offer,
            Err(e) => return spill(&refusal(&e), out, capacity),
        };
        let Ok(runners) = serde_json::from_str::<Vec<RunnerEntry>>(&runners_json) else {
            return spill(&malformed("runners"), out, capacity);
        };
        let budget = if budget_bytes == 0 { BUDGET } else { budget_bytes };
        match fit_manifest_from(&offer, runners, granted_by.as_deref(), budget) {
            Ok(fitted) => spill(&encode_manifest(&fitted.manifest), out, capacity),
            Err(e) => spill(&refusal(&e), out, capacity),
        }
    })
}
