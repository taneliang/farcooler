//! `pr.ready` and `pr.thread_reply` (ov-317): a phone marks a lane's draft pull
//! request ready for review, and replies to a review thread an agent escalated,
//! both as the runner's `gh` login.
//!
//! The phone names a lane (and which of its pull requests), or a card; never a
//! URL or a repository. The runner reads the rest from its own board. A runner
//! without the `pr_actions` capability is refused here, before anything is sent.

use farcooler_protocol::capability::PR_ACTIONS;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};
use uuid::Uuid;

use super::results::wrong;
use super::{Session, SessionError, require};

impl Session {
    /// The two PR verbs, as `ffi::dispatch` passes them. `pr.ready` takes
    /// `{lane, pr_number?}` and answers `{number, url, already_ready}`.
    /// `pr.thread_reply` takes `{task, body?, send?, note_id?, digest?}` and answers
    /// `{repository, number, login, sent, in_thread, note_id, digest}`: with `send` false it
    /// posts nothing and says where the reply would go and as whom.
    pub async fn pr_action(&self, method: &str, args: &Value) -> Result<Value, SessionError> {
        require(self.capabilities(), PR_ACTIONS, method)?;
        let id = |key: &str| -> Result<bytes::Bytes, SessionError> {
            args.get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<Uuid>().ok())
                .map(|u| bytes::Bytes::copy_from_slice(u.as_bytes()))
                .ok_or_else(|| SessionError::Protocol(format!("{method} needs a {key}")))
        };
        if method == "pr.ready" {
            let pr_number = args.get("pr_number").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let payload = request::Payload::PrReady(pb::PrReady { lane_id: id("lane")?, pr_number });
            return match self.value("pr.ready", None, Some(payload)).await? {
                result::Value::PrReadied(r) => Ok(ready_json(&r)),
                other => Err(wrong("pr_readied", &other)),
            };
        }
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let send = args.get("send").and_then(|v| v.as_bool()).unwrap_or(false);
        // What the preview answered, handed back unchanged: the runner posts only to the thread shown.
        let note_id = args.get("note_id").and_then(|v| v.as_str()).and_then(|s| s.parse::<Uuid>().ok());
        let note_id = note_id.map(|u| bytes::Bytes::copy_from_slice(u.as_bytes())).unwrap_or_default();
        let digest = args.get("digest").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let payload = request::Payload::PrThreadReply(pb::PrThreadReply { task_id: id("task")?, body, send, note_id, digest });
        match self.value("pr.thread_reply", None, Some(payload)).await? {
            result::Value::PrThreadReplied(r) => Ok(reply_json(&r)),
            other => Err(wrong("pr_thread_replied", &other)),
        }
    }
}

/// `pr.ready`'s answer as the apps read it.
pub(crate) fn ready_json(r: &pb::PrReadied) -> Value {
    json!({ "number": r.number, "url": r.url, "already_ready": r.already_ready })
}

/// `pr.thread_reply`'s answer as the apps read it.
pub(crate) fn reply_json(r: &pb::PrThreadReplied) -> Value {
    let note_id = Uuid::from_slice(&r.note_id).map(|u| u.to_string()).unwrap_or_default();
    json!({
        "repository": r.repository, "number": r.number, "login": r.login,
        "sent": r.sent, "in_thread": r.in_thread, "note_id": note_id, "digest": r.digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The answers reach the phones in the shape their decoders read.**
    /// `test/fixtures/pr-actions/` holds these bytes; the iPhone's and
    /// Android's tests decode them. Rewrite with `FARCOOLER_WRITE_FIXTURES=1`.
    #[test]
    fn the_answers_are_the_bytes_the_phones_read() {
        let ready = ready_json(&pb::PrReadied { number: 31, url: "https://github.com/acme/app/pull/31".into(), already_ready: false });
        let preview = reply_json(&pb::PrThreadReplied {
            repository: "acme/billing".into(),
            number: 213,
            login: "e-liang".into(),
            sent: false,
            in_thread: true,
            note_id: bytes::Bytes::copy_from_slice(Uuid::from_u128(0x0100_0000_0000_7000_8000_0000_0000_0015).as_bytes()),
            digest: "9f2c41d07b3a5e8812ac6d00".into(),
        });
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-actions");
        for (name, value) in [("ready.json", ready), ("reply-preview.json", preview)] {
            let out = serde_json::to_string_pretty(&value).unwrap() + "\n";
            if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(name), &out).unwrap();
            }
            let fixture = std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(out, fixture, "{name} no longer matches what the phones decode");
        }
    }
}
