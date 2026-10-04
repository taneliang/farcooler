//! A pane whose agent stopped (ov-174): what a message for it gets back, the
//! prompts it was holding, and a restart that sends them on. Its own file to
//! keep `agent_supervisor.rs` inside its size budget.

use super::*;
use farcooler_agent::event::QueuedPrompt;
use farcooler_core::error::DomainError;

impl AgentSupervisor {
    /// Hand a message to a terminal's shim, or say why nothing got it.
    ///
    /// Two refusals, because they need opposite responses. `AgentNotConnected`
    /// is a shim still dialing, and a client is right to try again in a
    /// moment. `AgentStopped` is a pane whose shim said its agent is gone
    /// (`failure`), and nothing but a restart brings it back. Both used to be
    /// the first, so the apps retried a dead agent and the phones said they
    /// couldn't reach a runner that had answered.
    ///
    /// The queue is the exception. A stopped pane's queued prompts are still
    /// on every screen, in the last `PromptQueue` its shim sent, and they are
    /// this daemon's to edit now: the shim that held them can't take an edit,
    /// and a restart sends what's left (`restarting`). Refusing those was what
    /// left them stuck where nobody could remove them.
    pub fn deliver(&self, terminal: Uuid, message: DaemonMessage) -> Result<(), DomainError> {
        if let Some(done) = self.rekeep(terminal, &message) {
            return done;
        }
        if self.failure(terminal).is_some() {
            return match message {
                DaemonMessage::CancelQueued { id } => {
                    self.restrand(terminal, &id, |queue, at| {
                        queue.remove(at);
                    })
                }
                DaemonMessage::EditQueued { id, text } => {
                    self.restrand(terminal, &id, |queue, at| queue[at].text = text)
                }
                _ => Err(DomainError::AgentStopped),
            };
        }
        if self.send(terminal, message) {
            return Ok(());
        }
        // Read again: a failure reported between the check above and the send
        // is still a stopped agent, not a slow one.
        Err(if self.failure(terminal).is_some() {
            DomainError::AgentStopped
        } else {
            DomainError::AgentNotConnected
        })
    }

    /// The prompts this pane's shim last said were queued, in order.
    ///
    /// Read off the transcript window, where the shim's `PromptQueue` events
    /// are. The shim sends one on every change, an empty one included, so the
    /// latest is the whole queue.
    pub(super) fn stranded(&self, terminal: Uuid) -> Vec<QueuedPrompt> {
        let Ok(recent) = self.recent.lock() else { return Vec::new() };
        recent
            .get(&terminal)
            .into_iter()
            .flatten()
            .rev()
            .find_map(|s| match &s.event {
                AgentEvent::PromptQueue { items } => Some(items.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Change one stranded prompt and say so to every reader, the way the
    /// shim would have: a new `PromptQueue`, through the one place events are
    /// numbered (`record`). `NotFound` for an id that isn't queued.
    fn restrand(
        &self,
        terminal: Uuid,
        id: &str,
        change: impl FnOnce(&mut Vec<QueuedPrompt>, usize),
    ) -> Result<(), DomainError> {
        let mut items = self.stranded(terminal);
        let at = items.iter().position(|q| q.id == id).ok_or(DomainError::NotFound)?;
        change(&mut items, at);
        self.record(terminal, vec![AgentEvent::PromptQueue { items }], &|_, _| {});
        Ok(())
    }

    /// Remove or rewrite a prompt a restart is keeping for the next shim.
    ///
    /// Between the restart and the new shim's `Established` the failure is
    /// gone and no shim is there, so a Remove would get the retryable
    /// `AgentNotConnected`, and its retry would find the prompt already
    /// resent under a new id: removed, and sent anyway. Changed in `resend`
    /// itself, under the lock `resend_stranded` takes it out with, so a
    /// prompt is either removed or sent, never both. `None` for anything
    /// else, which goes on as it would have.
    fn rekeep(&self, terminal: Uuid, message: &DaemonMessage) -> Option<Result<(), DomainError>> {
        let (id, text) = match message {
            DaemonMessage::CancelQueued { id } => (id, None),
            DaemonMessage::EditQueued { id, text } => (id, Some(text)),
            _ => return None,
        };
        let items = {
            let mut sessions = self.sessions.lock().ok()?;
            let kept = &mut sessions.get_mut(&terminal)?.resend;
            let at = kept.iter().position(|q| &q.id == id)?;
            match text {
                None => {
                    kept.remove(at);
                }
                Some(text) => kept[at].text = text.clone(),
            }
            kept.clone()
        };
        self.record(terminal, vec![AgentEvent::PromptQueue { items }], &|_, _| {});
        Some(Ok(()))
    }

    /// The pane is being restarted in agent mode (`Service::set_pane_mode`).
    ///
    /// Everything `left_agent_mode` drops goes, the failure first among it:
    /// the shim that reported it was just killed. What the old shim was
    /// holding is kept for the new one, which sends it once it establishes.
    /// Called after the pane is respawned, so the old shim can't reconnect
    /// and take them first.
    pub fn restarting(&self, terminal: Uuid) {
        let queued = self.stranded(terminal);
        self.left_agent_mode(terminal);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.entry(terminal).or_default().resend = queued;
        }
    }

    /// Send a new shim what a restart kept for it, oldest first, once.
    ///
    /// Each goes as a prompt, as though just typed: the first starts a turn
    /// and the shim queues the rest behind it, which is the order they were
    /// in. Taken out before sending, so a second `Established` (a daemon
    /// link that drops and comes back) doesn't send them twice.
    pub(super) fn resend_stranded(&self, terminal: Uuid) {
        let queued = match self.sessions.lock() {
            Ok(mut sessions) => sessions.get_mut(&terminal).map(|s| std::mem::take(&mut s.resend)),
            Err(_) => None,
        };
        for prompt in queued.unwrap_or_default() {
            let sent = self.send(terminal, DaemonMessage::Prompt { text: prompt.text, images: prompt.images });
            if !sent {
                tracing::warn!(terminal = %terminal, "a prompt kept over a restart found no shim to take it");
            }
        }
    }

    /// A new shim connecting to this pane, as `serve` and its `Established`
    /// register it, for a test outside this module (`service`).
    #[cfg(test)]
    pub(crate) fn connected_for_test(&self, terminal: Uuid) -> tokio::sync::mpsc::UnboundedReceiver<DaemonMessage> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.writers.lock().unwrap().insert(terminal, tx);
        let hello = ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() };
        self.apply(terminal, hello, &|_, _| {});
        rx
    }

    /// One prompt queued on this pane, as its shim would report it.
    #[cfg(test)]
    pub(crate) fn queued_for_test(&self, terminal: Uuid, text: &str) {
        let items = vec![QueuedPrompt { id: format!("q-{text}"), text: text.into(), images: Vec::new() }];
        self.record(terminal, vec![AgentEvent::PromptQueue { items }], &|_, _| {});
    }

    /// Mark a pane's agent as stopped, as its shim would, for a test outside
    /// this module (`rpc`).
    #[cfg(test)]
    pub(crate) fn stopped_for_test(&self, terminal: Uuid) {
        self.apply(terminal, ShimMessage::Failed { failure: AgentFailure::AdapterFailed }, &|_, _| {});
    }
}
