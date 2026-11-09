//! Who may send a workspace's engine a `workflow.*` command at all (ov-999).
use super::*;

impl Rpc {
    /// Who may send `workspace`'s engine a command at all: the owner, or a
    /// pane on the login of the member who leads it (ov-999). A member's
    /// device is never one, and a member reaches no workspace they don't
    /// lead. This only narrows: `admit` then proves the pane is that
    /// workspace's orchestrator on its lead's login, and refuses anything else.
    pub(super) fn engine_caller(&self, workspace: Uuid) -> Result<()> {
        match self.caller()? {
            crate::people::Person::Owner => Ok(()),
            crate::people::Person::Member(handle) if self.peer.client_id.is_none()
                && self.service.store.workspace_lead(workspace)?.as_deref() == Some(handle.as_str()) => Ok(()),
            _ => Err(DomainError::ScopeDenied { needed: "owner" }),
        }
    }
}
