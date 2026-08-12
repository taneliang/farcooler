//! The runtime inventory is a LIVE VIEW, not a timed cache.
//!
//! The distinction is load-bearing. A view aged by a clock would report a
//! terminal as `running` because it used to be, which is precisely what the
//! derivation design removed. A timer would have reintroduced that as a
//! performance optimization nobody would think to question.
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

    /// How long to wait before asking tmux a second time.
    ///
    /// Short, because this is not a backoff — nothing is overloaded. It is a
    /// pause long enough for a single blocked write to have been given up on by
    /// `TmuxServer::run`'s own deadline, so the retry is asking a server that is
    /// listening again rather than re-joining the same queue.
    const RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(150);

    /// Re-inventory the private server and replace the view.
    ///
    /// Asked twice before giving up. tmux answers one request at a time per
    /// server, so a pane whose program has stopped reading its stdin can block
    /// a `send-keys` and stall every read queued behind it until the command
    /// deadline fires. That is a lost race, not a broken server — but a single
    /// failed attempt here marks the inventory unavailable, and an unavailable
    /// inventory is every terminal on the machine reporting that it cannot be
    /// accounted for.
    ///
    /// One retry costs nothing in the common case, where the first attempt
    /// succeeds, and turns the great majority of those stalls into a hiccup
    /// nobody sees. It does not fix the underlying serialization — see
    /// `TmuxServer::run` — it stops one wedged pane from speaking for the whole
    /// machine.
    ///
    /// Reads only. A retried write could be performed twice, and nothing here
    /// needs one.
    pub async fn refresh(&self) -> RuntimeSnapshot {
        let mut last = match self.server.list_tagged_panes().await {
            Ok(panes) => {
                let snapshot = RuntimeSnapshot::healthy(panes);
                *self.view.write().expect("inventory lock") = snapshot.clone();
                return snapshot;
            }
            Err(e) => e,
        };

        tracing::debug!(error = %last, "tmux inventory read failed, asking once more");
        tokio::time::sleep(Self::RETRY_PAUSE).await;

        let snapshot = match self.server.list_tagged_panes().await {
            Ok(panes) => RuntimeSnapshot::healthy(panes),
            Err(e) => {
                last = e;
                // Twice in a row is no longer a lost race. The daemon serves
                // durable state with every terminal derived as `unknown` — not
                // `lost`, which would claim a finding this never made — and
                // reports the runtime as unhealthy so clients can say which
                // machine stopped answering.
                tracing::warn!(
                    error = %last,
                    "tmux inventory unavailable after a retry, deriving everything unknown"
                );
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
