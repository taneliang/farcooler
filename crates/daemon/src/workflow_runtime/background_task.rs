use super::{Error, Outcome, Result};

pub(super) type Slot = Option<std::pin::Pin<Box<dyn std::future::Future<Output = Result<Outcome>> + Send>>>;

/// An effect, on its own task once the loop has polled it (ov-1375).
///
/// The loop polls its effects only while it waits in `select!`. Its other work (a round of `lsof +D` over
/// every engine tree, about 20 seconds on a busy runner) left each effect's git unwatched past its
/// deadline, to be killed though it had finished in milliseconds. So the loop's first poll, which starts
/// the effect as before (merely admitting one still says Starting), hands it to a task of its own, which
/// keeps going whatever the loop is doing.
///
/// The task only polls the effect; the slot still owns it. Dropping the slot drops the effect there and
/// then, as before, so a sweep cut off mid-effect (a crash, in the tests) releases its locks and kills its
/// children before anything opens the runtime again, not whenever an aborted task is next scheduled.
pub(super) fn effect_slot(effect: impl std::future::Future<Output = Result<Outcome>> + Send + 'static) -> Slot {
    Some(Box::pin(BackgroundTask::new(effect)))
}
type Work<T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>;
pub(super) struct BackgroundTask<T> {
    work: std::sync::Arc<std::sync::Mutex<Option<Work<T>>>>,
    task: Option<tokio::task::JoinHandle<Option<Result<T>>>>,
}
impl<T: Send + 'static> BackgroundTask<T> {
    pub(super) fn new(work: impl std::future::Future<Output = Result<T>> + Send + 'static) -> Self {
        Self { work: std::sync::Arc::new(std::sync::Mutex::new(Some(Box::pin(work)))), task: None }
    }
    pub(super) fn start(&mut self) {
        // Cleanup starts here so even its first poll runs outside dispatch.
        let work = self.work.clone();
        self.task = Some(tokio::spawn(std::future::poll_fn(move |cx| Self::poll_work(&work, cx))));
    }
    /// One poll of the work, which is gone once it finishes or its owner is dropped.
    fn poll_work(work: &std::sync::Mutex<Option<Work<T>>>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<Result<T>>>
    {
        let mut work = work.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(running) = work.as_mut() else { return std::task::Poll::Ready(None) };
        let polled = running.as_mut().poll(cx);
        if polled.is_ready() { *work = None; }
        polled.map(Some)
    }
}
impl<T: Send + 'static> std::future::Future for BackgroundTask<T> {
    type Output = Result<T>;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if self.task.is_none() {
            if let std::task::Poll::Ready(result) = Self::poll_work(&self.work, cx) {
                return std::task::Poll::Ready(result.unwrap_or(Err(Error::Refused("workflow task dropped"))));
            }
            self.start();
        }
        let task = self.task.as_mut().expect("spawned above");
        std::pin::Pin::new(task).poll(cx).map(|joined| match joined {
            Ok(Some(outcome)) => outcome,
            // A panic in the work ends the loop, as it did when the loop polled it.
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Ok(None) | Err(_) => Err(Error::Refused("workflow task dropped")),
        })
    }
}
impl<T> Drop for BackgroundTask<T> {
    fn drop(&mut self) {
        let work = self.work.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        drop(work);
        if let Some(task) = &self.task { task.abort(); }
    }
}
/// Resolves with the first finished future and its position, without the `futures` crate.
pub(super) async fn next_finished(futures: &mut [Slot])
    -> (usize, Result<Outcome>)
{
    std::future::poll_fn(|cx| {
        for (index, slot) in futures.iter_mut().enumerate() {
            if let Some(future) = slot {
                if let std::task::Poll::Ready(result) = future.as_mut().poll(cx) {
                    return std::task::Poll::Ready((index, result));
                }
            }
        }
        std::task::Poll::Pending
    }).await
}
