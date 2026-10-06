//! The host-wide process reads a sampling tick makes, and when it makes none.
//!
//! The watcher samples once a second. Each sample used to read the whole
//! process table and then every listening socket on the host, whether or not
//! the daemon had a single pane to attribute them to: an idle daemon (and an
//! orphaned one) kept two walks a second going for nothing, and a handful of
//! daemons on one Mac kept the load in the hundreds.
//!
//! So this is the one place that decides whether to walk:
//!
//! - No live pane, no walk. Nothing reads the table without a pane to ask
//!   about, and both answers are the empty value, which every reader already
//!   handles (a pane that is absent from the table runs nothing).
//! - The process table is read in-process (`proc_table`), so a tick with panes
//!   spawns no `ps`.
//! - Listening sockets are asked about only for the processes under the panes,
//!   and not again until those processes change or `PORTS_EVERY` passes. A
//!   dev server's port appears when its process starts, which changes the set;
//!   one that binds a socket later is seen within `PORTS_EVERY`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use farcooler_core::inventory::RuntimeSnapshot;

use crate::foreground::Foreground;

/// How long a socket reading is trusted while the processes it was taken for
/// are unchanged.
const PORTS_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Sockets {
    at: Option<Instant>,
    pids: Vec<i32>,
    ports: HashMap<i32, Vec<u16>>,
}

/// What a tick learned about the host, and the memory that keeps the next
/// tick from repeating it.
#[derive(Default)]
pub struct HostWalk {
    sockets: Mutex<Sockets>,
    /// Table reads and socket reads this walk has made. A test seam: an idle
    /// daemon must leave both at zero.
    table_reads: AtomicUsize,
    socket_reads: AtomicUsize,
}

impl HostWalk {
    /// `(table reads, socket reads)` made so far.
    #[cfg(test)]
    pub fn reads(&self) -> (usize, usize) {
        (self.table_reads.load(Ordering::Relaxed), self.socket_reads.load(Ordering::Relaxed))
    }

    /// The foreground table and the listening ports by process group, for this
    /// tick's panes.
    pub async fn read(&self, snapshot: &RuntimeSnapshot) -> (Foreground, HashMap<i32, Vec<u16>>) {
        if snapshot.panes.iter().all(|p| p.dead) {
            *self.sockets.lock().unwrap_or_else(|e| e.into_inner()) = Sockets::default();
            return (Foreground::default(), HashMap::new());
        }
        self.table_reads.fetch_add(1, Ordering::Relaxed);
        let foreground = crate::foreground::read().await;

        let mut pids: Vec<i32> = snapshot
            .panes
            .iter()
            .filter(|p| !p.dead)
            .flat_map(|p| foreground.under_tty(p.tty.trim_start_matches("/dev/")))
            .collect();
        pids.sort_unstable();
        pids.dedup();

        let cached = {
            let held = self.sockets.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = held.at.is_some_and(|at| at.elapsed() < PORTS_EVERY);
            (fresh && held.pids == pids).then(|| held.ports.clone())
        };
        let ports = match cached {
            Some(ports) => ports,
            None if pids.is_empty() => HashMap::new(),
            None => {
                self.socket_reads.fetch_add(1, Ordering::Relaxed);
                // Off the executor: a blocking `Command::output`, and
                // `farcooler-core` has no async runtime to borrow.
                let list = pids.clone();
                let ports = tokio::task::spawn_blocking(move || farcooler_core::ports::listening_ports_of(&list))
                    .await
                    .unwrap_or_default();
                *self.sockets.lock().unwrap_or_else(|e| e.into_inner()) =
                    Sockets { at: Some(Instant::now()), pids, ports: ports.clone() };
                ports
            }
        };
        let by_group = foreground.ports_by_group(&ports);
        (foreground, by_group)
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;

    use farcooler_core::inventory::TaggedPane;
    use uuid::Uuid;

    use super::*;

    /// A program running in the foreground of a pty of its own, as a pane's
    /// program does. Killed on drop, by the pid this started.
    struct Pane {
        child: std::process::Child,
        _master: OwnedFd,
        tty: String,
    }

    impl Drop for Pane {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn pane_running(program: &str, args: &[&str]) -> Pane {
        // SAFETY: plain libc pty setup on descriptors this function owns; every
        // return value is checked.
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0, "posix_openpt");
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let name = std::ffi::CStr::from_ptr(libc::ptsname(master)).to_str().unwrap().to_string();
            let slave = libc::open(std::ffi::CString::new(name.clone()).unwrap().as_ptr(), libc::O_RDWR);
            assert!(slave >= 0, "open the pty slave");
            let slave = OwnedFd::from_raw_fd(slave);
            let mut command = std::process::Command::new(program);
            command
                .args(args)
                .stdin(std::process::Stdio::from(slave.try_clone().unwrap()))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let fd = slave.as_raw_fd();
            // The child becomes a session leader whose controlling terminal is
            // the pty, so its group is the terminal's foreground group: what a
            // shell does for the command typed into a pane.
            command.pre_exec(move || {
                if libc::setsid() < 0 || libc::ioctl(fd, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
            let child = command.spawn().expect("spawn the pane's program");
            Pane { child, _master: OwnedFd::from_raw_fd(master), tty: name }
        }
    }

    fn tagged(tty: &str, dead: bool) -> TaggedPane {
        TaggedPane {
            daemon_id: Uuid::nil(),
            worktree_id: Uuid::nil(),
            terminal_id: Uuid::nil(),
            schema_version: 1,
            pane_id: "%1".into(),
            window_id: "@1".into(),
            columns: 80,
            rows: 24,
            left: 0,
            top: 0,
            window_active: true,
            pane_active: true,
            zoomed: false,
            tty: tty.to_string(),
            dead,
            dead_status: None,
            dead_signal: None,
            command: "sleep".into(),
            title: String::new(),
        }
    }

    /// No pane, no read: neither the table nor the sockets, however many
    /// ticks go by. A pane that has exited and is only retained counts as no
    /// pane.
    #[tokio::test]
    async fn a_host_with_no_live_pane_reads_nothing() {
        let walk = HostWalk::default();
        for _ in 0..3 {
            let (foreground, ports) = walk.read(&RuntimeSnapshot::healthy(Vec::new())).await;
            assert!(foreground.pane("ttys001").is_none() && ports.is_empty());
            walk.read(&RuntimeSnapshot::healthy(vec![tagged("/dev/ttys001", true)])).await;
            walk.read(&RuntimeSnapshot::unavailable()).await;
        }
        assert_eq!(walk.reads(), (0, 0));
    }

    /// The pane's foreground program is named from the in-process table, with
    /// its arguments, and the sockets are not asked about again while the
    /// processes under the pane are the same.
    #[tokio::test]
    async fn a_live_pane_names_its_program_and_asks_about_sockets_once() {
        let pane = pane_running("sleep", &["31"]);
        let snapshot = RuntimeSnapshot::healthy(vec![tagged(&pane.tty, false)]);
        let walk = HostWalk::default();
        let name = pane.tty.trim_start_matches("/dev/").to_string();

        let (foreground, _) = walk.read(&snapshot).await;
        let running = foreground.pane(&name).expect("the pane's program is its tty's foreground");
        assert_eq!(running.pid, pane.child.id() as i32);
        assert!(running.command.starts_with("sleep"), "{running:?}");
        assert!(running.command.contains("31"), "its arguments are kept: {running:?}");

        walk.read(&snapshot).await;
        walk.read(&snapshot).await;
        assert_eq!(walk.reads(), (3, 1), "three ticks, one socket read");
    }

    /// A change in the processes under the panes is read at once, not after
    /// the cache runs out: a second pane's tree is a different pid set.
    #[tokio::test]
    async fn a_changed_set_of_processes_asks_about_sockets_again() {
        let first = pane_running("sleep", &["33"]);
        let walk = HostWalk::default();
        let one = RuntimeSnapshot::healthy(vec![tagged(&first.tty, false)]);
        walk.read(&one).await;
        walk.read(&one).await;
        assert_eq!(walk.reads(), (2, 1), "the same processes, one socket read");

        let second = pane_running("sleep", &["34"]);
        let two = RuntimeSnapshot::healthy(vec![tagged(&first.tty, false), tagged(&second.tty, false)]);
        walk.read(&two).await;
        assert_eq!(walk.reads(), (3, 2), "a new pane's processes are a new question");
    }

    /// The in-process table agrees with `ps` about the pane: the same pid is
    /// the tty's foreground, with the same label.
    #[tokio::test]
    async fn the_in_process_table_agrees_with_ps() {
        let pane = pane_running("sleep", &["32"]);
        let name = pane.tty.trim_start_matches("/dev/").to_string();
        let ours = crate::foreground::parse(&crate::proc_table::snapshot().expect("in-process table"));
        let ps = tokio::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,pgid=,tty=,stat=,args="])
            .output()
            .await
            .unwrap();
        let theirs = crate::foreground::parse(&String::from_utf8_lossy(&ps.stdout));
        assert_eq!(ours.pane(&name), theirs.pane(&name));
        assert!(ours.pane(&name).is_some());
    }
}
