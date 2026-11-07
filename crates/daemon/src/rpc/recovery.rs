//! The only method a recovery daemon answers: `workflow.cutover` for the
//! owner's own local socket. There is no Service here (ordinary startup is
//! refused), so everything else is an error and nothing in the request is
//! taken as authority: the caller must be a local socket client (no device,
//! host_admin) exactly as the ordinary daemon treats the owning user.
use super::*;
use farcooler_protocol::workflow_cutover::{self, Cutover};

pub struct RecoveryHandler { root: std::path::PathBuf, peer: Peer }
impl RecoveryHandler {
    pub fn new(root: std::path::PathBuf, peer: Peer) -> Self { Self { root, peer } }

    fn run(&self, req: &Request) -> Result<serde_json::Value> {
        if req.method != workflow_cutover::METHOD { return Err(DomainError::CapabilityUnsupported { needed: "an ordinary daemon; cutover recovery is in progress" }); }
        if self.peer.client_id.is_some() || self.peer.scope != farcooler_protocol::v1::Scope::HostAdmin {
            return Err(DomainError::ScopeDenied { needed: "owner" });
        }
        let Some(request::Payload::WorkflowCommand(p)) = &req.payload else { return Err(DomainError::NotFound) };
        if p.schema != 1 { return Err(DomainError::NotFound); }
        let op = workflow_cutover::parse(&p.json).map_err(|_| DomainError::NotFound)?;
        if matches!(op, Cutover::Prepare { .. }) { return Err(DomainError::Conflict { what: "prepare_needs_live_daemon" }); }
        crate::cutover_admin::recovery(&self.root, &op).map_err(|error| {
            tracing::warn!(%error, "workflow.cutover refused in recovery");
            DomainError::OperationFailed
        })
    }
}
impl Handler for RecoveryHandler {
    fn peer(&self) -> Peer { self.peer.clone() }
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let outcome = self.run(&req);
        async move {
            match outcome {
                Ok(value) => Response { request_id: req.request_id, outcome: Some(response::Outcome::Result(WireResult {
                    value: Some(result::Value::WorkflowReply(farcooler_protocol::v1::WorkflowReply { schema: 1, json: value.to_string() })) })) },
                Err(err) => error_response(req.request_id, err),
            }
        }
    }
}

/// Hold `socket` and answer only cutover recovery until a termination signal.
pub async fn serve(socket: &std::path::Path, root: std::path::PathBuf) -> std::io::Result<()> {
    let server = farcooler_transport::UnixListenerServer::bind(socket)?;
    tracing::warn!(socket = %socket.display(), "ordinary startup refused; serving cutover recovery only");
    let sessions = move |preamble: Option<farcooler_transport::SessionPreamble>, _pid: Option<i32>| {
        let peer = crate::sessions::peer_from_preamble(preamble.as_ref())?;
        Some((farcooler_transport::HandshakeConfig { daemon_version: farcooler_protocol::BUILD.to_string() }, RecoveryHandler::new(root.clone(), peer)))
    };
    let result = tokio::select! { served = server.serve_with_pid(sessions) => served, _ = crate::refusal::signaled() => Ok(()) };
    let _ = std::fs::remove_file(socket);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::{self as pb, Scope};

    fn ask(method: &str, json: &str) -> Request {
        Request { request_id: bytes::Bytes::from_static(b"r"), method: method.into(),
            payload: Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: json.into() })), ..Default::default() }
    }
    fn local() -> Peer { Peer { client_id: None, scope: Scope::HostAdmin } }
    fn reply(r: Response) -> std::result::Result<serde_json::Value, ()> {
        match r.outcome { Some(response::Outcome::Result(WireResult { value: Some(result::Value::WorkflowReply(w)) })) => Ok(serde_json::from_str(&w.json).unwrap()), _ => Err(()) }
    }

    #[tokio::test]
    async fn recovery_daemon_answers_only_owner_cutover_ops_from_database_truth() {
        let dir = tempfile::tempdir().unwrap();
        let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
        let root = service.root_dir().to_path_buf();
        let db = crate::cutover_admin::database(&root);
        let protected = service.store.engine_protected_resources().unwrap();
        let owners = service.join_writers("rec", &db, &protected).await.unwrap();
        let (frozen, _) = farcooler_store::engine_cutover::transfer::begin(&db, &owners).unwrap();
        drop((frozen, service));
        assert!(crate::service::Service::open_in(root.clone()).await.is_err(), "ordinary startup is refused");
        let handler = RecoveryHandler::new(root.clone(), local());
        let inspected = reply(handler.handle(ask("workflow.cutover", r#"{"op":"Inspect"}"#)).await).unwrap();
        assert_eq!(inspected["authority"]["phase"], "quiesced");
        assert!(inspected["ordinary_startup"].as_str().unwrap().starts_with("refused"));
        let remote = RecoveryHandler::new(root.clone(), Peer { client_id: Some("device".into()), scope: Scope::HostAdmin });
        assert!(reply(remote.handle(ask("workflow.cutover", r#"{"op":"Inspect"}"#)).await).is_err(), "a device is not the owner");
        assert!(reply(handler.handle(ask("host.get", r#"{"op":"Inspect"}"#)).await).is_err(), "no other method is served");
        for op in [r#"{"op":"Prepare","attempt":"x"}"#, r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#, r#"{"op":"Finish"}"#, r#"{"op":"Commit"}"#] {
            assert!(reply(handler.handle(ask("workflow.cutover", op)).await).is_err(), "{op} must refuse before a verified release");
        }
        farcooler_store::testing::exec_for_tests(&farcooler_store::engine_cutover::startup::open_recovery(&db).unwrap(), "UPDATE engine_cutover_authority SET phase='released', incarnation=1");
        reply(handler.handle(ask("workflow.cutover", r#"{"op":"Finish"}"#)).await).expect("finish after release");
        let after = reply(handler.handle(ask("workflow.cutover", r#"{"op":"Inspect"}"#)).await).unwrap();
        assert_eq!(after["ordinary_startup"], "Existing");
    }
}
