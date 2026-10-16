//! What `gh search prs` printed, when it is not what the daemon reads, is
//! logged by the kind of failure and never quoted (ov-315): a decoder's
//! message quotes its input, and a failed `gh` call prints tokens and
//! repository names.
//!
//! Its own test binary, alone in its process, because a log capture races
//! every other test that reaches the same call site on another thread.

use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Sink {
    type Writer = Sink;
    fn make_writer(&self) -> Sink {
        self.clone()
    }
}

#[test]
fn a_bad_answer_is_logged_by_kind_and_never_quoted() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        // The daemon's own default: a test at `debug` would pass against a
        // log that says nothing in production.
        .with_max_level(tracing::Level::INFO)
        .finish();
    let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
    let repository = "acme/secret-billing-service";
    tracing::subscriber::with_default(subscriber, || {
        for bad in [
            format!("{{\"message\": \"Bad credentials {token}\"}}"),
            format!("[{{\"number\": \"{token}\", \"repository\": \"{repository}\"}}]"),
            format!("gh: could not resolve {repository} with {token}"),
        ] {
            assert_eq!(farcooler_daemon::pr_review::parse(bad.as_bytes()), None);
        }
    });
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("could not read"), "it should still say something: {logged:?}");
    for quoted in [token, repository, "Bad credentials"] {
        assert!(!logged.contains(quoted), "{quoted} reached the log: {logged}");
    }
}
