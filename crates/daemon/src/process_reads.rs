//! A kernel process query can wait for another process to finish exec. Keep
//! queries off Tokio and its shared blocking pool, with independent capacity
//! for display scans, ancestry, and individual process reads.
use std::sync::{
    OnceLock, Mutex, mpsc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

pub(crate) struct Lane {
    workers: OnceLock<Option<mpsc::SyncSender<Job>>>,
    available: AtomicUsize,
    width: usize,
    name: &'static str,
    deadline: Duration,
}

pub(crate) static TOPOLOGY: Lane = Lane::new("process-topology", 4, Duration::from_millis(200));
pub(crate) static DETAILS: Lane = Lane::new("process-details", 4, Duration::from_millis(100));
pub(crate) static ARGUMENTS: Lane = Lane::new("process-arguments", 4, Duration::from_millis(100));
pub(crate) static LABELS: Lane = Lane::new("process-labels", 1, Duration::from_millis(500));
pub(crate) static OWNER: Lane = Lane::new("process-owner", 2, Duration::from_millis(500));

struct Place(&'static AtomicUsize);
impl Drop for Place {
    fn drop(&mut self) { self.0.fetch_add(1, Ordering::Release); }
}

impl Lane {
    const fn new(name: &'static str, width: usize, deadline: Duration) -> Self {
        Self { workers: OnceLock::new(), available: AtomicUsize::new(0), width, name, deadline }
    }

    fn submit(&'static self, read: impl FnOnce(Place) + Send + 'static) -> Option<()> {
        let workers = self.workers.get_or_init(|| {
            let (send, recv) = mpsc::sync_channel::<Job>(self.width);
            let recv = std::sync::Arc::new(Mutex::new(recv));
            for _ in 0..self.width {
                let recv = recv.clone();
                if std::thread::Builder::new().name(self.name.into()).spawn(move || loop {
                    let job = recv.lock().unwrap().recv();
                    let Ok(job) = job else { break };
                    job();
                }).is_ok() {
                    self.available.fetch_add(1, Ordering::Release);
                }
            }
            (self.available.load(Ordering::Acquire) > 0).then_some(send)
        }).as_ref()?;
        self.available.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1)).ok()?;
        let place = Place(&self.available);
        let started = Instant::now();
        workers.try_send(Box::new(move || {
            if started.elapsed() < self.deadline { read(place); }
        })).ok()
    }

    /// For checks that must stay synchronous at a commit or signal boundary.
    /// A timed-out read keeps its worker occupied until the kernel returns.
    pub(crate) fn read<T: Send + 'static>(&'static self, read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (send, recv) = mpsc::channel();
        self.submit(move |place| {
            let value = read();
            drop(place);
            let _ = send.send(value);
        })?;
        recv.recv_timeout(self.deadline).ok()
    }

    pub(crate) async fn read_async<T: Send + 'static>(&'static self, read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (send, recv) = tokio::sync::oneshot::channel();
        self.submit(move |place| {
            let value = read();
            drop(place);
            let _ = send.send(value);
        })?;
        tokio::time::timeout(self.deadline, recv).await.ok()?.ok()
    }
}
