use super::*;
use crate::server::Output;

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
