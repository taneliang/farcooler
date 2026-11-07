//! Writer admission for the hook ingress (cutover join).
use super::*;

impl HookIngress {
    pub(crate) fn with_writer_gate(mut self, gate: Arc<crate::writer_gate::WriterGate>) -> Self { self.gate = Some(gate); self }

    pub(super) fn admit(&self) -> Result<Option<crate::writer_gate::Ticket>, ()> {
        match &self.gate { Some(g) => g.admit("legacy-hooks").map(Some).map_err(|_| ()), None => Ok(None) }
    }

    /// Ask every transcript tail to stop; the join then waits for them.
    pub(crate) fn stop_tails(&self) {
        for alive in self.tails.lock().unwrap_or_else(|e| e.into_inner()).values() { alive.store(false, Ordering::Relaxed); }
    }
}
