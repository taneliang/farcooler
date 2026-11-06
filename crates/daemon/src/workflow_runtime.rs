//! Opt-in isolated coding fixture runtime. No installed-app or Service caller.
//! The local capability boundary is intended for authenticated embedding, not wire grants.
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use farcooler_store::{Store, workflow::{Receipt, WorkflowError}};
use farcooler_workflow::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tokio::sync::{mpsc, oneshot};

mod adapters;
use adapters::{Binding, Journal};
pub use adapters::coding_definition;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("runtime I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime store: {0}")]
    Store(#[from] WorkflowError),
    #[error("runtime encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("runtime refused: {0}")]
    Refused(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Never serialized or accepted from a request body. Authority is local to one open.
pub struct Controller { instance: Uuid }
pub struct DecisionSession { instance: Uuid, principal: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub workspace: String,
    pub expected_revision: u64,
    pub sequence: u64,
    pub command: Command,
}

/// Bounded local inbox. Capability checks still occur on the owning runtime.
#[derive(Clone)]
pub struct CommandHandle { send: mpsc::Sender<Inbox> }
enum Authority { Controller(Uuid), Decision(Uuid, String) }
enum Inbox {
    Apply(Authority, Box<Request>, i64, oneshot::Sender<Result<Receipt>>),
    State(String, oneshot::Sender<Result<Option<State>>>),
    Reconcile,
}
impl CommandHandle {
    pub async fn submit(&self, controller: &Controller, request: Request, now_ms: i64) -> Result<Receipt> {
        self.apply(Authority::Controller(controller.instance), request, now_ms).await
    }
    pub async fn decide(&self, session: &DecisionSession, request: Request, now_ms: i64) -> Result<Receipt> {
        self.apply(Authority::Decision(session.instance, session.principal.clone()), request, now_ms).await
    }
    async fn apply(&self, authority: Authority, request: Request, now_ms: i64) -> Result<Receipt> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::Apply(authority, Box::new(request), now_ms, send)).await
            .map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
    pub async fn state(&self, workspace: &str) -> Result<Option<State>> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::State(workspace.into(), send)).await
            .map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
    /// Trusted runner signals that a delayed local receipt is available; never supplies a result.
    pub async fn reconcile(&self) -> Result<()> {
        self.send.send(Inbox::Reconcile).await.map_err(|_| Error::Refused("runtime inbox closed"))
    }
}

pub struct Runtime {
    root: PathBuf,
    store: Store,
    instance: Uuid,
    resource: String,
    base: String,
    last_occupancy: Option<String>,
    inbox: mpsc::Receiver<Inbox>,
    send: mpsc::Sender<Inbox>,
    next_lease: Option<i64>,
    #[cfg(test)]
    sweeps: usize,
    // Kept after all state/process owners; dropping the runtime releases ownership.
    _writer: File,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // Concurrent fork/pre-exec in another thread may briefly inherit a CLOEXEC
        // descriptor. Explicit unlock ends our ownership even during that window.
        if let Err(error) = rustix::fs::flock(&self._writer, rustix::fs::FlockOperation::Unlock) {
            tracing::warn!(%error, "could not unlock isolated workflow writer");
        }
    }
}
impl Runtime {
    /// Only creates a fresh private fixture; cannot point at an existing repository/DB.
    pub async fn create(root: &Path) -> Result<Self> {
        std::fs::create_dir(root)?;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
        std::fs::create_dir(root.join("receipts"))?;
        std::fs::create_dir(root.join("repo"))?;
        let repo = root.join("repo");
        adapters::git(&repo, &["init", "--quiet"]).await?;
        adapters::git(&repo, &["-c", "user.name=Workflow Fixture", "-c", "user.email=fixture@localhost",
            "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", "fixture base"]).await?;
        let base = adapters::git(&repo, &["rev-parse", "HEAD"]).await?;
        adapters::git(&repo, &["update-ref", "refs/heads/landed", &base]).await?;
        durable(&root.join("fixture.json"), &serde_json::to_vec(&base)?)?;
        Self::open(root)
    }
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let metadata = std::fs::metadata(&root)?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Refused("fixture root must be private"));
        }
        let writer = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(root.join("writer.lock"))?;
        rustix::fs::flock(&writer, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error::Refused("fixture already has a writer"))?;
        for name in ["repo", "receipts"] {
            if root.join(name).canonicalize()? != root.join(name) {
                return Err(Error::Refused("fixture path is an alias"));
            }
        }
        for name in ["fixture.json", "workflow.sqlite", "workflow.sqlite-wal", "workflow.sqlite-shm", "repo/.git"] {
            let path = root.join(name);
            if path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink() || (m.is_file() && m.nlink() > 1)) {
                return Err(Error::Refused("fixture file is an alias"));
            }
        }
        if !root.join("repo/.git").is_dir() { return Err(Error::Refused("fixture git directory is external")); }
        let base: String = serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?;
        if base.len() != 40 || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Refused("fixture base must be a full SHA"));
        }
        let resource = digest(&("local-fixture/v1", root.join("repo")));
        let store = Store::open(root.join("workflow.sqlite"))
            .map_err(|_| Error::Refused("cannot open fixture database"))?;
        let (send, inbox) = mpsc::channel(32);
        Ok(Self { inbox, send, next_lease: None,
            #[cfg(test)] sweeps: 0,
            root, store, instance: Uuid::now_v7(), resource, base, last_occupancy: None, _writer: writer })
    }
    pub fn command_handle(&self, controller: &Controller) -> Result<CommandHandle> {
        self.authorize(controller.instance)?;
        Ok(CommandHandle { send: self.send.clone() })
    }
    pub fn resource(&self) -> &str { &self.resource }
    pub fn controller(&self) -> Controller { Controller { instance: self.instance } }
    pub fn decision_session(&self, controller: &Controller, principal: &str) -> Result<DecisionSession> {
        self.authorize(controller.instance)?;
        if !valid_id(principal) || principal.len() > 100 { return Err(Error::Refused("invalid principal")); }
        Ok(DecisionSession { instance: self.instance, principal: principal.into() })
    }
    fn authorize(&self, instance: Uuid) -> Result<()> {
        if instance == self.instance { Ok(()) } else { Err(Error::Refused("foreign local capability")) }
    }
    pub fn state(&self, workspace: &str) -> Result<Option<State>> {
        Ok(self.store.workflow_state(workspace)?)
    }
    pub fn notices(&self, workspace: &str, after_revision: u64)
        -> Result<Vec<farcooler_store::workflow::WorkflowNotice>>
    {
        Ok(self.store.workflow_notices(workspace, after_revision)?)
    }
    pub fn events(&self, workspace: &str) -> Result<Vec<Input>> {
        Ok(self.store.workflow_events(workspace)?)
    }
    pub fn submit(&mut self, controller: &Controller, request: Request, now_ms: i64) -> Result<Receipt> {
        self.authorize(controller.instance)?;
        match &request.command {
            Command::Enqueue { definition, resource, .. } => {
                if resource != &self.resource { return Err(Error::Refused("noncanonical resource")); }
                adapters::validate(definition)?;
            }
            Command::Cancel { .. } => {}
            _ => return Err(Error::Refused("not a controller command")),
        }
        self.apply(request, "controller", Grant::Controller, now_ms)
    }
    pub fn decide(&mut self, session: &DecisionSession, request: Request, now_ms: i64) -> Result<Receipt> {
        self.authorize(session.instance)?;
        if !matches!(request.command, Command::Approve { .. } | Command::Review { .. }) {
            return Err(Error::Refused("not a decision"));
        }
        self.apply(request, &format!("decision:{}", session.principal),
            Grant::Decision { principal: session.principal.clone() }, now_ms)
    }
    fn apply(&self, request: Request, source: &str, grant: Grant, now_ms: i64) -> Result<Receipt> {
        let epoch = self.state(&request.workspace)?.map_or(1, |s| s.epoch);
        Ok(self.store.workflow_apply(&Input { schema: 1, id: request.id, workspace: request.workspace,
            expected_revision: request.expected_revision, epoch, source: source.into(),
            source_sequence: request.sequence, now_ms, grant, command: request.command,
            busy_resources: BTreeSet::new() })?)
    }
    fn internal(&self, workspace: &str, command: Command, grant: Grant, now_ms: i64) -> Result<Receipt> {
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let sequence = state.source_sequences.get("runtime").copied().unwrap_or(0) + 1;
        self.apply(Request { id: format!("runtime:{}", Uuid::now_v7()), workspace: workspace.into(),
            expected_revision: state.revision, sequence, command }, "runtime", grant, now_ms)
    }
    /// One owned sweep. This API has no delivery cursor, informational ACK or controller dependency.
    pub async fn pump(&mut self, now_ms: i64) -> Result<usize> {
        #[cfg(test)] { self.sweeps += 1; }
        self.next_lease = None;
        let mut count = 0;
        let workspaces = self.store.workflow_workspaces()?;
        let mut occupancy = Vec::new();
        for workspace in &workspaces {
            if let Some(state) = self.state(workspace)? {
                for (id, item) in state.items.iter().filter(|(_, i)| owns_resource(i)) {
                    occupancy.push((workspace.clone(), id.clone(), item.resource.clone(), item.run.clone()));
                }
            }
        }
        let fingerprint = digest(&occupancy);
        let released_or_recovered = self.last_occupancy.as_ref() != Some(&fingerprint);
        self.last_occupancy = Some(fingerprint);
        for workspace in workspaces {
            let state = self.state(&workspace)?.ok_or(Error::Refused("missing state"))?;
            if released_or_recovered && state.items.values().any(|i| i.phase == Phase::Blocked) {
                // Store stamps actual cross-workspace occupancy in the same transaction.
                self.internal(&workspace, Command::Advance, Grant::Controller, now_ms)?;
            }
            for stored in self.store.workflow_outbox(&workspace)? {
                if !stored.active { continue; }
                let effect = stored.effect;
                let state = self.state(&workspace)?.ok_or(Error::Refused("missing state"))?;
                let item = &state.items[&effect.item];
                if item.pending.as_ref() != Some(&effect.id) { continue; }
                if matches!(stored.dispatch_state.as_str(), "admitted" | "uncertain") {
                    let binding = Binding::new(&workspace, &effect, stored.resource_generation
                        .ok_or(Error::Refused("admitted effect lacks generation"))?, item.iteration, &self.item_base(&workspace, &effect)?);
                    let outcome = match self.read_journal(&binding) {
                        Ok(journal) => journal.map(|j| j.outcome).unwrap_or(Outcome::Uncertain),
                        Err(error) => {
                            tracing::warn!(effect = %effect.id, %error, "invalid recovery receipt");
                            Outcome::Uncertain
                        }
                    };
                    if stored.dispatch_state == "uncertain" && outcome == Outcome::Uncertain { continue; }
                    self.observe(&workspace, &effect, outcome, stored.dispatch_state == "uncertain", now_ms)?;
                    count += 1;
                    continue;
                }
                if !matches!(item.phase, Phase::Running | Phase::Cancelling) { continue; }
                let Some(claim) = self.store.workflow_claim_effect(&workspace, &effect.id,
                    &format!("runtime:{}", self.instance), state.revision, state.epoch,
                    now_ms, now_ms.saturating_add(30_000))? else {
                    if let Some(deadline) = stored.lease_until.filter(|t| *t > now_ms) {
                        self.next_lease = Some(self.next_lease.map_or(deadline, |t| t.min(deadline)));
                    }
                    continue;
                };
                // Persist the base while ownership is held, before any external admission.
                self.pin_base(&workspace, &effect).await?;
                if !self.store.workflow_admit_effect(&claim, now_ms)? { continue; }
                let binding = Binding::new(&workspace, &effect, claim.resource_generation, item.iteration, &self.item_base(&workspace, &effect)?);
                // Any I/O failure beyond admission is ambiguous, not a retry permission.
                let outcome = match self.execute(&workspace, &binding).await {
                    Ok(outcome) => {
                        self.write_journal(&Journal { binding, outcome: outcome.clone() })?;
                        outcome
                    }
                    Err(error) => {
                        tracing::warn!(effect = %effect.id, %error, "workflow effect requires reconciliation");
                        Outcome::Uncertain
                    }
                };
                self.observe(&workspace, &effect, outcome, false, now_ms)?;
                count += 1;
            }
        }
        Ok(count)
    }
    fn observe(&self, workspace: &str, effect: &Effect, outcome: Outcome, resolving: bool, now_ms: i64) -> Result<()> {
        let command = if resolving {
            Command::Resolve { item: effect.item.clone(), effect: effect.id.clone(), run: effect.run.clone(),
                fence: effect.fence, outcome }
        } else {
            Command::Observe { item: effect.item.clone(), effect: effect.id.clone(), run: effect.run.clone(),
                fence: effect.fence, outcome }
        };
        let grant = if resolving { Grant::Controller } else {
            Grant::Operation { effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence }
        };
        let receipt = self.internal(workspace, command, grant, now_ms)?;
        if receipt.result.is_err() { return Err(Error::Refused("result admission refused")); }
        Ok(())
    }
    fn receive(&mut self, message: Inbox) {
        match message {
            Inbox::Apply(authority, request, time, reply) => {
                let request = *request;
                let result = match authority {
                    Authority::Controller(instance) => self.submit(&Controller { instance }, request, time),
                    Authority::Decision(instance, principal) =>
                        self.decide(&DecisionSession { instance, principal }, request, time),
                };
                // A dropped caller never rolls back an accepted durable command.
                let _ = reply.send(result);
            }
            Inbox::State(workspace, reply) => { let _ = reply.send(self.state(&workspace)); }
            Inbox::Reconcile => {}
        }
    }
    pub async fn run(&mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        loop {
            if *shutdown.borrow() { return Ok(()); }
            // Bounded drain prevents a busy sender from starving effects/shutdown.
            for _ in 0..32 {
                let Ok(message) = self.inbox.try_recv() else { break; };
                self.receive(message);
            }
            if self.pump(now()).await? > 0 { continue; }
            loop {
                let deadline = self.next_lease;
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                    }
                    message = self.inbox.recv() => {
                        let Some(message) = message else { return Ok(()); };
                        let read_only = matches!(message, Inbox::State(..));
                        self.receive(message);
                        if !read_only { break; }
                    }
                    _ = async {
                        match deadline {
                            Some(time) => tokio::time::sleep(Duration::from_millis(
                                time.saturating_sub(now()).max(0) as u64)).await,
                            None => std::future::pending::<()>().await,
                        }
                    } => break,
                }
            }
        }
    }
}
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis().min(i64::MAX as u128) as i64
}
fn durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(path)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    File::open(path.parent().ok_or(Error::Refused("missing parent"))?)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
