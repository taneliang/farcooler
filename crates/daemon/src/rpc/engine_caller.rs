//! Who may send a workspace's engine a `workflow.*` command at all (ov-999, ruling R-107).
use super::*;

impl Rpc {
    /// Who may send `workspace`'s engine `method` at all (ruling R-107):
    /// - the owner;
    /// - any member, to read its runs (`workflow.get`);
    /// - the member who leads `workspace`, from their own device or shell or
    ///   their orchestrator pane (ov-999), and a Maintainer on any workspace,
    ///   to act on its runs (`people::acts_on_runs`, ov-1000).
    ///
    /// Setting a workflow outright and settling a push stay the owner's. This
    /// only narrows: `admit` then proves who the caller is, and
    /// `Admission::permits` asks the same again when the command is used.
    pub(super) fn engine_caller(&self, workspace: Uuid, method: &str) -> Result<()> {
        use crate::people::Person;
        let person = self.caller()?;
        let owners = matches!(method, "workspace.set_workflow" | "workflow.settle");
        match &person {
            Person::Owner => Ok(()),
            Person::Member(_) if method == "workflow.get" => Ok(()),
            Person::Member(_) if !owners && crate::people::acts_on_runs(&self.service.store, &person, workspace)? => Ok(()),
            _ => Err(DomainError::ScopeDenied { needed: "owner" }),
        }
    }
}
