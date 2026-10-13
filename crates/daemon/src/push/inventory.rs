//! The key inventory on the heartbeat (ov-510, design ov-480 §3).
//!
//! Every beat carries a hash of the keys in `authorized_keys`. The list itself
//! goes only when the hash differs from the last one the relay acknowledged,
//! or when the relay answers a hash with `"inventory": "send"` because it has
//! nothing, or something else, on file. So a runner whose keys don't change
//! sends 64 bytes a beat, and a relay that lost them asks for them back.

use std::sync::Mutex;

use farcooler_fence::Inventory;

use super::{beat_body, Beat, Pairing};

/// The hash of the inventory the relay last took in full. Per daemon, in
/// memory: after a restart the first beat sends the hash alone and the relay
/// asks for the list if it needs it.
#[derive(Debug, Default)]
pub struct InventoryAck(Mutex<Option<String>>);

impl InventoryAck {
    fn get(&self) -> Option<String> {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<Entry>>,
}

#[derive(Debug, serde::Serialize)]
struct Entry {
    fingerprint: String,
    comment: String,
    inside: bool,
    scope: &'static str,
}

pub(super) fn field(inventory: &Inventory, hash: &str, full: bool) -> Field {
    Field {
        hash: hash.to_string(),
        unparsed: inventory.unparsed,
        entries: full.then(|| {
            inventory
                .entries
                .iter()
                .map(|e| Entry {
                    fingerprint: e.fingerprint.clone(),
                    comment: e.comment.clone(),
                    inside: e.inside,
                    scope: e.scope,
                })
                .collect()
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

    let hash = inventory.digest();
    let full = ack.get().as_deref() != Some(hash.as_str());
    let (status, asked) = post(client, url, pairing, body(install, Some(field(inventory, &hash, full)))).await?;
    if status.is_success() && full {
        ack.set(Some(hash.clone()));
    }
    if !asked || full {
        return Ok(status);
    }
    ack.set(None);
    let (status, _) = post(client, url, pairing, body(install, Some(field(inventory, &hash, true)))).await?;
    if status.is_success() {
        ack.set(Some(hash));
    }
    Ok(status)
}
