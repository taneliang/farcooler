//! The timing read's answer, and what a bad one never does.

use super::*;

#[test]
fn an_answer_that_is_not_the_answer_reads_as_a_miss_not_as_no_prs() {
    assert!(parse(b"not json").is_none());
    assert!(parse(br#"{"errors":[{"message":"rate limited"}]}"#).is_none());
    assert_eq!(parse(br#"{"data":{"repository":{}}}"#).map(|m| m.len()), Some(0));
}

#[test]
fn a_pr_github_returned_null_for_is_left_out_so_it_keeps_what_was_known() {
    let read = parse(br#"{"data":{"repository":{"p7":null,"p8":{"createdAt":"2026-10-01T00:00:00Z","isDraft":false}}}}"#).unwrap();
    assert!(!read.contains_key(&7) && read.contains_key(&8));
}

#[derive(Clone, Default)]
struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
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

/// A decode failure logs its kind and never the input: decoders quote theirs,
/// and an answer can carry a token.
#[test]
fn a_bad_answer_is_logged_by_kind_and_never_quoted() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish();
    let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz0123";
    tracing::subscriber::with_default(subscriber, || assert!(parse(token.as_bytes()).is_none()));
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("could not read gh's timing answer"), "it logged: {logged:?}");
    assert!(!logged.contains("ghp_") && !logged.contains("0123456789"), "{logged:?}");
}
