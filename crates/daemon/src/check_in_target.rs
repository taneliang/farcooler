//! Durable stop reservations around runner-controlled external pane changes.

use farcooler_core::{DomainError, Result};
use uuid::Uuid;

use crate::service::Service;

impl Service {
    pub(crate) fn reserve_target_stop(&self, terminal: Uuid, token: Uuid, operation: &str) -> Result<bool> {
        use crate::claude_registry::{Kernel, Processes};
        static PROCESS_STARTED: std::sync::OnceLock<Option<i64>> = std::sync::OnceLock::new();
        let started = *PROCESS_STARTED.get_or_init(|| i32::try_from(std::process::id()).ok().and_then(|pid| Kernel.started(pid)));
        self.store.reserve_check_in_target_change(terminal, token, operation, started)
    }

    /// Retain a visible hold when a watcher cannot prove a pane incarnation.
    /// The absent-row transaction also prevents a late registration. Repeated
    /// observations do not create a new hold for the same terminal identity.
    pub(crate) fn hold_unreapable_target(&self, terminal: Uuid) -> Result<bool> {
        use crate::claude_registry::{Kernel, Processes};
        let started = i32::try_from(std::process::id()).ok().and_then(|pid| Kernel.started(pid));
        let token = Uuid::now_v7();
        if !self.store.reserve_absent_check_in_target(terminal, token, started)? { return Ok(false) }
        if !self.store.mark_check_in_target_uncertain(token)? { return Err(DomainError::ResourceConflict) }
        Ok(true)
    }

    pub(crate) fn mark_target_stop_uncertain(&self, reserved: bool, token: Uuid) {
        if reserved && let Err(error) = self.store.mark_check_in_target_uncertain(token) {
            tracing::warn!(%token, error = %error, "could not mark target stop uncertain");
        }
    }

    pub(crate) fn complete_target_stop(&self, reserved: bool, token: Uuid) -> Result<()> {
        if reserved && !self.store.complete_check_in_target_change(token)? {
            return Err(DomainError::ResourceConflict);
        }
        Ok(())
    }

    /// A completed owned operation can release its exact token even if the
    /// subsequent row write lost a version race. Preserve that write's error.
    pub(crate) fn finish_target_stop<T>(&self, reserved: bool, token: Uuid, terminal: Uuid, expected: u64, result: Result<T>) -> Result<T> {
        if !reserved { return result }
        let reconciled = match &result {
            Ok(_) => true,
            Err(DomainError::ResourceConflict | DomainError::NotFound) => match self.store.get_terminal(terminal) {
                Ok(current) => current.resource_version != expected,
                Err(DomainError::NotFound) => true,
                Err(_) => false,
            },
            Err(_) => false,
        };
        if !reconciled { return result }
        let released = self.store.release_check_in_target_change(token);
        match (result, released) {
            (Err(error), release) => {
                if !matches!(release, Ok(true)) { tracing::warn!(%token, "completed target stop needs reconciliation"); }
                Err(error)
            }
            (Ok(value), Ok(true)) => Ok(value),
            (Ok(_), Ok(false)) => Err(DomainError::ResourceConflict),
            (Ok(_), Err(error)) => Err(error),
        }
    }

}
