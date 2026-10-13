//! The key inventory on the heartbeat (ov-510, design ov-480 §3).
//!
//! Every beat carries a hash of the keys in `authorized_keys` and
//! `authorized_keys2`. The list itself goes only when the hash differs from the
//! last one the relay acknowledged, or when the relay answers a hash with
//! `"inventory": "send"` because it has nothing, or something else, on file.
//! So a runner whose keys don't change sends 64 bytes a beat, and a relay that
//! lost them asks for them back.
//!
//! A file that can't be read is not silence: the beat carries `error` (see
//! `farcooler_fence::Inventory::error`), and the daemon logs the change at warn
//! once. An inventory failure never fails the beat itself.

use std::sync::Mutex;

use farcooler_fence::Inventory;

use super::{beat_body, Beat, Pairing};

/// The most keys one beat carries, and how much of a comment or an options
/// field. Together they keep a beat under the relay's body limit however much
/// text a file holds; the relay's own bounds are looser.
const MAX_KEYS: usize = 500;
/// The most the list may be once serialized, escapes and all: a comment of
/// control characters is six bytes a character in JSON. The relay drops an
/// inventory over 300 KB and keeps the beat; this keeps ours under it.
const MAX_LIST_BYTES: usize = 256 * 1024;
const COMMENT_CHARS: usize = 80;
const OPTIONS_CHARS: usize = 120;

/// The hash of the inventory the relay last took in full, and the last error
/// the daemon logged. Per daemon, in memory: after a restart the first beat
/// sends the full list, since nothing is known to be held.
#[derive(Debug, Default)]
pub struct InventoryAck(Mutex<Option<String>>, Mutex<Option<&'static str>>);

impl InventoryAck {
    /// Record the error of this reading, and say whether it is news: a
    /// failure that just began or changed, which is logged once.
    pub(super) fn note(&self, error: Option<&'static str>) -> bool {
        let mut last = self.1.lock().unwrap_or_else(|e| e.into_inner());
        let news = error.is_some() && *last != error;
        *last = error;
        news
    }

    pub(super) fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, hash: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = hash;
    }
}

/// The wire form of the inventory.
#[derive(Debug, serde::Serialize)]
pub(super) struct Field {
    hash: String,
    unparsed: u32,
    /// How many keys were read, before the cut to `MAX_KEYS`.
    total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(rename = "errorFile", skip_serializing_if = "Option::is_none")]
    error_file: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<Entry>>,
}

#[derive(Debug, serde::Serialize)]
struct Entry {
    fingerprint: String,
    comment: String,
    inside: bool,
    scope: &'static str,
    file: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    options: String,
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// Which keys go first when the list is cut: outside the fence and not ours
/// before the rest, since those are the ones worth finding.
fn priority(e: &farcooler_fence::Listed) -> u8 {
    match (e.inside, e.scope) {
        (false, _) => 0,
        (true, "other" | "ca") => 1,
        _ => 2,
    }
}

pub(super) fn field(inventory: &Inventory, hash: &str, full: bool) -> Field {
    // An error with nothing read has no list to send.
    let list = full && !(inventory.error.is_some() && inventory.entries.is_empty());
    Field {
        hash: hash.to_string(),
        unparsed: inventory.unparsed,
        total: inventory.entries.len(),
        error: inventory.error,
        error_file: inventory.error.and(inventory.error_file),
        entries: list.then(|| {
            let mut ordered: Vec<&farcooler_fence::Listed> = inventory.entries.iter().collect();
            ordered.sort_by_key(|e| priority(e));
            // Lowest priority dropped first: stop at the count or the bytes.
            let mut bytes = 0;
            let mut kept = Vec::new();
            for e in ordered.into_iter().take(MAX_KEYS) {
                let entry = Entry {
                    fingerprint: e.fingerprint.clone(),
                    comment: cut(&e.comment, COMMENT_CHARS),
                    inside: e.inside,
                    scope: e.scope,
                    file: e.file,
                    options: cut(&e.options, OPTIONS_CHARS),
                };
                bytes += serde_json::to_vec(&entry).map_or(usize::MAX, |v| v.len() + 1);
                if bytes > MAX_LIST_BYTES {
                    break;
                }
                kept.push(entry);
            }
            kept
        }),
    }
}

pub(super) fn body<'a>(install: Option<&'a str>, field: Option<Field>) -> Beat<'a> {
    Beat { inventory: field, ..beat_body(install) }
}

/// One post of a beat, and whether the relay asked for the list in answer.
async fn post(
    client: &reqwest::Client,
    url: &str,
    pairing: &Pairing,
    body: Beat<'_>,
) -> reqwest::Result<(reqwest::StatusCode, bool)> {
    let response = client.post(url).bearer_auth(&pairing.token).json(&body).send().await?;
    let status = response.status();
    // An old relay answers `{"ok":true}` and never asks.
    let asked = status.is_success()
        && response.json::<serde_json::Value>().await.ok().is_some_and(|v| v["inventory"] == "send");
    Ok((status, asked))
}

/// Post one beat, with the inventory's hash and, when it is due, its list.
/// Returns the status of the last post made.
///
/// A relay that answers `"inventory": "send"` to a hash-only beat gets the
/// list in a second post straight away, rather than at the next beat five
/// minutes on. `ack` moves only when the relay accepted a full send, so one
/// that didn't land is made again.
pub(super) async fn beat(
    client: &reqwest::Client,
    url: &str,
    pairing: &Pairing,
    install: Option<&str>,
    inventory: Option<(&Inventory, &InventoryAck)>,
) -> reqwest::Result<reqwest::StatusCode> {
    let Some((inventory, ack)) = inventory else { return Ok(post(client, url, pairing, body(install, None)).await?.0) };

    if ack.note(inventory.error) {
        tracing::warn!(error = inventory.error, "could not read the keys on this runner completely");
    }
    let hash = inventory.digest();
    // An error with nothing read is sent as it is, every beat, and moves nothing.
    let nothing = inventory.error.is_some() && inventory.entries.is_empty();
    let full = !nothing && ack.get().as_deref() != Some(hash.as_str());
    let (status, asked) = post(client, url, pairing, body(install, Some(field(inventory, &hash, full)))).await?;
    if status.is_success() && full {
        ack.set(Some(hash.clone()));
    }
    if !asked || full || nothing {
        return Ok(status);
    }
    ack.set(None);
    let (status, _) = post(client, url, pairing, body(install, Some(field(inventory, &hash, true)))).await?;
    if status.is_success() {
        ack.set(Some(hash));
    }
    Ok(status)
}
