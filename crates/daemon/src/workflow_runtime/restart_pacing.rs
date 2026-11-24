//! Limit the first new dispatch of each run found in the outbox at startup.
use super::{Effect, EffectKind, Result, Runtime};
use std::collections::BTreeSet;
use std::time::Duration;

#[derive(Default)]
pub(super) struct RestartPacing {
    pending: BTreeSet<(String, String)>,
    next: Option<tokio::time::Instant>,
}

impl Runtime {
    pub(super) fn retain_interrupted_runs(&mut self) -> Result<()> {
        for workspace in self.store.workflow_workspaces()? {
            for stored in self.store.workflow_outbox(&workspace)? {
                if stored.active && !matches!(stored.effect.kind, EffectKind::Stop { .. }) {
                    self.restart_pacing.pending.insert((workspace.clone(), stored.effect.run));
                }
            }
        }
        Ok(())
    }
}

impl RestartPacing {
    pub(super) fn delayed(&self, workspace: &str, effect: &Effect) -> bool {
        !matches!(effect.kind, EffectKind::Stop { .. })
            && self.next.is_some_and(|next| tokio::time::Instant::now() < next)
            && self.pending.contains(&(workspace.to_owned(), effect.run.clone()))
    }

    pub(super) fn started(&mut self, workspace: &str, effect: &Effect, per_minute: usize) {
        if !matches!(effect.kind, EffectKind::Stop { .. })
            && self.pending.remove(&(workspace.to_owned(), effect.run.clone()))
        {
            let gap = Duration::from_millis(60_000u64.div_ceil(per_minute as u64));
            self.next = Some(tokio::time::Instant::now() + gap);
        }
    }
}
