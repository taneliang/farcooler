//! The only method a recovery daemon answers: `workflow.cutover` for the
//! owner's own local socket. There is no Service here (ordinary startup is
//! refused), so everything else is an error and nothing in the request is
//! taken as authority. The ordinary daemon knows a member's or rep's pane from
//! its Store and inventory; this one cannot, so it is stricter: the caller must
//! be a local socket client with no device, host_admin, and a person outside
//! every pane (`caller_seat::recovery_seat`), asked again on every request
//! against the process that connected.
use super::*;
use crate::caller_seat::{Seat, recovery_seat};
use farcooler_protocol::workflow_cutover;

/// Who is at the far end of a connection; a parameter so tests can name a seat.
pub type SeatOf = fn(i32) -> Seat;
/// The caller's process start, to tell the process that connected from a pid
/// reused since.
pub type StartOf = fn(i32) -> Option<i64>;

/// The exit status that asks a supervisor for a restart: launchd and systemd
/// restart a daemon only on failure, and the next start is ordinary service.
pub const RESTART_EXIT: i32 = 75;

pub struct RecoveryHandler { root: std::path::PathBuf, peer: Peer, caller: Option<(i32, Option<i64>)>, seat_of: SeatOf, start_of: StartOf, done: std::sync::Arc<tokio::sync::Notify> }
impl RecoveryHandler {
    pub fn new(root: std::path::PathBuf, peer: Peer, pid: Option<i32>) -> Self {
        Self::with_walk(root, peer, pid, recovery_seat, crate::claude_registry::process_incarnation)
    }
    pub(crate) fn with_walk(root: std::path::PathBuf, peer: Peer, pid: Option<i32>, seat_of: SeatOf, start_of: StartOf) -> Self {
        Self { root, peer, caller: pid.map(|pid| (pid, start_of(pid))), seat_of, start_of, done: Default::default() }
    }
    fn ending(mut self, done: std::sync::Arc<tokio::sync::Notify>) -> Self { self.done = done; self }
    fn caller_is_a_person(&self) -> bool {
        let Some((pid, Some(start))) = self.caller else { return false };
        (self.start_of)(pid) == Some(start) && (self.seat_of)(pid) == Seat::Person
    }

    /// Pid, terminal and session leader of the caller, for the journal.
    fn caller_note(&self) -> String {
        let Some((pid, start)) = self.caller else { return "unknown caller".into() };
        // SAFETY: getsid reads another process's session id; it changes nothing.
        let session = unsafe { libc::getsid(pid) };
        let tty = std::process::Command::new("ps").args(["-o", "tty=", "-p", &pid.to_string()]).output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| "?".into());
        format!("pid {pid} (started {}), tty {tty}, session leader {session}", start.map_or("?".to_string(), |s| s.to_string()))
    }

    fn run(&self, req: &Request) -> Result<serde_json::Value> {
        if req.method != workflow_cutover::METHOD { return Err(DomainError::CutoverHeld); }
        if self.peer.client_id.is_some() || self.peer.scope != farcooler_protocol::v1::Scope::HostAdmin {
            return Err(DomainError::ScopeDenied { needed: "owner" });
        }
        if !self.caller_is_a_person() { return Err(DomainError::ScopeDenied { needed: "person" }); }
        let Some(request::Payload::WorkflowCommand(p)) = &req.payload else { return Err(DomainError::NotFound) };
        if p.schema != 1 { return Err(DomainError::NotFound); }
        let op = workflow_cutover::parse(&p.json).map_err(|_| DomainError::NotFound)?;
        crate::cutover_admin::recovery(&self.root, &op, &self.caller_note()).map_err(|error| {
            tracing::warn!(%error, "workflow.cutover refused in recovery");
            // `PageRefused` is the one error that carries a sentence over the
            // wire; the owner needs the holders named to act on a refusal.
            DomainError::PageRefused { said: error.said() }
        })
    }
}
impl Handler for RecoveryHandler {
    fn peer(&self) -> Peer { self.peer.clone() }
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let outcome = self.run(&req);
        let done = self.done.clone();
        // Past these there is nothing left for recovery to do. A moment's grace
        // lets the reply reach the caller, as a live Prepare does.
        let finished = outcome.is_ok() && matches!(&req.payload, Some(request::Payload::WorkflowCommand(p))
            if matches!(workflow_cutover::parse(&p.json), Ok(workflow_cutover::Cutover::Recover | workflow_cutover::Cutover::Finish)));
        async move {
            if finished {
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    done.notify_one();
                });
            }
            match outcome {
                Ok(value) => Response { request_id: req.request_id, outcome: Some(response::Outcome::Result(WireResult {
                    value: Some(result::Value::WorkflowReply(farcooler_protocol::v1::WorkflowReply { schema: 1, json: value.to_string() })) })) },
                Err(err) => error_response(req.request_id, err),
            }
        }
    }
}

/// Hold `socket` and answer only cutover recovery until a termination signal,
/// or until an attempt ends. True means it ended, and the next start is ordinary.
pub async fn serve(socket: &std::path::Path, root: std::path::PathBuf) -> std::io::Result<bool> {
    let server = farcooler_transport::UnixListenerServer::bind(socket)?;
    tracing::warn!(socket = %socket.display(), "ordinary startup refused; serving cutover recovery only");
    let done = std::sync::Arc::new(tokio::sync::Notify::new());
    let ended = done.clone();
    let sessions = move |preamble: Option<farcooler_transport::SessionPreamble>, pid: Option<i32>| {
        let peer = crate::sessions::peer_from_preamble(preamble.as_ref())?;
        Some((farcooler_transport::HandshakeConfig { daemon_version: farcooler_protocol::BUILD.to_string() }, RecoveryHandler::new(root.clone(), peer, pid).ending(ended.clone())))
    };
    let result = tokio::select! {
        served = server.serve_with_pid(sessions) => served.map(|()| false),
        _ = crate::refusal::signaled() => Ok(false),
        _ = done.notified() => Ok(true),
    };
    let _ = std::fs::remove_file(socket);
    result
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
