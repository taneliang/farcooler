use super::*;
use crate::server::Output;

#[tokio::test(flavor = "current_thread")]
async fn lost_pane_evidence_records_dead_tag_and_client_failure() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Self::Writer { self.clone() }
    }

    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = farcooler_core::scratch::dir("pane-exit-evidence");
    let terminal = Uuid::now_v7();
    let wrapper = dir.join("probe.sh");
    let tmux = TmuxServer::in_dir("evidence", terminal, &dir).with_program(wrapper.clone());
    for (script, expected) in [
        (format!("printf '1\\t{terminal}\\tpid=12 tty=/dev/test status=23 signal=9\\n'\n"), PaneLife::Dead),
        ("printf '0\\t\\tpid=12 tty=/dev/test status= signal=\\n'\n".into(), PaneLife::Gone),
    ] {
        std::fs::write(&wrapper, script).unwrap();
        assert_eq!(tmux.pane_life_with_exit_evidence("%1", terminal).await.unwrap(), expected);
    }
    std::fs::write(&wrapper, "echo 'client failed' >&2\nexit 1\n").unwrap();
    assert!(tmux.pane_life_with_exit_evidence("%1", terminal).await.is_err());
    let log = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains(&format!("1\t{terminal}\tpid=12 tty=/dev/test status=23 signal=9")), "{log}");
    assert!(log.contains("0\t\tpid=12 tty=/dev/test"), "{log}");
    assert!(log.contains("client failed"), "{log}");
    assert_eq!(log.matches("native tmux pane exit evidence before teardown").count(), 3, "{log}");
}

#[test]
fn only_an_explicit_absence_is_a_gone_pane() {
    for stderr in ["can't find pane: %99\n", "no server running on /tmp/tmux-1/test\n",
        "error connecting to /tmp/tmux-1/test (No such file or directory)\n",
        "error connecting to /tmp/tmux-1/test (Connection refused)\n"] {
        assert_eq!(probe(Output { status: Some(1), stdout: String::new(), stderr: stderr.into() }, Uuid::nil()).unwrap(), PaneLife::Gone);
    }
    for (status, stderr) in [(Some(1), "server exited unexpectedly"), (None, ""),
        (Some(1), "error connecting to /tmp/tmux-1/test (Permission denied)"), (Some(1), "") ] {
        assert!(probe(Output { status, stdout: String::new(), stderr: stderr.into() }, Uuid::nil()).is_err());
    }
}

/// A busy gate's unsuccessful tmux client is not evidence that its builder exited.
/// Use a private real pane and fail only its probe clients, then let the same program finish.
#[tokio::test]
async fn two_failed_probe_clients_leave_the_same_pane_alive_to_finish() {
    let dir = farcooler_core::scratch::dir("pane-probe-failure");
    let id = Uuid::now_v7();
    let tmux = TmuxServer::in_dir(&format!("probe-{id}"), id, &dir);
    struct Reap<'a>(&'a TmuxServer);
    impl Drop for Reap<'_> { fn drop(&mut self) { crate::reap_server(self.0.socket()); } }
    let _reap = Reap(&tmux);
    let window = tmux.create_terminal_window(id, id, "probe", dir.to_str().unwrap(),
        "exec /bin/sh -c 'while [ ! -f finish ]; do sleep 0.02; done; echo finished > result'").await.unwrap();
    let wrapper = dir.join("failed-client.sh");
    std::fs::write(&wrapper, "echo 'server exited unexpectedly' >&2\nexit 1\n").unwrap();
    let failed = tmux.clone().with_program(wrapper);
    for _ in 0..2 {
        assert!(failed.pane_life(&window.pane_id, id).await.is_err(),
            "an unsuccessful probe must be unknown, not a closed pane that makes the runner tear down its host");
        assert_eq!(tmux.pane_life(&window.pane_id, id).await.unwrap(), PaneLife::Live);
    }
    std::fs::write(dir.join("finish"), "go").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !dir.join("result").exists() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("result")).unwrap().trim(), "finished");
}
