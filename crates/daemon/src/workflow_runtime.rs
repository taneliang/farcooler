//! Workflow runtime for private fixtures and the sole ordinary Service owner.
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
mod coding;
mod service_owner;
mod coding_execute;
mod coding_native;
mod coding_landing;
mod coding_verbs;
mod coding_settle;
mod coding_pr;
pub use coding::{AGENT_REVIEWER, CodingConfigExt,CodingConfig, CodingCommand, CodingHarness, CodingReviewer, CodingCi, CodingLanding, CodingPullRequest, LaunchFile};
pub use coding_execute::{CodingReviewReceipt, CodingCiReceipt, write_agent_verdict};
mod wire_commands;
mod named_tree;
pub use named_tree::{RepoLocks, WorktreeHost};
mod admission;
pub(crate) use admission::Admission;
#[cfg(test)]
mod admission_tests;
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
    /// A refusal the caller can act on, said as the runner says it for a worktree (ov-967): a taken branch or
    /// name, or one that isn't valid. Every other workflow error reaches the caller as not found.
    #[error("runtime: {0}")]
    Domain(farcooler_core::error::DomainError),
}
/// How many effects may run at once. Waiting on a person or on CI is not an effect.
pub const DEFAULT_EFFECT_CAP: usize = 4;
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
    Wire(Box<Admission>, Box<farcooler_protocol::workflow_commands::Action>, oneshot::Sender<Result<serde_json::Value>>),
    /// Bring cards under the engine with no run: (workspace, item, column).
    Track(Vec<(String, String, String)>, oneshot::Sender<Result<usize>>),
    /// Move a tracker-only ticket to a column: (workspace, item, column).
    MoveTicket(String, String, String, oneshot::Sender<Result<bool>>),
    Reconcile,
}
impl CommandHandle {
    pub(crate) fn is_closed(&self) -> bool { self.send.is_closed() }
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
    /// Move a tracker-only ticket to `column` on the orchestrator's say. False when the item is not a ticket
    /// at rest (a run owns it), so the caller can say so.
    pub async fn move_ticket(&self, workspace: &str, item: &str, column: &str) -> Result<bool> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::MoveTicket(workspace.into(), item.into(), column.into(), send)).await
            .map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
    /// Bring cards under the engine as tracker-only tickets, each in the column it is in. Returns how many were taken.
    pub async fn track(&self, tickets: Vec<(String, String, String)>) -> Result<usize> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::Track(tickets, send)).await.map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
    /// Trusted runner signals that a delayed local receipt is available; never supplies a result.
    pub async fn reconcile(&self) -> Result<()> {
        self.send.send(Inbox::Reconcile).await.map_err(|_| Error::Refused("runtime inbox closed"))
    }
}

/// Everything one effect needs to run, and nothing a sweep owns. Cheap to clone, so each
/// concurrent effect gets its own copy with its own item's configuration (ov-952).
#[derive(Clone)]
pub struct Exec {
    root: PathBuf,
    store: std::sync::Arc<Store>,
    service_mode: bool,
    resource: String,
    base: String,
    coding: Option<CodingConfig>,
    /// When this copy was started, by the sweep's clock and by the monotonic one, so a wait measures
    /// its patience on the same clock its effect was created on.
    launched: Option<(i64, std::time::Instant)>,
    /// Set by the Service: makes a run's named worktree one the app lists (ov-967).
    pub host: Option<WorktreeHost>,
}
impl Exec {
    /// The sweep's clock, moved on by the time this effect has been running.
    pub(super) fn clock_ms(&self) -> i64 {
        match self.launched {
            Some((at, started)) => at.saturating_add(i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)),
            None => now(),
        }
    }
    /// Each item holds its own resource, so items in one repository never wait on each other
    /// for it. What serializes work on the repository is its own refs, not this name (ov-952).
    pub(crate) fn item_resource(workspace: &str, item: &str) -> String {
        digest(&("workflow-item/v1", workspace, item))
    }
    /// Items enqueued before per-item resources hold the repository's resource instead.
    pub(crate) fn resource_matches(&self, workspace: &str, effect: &Effect) -> bool {
        effect.resource == self.resource || effect.resource == Self::item_resource(workspace, &effect.item)
            // A group holds the landing target for everyone who lands on it.
            || self.coding.as_ref().and_then(|c| c.landing_resource()).is_some_and(|r| r == effect.resource)
    }
    /// An item's definition against what the configuration pins: a group item runs the group's steps.
    pub(super) fn validate_item_definition(&self, item: &Item) -> Result<()> {
        use coding::PinnedDefinition;
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        if item.group_of.is_some() { config.validate_pinned_group(&item.definition) } else { config.validate_pinned_definition(&item.definition) }
    }
    pub fn state(&self, workspace: &str) -> Result<Option<State>> {
        Ok(self.store.workflow_state(workspace)?)
    }
}
impl std::ops::Deref for Runtime { type Target = Exec; fn deref(&self) -> &Exec { &self.exec } }
impl std::ops::DerefMut for Runtime { fn deref_mut(&mut self) -> &mut Exec { &mut self.exec } }

pub struct Runtime {
    exec: Exec,
    pub(crate) notice_wake: Option<std::sync::Arc<tokio::sync::Notify>>,
    configuration_locks: std::collections::BTreeMap<PathBuf, File>,
    shutdown: Option<tokio::sync::watch::Receiver<bool>>,
    instance: Uuid,
    last_occupancy: Option<String>,
    inbox: mpsc::Receiver<Inbox>,
    send: mpsc::Sender<Inbox>,
    next_lease: Option<i64>,
    effect_cap: usize,
    continuous: bool,
    #[cfg(test)]
    sweeps: usize,
    /// Test hook: make the next result write fail, and count effects joined after a failed sweep.
    #[cfg(test)]
    fail_observe: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    drained: usize,
    // Kept after all state/process owners; dropping the runtime releases ownership.
    _writer: File,
    _repository_writer: Option<File>,
    _landing_writer: Option<File>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // Concurrent fork/pre-exec in another thread may briefly inherit a CLOEXEC
        // descriptor. Explicit unlock ends our ownership even during that window.
        if let Err(error) = rustix::fs::flock(&self._writer, rustix::fs::FlockOperation::Unlock) {
            tracing::warn!(%error, "could not unlock isolated workflow writer");
        }
        for file in [&self._repository_writer, &self._landing_writer].into_iter().flatten() {
            let _ = rustix::fs::flock(file, rustix::fs::FlockOperation::Unlock);
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
        let coding_path = root.join("coding.json");
        if coding_path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink() || m.nlink() > 1) {
            return Err(Error::Refused("coding configuration is an alias"));
        }
        let coding: Option<CodingConfig> = if coding_path.try_exists()? {
            let config: CodingConfig = serde_json::from_slice(&std::fs::read(&coding_path)?)?;
            config.validate()?;
            Some(config)
        } else { None };
        let repository_writer = if let Some(config) = &coding {
            let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
                .mode(0o600).custom_flags(libc::O_NOFOLLOW)
                .open(config.repository.join(".git/farcooler-workflow.lock"))?;
            if file.metadata()?.nlink() != 1 { return Err(Error::Refused("repository writer lock is aliased")); }
            rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| Error::Refused("repository already has a configured writer"))?;
            Some(file)
        } else { None };
        let landing_writer = if let Some(identity) = coding.as_ref().and_then(|c| c.landing.as_ref()) {
            let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
                .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(identity.repository.join("farcooler-workflow.lock"))?;
            if file.metadata()?.nlink() != 1 { return Err(Error::Refused("landing writer lock is aliased")); }
            rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| Error::Refused("landing repository already has a configured writer"))?;
            Some(file)
        } else { None };
        for name in if coding.is_some() { vec!["receipts"] } else { vec!["repo", "receipts"] } {
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
        if coding.is_none() && !root.join("repo/.git").is_dir() { return Err(Error::Refused("fixture git directory is external")); }
        let base: String = match &coding {
            Some(config) => config.base.clone(),
            None => serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?,
        };
        if base.len() != 40 || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Refused("fixture base must be a full SHA"));
        }
        let resource = match &coding {
            Some(config) => digest(&("configured-repository/v1", &config.repository)),
            None => digest(&("local-fixture/v1", root.join("repo"))),
        };
        let store = Store::open(root.join("workflow.sqlite"))
            .map_err(|_| Error::Refused("cannot open fixture database"))?;
        let (send, inbox) = mpsc::channel(32);
        let runtime = Self { inbox, send, next_lease: None, effect_cap: DEFAULT_EFFECT_CAP, continuous: false,
            #[cfg(test)] sweeps: 0, #[cfg(test)] fail_observe: Default::default(), #[cfg(test)] drained: 0,
            exec: Exec { root, store: std::sync::Arc::new(store), service_mode: false, resource, base, coding, launched: None, host: None },
            notice_wake: None, configuration_locks: Default::default(), shutdown: None, instance: Uuid::now_v7(), last_occupancy: None,
            _writer: writer, _repository_writer: repository_writer, _landing_writer: landing_writer };
        runtime.validate_native_host()?;
        Ok(runtime)
    }
    /// Most effects (builds, checks, reviews, CI) that run at once across all workspaces.
    pub fn set_effect_cap(&mut self, cap: usize) { self.effect_cap = cap.max(1); }
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
    pub fn notices(&self, workspace: &str, after_revision: u64)
        -> Result<Vec<farcooler_store::workflow::WorkflowNotice>>
    {
        Ok(self.store.workflow_notices(workspace, after_revision)?)
    }
    pub fn events(&self, workspace: &str) -> Result<Vec<Input>> {
        Ok(self.store.workflow_events(workspace)?)
    }
    pub fn submit(&self, controller: &Controller, request: Request, now_ms: i64) -> Result<Receipt> {
        self.authorize(controller.instance)?;
        match &request.command {
            Command::Enqueue { definition, resource, .. } => {
                let item = match &request.command { Command::Enqueue { item, .. } => item, _ => unreachable!() };
                if resource != &self.resource && resource != &Exec::item_resource(&request.workspace, item) {
                    return Err(Error::Refused("noncanonical resource"));
                }
                self.validate_definition(definition)?;
            }
            Command::Cancel { .. } | Command::Retry { .. } | Command::Queue { .. } | Command::Leave { .. } | Command::Move { .. } => {}
            _ => return Err(Error::Refused("not a controller command")),
        }
        self.apply(request, "controller", Grant::Controller, now_ms)
    }
    pub fn decide(&self, session: &DecisionSession, request: Request, now_ms: i64) -> Result<Receipt> {
        self.authorize(session.instance)?;
        if !matches!(request.command, Command::Approve { .. } | Command::Review { .. }) {
            return Err(Error::Refused("not a decision"));
        }
        self.apply(request, &format!("decision:{}", session.principal),
            Grant::Decision { principal: session.principal.clone() }, now_ms)
    }
    fn apply(&self, request: Request, source: &str, grant: Grant, now_ms: i64) -> Result<Receipt> {
        let epoch = self.state(&request.workspace)?.map_or(1, |s| s.epoch);
        let receipt = self.store.workflow_apply(&Input { schema: 1, id: request.id, workspace: request.workspace,
            expected_revision: request.expected_revision, epoch, source: source.into(),
            source_sequence: request.sequence, now_ms, grant, command: request.command,
            busy_resources: BTreeSet::new() })?;
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(receipt)
    }
    fn internal(&self, workspace: &str, command: Command, grant: Grant, now_ms: i64) -> Result<Receipt> {
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let sequence = state.source_sequences.get("runtime").copied().unwrap_or(0) + 1;
        self.apply(Request { id: format!("runtime:{}", Uuid::now_v7()), workspace: workspace.into(),
            expected_revision: state.revision, sequence, command }, "runtime", grant, now_ms)
    }
    /// One owned sweep. This API has no delivery cursor, informational ACK or controller dependency.
    /// Effects run concurrently, at most `effect_cap` at a time. A plain sweep starts what is
    /// ready now and waits for it; the service loop also starts follow-on effects as each ends.
    pub async fn pump(&mut self, now_ms: i64) -> Result<usize> {
        #[cfg(test)] { self.sweeps += 1; }
        self.next_lease = None;
        // The inbox is read while effects run, so cancels and decisions are never held up.
        let (_, placeholder) = mpsc::channel(1);
        let mut inbox = std::mem::replace(&mut self.inbox, placeholder);
        let result = self.sweep(now_ms, &mut inbox).await;
        self.inbox = inbox;
        result
    }
    async fn sweep(&mut self, now_ms: i64, inbox: &mut mpsc::Receiver<Inbox>) -> Result<usize> {
        let mut running: Vec<Running> = Vec::new();
        let mut futures: Vec<Slot> = Vec::new();
        let result = self.sweep_loop(now_ms, inbox, &mut running, &mut futures).await;
        if result.is_err() {
            // One failed write must not kill its siblings mid-effect: stop them, wait for
            // each to end, and keep what each one learned so recovery can read it.
            for job in &running { job.cancel.send_replace(true); }
            while !running.is_empty() {
                let (index, outcome) = next_finished(&mut futures).await;
                let job = running.remove(index);
                futures.remove(index);
                #[cfg(test)] { self.drained += 1; }
                if let Ok(outcome) = outcome {
                    if let Err(error) = self.write_journal(&Journal { binding: job.binding, outcome }) {
                        tracing::warn!(effect = %job.effect.id, %error, "workflow settlement could not be retained");
                    }
                }
            }
        }
        result
    }
    async fn sweep_loop(&mut self, now_ms: i64, inbox: &mut mpsc::Receiver<Inbox>,
        running: &mut Vec<Running>, futures: &mut Vec<Slot>) -> Result<usize>
    {
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
        for workspace in &workspaces {
            let state = self.state(workspace)?.ok_or(Error::Refused("missing state"))?;
            if released_or_recovered && state.items.values().any(|i| i.phase == Phase::Blocked) {
                // Store stamps actual cross-workspace occupancy in the same transaction.
                self.internal(workspace, Command::Advance, Grant::Controller, now_ms)?;
            }
        }
        let mut stopping = false;
        let mut scan = true;
        loop {
            if scan {
                scan = self.continuous;
                self.launch_ready(&workspaces, now_ms, running, futures, &mut count).await?;
            }
            if running.is_empty() { break; }
            let mut shutdown = self.shutdown.clone();
            let event = tokio::select! {
                (index, result) = next_finished(futures) => Event::Finished(index, result),
                _ = async {
                    match &mut shutdown {
                        Some(signal) => { if !*signal.borrow() { let _ = signal.changed().await; } },
                        None => std::future::pending::<()>().await,
                    }
                }, if !stopping => Event::Shutdown,
                message = inbox.recv() => Event::Message(message),
            };
            match event {
                Event::Shutdown => {
                    stopping = true;
                    for job in running.iter() { job.cancel.send_replace(true); }
                }
                Event::Message(message) => {
                    if let Some(message) = message {
                        self.receive(message).await;
                        // A command may have started work, and a long wait must not hold it back.
                        scan = scan || self.continuous;
                        for job in running.iter() {
                            if self.state(&job.workspace)?.is_some_and(|s| s.items.get(&job.effect.item)
                                .is_some_and(|i| i.phase == Phase::Cancelling)) {
                                job.cancel.send_replace(true);
                            }
                        }
                    }
                }
                Event::Finished(index, result) => {
                    let job = running.remove(index);
                    futures.remove(index);
                    let (workspace, effect, binding) = (job.workspace, job.effect, job.binding);
                    let outcome = match result {
                        Ok(outcome) => {
                            match self.write_journal(&Journal { binding, outcome: outcome.clone() }) {
                                Ok(()) => outcome,
                                Err(error) => {
                                    tracing::warn!(effect = %effect.id, %error, "workflow settlement could not be retained");
                                    Outcome::Uncertain
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(effect = %effect.id, %error, "workflow effect requires reconciliation");
                            Outcome::Uncertain
                        }
                    };
                    // A cancel fences the original Observe; its private settlement
                    // remains available for the Stop effect's quiescence check.
                    let current = self.state(&workspace)?.ok_or(Error::Refused("missing state"))?;
                    if current.items.get(&effect.item).is_some_and(|i| i.pending.as_ref() == Some(&effect.id)) {
                        self.observe(&workspace, &effect, outcome, false, now_ms)?;
                    }
                    count += 1;
                    scan = scan || self.continuous;
                    if stopping { scan = false; }
                }
            }
        }
        Ok(count)
    }
    /// Recover or start every effect that is ready and not already running, within the cap.
    async fn launch_ready(&mut self, workspaces: &[String], now_ms: i64, running: &mut Vec<Running>,
        futures: &mut Vec<Slot>,
        count: &mut usize) -> Result<()>
    {
        for workspace in workspaces {
            for stored in self.store.workflow_outbox(workspace)? {
                if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { return Ok(()); }
                if !stored.active { continue; }
                let effect = stored.effect;
                if running.iter().any(|j| j.effect.id == effect.id && &j.workspace == workspace) { continue; }
                let state = self.state(workspace)?.ok_or(Error::Refused("missing state"))?;
                // A finished item has left live state and has nothing left to run.
                let Some(item) = state.items.get(&effect.item) else { continue };
                if item.pending.as_ref() != Some(&effect.id) { continue; }
                let selected = self.select_item_configuration(workspace, &effect.item, item);
                if matches!(stored.dispatch_state.as_str(), "admitted" | "uncertain") {
                    if let Err(error) = selected {
                        tracing::warn!(effect = %effect.id, %error, "retained configuration unavailable; ownership remains uncertain");
                        if stored.dispatch_state != "uncertain" && self.observe(workspace, &effect, Outcome::Uncertain, false, now_ms)? {
                            *count += 1;
                        }
                        continue;
                    }
                    let recovered = async {
                        let binding = Binding::new(workspace, &effect, stored.resource_generation
                            .ok_or(Error::Refused("admitted effect lacks generation"))?, item.iteration, &self.item_base(workspace, &effect)?);
                        if self.coding.is_some() {
                            self.reconcile_coding(&binding).await.map(Some)
                        } else {
                            self.read_journal(&binding).map(|journal| journal.map(|j| j.outcome))
                        }
                    }.await;
                    let outcome = match recovered {
                        Ok(outcome) => outcome.unwrap_or(Outcome::Uncertain),
                        Err(error) => {
                            tracing::warn!(effect = %effect.id, %error, "invalid recovery receipt");
                            Outcome::Uncertain
                        }
                    };
                    if stored.dispatch_state == "uncertain" && outcome == Outcome::Uncertain { continue; }
                    if self.observe(workspace, &effect, outcome, stored.dispatch_state == "uncertain", now_ms)? { *count += 1; }
                    continue;
                }
                if !matches!(item.phase, Phase::Running | Phase::Cancelling) { continue; }
                // A stop waits for the effect it stops to end; they must not touch one pane at once.
                if let EffectKind::Stop { operation } = &effect.kind {
                    if running.iter().any(|j| &j.effect.id == operation && &j.workspace == workspace) { continue; }
                }
                // A stop must always get through; everything else waits for a free slot.
                let wait = matches!(effect.kind, EffectKind::Execute(StageKind::AwaitBase | StageKind::AwaitCi | StageKind::AwaitPr));
                if !matches!(effect.kind, EffectKind::Stop { .. }) && !wait
                    && running.iter().filter(|j| !j.wait).count() >= self.effect_cap { continue; }
                let Some(claim) = self.store.workflow_claim_effect(workspace, &effect.id,
                    &format!("runtime:{}", self.instance), state.revision, state.epoch,
                    now_ms, now_ms.saturating_add(30_000))? else {
                    if let Some(deadline) = stored.lease_until.filter(|t| *t > now_ms) {
                        self.next_lease = Some(self.next_lease.map_or(deadline, |t| t.min(deadline)));
                    }
                    continue;
                };
                // Stop uses retained evidence and never needs current launch files
                // or a new repository lock. A fresh launch still validates all of them.
                let prepared = async {
                    selected?;
                    if matches!(effect.kind, EffectKind::Stop { .. }) {
                        return self.item_base(workspace, &effect);
                    }
                    self.validate_item_launch(workspace, item)?;
                    self.pin_base(workspace, &effect).await
                }.await;
                let base = match prepared {
                    Ok(base) => base,
                    Err(error) => {
                        tracing::warn!(effect = %effect.id, %error, "workflow item refused before external execution");
                        // No external operation was invoked. Persist the refusal
                        // through the normal admission/observation fence, so Get
                        // exposes Failed rather than silently abandoning the item.
                        if self.store.workflow_admit_effect(&claim, now_ms)? {
                            let outcome = if matches!(effect.kind, EffectKind::Stop { .. }) {
                                Outcome::Uncertain
                            } else { Outcome::SafeFailure };
                            if self.observe(workspace, &effect, outcome, false, now_ms)? { *count += 1; }
                        }
                        continue;
                    }
                };
                if !self.store.workflow_admit_effect(&claim, now_ms)? { continue; }
                let binding = Binding::new(workspace, &effect, claim.resource_generation, item.iteration, &base);
                // Any I/O failure beyond admission is ambiguous, not a retry permission.
                let (cancel, cancelled) = tokio::sync::watch::channel(false);
                // This effect runs on its own copy of the item's configuration, so the next
                // item's selection cannot change what it sees.
                let mut exec = self.exec.clone();
                exec.launched = Some((now_ms, std::time::Instant::now()));
                let (run_workspace, run_binding) = (workspace.clone(), binding.clone());
                futures.push(Some(Box::pin(async move {
                    if exec.coding.is_some() { exec.execute_coding(&run_workspace, &run_binding, cancelled).await }
                    else { exec.execute(&run_workspace, &run_binding).await }
                })));
                running.push(Running { workspace: workspace.clone(), effect, binding, cancel, wait });
            }
        }
        Ok(())
    }
    /// Write what an effect found. The engine may refuse the result (evidence for another policy, a
    /// label the step does not declare, a quarantined resource). That must fail the one item, never
    /// end the service for every workspace: a refused result settles the item as `Uncertain`, which
    /// always fits, and a refusal that cannot even be settled is left for a person. True when something was written.
    fn observe(&self, workspace: &str, effect: &Effect, outcome: Outcome, resolving: bool, now_ms: i64) -> Result<bool> {
        #[cfg(test)]
        if !resolving && self.fail_observe.swap(false, std::sync::atomic::Ordering::SeqCst) { return Err(Error::Refused("injected write failure")); }
        let uncertain = outcome == Outcome::Uncertain;
        match self.write_result(workspace, effect, outcome, resolving, now_ms)? {
            Ok(()) => Ok(true),
            Err(reason) => {
                tracing::warn!(effect = %effect.id, item = %effect.item, ?reason, "workflow result refused");
                // A refused settlement of an unknown outcome has nothing further to say.
                if resolving || uncertain { return Ok(false); }
                Ok(self.write_result(workspace, effect, Outcome::Uncertain, false, now_ms)?.is_ok())
            }
        }
    }
    /// One result write: its refusal is an answer, not an error. Only a store failure is an error
    /// (losing engine authority among them, which should stop the service).
    fn write_result(&self, workspace: &str, effect: &Effect, outcome: Outcome, resolving: bool, now_ms: i64)
        -> Result<std::result::Result<(), Refusal>>
    {
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
        Ok(self.internal(workspace, command, grant, now_ms)?.result.map(|_| ()))
    }
    async fn receive(&self, message: Inbox) {
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
            Inbox::Wire(admission, action, reply) => {
                let result = async {
                    admission.recheck().and_then(|()| admission.permits(&action))
                        .map_err(|_| Error::Refused("workflow caller authority changed"))?;
                    self.validate_wire_publication(&admission.workspace(), admission.principal(), &action).await?;
                    let tree = self.prepare_start(&admission.workspace(), admission.principal(), &action).await?;
                    // No await separates final retained authority recheck from receipt/transaction.
                    admission.recheck().and_then(|()| admission.permits(&action))
                        .map_err(|_| Error::Refused("workflow caller authority changed"))?;
                    let get = matches!(*action, farcooler_protocol::workflow_commands::Action::Get);
                    let mut result = if let farcooler_protocol::workflow_commands::Action::Settle { mutation, item, run, pin, landed } = &*action {
                        self.settle_wire(&admission.workspace(), item, run, pin, *landed, mutation.epoch).await?
                    } else {
                        self.wire_command_with(&admission.workspace(), admission.principal(), *action, tree)?
                    };
                    if get {
                        let epoch = result.pointer("/state/epoch").and_then(|v| v.as_u64()).unwrap_or(1);
                        result["context"] = admission.context(epoch);
                    }
                    Ok(result)
                }.await;
                let _ = reply.send(result);
            }
            Inbox::Track(tickets, reply) => { let _ = reply.send(self.track_tickets(&tickets)); }
            Inbox::MoveTicket(workspace, item, column, reply) => { let _ = reply.send(self.move_ticket_now(&workspace, &item, &column)); }
            Inbox::Reconcile => {}
        }
    }
    fn move_ticket_now(&self, workspace: &str, item: &str, column: &str) -> Result<bool> {
        let Some(state) = self.state(workspace)? else { return Ok(false) };
        if !state.items.get(item).is_some_and(|i| i.definition.is_ticket()) { return Ok(false); }
        let receipt = self.internal(workspace, Command::Move { item: item.into(), to: column.into() }, Grant::Controller, now())?;
        // Moving a card to the column it is in changes nothing, and that is not a failure.
        Ok(receipt.result.is_ok() || receipt.result == Err(Refusal::Invalid))
    }
    /// A card the engine refuses (a live worker or landing owns it) stays out and is not an error.
    fn track_tickets(&self, tickets: &[(String, String, String)]) -> Result<usize> {
        let mut taken = 0;
        for (workspace, item, column) in tickets {
            let Some(definition) = Definition::ticket_at(column) else { continue };
            let policy_pin = self.store.workflow_ticket_bundle(&definition)?;
            let Ok(Some(state)) = self.state(workspace).map(|s| Some(s.unwrap_or_else(|| State::empty(workspace.clone(), 1)))) else { continue };
            let sequence = state.source_sequences.get("runtime").copied().unwrap_or(0) + 1;
            let receipt = self.apply(Request { id: format!("track:{}", Uuid::now_v7()), workspace: workspace.clone(),
                expected_revision: state.revision, sequence, command: Command::Enqueue { item: item.clone(),
                    initial_candidate: Some(definition.pin()), definition, policy_pin,
                    resource: Exec::item_resource(workspace, item), dependencies: BTreeSet::new() } },
                "runtime", Grant::Controller, now());
            if receipt.is_ok_and(|r| r.result.is_ok()) { taken += 1; }
        }
        Ok(taken)
    }
    pub async fn run(&mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        self.shutdown = Some(shutdown.clone());
        self.continuous = true;
        loop {
            if *shutdown.borrow() { return Ok(()); }
            // Bounded drain prevents a busy sender from starving effects/shutdown.
            for _ in 0..32 {
                let Ok(message) = self.inbox.try_recv() else { break; };
                self.receive(message).await;
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
                        self.receive(message).await;
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
type Slot = Option<std::pin::Pin<Box<dyn std::future::Future<Output = Result<Outcome>> + Send>>>;
/// An effect that has been admitted and is executing.
struct Running {
    workspace: String,
    effect: Effect,
    binding: Binding,
    cancel: tokio::sync::watch::Sender<bool>,
    /// Waits for something else (the base, CI) and does no work of its own, so the cap leaves it out.
    wait: bool,
}
enum Event {
    Finished(usize, Result<Outcome>),
    Shutdown,
    Message(Option<Inbox>),
}
/// Resolves with the first finished future and its position, without the `futures` crate.
async fn next_finished(futures: &mut [Slot])
    -> (usize, Result<Outcome>)
{
    std::future::poll_fn(|cx| {
        for (index, slot) in futures.iter_mut().enumerate() {
            if let Some(future) = slot {
                if let std::task::Poll::Ready(result) = future.as_mut().poll(cx) {
                    return std::task::Poll::Ready((index, result));
                }
            }
        }
        std::task::Poll::Pending
    }).await
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

#[cfg(test)]
mod coding_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod observe_tests;
#[cfg(test)]
mod coding_verbs_tests;
#[cfg(test)]
mod landing_templates_tests;

#[cfg(test)]
mod coding_pr_tests;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod send_back_tests;
