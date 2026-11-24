//! A kernel process query can wait for another process to finish exec. Keep
//! queries off Tokio and its shared blocking pool, with independent capacity
//! for display scans, ancestry, and individual process reads.
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

type Job = Box<dyn FnOnce() + Send>;

pub(crate) struct Lane {
    workers: OnceLock<Option<mpsc::SyncSender<Job>>>,
    available: Mutex<usize>,
    free_sync: Condvar,
    free_async: Notify,
    width: usize,
    name: &'static str,
    deadline: Duration,
}

pub(crate) static TOPOLOGY: Lane = Lane::new("process-topology", 4, Duration::from_millis(200));
pub(crate) static DETAILS: Lane = Lane::new("process-details", 4, Duration::from_millis(100));
pub(crate) static ARGUMENTS: Lane = Lane::new("process-arguments", 4, Duration::from_millis(100));
pub(crate) static LABELS: Lane = Lane::new("process-labels", 1, Duration::from_millis(500));
// Log discovery can wait on subprocesses without using the caller-attribution lane.
pub(crate) static LOG_TOPOLOGY: Lane = Lane::new("log-topology", 2, Duration::from_millis(200));
pub(crate) static OPEN_FILES: Lane = Lane::new("process-open-files", 2, Duration::from_secs(1));

struct Place(&'static Lane);
impl Drop for Place {
    fn drop(&mut self) {
        *self.0.available.lock().unwrap() += 1;
        self.0.free_sync.notify_all();
        self.0.free_async.notify_waiters();
    }
}

impl Lane {
    const fn new(name: &'static str, width: usize, deadline: Duration) -> Self {
        Self {
            workers: OnceLock::new(),
            available: Mutex::new(0),
            free_sync: Condvar::new(),
            free_async: Notify::const_new(),
            width, name, deadline,
        }
    }

    fn workers(&self) -> Option<&mpsc::SyncSender<Job>> {
        self.workers.get_or_init(|| {
            let (send, recv) = mpsc::sync_channel::<Job>(self.width);
            let recv = Arc::new(Mutex::new(recv));
            let mut available = self.available.lock().unwrap();
            for _ in 0..self.width {
                let recv = recv.clone();
                if std::thread::Builder::new().name(self.name.into()).spawn(move || loop {
                    let job = recv.lock().unwrap().recv();
                    let Ok(job) = job else { break };
                    job();
                }).is_ok() {
                    *available += 1;
                }
            }
            (*available > 0).then_some(send)
        }).as_ref()
    }

    fn reserve(&'static self, available: &mut usize, until: Instant) -> Option<Place> {
        if Instant::now() >= until { return None; }
        *available = available.checked_sub(1)?;
        Some(Place(self))
    }

    /// For checks that must stay synchronous at a commit or signal boundary.
    /// A timed-out read keeps its worker occupied until the kernel returns.
    pub(crate) fn read<T: Send + 'static>(&'static self, read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let until = Instant::now() + self.deadline;
        let workers = self.workers()?;
        let mut available = self.available.lock().unwrap();
        let place = loop {
            if let Some(place) = self.reserve(&mut available, until) { break place; }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return None; }
            available = self.free_sync.wait_timeout(available, remaining).unwrap().0;
        };
        drop(available);
        let (send, recv) = mpsc::channel();
        workers.try_send(Box::new(move || {
            if Instant::now() >= until { return; }
            let value = read();
            let in_time = Instant::now() < until;
            drop(place);
            if in_time { let _ = send.send(value); }
        })).ok()?;
        recv.recv_timeout(until.saturating_duration_since(Instant::now())).ok()
    }

    pub(crate) async fn read_async<T: Send + 'static>(&'static self, read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let until = Instant::now() + self.deadline;
        let deadline = tokio::time::Instant::now() + self.deadline;
        let workers = self.workers()?;
        let place = loop {
            // Listen before checking capacity so a release cannot be missed.
            let freed = self.free_async.notified();
            if let Some(place) = self.reserve(&mut self.available.lock().unwrap(), until) { break place; }
            if Instant::now() >= until { return None; }
            tokio::time::timeout_at(deadline, freed).await.ok()?;
        };
        let (send, recv) = tokio::sync::oneshot::channel();
        workers.try_send(Box::new(move || {
            if Instant::now() >= until || send.is_closed() { return; }
            let value = read();
            let in_time = Instant::now() < until;
            drop(place);
            if in_time { let _ = send.send(value); }
        })).ok()?;
        tokio::time::timeout_at(deadline, recv).await.ok()?.ok()
    }
}

/// One `struct kinfo_proc` on 64-bit macOS (`<sys/sysctl.h>`), and the offsets
/// this reads from it. The `libc` crate has no definition. They were read off
/// the kernel's own answer against `ps`'s for every process on a live host
/// (all 979 agreed), and the layout has been fixed since the 64-bit ABI.
#[cfg(target_os = "macos")]
pub(crate) mod kinfo {
    pub const SIZE: usize = 648;
    pub const P_START: usize = 0;
    pub const P_FLAG: usize = 32;
    pub const P_PID: usize = 40;
    pub const E_PPID: usize = 560;
    pub const E_PGID: usize = 564;
    pub const E_TDEV: usize = 572;
    pub const E_TPGID: usize = 576;
    /// `p_flag`: the process has a controlling terminal.
    pub const P_CONTROLT: i32 = 0x2;
}

/// A retained child can already be a zombie when its owner receipt is written.
/// KERN_PROC_PID includes that child, whereas proc_pidinfo only finds live processes.
#[cfg(target_os = "macos")]
pub(crate) fn owner_start_micros(pid: i32) -> Option<i64> {
    DETAILS.read(move || {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let mut record = [0u8; kinfo::SIZE];
        let mut size = record.len();
        // SAFETY: sysctl receives valid MIB, output buffer, and in/out length pointers.
        let result = unsafe { libc::sysctl(mib.as_mut_ptr(), 4, record.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
        if result != 0 || size != record.len() || i32::from_ne_bytes(record[kinfo::P_PID..][..4].try_into().ok()?) != pid {
            return None;
        }
        let start = &record[kinfo::P_START..];
        let seconds = i64::from_ne_bytes(start[..8].try_into().ok()?);
        let micros = i32::from_ne_bytes(start[8..12].try_into().ok()?);
        if seconds <= 0 || !(0..1_000_000).contains(&micros) { return None; }
        seconds.checked_mul(1_000_000)?.checked_add(i64::from(micros))
    }).flatten()
}

#[cfg(target_os = "linux")]
pub(crate) fn owner_start_micros(pid: i32) -> Option<i64> {
    let ticks: i64 = linux_stat(pid)?.get(19)?.parse().ok()?;
    let boot: i64 = std::fs::read_to_string("/proc/stat").ok()?.lines()
        .find_map(|line| line.strip_prefix("btime ")?.trim().parse().ok())?;
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz <= 0 { return None }
    boot.checked_mul(1_000_000)?.checked_add(ticks.checked_mul(1_000_000)?.checked_div(hz)?)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn owner_start_micros(_: i32) -> Option<i64> { None }

#[cfg(target_os = "linux")]
pub(crate) fn linux_stat(pid: i32) -> Option<Vec<String>> {
    DETAILS.read(move || read_linux_stat(pid)).flatten()
}

#[cfg(target_os = "linux")]
fn read_linux_stat(pid: i32) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesized and may hold spaces; fields resume
    // after its closing parenthesis, at field 3 (state).
    let after = &text[text.rfind(')')? + 1..];
    Some(after.split_whitespace().map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::Lane;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    #[tokio::test]
    async fn width_plus_one_concurrent_reads_all_succeed() {
        static LANE: Lane = Lane::new("test-overflow", 2, Duration::from_secs(2));
        let (started, mut running) = tokio::sync::mpsc::unbounded_channel();
        let (release_sync, hold_sync) = mpsc::channel();
        let (release_async, hold_async) = mpsc::channel();
        let sync_started = started.clone();
        let sync = std::thread::spawn(move || LANE.read(move || {
            sync_started.send(()).unwrap();
            let _ = hold_sync.recv();
            1
        }));
        let asynchronous = tokio::spawn(LANE.read_async(move || {
            started.send(()).unwrap();
            let _ = hold_async.recv();
            2
        }));
        for _ in 0..2 {
            tokio::time::timeout(Duration::from_secs(1), running.recv()).await.unwrap().unwrap();
        }

        let mut overflow = std::pin::pin!(LANE.read_async(|| 3));
        poll_fn(|cx| {
            assert!(overflow.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        }).await;
        release_sync.send(()).unwrap();
        assert_eq!(overflow.await, Some(3));
        release_async.send(()).unwrap();
        assert_eq!(sync.join().unwrap(), Some(1));
        assert_eq!(asynchronous.await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn busy_lane_expires_waiters_without_running_their_reads() {
        static LANE: Lane = Lane::new("test-stuck", 1, Duration::from_millis(100));
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, hold) = mpsc::channel();
        let stuck = tokio::spawn(LANE.read_async(move || {
            started.send(()).unwrap();
            hold.recv().unwrap();
        }));
        running.await.unwrap();
        let until = Instant::now() + LANE.deadline;
        let sync = std::thread::spawn(|| {
            let started = Instant::now();
            assert_eq!(LANE.read(|| panic!("expired sync read ran")), None::<()>);
            assert!(started.elapsed() >= LANE.deadline);
        });
        assert_eq!(LANE.read_async(|| panic!("expired async read ran")).await, None::<()>);
        sync.join().unwrap();
        assert!(Instant::now() >= until);
        assert_eq!(stuck.await.unwrap(), None);
        release.send(()).unwrap();
        assert_eq!(LANE.read_async(|| 1).await, Some(1));
    }
}
