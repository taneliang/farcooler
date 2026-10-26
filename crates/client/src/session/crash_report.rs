//! `crash.report` (ov-797): a phone sends one crash's signature to its
//! owner's runner.
//!
//! The args are decoded with `deny_unknown_fields`, so a key beyond the
//! signature's (a message, a thread name, a call tree) is refused here,
//! before anything is sent, rather than dropped quietly: a phone that grew a
//! field it shouldn't send finds out in its own tests. A runner without the
//! `crash_reports` capability is refused here too.

use farcooler_protocol::capability::CRASH_REPORTS;
use farcooler_protocol::v1::{self as pb, request, result};
use serde::Deserialize;
use serde_json::{Value, json};

use super::results::wrong;
use super::{Session, SessionError, require};

/// What a phone may send: the signature, and nothing else.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    kind: String,
    exception_type: Option<i32>,
    exception_code: Option<i64>,
    signal: Option<i32>,
    #[serde(default)]
    exception_name: String,
    frames: Vec<Frame>,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    app_build: String,
    #[serde(default)]
    os: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    binary: String,
    offset: u64,
    #[serde(default)]
    symbol: String,
    depth: u32,
    #[serde(default)]
    own: bool,
}

/// `args` as the wire's payload, or why not.
pub(crate) fn payload_of(args: &Value) -> Result<pb::CrashReport, SessionError> {
    let a: Args = serde_json::from_value(args.clone())
        .map_err(|e| SessionError::Protocol(format!("crash.report takes a signature only: {e}")))?;
    Ok(pb::CrashReport {
        kind: a.kind,
        exception_type: a.exception_type,
        exception_code: a.exception_code,
        signal: a.signal,
        exception_name: a.exception_name,
        frames: a
            .frames
            .into_iter()
            .map(|f| pb::CrashFrame { binary: f.binary, offset: f.offset, symbol: f.symbol, depth: f.depth, own: f.own })
            .collect(),
        app_version: a.app_version,
        app_build: a.app_build,
        os: a.os,
    })
}

impl Session {
    /// `crash.report`, as `ffi::dispatch` passes it: answers `{count, task}`.
    pub async fn crash_report(&self, args: &Value) -> Result<Value, SessionError> {
        require(self.capabilities(), CRASH_REPORTS, "crash.report")?;
        let payload = request::Payload::CrashReport(payload_of(args)?);
        match self.value("crash.report", None, Some(payload)).await? {
            result::Value::CrashRecorded(r) => Ok(json!({ "count": r.count, "task": r.task_key })),
            other => Err(wrong("crash_recorded", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature() -> Value {
        json!({
            "kind": "crash", "exception_type": 1, "exception_code": 0, "signal": 11,
            "exception_name": "", "app_version": "1.4", "app_build": "1093", "os": "iPhone OS 26.1 (23B85)",
            "frames": [{ "binary": "FarCooler", "offset": 7982, "symbol": "", "depth": 0, "own": true }],
        })
    }

    /// The signature's fields reach the payload as they were given.
    #[test]
    fn a_signature_becomes_the_payload() {
        let p = payload_of(&signature()).unwrap();
        assert_eq!((p.kind.as_str(), p.exception_type, p.signal, p.app_build.as_str()), ("crash", Some(1), Some(11), "1093"));
        assert_eq!(p.frames, vec![pb::CrashFrame { binary: "FarCooler".into(), offset: 7982, symbol: String::new(), depth: 0, own: true }]);
    }

    /// What the phone reads out of MetricKit's JSON (`test/fixtures/crash-report/`,
    /// which `CrashSignatureTests` writes the same bytes against) is a
    /// signature this end takes whole.
    #[test]
    fn the_phones_signature_is_taken() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/crash-report/signature.json");
        let args: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let p = payload_of(&args).unwrap();
        assert_eq!((p.exception_name.as_str(), p.frames.len()), ("NSInvalidArgumentException", 11));
        assert_eq!(p.frames.iter().filter(|f| f.own).map(|f| f.depth).collect::<Vec<_>>(), [11]);
    }

    /// A key beyond the signature's, at the top or in a frame, is refused
    /// before anything is sent.
    #[test]
    fn anything_more_is_refused() {
        for (at, key) in [(None, "composedMessage"), (None, "callStackTree"), (Some(0), "threadName"), (Some(0), "subFrames")] {
            let mut args = signature();
            match at {
                None => args[key] = json!("x"),
                Some(i) => args["frames"][i][key] = json!("x"),
            }
            assert!(matches!(payload_of(&args), Err(SessionError::Protocol(m)) if m.contains(key)), "{key} was let through");
        }
    }
}
