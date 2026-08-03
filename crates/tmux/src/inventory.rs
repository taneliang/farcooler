//! The runtime inventory is a LIVE VIEW, not a timed cache.
//!
//! The distinction is load-bearing. A view aged by a clock would report a
//! terminal as `running` because it used to be, which is precisely what the
//! derivation design removed. A timer would have reintroduced that as a
//! performance optimisation nobody would think to question.
//!
//! So the view updates when tmux says something changed: pane exit, window
//! close, session close. A low-frequency full reconcile runs as a backstop
//! against a missed notification, and any divergence it finds is LOGGED AS A
//! DEFECT rather than silently corrected, because silent correction would hide
//! exactly the bug the backstop exists to catch.

use std::sync::{Arc, RwLock};

use farcooler_core::inventory::{RuntimeInventory, RuntimeSnapshot};

use crate::server::TmuxServer;

#[derive(Clone)]
pub struct LiveInventory {
    server: TmuxServer,
    view: Arc<RwLock<RuntimeSnapshot>>,
}

impl LiveInventory {
    pub fn new(server: TmuxServer) -> Self {
        Self {
            server,
            // Until the first successful refresh we know nothing, and "nothing
            // known" must never read as "alive".
            view: Arc::new(RwLock::new(RuntimeSnapshot::unavailable())),
        }
    }

    pub fn server(&self) -> &TmuxServer {
        &self.server
    }

    /// Re-inventory the private server and replace the view.
    pub async fn refresh(&self) -> RuntimeSnapshot {
        let snapshot = match self.server.list_tagged_panes().await {
            Ok(panes) => RuntimeSnapshot::healthy(panes),
            Err(e) => {
                // If the private tmux server cannot be inventoried safely, the
                // daemon serves durable state with every terminal derived as
                // `lost` and shows a visible degraded state rather than guessing.
                tracing::warn!(error = %e, "tmux inventory unavailable, deriving everything lost");
                RuntimeSnapshot::unavailable()
            }
        };
        *self.view.write().expect("inventory lock") = snapshot.clone();
        snapshot
    }

    /// Backstop reconcile. Divergence is a defect in the notification path, so
    /// it is reported rather than quietly fixed.
    pub async fn backstop_reconcile(&self) {
        let before = self.snapshot();
        let after = self.refresh().await;

        if before.inventory_healthy && after.inventory_healthy && before != after {
            tracing::error!(
                before_panes = before.panes.len(),
                after_panes = after.panes.len(),
                "DEFECT: backstop reconcile found inventory divergence, a control-mode \
                 notification was missed"
            );
        }
    }
}

impl RuntimeInventory for LiveInventory {
    fn snapshot(&self) -> RuntimeSnapshot {
        self.view.read().expect("inventory lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn an_unrefreshed_view_never_claims_life() {
        let inv = LiveInventory::new(TmuxServer::new("test-unused", Uuid::from_u128(1)));
        let snap = inv.snapshot();
        assert!(
            !snap.inventory_healthy,
            "before the first refresh, nothing is proved alive"
        );
        assert!(snap.panes.is_empty());
    }
}
