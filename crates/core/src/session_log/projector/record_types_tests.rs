//! claude's record types (ov-621): the prompt-cache records claude 2.1.300
//! writes, `api-request`, `api-request-blob` and `api-request-shape`, are
//! read silently; a type nothing knows is still a `Gap`, and its name is
//! logged once per session. Fixtures synthetic, in the shapes the owner's
//! transcript took (field names and order; every value invented).

use std::sync::{Arc, Mutex};

use super::fixtures::*;
use super::rows::*;

const API_REQUEST: &str = include_str!("fixtures/api-request-records.jsonl");
const UNKNOWN_TYPE: &str = include_str!("fixtures/unknown-type.jsonl");
const SCHEDULED_API_REQUEST: &str = include_str!("fixtures/scheduled-api-request.jsonl");

/// Every top-level type a live claude 2.1.300 transcript held (the census
/// in ov-621's investigation), one line each.
const KNOWN_TYPES: &[&str] = &[
    "user", "assistant", "system", "queue-operation", "attachment", "frame-link", "artifact-autoreact-ledger",
    "artifact-comment-monitor", "mode", "permission-mode", "atis-latch", "last-prompt", "cost-state",
    "file-history-snapshot", "file-history-delta", "ai-title", "agent-name", "custom-title", "bridge-session", "pr-link",
    "summary", "tag", "api-request", "api-request-blob", "api-request-shape",
];

fn unknown_gaps(p: &super::Projection) -> Vec<String> {
    p.rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Gap(Gap { reason: GapReason::Unknown(kind), .. }) => Some(kind.clone()),
            _ => None,
        })
        .collect()
}

/// One line of each known type: no `Unknown` gap among them.
#[test]
fn every_known_record_type_folds_without_a_gap() {
    for kind in KNOWN_TYPES {
        let p = fold(&format!(r#"{{"type":"{kind}","sessionId":"a9000000-0000-4000-8000-000000000009"}}"#));
        assert_eq!(unknown_gaps(&p), Vec::<String>::new(), "{kind}");
    }
}

/// The prompt-cache records draw nothing: no gap, no turn, though a blob
/// carries a user's `message.role`; laid among a recorded session's lines,
/// its rows are the same as without them. A fourth `api-request-*` too.
#[test]
fn the_prompt_cache_records_draw_nothing() {
    let alone = fold(API_REQUEST);
    assert!(alone.rows().is_empty(), "{:?}", alone.rows());
    assert_eq!(alone.stats().gaps, 0);
    let lines: Vec<&str> = RECORDED.lines().collect();
    let mut mixed = vec![lines[0]];
    mixed.extend(API_REQUEST.lines());
    mixed.extend(&lines[1..]);
    mixed.extend(API_REQUEST.lines());
    let rows = |p: &super::Projection| p.rows().iter().map(|r| (r.id.clone(), r.kind.clone())).collect::<Vec<_>>();
    assert_eq!(rows(&fold(&mixed.join("\n"))), rows(&fold(RECORDED)));
    let sibling = fold(r#"{"type":"api-request-digest","sessionId":"a9000000-0000-4000-8000-000000000001"}"#);
    assert_eq!(sibling.stats().gaps, 0, "a sibling claude adds later");
}

/// A scheduled task's firing beside its prompt-cache records: one
/// scheduled turn, no gap.
#[test]
fn a_scheduled_turn_beside_its_cache_records_has_no_gap() {
    let p = fold(SCHEDULED_API_REQUEST);
    assert_eq!(turn(&p, "turn:p2").origin, TurnOrigin::Scheduled);
    assert_eq!(p.rows().iter().filter(|r| matches!(r.kind, RowKind::Gap(_))).count(), 0, "{:?}", p.rows());
    assert_eq!(p.stats().gaps, 0);
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// A type nothing reads is a gap, counted, and its name logged once per
/// session; a name that isn't a plain word is logged as no name at all,
/// so nothing a transcript wrote beyond a type's name reaches the log.
#[test]
fn an_unknown_type_is_a_gap_and_logged_once_by_name() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish();
    let p = tracing::subscriber::with_default(subscriber, || fold(UNKNOWN_TYPE));
    let odd = "a type with spaces ghp_0123456789abcdefghijklmnopqrstuvwxyz0123";
    assert_eq!(unknown_gaps(&p), ["brand-new-record", "another-new-record", odd]);
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logged.matches("brand-new-record").count(), 1, "once per session: {logged}");
    assert_eq!(logged.matches("another-new-record").count(), 1, "{logged}");
    assert_eq!(logged.lines().count(), 3, "{logged}");
    assert!(!logged.contains("ghp_") && !logged.contains("invented") && !logged.contains("spaces"), "{logged}");
}

/// A transcript inventing types can't fill the log: the first 32 distinct
/// ones are logged, and every one is still a gap.
#[test]
fn a_session_logs_at_most_32_unknown_types() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish();
    let lines: Vec<String> = (0..40).map(|n| format!(r#"{{"type":"invented-{n}"}}"#)).collect();
    let p = tracing::subscriber::with_default(subscriber, || fold(&lines.join("\n")));
    assert_eq!(unknown_gaps(&p).len(), 40);
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logged.lines().count(), 32, "{logged}");
}
