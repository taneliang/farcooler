//! Filesystem inspection and status scans belong on workers, including cache misses and rollback.
use super::*;

/// Poll the whole operation on a blocking worker, preserving its result and safety checks.
/// Waiting on git still uses the daemon runtime's I/O driver, while synchronous filesystem
/// walks and JSON/diff construction never occupy the RPC executor.
pub(super) async fn run<F, Fut, T>(svc: Arc<Service>, work: F) -> Result<T>
where
    F: FnOnce(Arc<Service>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>>,
    T: Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(work(svc)))
        .await.map_err(|error| {
            tracing::warn!(%error, "RPC status worker failed");
            DomainError::OperationFailed
        })?
}

/// Wait for fresh evidence before deciding whether the typed name is required.
/// Failure never permits removal.
pub(super) async fn needs_confirmation(svc: Arc<Service>, id: Uuid) -> Result<bool> {
    run(svc, move |svc| async move { svc.removal_needs_confirmation(id).await }).await
}
