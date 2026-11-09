//! Who may send a workspace's engine a `workflow.*` command at all (ov-999, ov-1000).
use super::*;

impl Rpc {
    /// Who may send `workspace`'s engine `method` at all:
    /// - the owner;
    /// - a pane on the login of the member who leads `workspace` (ov-999),
    ///   never a member's device;
    /// - any member, to read its runs (`workflow.get`), and a Maintainer to
    ///   cancel and send back (ov-1000), the way they keep workspace settings.
    ///
    /// A member reaches no other command, and no workspace they don't lead
    /// but for those. This only narrows: `admit` then proves who the caller
    /// is, and `Admission::permits` holds a member to the same list when the
    /// command is used.
    pub(super) fn engine_caller(&self, workspace: Uuid, method: &str) -> Result<()> {
        use crate::people::Person;
        let person = self.caller()?;
        let Person::Member(handle) = &person else {
            return if person == Person::Owner { Ok(()) } else { Err(DomainError::ScopeDenied { needed: "owner" }) };
        };
        if self.peer.client_id.is_none() && self.service.store.workspace_lead(workspace)?.as_deref() == Some(handle.as_str()) {
            return Ok(());
        }
        match method {
            "workflow.get" => Ok(()),
            "workflow.cancel" | "workflow.send_back" if crate::people::maintains(&self.service.store, &person)? => Ok(()),
            "workflow.cancel" | "workflow.send_back" => Err(DomainError::NotYours { what: "not_yours" }),
            _ => Err(DomainError::ScopeDenied { needed: "owner" }),
        }
    }
}
