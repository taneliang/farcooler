//! A stream on a real tmux pane, through a death, a restart, a respawn and
//! the pane going (ov-554). Whatever tmux is on the PATH: on 3.8 and later the
//! pipe ends when the program dies and the stream has to pipe the pane again;
//! before 3.8 the pipe lives on and the same tests hold without that.
//!
//! tmux is required, and its absence fails rather than skips.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use uuid::Uuid;

use farcooler_tmux::PaneLife;
use tokio::time::Instant;

use crate::runtime::Runtime;

/// The pane's program. Says `ready <name>`; repaints a full-width row of `W`
/// on SIGWINCH; says `here` on SIGUSR2; on SIGUSR1 says its last words and
/// exits 3.
const PROGRAM: &str = r#"paint() {
  set -- $(stty size)
  printf '\033[H\033[2J'
  printf "%${2}s" '' | tr ' ' W
  printf '\r\nwinched %s %s\r\n' "$2" "$1"
}
trap paint WINCH
trap 'printf "here\r\n"' USR2
trap 'printf "last words\r\n"; exit 3' USR1
printf 'ready %s\r\n' "$1"
while :; do sleep 0.05; done
"#;

/// One terminal's pane on a tmux server of its own, killed however the test
/// ends.
struct Pane {
    runtime: Runtime,
    terminal: Uuid,
    pane: String,
    script: String,
}

impl Drop for Pane {
    fn drop(&mut self) {
        farcooler_tmux::reap_server(self.runtime.tmux.socket());
    }
}

impl Pane {
    async fn start() -> Self {
        assert!(farcooler_core::programs::find("tmux").is_some(), "these tests need tmux");
        let dir = farcooler_core::scratch::dir("stream-end");
        let script = dir.join("pane.sh");
        std::fs::write(&script, PROGRAM).unwrap();
        let script = format!("sh '{}'", script.display());
        let install = format!("test-{}", Uuid::now_v7().simple());
        let tmux = farcooler_tmux::TmuxServer::in_dir(&install, Uuid::now_v7(), &dir);
        let inventory = farcooler_tmux::LiveInventory::new(tmux.clone());
        let terminal = Uuid::now_v7();
        let window = tmux
            .create_terminal_window(Uuid::now_v7(), terminal, "t", "/tmp", &format!("{script} first"))
            .await
            .expect("a window");
        inventory.refresh().await;
        let runtime = Runtime { tmux, inventory, marks: None };
        Self { runtime, terminal, pane: window.pane_id, script }
    }

    async fn tmux(&self, args: &[&str]) -> String {
        let out = self.runtime.tmux.run(args).await.expect("tmux runs");
        assert!(out.ok(), "tmux {args:?}: {}", out.stderr);
        out.stdout.trim().to_string()
    }

    /// Signal the program.
    async fn signal(&self, signal: &str) {
        let pid = self.show("#{pane_pid}").await;
        let sent = std::process::Command::new("kill").args([signal, &pid]).status().unwrap();
        assert!(sent.success());
    }

    /// Wait until what the program writes reaches `seen` through the live
    /// pipe, and not only in the opening picture, which is all `ready` proves.
    async fn piped(&self, from: &mut (impl AsyncReadExt + Unpin), seen: &mut Vec<u8>) {
        for _ in 0..600 {
            let heres = String::from_utf8_lossy(seen).matches("here").count();
            self.signal("-USR2").await;
            let more = tokio::time::timeout(Duration::from_millis(100), read_until(from, seen, |s| s.matches("here").count() > heres));
            if more.await.is_ok() {
                return;
            }
        }
        panic!("the pipe never came up: {:?}", String::from_utf8_lossy(seen));
    }

    async fn show(&self, format: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", &self.pane, format]).await
    }

    /// End the program with its last words, and wait until tmux has the pane
    /// dead.
    async fn die(&self) {
        self.signal("-USR1").await;
        for _ in 0..6000 {
            if self.show("#{pane_dead}").await == "1" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the pane never died");
    }
}

/// Read until `done` says the bytes so far are enough, or fail after a while.
async fn read_until(from: &mut (impl AsyncReadExt + Unpin), seen: &mut Vec<u8>, done: impl Fn(&str) -> bool) {
    let mut buf = [0u8; 4096];
    while !done(&String::from_utf8_lossy(seen)) {
        let n = tokio::time::timeout(Duration::from_secs(60), from.read(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("timed out; so far: {:?}", String::from_utf8_lossy(seen)))
            .expect("read");
        assert!(n > 0, "the stream ended; so far: {:?}", String::from_utf8_lossy(seen));
        seen.extend_from_slice(&buf[..n]);
    }
}

fn marker(columns: u16, rows: u16) -> String {
    String::from_utf8(farcooler_vt::size_marker(columns, rows)).unwrap()
}

/// The card's first two lines: a watcher gets the dead program's last
/// words; after a restart the same stream carries the new program, with its
/// size announced ahead of its repaint; and once the pane goes, the stream
/// ends, with nothing left running.
#[tokio::test]
async fn a_restarted_pane_streams_again_to_the_same_watcher_and_ends_when_it_goes() {
    let t = Pane::start().await;
    let (mut client, out) = tokio::io::duplex(1 << 20);
    let streaming = tokio::spawn({
        let (runtime, terminal) = (t.runtime.clone(), t.terminal);
        async move { runtime.stream_to(terminal, true, out, std::future::pending()).await }
    });
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, |s| s.contains("ready first")).await;
    t.piped(&mut client, &mut seen).await;

    t.die().await;
    read_until(&mut client, &mut seen, |s| s.contains("last words")).await;
    assert!(!streaming.is_finished(), "the stream ended with the program, not the pane");

    // A restart: the pane respawned in place, as `Service::relaunch` does.
    t.tmux(&["respawn-pane", "-t", &t.pane, &format!("{} second", t.script)]).await;
    read_until(&mut client, &mut seen, |s| s.contains("ready second")).await;

    t.tmux(&["resize-window", "-t", &t.pane, "-x", "101", "-y", "29"]).await;
    read_until(&mut client, &mut seen, |s| s.contains("winched 101 29")).await;
    let text = String::from_utf8_lossy(&seen).into_owned();
    let announced = text.rfind(&marker(101, 29)).unwrap_or_else(|| panic!("no 101x29 marker: {text:?}"));
    let repaint = text.rfind(&"W".repeat(101)).expect("the repaint");
    assert!(announced < repaint, "the size came after the repaint: {text:?}");

    // Gone: the stream ends, cleanly.
    t.tmux(&["kill-pane", "-t", &t.pane]).await;
    let ended = tokio::time::timeout(Duration::from_secs(60), streaming).await.expect("the stream never ended");
    ended.unwrap().expect("a clean end");
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).await.unwrap();
}

/// The card's third line: `respawn-pane -k` on a live pane (a chat/terminal
/// switch) keeps the pane's pipe, so the stream goes straight on through it,
/// with no new pipe and no new picture.
#[tokio::test]
async fn a_live_pane_respawned_keeps_its_pipe() {
    let t = Pane::start().await;
    let (mut client, out) = tokio::io::duplex(1 << 20);
    let streaming = tokio::spawn({
        let (runtime, terminal) = (t.runtime.clone(), t.terminal);
        async move { runtime.stream_to(terminal, false, out, std::future::pending()).await }
    });
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, |s| s.contains("ready first")).await;
    t.piped(&mut client, &mut seen).await;
    let pictures = String::from_utf8_lossy(&seen).matches("\x1bc").count();

    t.tmux(&["respawn-pane", "-k", "-t", &t.pane, &format!("{} second", t.script)]).await;
    read_until(&mut client, &mut seen, |s| s.contains("ready second")).await;
    assert_eq!(t.show("#{pane_pipe}").await, "1", "the respawn ended the pipe");
    let text = String::from_utf8_lossy(&seen).into_owned();
    assert_eq!(text.matches("\x1bc").count(), pictures, "the stream was piped again: {text:?}");
    assert!(!streaming.is_finished());
    streaming.abort();
}

/// What a client attached over the wire is sent. While `behind` is set it
/// drops what it is offered, as a client too slow to keep up does, and it
/// catches up when `go` is notified.
#[derive(Clone, Default)]
struct Collected {
    sent: Arc<Mutex<Vec<u8>>>,
    behind: Arc<std::sync::atomic::AtomicBool>,
    go: Arc<tokio::sync::Notify>,
}

impl crate::resync::AttachSink for Collected {
    fn open(&mut self, picture: Vec<u8>) -> bool {
        self.sent.lock().unwrap().extend(crate::resync::reset_then(picture));
        true
    }
    fn output(&mut self, bytes: Vec<u8>) -> crate::resync::Taken {
        if self.behind.load(std::sync::atomic::Ordering::SeqCst) {
            return crate::resync::Taken::Behind;
        }
        self.sent.lock().unwrap().extend(bytes);
        crate::resync::Taken::Sent
    }
    fn caught_up(&self) -> impl std::future::Future<Output = bool> + Send {
        let (behind, go) = (self.behind.clone(), self.go.clone());
        async move {
            if behind.load(std::sync::atomic::Ordering::SeqCst) {
                go.notified().await;
                behind.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            true
        }
    }
    fn resync(&mut self, picture: Vec<u8>) -> bool {
        self.open(picture)
    }
}

impl Collected {
    async fn until(&self, what: &str) {
        for _ in 0..6000 {
            if self.has(what) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never sent {what:?}: {:?}", String::from_utf8_lossy(&self.sent.lock().unwrap()));
    }

    fn has(&self, what: &str) -> bool {
        String::from_utf8_lossy(&self.sent.lock().unwrap()).contains(what)
    }

    /// Until the program's output reaches the sink through the live pipe.
    async fn piped(&self, t: &Pane) {
        for _ in 0..600 {
            t.signal("-USR2").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.has("here") {
                return;
            }
        }
        panic!("the pipe never came up");
    }
}

/// The same over the wire (`Runtime::attach`): the last words, then the
/// restarted program, on one attachment, which ends when the pane goes.
#[tokio::test]
async fn a_restarted_pane_streams_again_over_the_wire() {
    let t = Pane::start().await;
    let pane = t.runtime.live_pane(t.terminal).unwrap();
    let sent = Collected::default();
    let attached = tokio::spawn({
        let (runtime, sink) = (t.runtime.clone(), sent.clone());
        async move { runtime.attach(pane, sink).await }
    });
    sent.until("ready first").await;
    sent.piped(&t).await;
    t.die().await;
    sent.until("last words").await;
    t.tmux(&["respawn-pane", "-t", &t.pane, &format!("{} second", t.script)]).await;
    sent.until("ready second").await;
    assert!(!attached.is_finished());

    t.tmux(&["kill-pane", "-t", &t.pane]).await;
    let ended = tokio::time::timeout(Duration::from_secs(60), attached).await.expect("the attachment never ended");
    ended.unwrap().expect("a clean end");
}

/// A client that has fallen behind when the pane dies (the wire path's
/// `behind` branch) is still there for the restart: the resync it is owed
/// is a picture of the new program.
#[tokio::test]
async fn a_client_behind_when_the_pane_dies_gets_the_restarted_program() {
    let t = Pane::start().await;
    let pane = t.runtime.live_pane(t.terminal).unwrap();
    let sent = Collected::default();
    let attached = tokio::spawn({
        let (runtime, sink) = (t.runtime.clone(), sent.clone());
        async move { runtime.attach(pane, sink).await }
    });
    sent.until("ready first").await;
    sent.piped(&t).await;
    sent.behind.store(true, std::sync::atomic::Ordering::SeqCst);
    t.die().await;
    t.tmux(&["respawn-pane", "-t", &t.pane, &format!("{} second", t.script)]).await;
    // Ready is said before the new pipe can be up, so it is only ever in a
    // picture: the one the resync takes once the client catches up. Caught up
    // once the respawned pane has a pipe again, so the attachment is past the
    // revival rather than racing it.
    for _ in 0..6000 {
        if t.show("#{pane_pipe}").await == "1" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sent.go.notify_one();
    sent.until("ready second").await;
    assert!(!attached.is_finished());
    attached.abort();
}

/// A `farcoolerd stream` whose client hangs up while its pane is dead ends
/// then, rather than waiting on the pane forever.
#[tokio::test]
async fn a_stream_waiting_on_a_dead_pane_ends_when_its_client_hangs_up() {
    let t = Pane::start().await;
    let (mut client, out) = tokio::io::duplex(1 << 20);
    let (hang_up, hung_up) = tokio::sync::oneshot::channel::<()>();
    let streaming = tokio::spawn({
        let (runtime, terminal) = (t.runtime.clone(), t.terminal);
        async move {
            let hangup = async {
                let _ = hung_up.await;
            };
            runtime.stream_to(terminal, true, out, hangup).await
        }
    });
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, |s| s.contains("ready first")).await;
    t.piped(&mut client, &mut seen).await;
    t.die().await;
    read_until(&mut client, &mut seen, |s| s.contains("last words")).await;
    hang_up.send(()).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(60), streaming).await.expect("the stream outlived its client");
    ended.unwrap().expect("a clean end");
}

/// Over the wire a look at a dead pane is the daemon's inventory, not a tmux
/// process: with the server gone, the inventory still reads it dead, where
/// tmux has nothing to say.
#[tokio::test]
async fn the_wire_path_reads_a_dead_pane_from_the_inventory() {
    let t = Pane::start().await;
    let pane = t.runtime.live_pane(t.terminal).unwrap();
    t.die().await;
    t.runtime.inventory.refresh().await;
    farcooler_tmux::reap_server(t.runtime.tmux.socket());
    let wire = super::Revival::reading_inventory();
    assert_eq!(wire.look(&t.runtime, &pane).await, Some(PaneLife::Dead));
    let process = super::Revival::asking_tmux();
    assert_eq!(process.look(&t.runtime, &pane).await, Some(PaneLife::Gone));
}

/// Looks at a dead pane slow down: doubling to `LONGEST`, then `DEAD_PAUSE`
/// once it has been dead for `LONG_DEAD`; and output flowing again starts
/// them afresh. The clock is the caller's, so no test waits 30 s.
#[test]
fn a_pane_dead_for_long_is_looked_at_less_often() {
    use super::Revival;
    let start = Instant::now();
    let mut revival = Revival::asking_tmux();
    let mut at = start;
    let mut first_look = None;
    let mut pauses = Vec::new();
    while at.duration_since(start) < Duration::from_secs(40) {
        at += revival.pause;
        revival.looked(Some(PaneLife::Dead), at);
        let dead_for = at.duration_since(*first_look.get_or_insert(at));
        pauses.push((dead_for, revival.pause));
    }
    let ms = |n| Duration::from_millis(n);
    assert_eq!(pauses.iter().take(5).map(|p| p.1).collect::<Vec<_>>(), [ms(100), ms(200), ms(400), ms(500), ms(500)]);
    for (dead_for, pause) in &pauses {
        if *dead_for < Revival::LONG_DEAD {
            assert!(*pause <= Revival::LONGEST, "{pause:?} after {dead_for:?} dead");
        } else {
            assert_eq!(*pause, Revival::DEAD_PAUSE, "after {dead_for:?} dead");
        }
    }
    assert!(pauses.iter().filter(|p| p.1 == Revival::DEAD_PAUSE).count() >= 3);

    revival.flowing();
    revival.looked(Some(PaneLife::Dead), at + Duration::from_secs(60));
    assert_eq!(revival.pause, ms(100), "a new death starts the looks afresh");
    // A pane tmux could not be asked about is not counted as dead for long.
    let mut unknown = Revival::asking_tmux();
    for n in 1..100 {
        unknown.looked(None, start + Duration::from_secs(n));
    }
    assert_eq!(unknown.pause, Revival::LONGEST);
}
