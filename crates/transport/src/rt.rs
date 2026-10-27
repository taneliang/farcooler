//! Spawning a task and timing one out, on whatever runs this client (ov-768).
//!
//! Natively that is tokio: `tokio::spawn` and `tokio::time::timeout`, exactly
//! what the client called before this module existed. A browser has no tokio
//! runtime and no tokio clock. Its tasks run on the page's one thread through
//! `wasm_bindgen_futures::spawn_local`, and its clock is the page's
//! `setTimeout`. The client and `farcooler-client`'s `Session` call these two
//! functions and never `tokio::spawn` or `tokio::time`, so one copy of the
//! protocol client runs in both.
//!
//! Chosen by target, not by feature: crates/web's tests build the web feature
//! natively and run it on tokio, which is the right runtime there.

use std::future::Future;
use std::time::Duration;

/// A deadline passed before the future finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline has elapsed")
    }
}

impl std::error::Error for Elapsed {}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    /// A spawned task. Dropping it detaches the task; `abort` ends it.
    pub type Task<T> = tokio::task::JoinHandle<T>;

    pub fn spawn<F>(future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future)
    }

    pub async fn timeout<F: Future>(after: Duration, future: F) -> Result<F::Output, Elapsed> {
        tokio::time::timeout(after, future).await.map_err(|_| Elapsed)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::sync::oneshot;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::JsCast;

    // The `unsafe impl Send` below is sound only because the page has one
    // thread. A build with wasm threads would make it a data race, so it is
    // refused rather than left to be discovered.
    #[cfg(target_feature = "atomics")]
    compile_error!("farcooler-transport's browser runtime assumes one thread; wasm atomics break that");

    /// The task ended without finishing: it was aborted.
    #[derive(Debug)]
    pub struct Aborted;

    /// A spawned task, shaped as tokio's `JoinHandle` is where the client
    /// reads it: awaited for its output, or `abort`ed. Dropping it detaches the
    /// task, as dropping a `JoinHandle` does.
    pub struct Task<T> {
        abort: std::sync::Mutex<Option<oneshot::Sender<()>>>,
        done: oneshot::Receiver<T>,
    }

    impl<T> Task<T> {
        pub fn abort(&self) {
            let abort = self.abort.lock().unwrap_or_else(|p| p.into_inner()).take();
            if let Some(abort) = abort {
                let _ = abort.send(());
            }
        }
    }

    impl<T> Future for Task<T> {
        type Output = Result<T, Aborted>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.done).poll(cx).map(|r| r.map_err(|_| Aborted))
        }
    }

    pub fn spawn<F>(future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (abort, aborted) = oneshot::channel::<()>();
        let (finished, done) = oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            // Only a sent abort ends the task. A dropped handle closes the
            // channel instead, and that is a detach, so it waits forever.
            let aborted = async {
                if aborted.await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                biased;
                _ = aborted => {}
                output = future => {
                    let _ = finished.send(output);
                }
            }
        });
        Task { abort: std::sync::Mutex::new(Some(abort)), done }
    }

    /// The page's `setTimeout`, as a future. `Send` by assertion: see the
    /// `compile_error!` above for why that is true here.
    struct Sleep(wasm_bindgen_futures::JsFuture);

    // SAFETY: wasm32 without atomics has exactly one thread (refused above
    // otherwise), so nothing can move this to another.
    unsafe impl Send for Sleep {}

    impl Future for Sleep {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            Pin::new(&mut self.0).poll(cx).map(|_| ())
        }
    }

    fn sleep(after: Duration) -> Sleep {
        let millis = after.as_millis().min(i32::MAX as u128) as f64;
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            // `setTimeout` from the global object, so a worker has it as a
            // window does. A page without one never wakes the sleep, which is
            // the same as no deadline: the call still ends with its link.
            let global = js_sys::global();
            if let Ok(set_timeout) = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout"))
                && let Ok(set_timeout) = set_timeout.dyn_into::<js_sys::Function>()
            {
                let _ = set_timeout.call2(&global, &resolve, &JsValue::from_f64(millis));
            }
        });
        Sleep(wasm_bindgen_futures::JsFuture::from(promise))
    }

    pub async fn timeout<F: Future>(after: Duration, future: F) -> Result<F::Output, Elapsed> {
        tokio::select! {
            biased;
            output = future => Ok(output),
            _ = sleep(after) => Err(Elapsed),
        }
    }
}

pub use imp::{Task, spawn, timeout};
#[cfg(target_arch = "wasm32")]
pub use imp::Aborted;
