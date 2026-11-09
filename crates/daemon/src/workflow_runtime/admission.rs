//! Server-only proof retained across caller-seat and bounded inbox waits.
//! No serialization, public constructor, or caller-selected principal.
use std::sync::{Arc, OnceLock};
use uuid::Uuid;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::{v1::Scope, workflow_commands::Action};
use farcooler_store::models::{Terminal, TerminalRole};
use farcooler_transport::Peer;
use crate::{caller_seat::{self, Seat}, claude_logins::Login, people::Person,
    person_access::{self, Authority}, service::Service, sessions::Session};

pub(crate) struct Admission {
    service: Arc<Service>,
    peer: Peer,
    session: Arc<Session>,
    authority: Authority,
    owner_device: Option<farcooler_store::access::OwnerDeviceState>,
    caller_pid: Option<i32>,
    incarnation: Option<i64>,
    caller_pane: Arc<OnceLock<Uuid>>,
    workspace: Uuid,
    seat: Seat,
    terminal: Option<(Terminal, Login, Option<String>)>,
    /// Whose the caller is (`caller_person`) when this was captured; every recheck must find the same.
    person: Person,
}

/// The actions an orchestrator may never take on a person's behalf: publishing a definition, and saying
/// whether a lost push landed.
pub(super) fn person_only(action: &Action) -> bool { matches!(action, Action::Publish { .. } | Action::Settle { .. }) }

impl Admission {
    /// The workspace of a command about the whole host, such as a cutover. No
    /// orchestrator matches it, so only a person can be admitted to one.
    pub(crate) const HOST_WIDE: Uuid = Uuid::nil();

    pub(crate) async fn capture(
        service: Arc<Service>, peer: Peer, session: Arc<Session>, caller_pid: Option<i32>,
        caller_pane: Arc<OnceLock<Uuid>>, workspace: Uuid,
    ) -> Result<Self> {
        // Capture access/process generations BEFORE caller_seat can await tmux.
        let authority = Authority::read(&service.store, peer.client_id.as_deref())?;
        let owner_device = match peer.client_id.as_deref() {
            Some(id) => service.store.stored_access()?.owner_devices.into_iter()
                .find_map(|(device, state)| (device == id).then_some(state)),
            None => None,
        };
        let incarnation = match caller_pid {
            Some(pid) => Some(crate::claude_registry::process_incarnation(pid).ok_or(DomainError::AuthRequired)?),
            None => None, // Existing trusted in-process/stdio construction.
        };
        let seat = caller_seat::caller_seat(&service, caller_pid).await;
        let terminal = match seat {
            Seat::Person => None,
            Seat::Agent(Some(id)) => {
                let term = service.store.get_terminal(id)?;
                if term.workspace_id != Some(workspace) || term.role != TerminalRole::Orchestrator {
                    return Err(DomainError::ScopeDenied { needed: "workspace orchestrator" });
                }
                let login = service.recorded_login(&term)?;
                service.refuse_another_leads_login(&term, workspace)?;
                let lead = service.store.workspace_lead(workspace)?;
                Some((term, login, lead))
            }
            _ => return Err(DomainError::ScopeDenied { needed: "known caller" }),
        };
        let person = crate::caller_person::caller_person(&service, &peer, caller_pid, &caller_pane)?;
        let admission = Self { service, peer, session, authority, owner_device, caller_pid, incarnation,
            caller_pane, workspace, seat, terminal, person };
        admission.recheck()?;
        Ok(admission)
    }

    pub(super) fn workspace(&self) -> String { self.workspace.to_string() }
    /// Whom the engine records and decides for. An orchestrator pane is `orchestrator`, whoever leads it;
    /// a member in person is `member:<handle>` (ov-1000), so one member's commands and receipts are never
    /// another's. Never read off the wire.
    pub(super) fn principal(&self) -> String {
        match (self.seat, &self.person) {
            (Seat::Agent(Some(_)), _) => "orchestrator".into(),
            (Seat::Person, Person::Member(handle)) => format!("member:{handle}"),
            (Seat::Person, _) => "owner".into(),
            _ => unreachable!(),
        }
    }
    /// A member in person, rather than the owner or an orchestrator pane: they read runs, and a Maintainer
    /// cancels and sends back (ov-1000), and nothing else.
    fn member_in_person(&self) -> bool { self.seat == Seat::Person && matches!(self.person, Person::Member(_)) }
    // Informational context only. Generation and all retained caller proofs bind
    // the Session guard; receipt/mutation authority is always recomputed here.
    pub(super) fn context(&self, epoch: u64) -> serde_json::Value {
        let binding = farcooler_workflow::digest(&("workflow-admission/v1", self.service.host_id.to_string(), self.workspace.to_string(),
            format!("{:?}", (&self.authority, &self.owner_device, &self.peer.client_id, self.peer.scope,
                self.caller_pid, self.incarnation, self.seat, &self.terminal, self.session.workflow_auth_generation()))));
        serde_json::json!({"binding":binding,"capabilities":{
            "workspace":self.workspace.to_string(),"epoch":epoch,"inspect":true,
            "controller":self.permits_cancel(),"decision_principal":self.principal()}})
    }
    /// For the commands an agent must not run on a person's behalf.
    pub(crate) fn require_person(&self) -> Result<()> {
        if self.seat != Seat::Person { return Err(DomainError::ScopeDenied { needed: "person" }); }
        Ok(())
    }
    /// For what stays the owner's when a member's orchestrator drives the engine (ov-999): a cutover.
    pub(crate) fn require_owner(&self) -> Result<()> {
        if crate::caller_person::caller_person(&self.service, &self.peer, self.caller_pid, &self.caller_pane)? != Person::Owner {
            return Err(DomainError::ScopeDenied { needed: "owner" });
        }
        Ok(())
    }
    pub(crate) fn recheck_now(&self) -> Result<()> { self.recheck() }
    /// The orchestrator pane this admitted, if it admitted one rather than a person.
    pub(crate) fn orchestrator(&self) -> Option<&Terminal> { self.terminal.as_ref().map(|(term, _, _)| term) }
    pub(crate) fn permits(&self, action: &Action) -> Result<()> {
        if person_only(action) && self.seat != Seat::Person {
            return Err(DomainError::ScopeDenied { needed: "person" });
        }
        if self.member_in_person() {
            // Read now, at consumption too: a role taken away while a command waits refuses it.
            let maintains = crate::people::maintains(&self.service.store, &self.person)?;
            return match action {
                Action::Get => Ok(()),
                Action::Cancel { .. } | Action::SendBack { .. } if maintains => Ok(()),
                // As the people gate answers a member's device asking for the owner's (`Ruling::Owners`).
                _ => Err(DomainError::NotYours { what: "not_yours" }),
            };
        }
        Ok(())
    }
    /// Whether this caller may cancel a run, for the apps' `controller` (informational only).
    fn permits_cancel(&self) -> bool {
        !self.member_in_person() || crate::people::maintains(&self.service.store, &self.person).unwrap_or(false)
    }

    /// Whether `person`, whose this caller is, may reach this workspace's
    /// engine at all, by what the daemon reads, never the wire. The local
    /// socket's `authority` is the owner's; a device's is its own person's.
    /// - The owner, from the owner's device or the local socket.
    /// - The orchestrator pane a member leads, on their login, while they
    ///   lead this workspace (ov-999). It is still the `orchestrator`
    ///   principal, refused what the owner's orchestrator is refused.
    /// - A member in person, from their own device or a shell pane on their
    ///   login (ov-1000): `permits` then holds them to reading runs, and a
    ///   Maintainer to cancelling and sending back too.
    fn admits(&self, person: &Person) -> bool {
        let local = self.peer.client_id.is_none() && self.authority.person == Person::Owner;
        match person {
            Person::Owner => self.authority.person == Person::Owner,
            Person::Member(handle) => match &self.terminal {
                Some((_, Login::Member { handle: login, .. }, Some(lead))) => local && login == handle && lead == handle,
                Some(_) => false,
                None => local || self.authority.person == *person,
            },
            Person::Released => false,
        }
    }

    /// The owning runtime calls this after ALL awaits, at consumption before
    /// receipt lookup/read/fresh admission. No await separates this final check
    /// from its synchronous Store transaction. Later access withdrawal cannot
    /// undo a committed receipt; reply delivery is never an admission ACK.
    pub(super) fn recheck(&self) -> Result<()> {
        let svc = &self.service;
        if self.session.is_closed() || self.session.revoked_since_open() {
            return Err(DomainError::AuthRequired);
        }
        self.authority.recheck(&svc.store, self.peer.client_id.as_deref())?;
        if let Some(id) = self.peer.client_id.as_deref() {
            // device_is_owners intentionally treats an unknown legacy id as
            // owner too. Compare the positive enrollment row instead so row
            // removal cannot silently become that legacy fallback.
            let current = svc.store.stored_access()?.owner_devices.into_iter()
                .find_map(|(device, state)| (device == id).then_some(state));
            if current != self.owner_device { return Err(DomainError::AuthRequired); }
        }
        let person = crate::caller_person::caller_person(svc, &self.peer, self.caller_pid, &self.caller_pane)?;
        if person != self.person
            || !self.admits(&person)
            || person_access::effective_scope(&svc.store, &self.peer) != self.peer.scope
            || !matches!(self.peer.scope, Scope::Control | Scope::HostAdmin)
        {
            return Err(DomainError::AuthRequired);
        }
        if self.workspace != Self::HOST_WIDE { svc.store.get_workspace(self.workspace)?; }
        let seat = match self.caller_pid {
            None => Seat::Person,
            Some(pid) => {
                if crate::claude_registry::process_incarnation(pid) != self.incarnation {
                    return Err(DomainError::AuthRequired);
                }
                // Synchronous current process walk: no tmux fallback/await at
                // the commit boundary; unreadable proof refuses closed.
                caller_seat::walk_seat(svc, pid)
            }
        };
        if seat != self.seat { return Err(DomainError::AuthRequired); }
        if let Some((term, login, lead)) = &self.terminal {
            let current = svc.store.get_terminal(term.id)?;
            // resource_version also prevents role/workspace ABA while queued.
            if &current != term || &svc.recorded_login(&current)? != login
                || &svc.store.workspace_lead(self.workspace)? != lead
            {
                return Err(DomainError::AuthRequired);
            }
            svc.refuse_another_leads_login(&current, self.workspace)?;
        }
        Ok(())
    }
}
