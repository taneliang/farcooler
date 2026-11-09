//! `workflow.watch`: a running engine agent's terminal, read-only (ov-972), for a person (R-111, L2).
use super::*;

impl Rpc {
    /// Attach this connection to the agent the payload names, read-only (`watch_view`). Addressed by workspace and
    /// item, never by terminal id. A person's to watch, never an agent's; and only while the caller is still the
    /// person who started it: removed, or their device released, the watch ends (`watch_view::start`'s `still`).
    pub(super) async fn workflow_watch(&self, req: Request) -> Result<result::Value> {
        let svc = &self.service;
        let workspace = Self::target(&req)?;
        svc.store.get_workspace(workspace)?;
        let Some(request::Payload::WorkflowCommand(p)) = req.payload else { return Err(DomainError::NotFound) };
        if crate::caller_seat::caller_seat(svc, self.caller_pid).await != crate::caller_seat::Seat::Person {
            return Err(DomainError::ScopeDenied { needed: "person" });
        }
        let person = self.caller()?;
        let still = {
            let (svc, peer, pid, pane, person) = (svc.clone(), self.peer.clone(), self.caller_pid, self.caller_pane.clone(), person.clone());
            move || crate::caller_person::caller_person(&svc, &peer, pid, &pane).is_ok_and(|now| now == person)
        };
        let (task, reply) = crate::watch_view::start(self.push.clone(), person, workspace, &p, still)?;
        if let Ok(mut held) = self.attachment.lock() {
            held.replace(Attachment(task));
        }
        Ok(result::Value::TerminalAttach(reply))
    }
}
