//! The heartbeat's key inventory (ov-510): a hash on every beat, the list only
//! when the hash changed or the relay asks. Its own file to keep `push.rs`
//! inside the size budget.

use std::sync::{Arc, Mutex};

use super::*;
use farcooler_fence::{inventory_of, AUTHORIZED_KEYS};

const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const OTHER_KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER";

fn keys(text: &str) -> farcooler_fence::Inventory {
    inventory_of(text, AUTHORIZED_KEYS)
}

/// A relay that records every body posted and answers each with `reply`.
/// Serves keep-alive connections, since reqwest reuses them.
async fn a_relay(reply: impl Fn(&serde_json::Value) -> String + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (log, reply) = (seen.clone(), Arc::new(reply));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (log, reply) = (log.clone(), reply.clone());
            tokio::spawn(async move {
                let mut pending = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let text = String::from_utf8_lossy(&pending).to_string();
                    let whole = text.find("\r\n\r\n").and_then(|end| {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        (pending.len() >= end + 4 + length).then_some((end + 4, length))
                    });
                    if let Some((start, length)) = whole {
                        let body: serde_json::Value = serde_json::from_slice(&pending[start..start + length]).unwrap();
                        let answer = reply(&body);
                        log.lock().unwrap().push(body);
                        pending.drain(..start + length);
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{answer}", answer.len());
                        if socket.write_all(head.as_bytes()).await.is_err() {
                            return;
                        }
                        continue;
                    }
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => pending.extend_from_slice(&buf[..n]),
                    }
                }
            });
        }
    });
    (url, seen)
}

fn pairing(relay: &str) -> Pairing {
    Pairing { relay: relay.to_string(), token: "t".into() }
}

/// The first beat names the hash and, since the relay has acknowledged
/// nothing, carries the list. The next, with the same keys, is the hash alone.
/// A changed file sends the list again.
///
/// Mutation: make `beat` send `full = true` always, or never; both red.
#[tokio::test]
async fn a_beat_sends_the_list_only_when_the_hash_changed() {
    let (url, seen) = a_relay(|_| r#"{"ok":true}"#.into()).await;
    let (client, ack) = (reqwest::Client::new(), InventoryAck::default());
    let one = keys(&format!("ssh-ed25519 {KEY} a@laptop\n"));
    let two = keys(&format!("ssh-ed25519 {KEY} a@laptop\nssh-ed25519 {OTHER_KEY} planted\n"));

    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await);
    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await);
    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&two, &ack))).await);
    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&two, &ack))).await);

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4, "one post per beat when the relay never asks: {seen:?}");
    let inv = |i: usize| &seen[i]["inventory"];
    assert_eq!(inv(0)["hash"], one.digest());
    assert_eq!(inv(0)["entries"].as_array().map(Vec::len), Some(1), "first beat: the list");
    assert_eq!(inv(0)["entries"][0]["comment"], "a@laptop");
    assert_eq!(inv(0)["entries"][0]["inside"], false);
    assert_eq!(inv(1)["hash"], one.digest());
    assert!(inv(1).get("entries").is_none(), "same keys: the hash alone: {}", seen[1]);
    assert_eq!(inv(2)["hash"], two.digest());
    assert_eq!(inv(2)["entries"].as_array().map(Vec::len), Some(2), "changed: the list again");
    assert!(inv(3).get("entries").is_none(), "{}", seen[3]);
    assert_ne!(one.digest(), two.digest());
}

/// A relay that has lost the list says so, and gets it in the same beat.
///
/// Mutation: ignore `"inventory": "send"` in `post`; red.
#[tokio::test]
async fn a_relay_that_asks_for_the_list_gets_it_at_once() {
    let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = asked.clone();
    let (url, seen) = a_relay(move |body| {
        let wants = body["inventory"].get("entries").is_none() && flag.swap(false, std::sync::atomic::Ordering::SeqCst);
        if wants { r#"{"ok":true,"inventory":"send"}"#.into() } else { r#"{"ok":true}"#.into() }
    })
    .await;
    let (client, ack) = (reqwest::Client::new(), InventoryAck::default());
    let one = keys(&format!("ssh-ed25519 {KEY} a@laptop\n"));

    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await);
    assert_eq!(seen.lock().unwrap().len(), 1, "first beat carried the list");

    // The relay now claims it has nothing.
    asked.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "a hash-only beat, then the list: {seen:?}");
    assert!(seen[1]["inventory"].get("entries").is_none());
    assert_eq!(seen[2]["inventory"]["entries"].as_array().map(Vec::len), Some(1));
}

/// A send the relay refused is not counted as taken: the next beat sends the
/// list again.
#[tokio::test]
async fn a_list_the_relay_refused_is_sent_again() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let (client, ack) = (reqwest::Client::new(), InventoryAck::default());
    let one = keys(&format!("ssh-ed25519 {KEY} a@laptop\n"));
    assert!(!heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await, "nobody home");

    let (url, seen) = a_relay(|_| r#"{"ok":true}"#.into()).await;
    assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&one, &ack))).await);
    assert!(seen.lock().unwrap()[0]["inventory"].get("entries").is_some());
}

/// With no inventory (the file could not be read) the beat is the old one,
/// five keys and no `inventory`.
#[tokio::test]
async fn a_beat_without_an_inventory_is_the_old_beat() {
    let (url, seen) = a_relay(|_| r#"{"ok":true}"#.into()).await;
    assert!(heartbeat(&reqwest::Client::new(), &pairing(&url), Some("i"), None).await);
    assert!(seen.lock().unwrap()[0].get("inventory").is_none());
}

/// A file that can't be read is reported, not hidden: the beat carries the
/// error with no list, every beat, and still lands. The same error is logged
/// once, not every five minutes.
///
/// Mutation: drop `error` from `field`, or let `note` always answer true.
#[tokio::test]
async fn a_read_that_failed_is_reported_and_the_beat_still_lands() {
    let (url, seen) = a_relay(|_| r#"{"ok":true}"#.into()).await;
    let (client, ack) = (reqwest::Client::new(), InventoryAck::default());
    let failed = farcooler_fence::Inventory { error: Some("refused"), ..Default::default() };
    for _ in 0..2 {
        assert!(heartbeat(&client, &pairing(&url), Some("i"), Some((&failed, &ack))).await, "the beat itself lands");
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1]["inventory"]["error"], "refused");
    assert!(seen[1]["inventory"].get("entries").is_none(), "{}", seen[1]);
    assert!(ack.get().is_none(), "an error is not an acknowledged list");
    assert!(!ack.note(Some("refused")), "the same failure is not news twice");
    assert!(ack.note(Some("damaged")), "a different one is");
    assert!(!ack.note(None));
    assert!(ack.note(Some("damaged")), "and it is news again after a clean read");
}

/// The cut keeps what is worth finding: keys outside the fence and not ours go
/// first, the total says how many there were, and a long comment is cut.
#[tokio::test]
async fn a_cut_list_keeps_outside_keys_first_and_says_how_many_there_were() {
    let mut inv = farcooler_fence::Inventory::default();
    let listed = |i: usize, inside: bool, scope: &'static str| farcooler_fence::Listed {
        fingerprint: format!("SHA256:{i}"),
        comment: "c".repeat(300),
        inside,
        scope,
        file: "authorized_keys",
        options: "o".repeat(300),
    };
    for i in 0..600 {
        inv.entries.push(listed(i, true, "control"));
    }
    inv.entries.push(listed(600, false, "other"));
    inv.entries.push(listed(601, true, "other"));
    let wire = serde_json::to_value(super::inventory::field(&inv, &inv.digest(), true)).unwrap();
    assert_eq!(wire["total"], 602);
    let entries = wire["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 500);
    assert_eq!(entries[0]["fingerprint"], "SHA256:600", "outside first");
    assert_eq!(entries[1]["fingerprint"], "SHA256:601", "then foreign inside");
    assert_eq!(entries[0]["comment"].as_str().unwrap().len(), 80);
    assert_eq!(entries[0]["options"].as_str().unwrap().len(), 120);
}
