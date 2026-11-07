//! Drains the daemon's own writers before it stops for a cutover: the writer
//! gate (legacy scheduler, line jobs, landing jobs, hook tasks and delivery
//! passes), the workflow owner and its native notice task. Closing is permanent
//! for this process. This is an orderly stop and authorizes nothing: only the
//! recovery process's kernel proof does (`engine_cutover::owners`).
use std::time::Duration;
use crate::service::Service;

const LIMIT: Duration = Duration::from_secs(30);

impl Service {
    /// Admission for a writer spawned outside this crate's library (the binary's helpers).
    pub fn admit_writer(&self, identity: &'static str) -> Result<crate::writer_gate::Ticket, crate::writer_gate::Closed> { self.writers.admit(identity) }

    /// Close admission everywhere, stop and join the workflow owner, drain every ticket.
    pub async fn drain_writers(&self) -> Result<(), String> { self.drain_writers_within(LIMIT).await }

    pub(crate) async fn drain_writers_within(&self, limit: Duration) -> Result<(), String> {
        self.hooks().stop_tails();
        let gate = self.writers.close_and_join(limit).await;
        let workflow = self.stop_workflows().await;
        gate?;
        workflow.map_err(|e| format!("workflow owner did not join: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().join("state")).await.unwrap();
        (dir, service)
    }
    #[tokio::test]
    async fn drain_closes_admission_and_joins_the_workflow_owner() {
        let (_dir, service) = service().await;
        service.drain_writers_within(Duration::from_secs(2)).await.unwrap();
        assert!(service.writers.admit("landing-jobs").is_err(), "admission stays closed after the drain");
        assert!(service.workflow_handle().is_none());
    }
    #[tokio::test]
    async fn a_running_landing_job_or_dropped_delivery_pass_fails_the_drain() {
        let (_dir, service) = service().await;
        let held = service.writers.admit("landing-jobs").unwrap();
        let err = service.drain_writers_within(Duration::from_millis(100)).await.unwrap_err();
        assert!(err.contains("landing-jobs"), "{err}");
        drop(held);
        let (_dir, service) = self::service().await;
        drop(service.writers.admit("delivery").unwrap());
        let err = service.drain_writers_within(Duration::from_millis(100)).await.unwrap_err();
        assert!(err.contains("delivery"), "{err}");
    }
    #[tokio::test]
    async fn a_live_service_blocks_the_proof_and_stopping_it_releases_the_kernel_lock() {
        use farcooler_store::engine_cutover::JoinedOwners;
        let (dir, service) = service().await;
        let db = dir.path().join("state/farcooler.db");
        assert!(JoinedOwners::acquire(&db, "a").is_err(), "a drained but open Service still owns the file");
        service.drain_writers_within(Duration::from_secs(2)).await.unwrap();
        assert!(JoinedOwners::acquire(&db, "a").is_err(), "draining is not ownership proof");
        drop(service);
        drop(JoinedOwners::acquire(&db, "a").unwrap());
    }
}
