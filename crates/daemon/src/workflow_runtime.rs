//! Workflow runtime for private fixtures and the sole ordinary Service owner.
//! The local capability boundary is intended for authenticated embedding, not wire grants.
pub(crate) mod execution;
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

mod scheduling;
mod waiting_join;
mod adapters;
mod process_logs;
mod script_hooks;
pub(crate) mod coding;
mod coding_review_policy;
mod service_owner;
mod coding_execute;
mod ticket_brief;
#[cfg(test)]
macro_rules! wait_until_async {
    ($deadline:expr, $interval:expr, async || $condition:block) => {{
        let deadline = $deadline;
        let interval = $interval;
        loop {
            if (async { $condition }).await { break true; }
            if tokio::time::Instant::now() >= deadline { break false; }
            tokio::time::sleep_until((tokio::time::Instant::now() + interval).min(deadline)).await;
        }
    }};
}
mod coding_native;
mod native_control;
mod native_messages;


#[cfg(test)]
mod native_steering_live_tests;
mod coding_landing;
mod coding_verbs;
mod coding_candidate;
mod coding_ci_rerun;
mod coding_settle;
mod close_run;
mod coding_pr;
mod review_agent;
mod fold_trees;
mod spotlight;
mod builder_resources;
mod places;
pub use places::{DEFAULT_EFFECT_CAP, DEFAULT_LANDING_PLACES, DEFAULT_WORK_PLACES};
use places::{Placed, Places, Pool};
mod build_cleanup;
#[cfg(test)]
mod repin_reply_tests;
#[cfg(test)]
mod build_cleanup_tests;
mod recovery_backoff;
#[cfg(test)]
mod recovery_backoff_tests;
pub use coding::{AGENT_REVIEWER, CodingConfigExt,CodingConfig, CodingCommand, CodingHarness, CodingReviewer, CodingCi, CodingLanding, CodingPullRequest, LaunchFile, MAX_COMMAND_MS};
pub use coding_execute::{CodingReviewReceipt, CodingCiReceipt, write_agent_verdict};
mod wire_commands;
mod named_tree;
pub use named_tree::{RepoLocks, WorktreeHost};
mod admission;
pub(crate) use admission::Admission;
#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod publication_bounds_tests;
#[cfg(test)]
mod seat_tests;
#[cfg(test)]
mod member_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod decider_tests;
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
    /// The run stopped on changes in its named tree it can't show it made (ov-979), listed for the person.
    #[error("runtime stopped: {0}")]
    Kept(String),
    /// A refusal the caller can act on, said as the runner says it for a worktree (ov-967): a taken branch or
    /// name, or one that isn't valid. Every other workflow error reaches the caller as not found.
    #[error("runtime: {0}")]
    Domain(farcooler_core::error::DomainError),
    /// A file the published workflow pins by digest no longer holds what was published (ov-1259). Kept apart from
    /// `Refused` so the item records which file, in words, instead of failing with no reason.
    #[error("runtime refused: native settings changed: {}", .0.display())]
    Changed(std::path::PathBuf),
}
impl Error {
    /// What a person reads on an item this error stopped before anything ran, when it is one they can act on.
    pub(crate) fn reason(&self) -> Option<String> {
        match self {
            Error::Domain(refused) => Some(refused.to_string()),
            Error::Changed(path) => Some(format!("{} changed after this workflow was published, so its pin no longer matches \
                and nothing ran. Re-pin the workflow (`farcooler workspace re-pin`) and publish it, then retry.", path.display())),
            _ => None,
        }
    }
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

/// Bounded mutation inbox and committed reads. Both paths recheck caller admission before use.
#[derive(Clone)]
pub struct CommandHandle {
    send: mpsc::Sender<Inbox>,
    // Reads use the committed store without waiting for the effect owner or its git/cleanup work.
    reader: Exec,
}
enum Authority { Controller(Uuid), Decision(Uuid, String) }
enum Inbox {
    Apply(Authority, Box<Request>, i64, oneshot::Sender<Result<Receipt>>),
    State(String, oneshot::Sender<Result<Option<State>>>),
    Wire(Box<Admission>, Box<farcooler_protocol::workflow_commands::Action>, oneshot::Sender<Result<serde_json::Value>>),
    /// Bring cards under the engine with no run: (workspace, item, column).
    Track(Vec<(String, String, String)>, oneshot::Sender<Result<usize>>),
    /// Move a tracker-only ticket to a column: (workspace, item, column), as `MoveBy` says.
    MoveTicket(String, String, String, MoveBy, oneshot::Sender<Result<bool>>),
    Reconcile,
    /// `resolve_provision` as the wire's Resolve runs it, inside the loop, without an RPC admission (ov-1220).
    #[cfg(test)]
    Resolve(String, String, oneshot::Sender<Result<()>>),
}
/// Who moves a ticket, for its card's history, and whether they still may when the engine takes the move: asked
/// in the inbox, with no await before the write (R-111, L8). A move the daemon makes for itself always may.
pub struct MoveBy { pub actor: farcooler_store::models::Actor, pub still: Box<dyn FnOnce() -> bool + Send> }
impl MoveBy {
    /// The daemon's own move (a draft's card closed): the runner's, with nobody to recheck.
    pub fn runner() -> Self { Self { actor: farcooler_store::models::Actor::Runner, still: Box::new(|| true) } }
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
    pub async fn move_ticket(&self, workspace: &str, item: &str, column: &str, by: MoveBy) -> Result<bool> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::MoveTicket(workspace.into(), item.into(), column.into(), by, send)).await
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
    /// Shared observations for reads outside the writer inbox.
    activities: execution::Activities,
    /// Each pool's places, what holds them and what waits for one (ov-1198).
    places: std::sync::Arc<std::sync::Mutex<Places>>,
    root: PathBuf,
    store: std::sync::Arc<Store>,
    service_mode: bool,
    resource: String,
    base: String,
    coding: Option<CodingConfig>,
    /// When this copy was started, by the sweep's clock and by the monotonic one, so a wait measures
    /// its patience on the same clock its effect was created on.
    launched: Option<(i64, std::time::Instant)>,
    /// Cleanup takes exclusive access to a tree. Dispatch only tries the lock, so other trees can proceed.
    tree_locks: build_cleanup::TreeLocks,
    /// Set by the Service: makes a run's named worktree one the app lists (ov-967).
    pub host: Option<WorktreeHost>,
    /// One first download of a landing remote's main at a time, per remote, shared by every copy (ov-1058 review 3):
    /// four Provisions starting together wait on one download instead of each fetching all of main, and share its failure.
    seeding: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, std::sync::Arc<coding_landing::Seeding>>>>,
    /// A test's stand-in for the runner user's home, where git and ssh read their own configuration (ov-1026).
    #[cfg(test)]
    pub(super) user_home: Option<PathBuf>,
    #[cfg(test)]
    pub(super) build_free_bytes: Option<u64>,
    #[cfg(test)]
    cleanup_check: Option<std::sync::Arc<(tokio::sync::Notify, tokio::sync::Notify)>>,
    /// Builders pause while a landing gate runs, in every copy (ov-1214).
    pub(crate) yielding: crate::yielding::Yielding,
}
impl Exec {
    fn places(&self) -> std::sync::MutexGuard<'_, Places> {
        self.places.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
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
    /// Whether a sweep has cleared the trees an earlier daemon left half deleted (ov-1084).
    trees_recovered: bool,
    /// Runs on a deleted board whose cancel the engine refused, so the sweep asks once and not on every pass: each
    /// refusal is a permanent receipt (ov-1047 review).
    cancel_refused: std::collections::BTreeSet<(String, String)>,
    /// Uncertain effects the sweep couldn't recover, looked at less and less often (ov-1301).
    recovery: recovery_backoff::RecoveryBackoff,
    scheduling: std::sync::Mutex<scheduling::Scheduler>,
    inbox: mpsc::Receiver<Inbox>,
    send: mpsc::Sender<Inbox>,
    next_lease: Option<i64>,
    /// Pending transient cleanup must wake both idle and occupied dispatch loops (ov-1203).
    next_cleanup: Option<tokio::time::Instant>,
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
/// Whether the sweep should ask the engine to cancel a run on a deleted board. A follower ends with the run it
/// follows, and a group whose push is in flight or landed cannot be called back: the engine refuses both, and every
/// refusal is a receipt that stays, so the sweep never asks.
pub(crate) fn cancellable_on_deleted_board(i: &Item) -> bool {
    !matches!(i.phase, Phase::Done | Phase::Cancelled | Phase::Cancelling) && i.follows.is_none()
        && !(!i.members.is_empty() && (i.pushed.is_some() || matches!(i.definition.stages[i.stage].kind, StageKind::Push)))
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
        // A network remote can't be locked from here; its push leases instead (ov-1026).
        let landing_writer = if let Some(identity) = coding.as_ref().and_then(|c| c.landing.as_ref()).filter(|l| coding::landing_url(l).is_none()) {
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
        let runtime = Self { inbox, send, next_lease: None, next_cleanup: None, continuous: false,
            #[cfg(test)] sweeps: 0, #[cfg(test)] fail_observe: Default::default(), #[cfg(test)] drained: 0,
            exec: Exec { yielding: crate::yielding::Yielding::at(&root), activities: Default::default(), places: std::sync::Arc::new(std::sync::Mutex::new(Places::load(&root)?)), root, store: std::sync::Arc::new(store), service_mode: false, resource, base, coding, launched: None, host: None,
                seeding: Default::default(), tree_locks: Default::default(), #[cfg(test)] user_home: None, #[cfg(test)] build_free_bytes: Some(u64::MAX), #[cfg(test)] cleanup_check: None },
            notice_wake: None, configuration_locks: Default::default(), shutdown: None, instance: Uuid::now_v7(), last_occupancy: None, trees_recovered: false, cancel_refused: Default::default(), recovery: Default::default(), scheduling: Default::default(),
            _writer: writer, _repository_writer: repository_writer, _landing_writer: landing_writer };
        runtime.validate_native_host()?;
        Ok(runtime)
    }
    /// Most agent sessions (builds and reviews) that run at once across all workspaces. A landing group's steps and
    /// short steps such as provisions have places of their own, so they never wait behind these (ov-1198).
    pub fn set_effect_cap(&mut self, cap: usize) { self.places().agent = cap.max(1); }
    /// Record what holds a place now, for `workflow get`.
    fn note_holding(&self, running: &[Running]) {
        self.places().holding = running.iter().filter_map(|j| j.pool.map(|pool| Placed::new(&j.workspace, &j.effect, pool))).collect();
    }
    pub fn command_handle(&self, controller: &Controller) -> Result<CommandHandle> {
        self.authorize(controller.instance)?;
        Ok(CommandHandle { send: self.send.clone(), reader: self.exec.clone() })
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
    pub async fn submit(&self, controller: &Controller, request: Request, now_ms: i64) -> Result<Receipt> {
        self.authorize(controller.instance)?;
        if let Command::Queue { item } = &request.command
            && !self.prepare_queue(&request.workspace, item, request.expected_revision, None, None, now_ms).await
            && self.state(&request.workspace)?.is_some_and(|s| s.revision == request.expected_revision)
        {
            return Err(Error::Refused("waiting branch could not be rebased onto the landing head"));
        }
        self.submit_now(controller, request, now_ms)
    }
    fn submit_now(&self, controller: &Controller, request: Request, now_ms: i64) -> Result<Receipt> {
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
        self.apply_by(request, source, grant, now_ms, None)
    }
    /// `apply`, with the card history naming `by` for any card the command moves (R-108).
    fn apply_by(&self, request: Request, source: &str, grant: Grant, now_ms: i64, by: Option<&farcooler_store::models::Actor>) -> Result<Receipt> {
        let epoch = self.state(&request.workspace)?.map_or(1, |s| s.epoch);
        let receipt = self.store.workflow_apply_by(&Input { schema: 1, id: request.id, workspace: request.workspace,
            expected_revision: request.expected_revision, epoch, source: source.into(),
            source_sequence: request.sequence, now_ms, grant, command: request.command,
            busy_resources: BTreeSet::new() }, by)?;
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(receipt)
    }
    fn internal(&self, workspace: &str, command: Command, grant: Grant, now_ms: i64) -> Result<Receipt> {
        let state = self.state(workspace)?.ok_or(Error::Refused("missing workspace"))?;
        let sequence = state.source_sequences.get("runtime").copied().unwrap_or(0) + 1;
        self.apply(Request { id: format!("runtime:{}", Uuid::now_v7()), workspace: workspace.into(),
            expected_revision: state.revision, sequence, command }, "runtime", grant, now_ms)
    }
    /// Rebind only idle landing work; executing effects keep the bundle they were admitted under.
    fn refresh_groups(&self, workspace: &str, now_ms: i64) -> Result<()> {
        if let Some(published) = self.store.workflow_configuration(workspace)? {
            let grouping = Grouping { definition: published.definition, policy_pin: published.pin };
            // Ungrouped workflows need no grouping event. Once a grouping exists, an empty
            // publication must still replace it so old landing rules cannot survive removal.
            if self.state(workspace)?.is_some_and(|s| s.grouping.as_ref() != Some(&grouping)
                && (s.grouping.is_some() || !grouping.definition.groups.is_empty())) {
                self.internal(workspace, Command::RefreshGroups { definition: grouping.definition,
                    policy_pin: grouping.policy_pin }, Grant::Controller, now_ms)?;
            }
        }
        Ok(())
    }
    /// One owned sweep. This API has no delivery cursor, informational ACK or controller dependency.
    /// Effects run concurrently, within the places of each one's pool. A plain sweep starts what is
    /// ready now and waits for it; the service loop also starts follow-on effects as each ends.
    pub async fn pump(&mut self, now_ms: i64) -> Result<usize> {
        #[cfg(test)] { self.sweeps += 1; }
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
                self.activities.lock().unwrap().remove(&(job.workspace.clone(), job.effect.id.clone()));
                futures.remove(index);
                #[cfg(test)] { self.drained += 1; }
                if let Ok(outcome) = outcome {
                    if let Err(error) = self.write_journal(&Journal { binding: job.binding, outcome }) {
                        tracing::warn!(effect = %job.effect.id, %error, "workflow settlement could not be retained");
                    }
                }
            }
        }
        self.note_holding(&running);
        self.places().waiting.clear();
        result
    }
    async fn sweep_loop(&mut self, now_ms: i64, inbox: &mut mpsc::Receiver<Inbox>,
        running: &mut Vec<Running>, futures: &mut Vec<Slot>) -> Result<usize>
    {
        let started = std::time::Instant::now();
        let continuous_started = tokio::time::Instant::now();
        let continuous = self.continuous;
        let clock = || {
            let elapsed = if continuous { continuous_started.elapsed() } else { started.elapsed() };
            now_ms.saturating_add(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        };
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
        // A group that ended since the last sweep leaves its tree; the first sweep also clears what a stopped daemon left (ov-1084).
        self.exec.prune_trees(&workspaces, !self.trees_recovered).await;
        self.trees_recovered = true;
        let fingerprint = digest(&occupancy);
        let released_or_recovered = self.last_occupancy.as_ref() != Some(&fingerprint);
        self.last_occupancy = Some(fingerprint);
        for workspace in &workspaces {
            self.refresh_groups(workspace, now_ms)?;
            let state = self.state(workspace)?.ok_or(Error::Refused("missing state"))?;
            if released_or_recovered && state.items.values().any(|i| i.phase == Phase::Blocked) {
                // Store stamps actual cross-workspace occupancy in the same transaction.
                self.internal(workspace, Command::Advance, Grant::Controller, clock())?;
            }
        }
        // Nobody can decide a run on a board that was deleted, so it stops, with the reason on the run (ov-1047).
        for workspace in &workspaces {
            match self.store.workflow_board_deleted(workspace) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => { tracing::warn!(%workspace, %error, "could not tell whether a board was deleted"); continue; }
            }
            let Some(state) = self.state(workspace)? else { continue };
            let asks: Vec<(String, Phase)> = state.items.iter().filter(|(id, i)| cancellable_on_deleted_board(i) && !self.cancel_refused.contains(&(workspace.clone(), (*id).clone())))
                .map(|(id, i)| (id.clone(), i.phase.clone())).collect();
            for (id, phase) in asks {
                let reason = Some("This run was cancelled because its board was deleted.".to_string());
                match self.internal(workspace, Command::Cancel { item: id.clone(), reason }, Grant::Controller, clock()) {
                    Ok(Receipt { result: Err(refusal), .. }) => {
                        tracing::warn!(%workspace, item = %id, phase = ?phase, ?refusal, "the engine refused to cancel a run on a deleted board");
                        self.cancel_refused.insert((workspace.clone(), id));
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%workspace, item = %id, phase = ?phase, %error, "a run on a deleted board could not be cancelled"),
                }
            }
        }
        let mut stopping = false;
        let mut scan = true;
        let mut cleanup_requested = true;
        let mut cleanup = None;
        loop {
            // Ended groups' candidate branches leave the landing remote (ov-1269), off this loop.
            self.exec.sweep_candidate_branches();
            if scan {
                scan = self.continuous;
                let current = self.store.workflow_workspaces()?;
                for workspace in &current { self.refresh_groups(workspace, clock())?; }
                let before = count;
                let scan_at = clock();
                // Idle polling is due one second after the latest scan. A long cleanup
                // can leave this sweep open for several scans.
                self.next_lease = Some(scan_at.saturating_add(1_000));
                self.launch_ready(&current, scan_at, running, futures, &mut count).await?;
                cleanup_requested |= count != before;
            }
            // A round may walk many trees (ov-1376). Coalesce requests while it is running
            // and keep dispatching commands and effects, even if cleanup is the only work left.
            if cleanup_requested && cleanup.is_none() {
                let exec = self.exec.clone();
                let mut task = BackgroundTask::new(async move {
                    let workspaces = exec.store.workflow_workspaces()?;
                    exec.release_builds(&workspaces).await
                });
                task.start();
                cleanup = Some(task);
                cleanup_requested = false;
                self.next_cleanup = None;
            }
            if running.is_empty() && cleanup.is_none() { break; }
            let mut shutdown = self.shutdown.clone();
            let event = tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)), if self.continuous && !stopping => Event::Tick,
                (index, result) = next_finished(futures) => Event::Finished(index, result),
                _ = async {
                    match &mut shutdown {
                        Some(signal) => { if !*signal.borrow() { let _ = signal.changed().await; } },
                        None => std::future::pending::<()>().await,
                    }
                }, if !stopping => Event::Shutdown,
                message = inbox.recv() => Event::Message(message),
                result = async { cleanup.as_mut().expect("cleanup in progress").await }, if cleanup.is_some() => Event::CleanupFinished(result),
                _ = build_cleanup::wait_for_retry(self.next_cleanup), if !stopping => Event::CleanupRetry,
            };
            match event {
                Event::Tick => { scan = true; }
                Event::CleanupRetry => { cleanup_requested = true; }
                Event::CleanupFinished(result) => {
                    cleanup = None;
                    self.next_cleanup = result?.then(|| tokio::time::Instant::now() + Duration::from_secs(1));
                    // A pending effect may have been waiting for cleanup to release its tree.
                    scan = self.continuous && !stopping;
                }
                Event::Shutdown => {
                    stopping = true;
                    scan = false;
                    for job in running.iter() { job.cancel.send_replace(true); }
                }
                Event::Message(message) => {
                    if let Some(message) = message {
                        let read_only = matches!(message, Inbox::State(..));
                        self.receive(message).await;
                        if !read_only {
                            // A cancel may have ended a group.
                            self.exec.prune_trees(&workspaces, false).await;
                            cleanup_requested = true;
                        }
                        // A command may have started work, and a long wait must not hold it back.
                        scan |= self.continuous && !stopping;
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
                    self.activities.lock().unwrap().remove(&(job.workspace.clone(), job.effect.id.clone()));
                    futures.remove(index);
                    self.note_holding(running);
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
                        self.observe(&workspace, &effect, outcome, false, clock())?;
                    }
                    count += 1;
                    // Not at the end of the sweep, which a long gate elsewhere may hold for hours.
                    self.exec.prune_trees(&workspaces, false).await;
                    cleanup_requested = true;
                    scan |= self.continuous && !stopping;
                }
            }
        }
        Ok(count)
    }
    /// Recover or start every effect that is ready and not already running, within its pool's places. What finds
    /// its pool full is noted as waiting, in the order it would start (ov-1198).
    async fn launch_ready(&mut self, workspaces: &[String], now_ms: i64, running: &mut Vec<Running>,
        futures: &mut Vec<Slot>,
        count: &mut usize) -> Result<()>
    {
        let mut waiting = Vec::new();
        let result = self.launch_ready_noting(workspaces, now_ms, running, futures, count, &mut waiting).await;
        self.note_holding(running);
        self.places().waiting = waiting;
        result
    }
    async fn launch_ready_noting(&mut self, workspaces: &[String], now_ms: i64, running: &mut Vec<Running>,
        futures: &mut Vec<Slot>, count: &mut usize, waiting: &mut Vec<Placed>) -> Result<()>
    {
        let ready_queues = self.refresh_waiting_joins(workspaces, now_ms).await?;
        self.schedule_ready(workspaces, now_ms, &ready_queues)?;
        let started = std::time::Instant::now();
        let clock = || now_ms.saturating_add(i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX));
        for workspace in workspaces {
            let mut uncertain = BTreeSet::new();
            for stored in self.store.workflow_outbox(workspace)? {
                if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { return Ok(()); }
                if !stored.active { continue; }
                let effect = stored.effect;
                if running.iter().any(|j| j.effect.id == effect.id && &j.workspace == workspace) { continue; }
                // An Uncertain effect that recovery couldn't settle waits before it is looked at again (ov-1301).
                if stored.dispatch_state == "uncertain" {
                    uncertain.insert(effect.id.clone());
                    if self.recovery.waiting(workspace, &effect.id, now_ms) { continue; }
                }
                let state = self.state(workspace)?.ok_or(Error::Refused("missing state"))?;
                // A finished item has left live state and has nothing left to run.
                let Some(item) = state.items.get(&effect.item) else { continue };
                if item.pending.as_ref() != Some(&effect.id) { continue; }
                // Resource holds retry at most once a minute, including after a restart.
                if item.phase == Phase::Running && item.failure.as_deref().is_some_and(|reason| reason.starts_with("resource hold:"))
                    && matches!(effect.kind, EffectKind::Execute(StageKind::Run))
                    && now_ms < effect.created_ms.saturating_add(60_000) { continue; }
                let selected = self.select_item_configuration(workspace, &effect.item, item);
                if matches!(stored.dispatch_state.as_str(), "admitted" | "uncertain") {
                    let tree = self.effect_tree(workspace, &effect);
                    let Ok(_tree) = self.tree_lock(&tree).try_read_owned() else { continue };
                    if let Err(error) = selected {
                        if stored.dispatch_state != "uncertain" {
                            tracing::warn!(effect = %effect.id, %error, "retained configuration unavailable; ownership remains uncertain");
                            if self.observe(workspace, &effect, Outcome::Uncertain, false, clock())? { *count += 1; }
                        } else {
                            self.recovery.unsettled(workspace, &effect.id, Some(format!("retained configuration unavailable: {error}")), now_ms);
                        }
                        continue;
                    }
                    self.recovery.looking();
                    let recovered = async {
                        let binding = Binding::new(workspace, &effect, stored.resource_generation
                            .ok_or(Error::Refused("admitted effect lacks generation"))?, item.iteration, &self.item_base(workspace, &effect)?);
                        if matches!(effect.kind, EffectKind::Decide { .. }) {
                            self.recover_hook(&binding, now_ms).map(Some)
                        } else if self.coding.is_some() {
                            self.reconcile_coding(&binding).await.map(Some)
                        } else {
                            self.read_journal(&binding).map(|journal| journal.map(|j| j.outcome))
                        }
                    }.await;
                    let (outcome, why) = match recovered {
                        Ok(outcome) => (outcome.unwrap_or(Outcome::Uncertain), None),
                        Err(error) => (Outcome::Uncertain, Some(error.to_string())),
                    };
                    // Decided once: an effect recovery can't settle stays Uncertain, for the orchestrator, and is
                    // looked at again less and less often, its reason said once (ov-1301).
                    if outcome == Outcome::Uncertain { self.recovery.unsettled(workspace, &effect.id, why, now_ms); }
                    else { self.recovery.settled(workspace, &effect.id); }
                    if stored.dispatch_state == "uncertain" && outcome == Outcome::Uncertain { continue; }
                    if self.observe(workspace, &effect, outcome, stored.dispatch_state == "uncertain", clock())? { *count += 1; }
                    continue;
                }
                if item.deciding.is_none() && !matches!(item.phase, Phase::Running | Phase::WaitingSlot | Phase::Cancelling) { continue; }
                // A stop waits for the effect it stops to end; they must not touch one pane at once.
                if let EffectKind::Stop { operation } = &effect.kind {
                    if running.iter().any(|j| &j.effect.id == operation && &j.workspace == workspace) { continue; }
                }
                // A stop and a wait take no place; everything else waits for a free place in its own pool, so a
                // landing group or a provision never waits behind a build (ov-1198).
                let pool = Pool::of(&effect, item);
                if let Some(pool) = pool
                    && running.iter().filter(|j| j.pool == Some(pool)).count() >= self.places().cap(pool)
                {
                    waiting.push(Placed::new(workspace, &effect, pool));
                    continue;
                }
                if selected.is_ok() && let Some(command) = self.pending_repin(workspace, item, &effect)? {
                    // Re-pinning replaces only an unstarted outbox entry. The next scan
                    // validates and claims the new entry through ordinary admission.
                    if self.internal(workspace, command, Grant::Controller, clock())?.result.is_ok() { *count += 1; }
                    continue;
                }
                let tree = self.effect_tree(workspace, &effect);
                let Ok(tree) = self.tree_lock(&tree).try_read_owned() else { continue };
                let Some(claim) = self.store.workflow_claim_effect(workspace, &effect.id,
                    &format!("runtime:{}", self.instance), state.revision, state.epoch,
                    clock(), clock().saturating_add(30_000))? else {
                    if let Some(deadline) = stored.lease_until.filter(|t| *t > clock()) {
                        self.next_lease = Some(self.next_lease.map_or(deadline, |t| t.min(deadline)));
                    }
                    continue;
                };
                // Stop uses retained evidence and never needs current launch files
                // or a new repository lock. A fresh launch still validates all of them.
                let prepared = async {
                    selected?;
                    if matches!(effect.kind, EffectKind::Decide { .. }) { return self.item_base(workspace, &effect); }
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
                        if self.store.workflow_admit_effect(&claim, clock())? {
                            // A refusal that is a sentence (an unreadable landing remote, ov-1059) stays on the run.
                            let outcome = if matches!(effect.kind, EffectKind::Stop { .. }) {
                                Outcome::Uncertain
                            } else if let Some(reason) = error.reason() {
                                Outcome::SafeFailureBecause(reason)
                            } else { Outcome::SafeFailure };
                            if self.observe(workspace, &effect, outcome, false, clock())? { *count += 1; }
                        }
                        continue;
                    }
                };
                if !self.store.workflow_admit_effect(&claim, clock())? { continue; }
                let activity = std::sync::Arc::new(execution::Activity::default());
                self.activities.lock().unwrap().insert((workspace.clone(), effect.id.clone()), activity.clone());
                let binding = Binding::new(workspace, &effect, claim.resource_generation, item.iteration, &base);
                // Any I/O failure beyond admission is ambiguous, not a retry permission.
                let (cancel, cancelled) = tokio::sync::watch::channel(false);
                // This effect runs on its own copy of the item's configuration, so the next
                // item's selection cannot change what it sees.
                let mut exec = self.exec.clone();
                exec.launched = Some((clock(), std::time::Instant::now()));
                let (run_workspace, run_binding) = (workspace.clone(), binding.clone());
                futures.push(effect_slot(execution::ACTIVITY.scope(activity.clone(), async move {
                    // Internal adapters own an async session; native agents and configured commands
                    // publish presence only after the real PTY or child exists.
                    let process = exec.coding.is_some() && matches!(run_binding.effect.kind,
                        EffectKind::Execute(StageKind::Build | StageKind::Check | StageKind::Run | StageKind::Review(Reviewer::Agent)));
                    if !process { activity.session.store(true, std::sync::atomic::Ordering::SeqCst); }
                    if matches!(run_binding.effect.kind, EffectKind::Decide { .. }) { exec.execute_hook(&run_binding, Some(cancelled)).await }
                    else if exec.coding.is_some() { exec.execute_coding(&run_workspace, &run_binding, cancelled).await }
                    else { exec.execute(&run_workspace, &run_binding).await }
                })));
                running.push(Running { workspace: workspace.clone(), effect, binding, cancel, pool, _tree: tree });
            }
            self.recovery.keep_only(workspace, &uncertain);
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
        let command = if let EffectKind::Decide { input, .. } = &effect.kind {
            let result = match outcome { Outcome::Hook(result) => result, _ => HookResult::Missing };
            Command::Decide { item: effect.item.clone(), call: input.call.clone(), fence: effect.fence,
                policy_pin: effect.policy_pin.clone(), result }
        } else if resolving {
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
        // The orchestrator's word may have written the receipt an Uncertain effect waits on: look at once (ov-1301).
        let read = matches!(&message, Inbox::State(..))
            || matches!(&message, Inbox::Wire(_, action, _) if matches!(**action, farcooler_protocol::workflow_commands::Action::Get));
        if !read { self.recovery.wake(); }
        match message {
            Inbox::Apply(authority, request, time, reply) => {
                let request = *request;
                let result = match authority {
                    Authority::Controller(instance) => self.submit(&Controller { instance }, request, time).await,
                    Authority::Decision(instance, principal) =>
                        self.decide(&DecisionSession { instance, principal }, request, time),
                };
                // A dropped caller never rolls back an accepted durable command.
                let _ = reply.send(result);
            }
            Inbox::State(workspace, reply) => { let _ = reply.send(self.execution_state(&workspace)); }
            #[cfg(test)]
            Inbox::Resolve(workspace, item, reply) => {
                let _ = reply.send(self.resolve_provision(&self.controller(), &workspace, &item).await);
            }
            Inbox::Wire(admission, action, reply) => {
                let result = async {
                    admission.recheck().and_then(|()| admission.permits(&action))
                        .map_err(|_| Error::Refused("workflow caller authority changed"))?;
                    self.validate_wire_publication(&admission.workspace(), &admission.principal(), &action).await?;
                    let tree = self.prepare_start(&admission.workspace(), &admission.principal(), &action).await?;
                    let close_verified = self.verify_close(&admission.workspace(), &admission.principal(), &action).await?;
                    let queue_held = self.wire_queue_held(&admission.workspace(), &admission.principal(), &action).await?;
                    // The remote is read before anything is written: a commit it does not hold lands nothing.
                    // It is the last await, so the recheck below sees any authority lost while it was pending.
                    let landed_refusal = if let farcooler_protocol::workflow_commands::Action::Landed { item, run, pin, sha, .. } = &*action {
                        self.landed_check(&admission.workspace(), item, run, pin, sha).await?
                    } else { None };
                    // No await separates final retained authority recheck from receipt/transaction.
                    admission.recheck().and_then(|()| admission.permits(&action))
                        .map_err(|_| Error::Refused("workflow caller authority changed"))?;
                    let deciders = admission.deciders().map_err(|_| Error::Refused("workflow caller authority changed"))?;
                    let get = matches!(*action, farcooler_protocol::workflow_commands::Action::Get);
                    let mut result = if let farcooler_protocol::workflow_commands::Action::Settle { mutation, item, run, pin, landed } = &*action {
                        self.settle_wire(&admission.workspace(), item, run, pin, *landed, mutation.epoch).await?
                    } else if let Some(why) = landed_refusal {
                        serde_json::json!({ "schema": 1, "receipt": { "result": { "Err": why } } })
                    } else if let farcooler_protocol::workflow_commands::Action::Resolve { mutation, item, run, pin } = &*action {
                        self.resolve_wire(&admission.workspace(), item, run, pin, mutation.epoch).await?
                    } else {
                        self.wire_command_with(&admission.workspace(), &admission.principal(), &deciders, Some(&admission.actor()), *action, wire_commands::Prepared { tree, close_verified, queue_held })?
                    };
                    if get {
                        let epoch = result.pointer("/state/epoch").and_then(|v| v.as_u64()).unwrap_or(1);
                        result["context"] = admission.context(epoch);
                        // A gate's settings can carry secrets the owner set (`--gate-env`): theirs alone (R-111, M1).
                        if !admission.is_owner() { wire_commands::hide_environment(&mut result); }
                    }
                    Ok(result)
                }.await;
                let _ = reply.send(result);
            }
            Inbox::Track(tickets, reply) => { let _ = reply.send(self.track_tickets(&tickets)); }
            Inbox::MoveTicket(workspace, item, column, MoveBy { actor, still }, reply) => {
                let result = if still() { self.move_ticket_now(&workspace, &item, &column, actor) }
                    else { Err(Error::Refused("ticket mover authority changed")) };
                let _ = reply.send(result);
            }
            Inbox::Reconcile => {}
        }
    }
    fn move_ticket_now(&self, workspace: &str, item: &str, column: &str, by: farcooler_store::models::Actor) -> Result<bool> {
        let Some(state) = self.state(workspace)? else { return Ok(false) };
        if !state.items.get(item).is_some_and(|i| i.definition.is_ticket()) { return Ok(false); }
        let sequence = state.source_sequences.get("runtime").copied().unwrap_or(0) + 1;
        let receipt = self.apply_by(Request { id: format!("runtime:{}", Uuid::now_v7()), workspace: workspace.into(),
            expected_revision: state.revision, sequence, command: Command::Move { item: item.into(), to: column.into() } },
            "runtime", Grant::Controller, now(), Some(&by))?;
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
                    _ = build_cleanup::wait_for_retry(self.next_cleanup) => break,
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
    /// The places it holds; None for a stop or a wait on the base, CI or a pull request, which take none.
    pool: Option<Pool>,
    _tree: tokio::sync::OwnedRwLockReadGuard<()>,
}
enum Event {
    Tick,
    CleanupRetry,
    CleanupFinished(Result<bool>),
    Finished(usize, Result<Outcome>),
    Shutdown,
    Message(Option<Inbox>),
}
/// An effect, on its own task once the loop has polled it (ov-1375).
///
/// The loop polls its effects only while it waits in `select!`. Its other work (a round of `lsof +D` over
/// every engine tree, about 20 seconds on a busy runner) left each effect's git unwatched past its
/// deadline, to be killed though it had finished in milliseconds. So the loop's first poll, which starts
/// the effect as before (merely admitting one still says Starting), hands it to a task of its own, which
/// keeps going whatever the loop is doing.
///
/// The task only polls the effect; the slot still owns it. Dropping the slot drops the effect there and
/// then, as before, so a sweep cut off mid-effect (a crash, in the tests) releases its locks and kills its
/// children before anything opens the runtime again, not whenever an aborted task is next scheduled.
fn effect_slot(effect: impl std::future::Future<Output = Result<Outcome>> + Send + 'static) -> Slot {
    Some(Box::pin(BackgroundTask::new(effect)))
}
type Work<T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>;
struct BackgroundTask<T> {
    work: std::sync::Arc<std::sync::Mutex<Option<Work<T>>>>,
    task: Option<tokio::task::JoinHandle<Option<Result<T>>>>,
}
impl<T: Send + 'static> BackgroundTask<T> {
    fn new(work: impl std::future::Future<Output = Result<T>> + Send + 'static) -> Self {
        Self { work: std::sync::Arc::new(std::sync::Mutex::new(Some(Box::pin(work)))), task: None }
    }
    fn start(&mut self) {
        // Cleanup starts here so even its first poll runs outside dispatch.
        let work = self.work.clone();
        self.task = Some(tokio::spawn(std::future::poll_fn(move |cx| Self::poll_work(&work, cx))));
    }
    /// One poll of the work, which is gone once it finishes or its owner is dropped.
    fn poll_work(work: &std::sync::Mutex<Option<Work<T>>>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<Result<T>>>
    {
        let mut work = work.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(running) = work.as_mut() else { return std::task::Poll::Ready(None) };
        let polled = running.as_mut().poll(cx);
        if polled.is_ready() { *work = None; }
        polled.map(Some)
    }
}
impl<T: Send + 'static> std::future::Future for BackgroundTask<T> {
    type Output = Result<T>;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if self.task.is_none() {
            if let std::task::Poll::Ready(result) = Self::poll_work(&self.work, cx) {
                return std::task::Poll::Ready(result.unwrap_or(Err(Error::Refused("workflow task dropped"))));
            }
            self.start();
        }
        let task = self.task.as_mut().expect("spawned above");
        std::pin::Pin::new(task).poll(cx).map(|joined| match joined {
            Ok(Some(outcome)) => outcome,
            // A panic in the work ends the loop, as it did when the loop polled it.
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Ok(None) | Err(_) => Err(Error::Refused("workflow task dropped")),
        })
    }
}
impl<T> Drop for BackgroundTask<T> {
    fn drop(&mut self) {
        let work = self.work.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        drop(work);
        if let Some(task) = &self.task { task.abort(); }
    }
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
mod coding_landing_head_tests;
#[cfg(test)]
mod coding_fold_skip_tests;
#[cfg(test)]
mod coding_gate_tests;
#[cfg(test)]
mod coding_red_base_tests;
#[cfg(test)]
mod coding_setup_rerun_tests;
#[cfg(test)]
mod coding_candidate_tests;
#[cfg(test)]
mod coding_ci_rerun_tests;
#[cfg(test)]
mod coding_remote_tests;
#[cfg(test)]
mod landing_templates_tests;

#[cfg(test)]
mod coding_pr_tests;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod send_back_tests;
#[cfg(test)]
mod landed_tests;
#[cfg(test)]
mod fold_trees_tests;
#[cfg(test)]
mod main_bundle_tests;

#[cfg(test)]
mod script_hooks_tests;

#[cfg(test)]
async fn wait_until(deadline: tokio::time::Instant, interval: std::time::Duration, mut condition: impl FnMut() -> bool) -> bool {
    loop {
        if condition() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + interval).min(deadline)).await;
    }
}
