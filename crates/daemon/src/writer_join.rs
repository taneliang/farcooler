//! Issues the cutover join proof from the daemon's actual retained owners:
//! the writer gate (legacy scheduler, line jobs, landing jobs, hook tasks and
//! delivery passes), the workflow owner and its native notice task. Closing is
//! permanent for this process; a joined daemon restarts into recovery, never
//! back into ordinary service. Anything not joined yields no proof.
use std::collections::BTreeSet;
use std::time::Duration;
use farcooler_store::engine_cutover::{JoinedOwners, OwnedWriter, OwnerJoin};
use crate::service::Service;

const LIMIT: Duration = Duration::from_secs(30);

struct Joined { identity: &'static str, incarnation: String, resources: BTreeSet<String>, receipt: String }
impl OwnedWriter for Joined {
    fn identity(&self) -> String { self.identity.into() }
    fn incarnation(&self) -> String { self.incarnation.clone() }
    fn resources(&self) -> BTreeSet<String> { self.resources.clone() }
    /// Already joined by `join_writers` before this value existed.
    fn close_and_join(&mut self) -> Result<String, String> { Ok(self.receipt.clone()) }
}

/// Proof for a recovery process that started no writer of its own and holds the
/// daemon lock. It says nothing about processes outside this one.
pub(crate) fn recovery_proof(attempt: &str, db: &std::path::Path, protected: &BTreeSet<String>) -> Result<JoinedOwners, String> {
    let incarnation = format!("recovery:{}", std::process::id());
    let mut join = OwnerJoin::new(attempt);
    for identity in farcooler_store::engine_cutover::REQUIRED_OWNERS {
        let resources = if identity == "workflow-owner" { protected.clone() } else { BTreeSet::new() };
        join.retain(Box::new(Joined { identity, incarnation: incarnation.clone(), resources,
            receipt: format!("{identity}: recovery process started no writer") }));
    }
    join.join(db, protected).map_err(|e| e.to_string())
}

impl Service {
    /// Admission for a writer spawned outside this crate's library (the binary's helpers).
    pub fn admit_writer(&self, identity: &'static str) -> Result<crate::writer_gate::Ticket, crate::writer_gate::Closed> { self.writers.admit(identity) }

    /// Close admission everywhere, stop and join the workflow owner, drain every
    /// ticket, then issue the one-attempt proof bound to `db`.
    pub async fn join_writers(&self, attempt: &str, db: &std::path::Path, protected: &BTreeSet<String>) -> Result<JoinedOwners, String> {
        self.join_writers_within(attempt, db, protected, LIMIT).await
    }

    pub(crate) async fn join_writers_within(&self, attempt: &str, db: &std::path::Path, protected: &BTreeSet<String>, limit: Duration) -> Result<JoinedOwners, String> {
        self.hooks().stop_tails();
        let gate = self.writers.close_and_join(limit).await;
        let workflow = self.stop_workflows().await;
        gate?;
        workflow.map_err(|e| format!("workflow owner did not join: {e}"))?;
        let incarnation = format!("{}:{}", self.install_id(), std::process::id());
        let mut join = OwnerJoin::new(attempt);
        for identity in farcooler_store::engine_cutover::REQUIRED_OWNERS {
            let resources = if identity == "workflow-owner" { protected.clone() } else { BTreeSet::new() };
            join.retain(Box::new(Joined { identity, incarnation: incarnation.clone(), resources,
                receipt: format!("{identity}: admission closed and every ticket joined by this process") }));
        }
        join.join(db, protected).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_store::engine_cutover::transfer;

    async fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().join("state")).await.unwrap();
        (dir, service)
    }
    #[tokio::test]
    async fn an_idle_service_joins_and_its_proof_drives_the_atomic_transfer() {
        let (dir, service) = service().await;
        let db = dir.path().join("state/farcooler.db");
        let protected = service.store.engine_protected_resources().unwrap();
        let owners = service.join_writers_within("join-1", &db, &protected, Duration::from_secs(2)).await.unwrap();
        assert!(service.writers.admit("landing-jobs").is_err(), "admission stays closed after the join");
        assert!(service.workflow_handle().is_none());
        drop(service);
        let (store, plan) = transfer::begin(&db, &owners).unwrap();
        assert_eq!(store.engine_authority().unwrap().phase, "quiesced");
        assert_eq!(plan.registry, 67);
        assert!(farcooler_store::Store::open(&db).is_err(), "the restarted daemon is refused by the gate");
    }
    #[tokio::test]
    async fn a_running_landing_job_or_dropped_delivery_pass_blocks_the_proof() {
        let (dir, service) = service().await;
        let db = dir.path().join("state/farcooler.db");
        let protected = service.store.engine_protected_resources().unwrap();
        let held = service.writers.admit("landing-jobs").unwrap();
        let err = service.join_writers_within("join-2", &db, &protected, Duration::from_millis(100)).await.unwrap_err();
        assert!(err.contains("landing-jobs"), "{err}");
        drop(held);
        let (_dir, service) = self::service().await;
        drop(service.writers.admit("delivery").unwrap());
        let db = _dir.path().join("state/farcooler.db");
        let err = service.join_writers_within("join-3", &db, &BTreeSet::new(), Duration::from_millis(100)).await.unwrap_err();
        assert!(err.contains("delivery"), "{err}");
    }
}
