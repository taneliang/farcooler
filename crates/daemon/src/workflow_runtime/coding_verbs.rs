//! The landing verbs of a group: wait for the base, fold, push, and wait for CI (ov-956).
//! `run` is the ordinary gate command and lives with the other processes in coding_execute.
//!
//! Each verb ends by writing one fact (its label, its candidate and what it saw) and then settles
//! from that fact, so the first run and a recovery after a crash take the same road. What a verb may
//! do with no fact after a crash depends on its class: a read is read again and a fold is made again
//! (`SafeFailure` runs the step anew), but an attempted push is never repeated and stays uncertain.
//! A durable pre-send record lets a push interrupted while waiting for predecessor CI resume safely. A predecessor
//! whose CI went red doesn't hold the push back while the target is still at the head the gate ran on (ov-1291).
use super::*;
use super::coding::{sha, wait_times};
use farcooler_workflow::FailedStep;
use farcooler_store::board_ci::CiStatus;
use farcooler_protocol::workflow_config::CiUnrun;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerbFact {
    binding: Binding,
    label: String,
    candidate: String,
    detail: serde_json::Value,
}

struct CiObservation {
    status: CiStatus,
    run: Option<CiRun>,
    retry_pending: bool,
}

/// What a push came to, from its exit, its porcelain output and the remote afterwards.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PushVerdict {
    /// The remote has the candidate.
    Landed,
    /// The remote refused it (a stale lease, a hook) and does not have it: starting over from the base is safe.
    Refused,
    /// A timeout, a lost connection, a kill: it may or may not have landed.
    Unknown,
}
pub(super) fn classify_push(exit: Option<i32>, porcelain: &str, remote_after: Option<&str>, candidate: &str) -> PushVerdict {
    match exit {
        Some(0) if remote_after == Some(candidate) => PushVerdict::Landed,
        // A zero exit with the candidate nowhere is not a landing, and not a refusal either.
        Some(0) | None => PushVerdict::Unknown,
        // Porcelain marks a ref the remote refused with `!` and says why. Only a refusal (not
        // `[remote failure]`, where the ref may have moved) counts, and only if the candidate is not there.
        Some(_) if porcelain.lines().any(|l| l.starts_with('!') && (l.contains("[rejected]") || l.contains("[remote rejected]")))
            && remote_after.is_some_and(|r| r != candidate) => PushVerdict::Refused,
        Some(_) => PushVerdict::Unknown,
    }
}
pub(super) fn is_verb(kind: &StageKind) -> bool {
    matches!(kind, StageKind::AwaitBase | StageKind::Fold | StageKind::Push | StageKind::AwaitCi | StageKind::RerunCi | StageKind::Route
        | StageKind::PushCandidate) || super::coding_pr::is_pr_verb(kind)
}
const COMMIT_IDENTITY: [&str; 8] = ["-c", "user.name=Far Cooler", "-c", "user.email=landing@farcooler.invalid",
    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];
const SWIFT_FLOORS: [&str; 2] = ["apps/macos/test-floor.txt", "apps/shared/AgentKit/test-floor.txt"];

enum FloorPick { Applied, Skipped }

#[derive(Serialize)]
pub(super) struct PickConflict {
    member: String,
    commit: String,
    pub files: Vec<String>,
}

impl Exec {
    pub(super) fn verb_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.verb.json", digest(&binding.effect)))
    }
    fn push_phase_path(&self, binding: &Binding, phase: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push-{phase}.json", digest(&binding.effect)))
    }
    pub(super) fn push_lock_path(&self, effect: &Effect) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push.lock", digest(effect)))
    }
    /// Older admitted pushes lack the lock and may have started an untracked writer.
    pub(super) fn prepare_push_lock(&self, effect: &Effect) -> Result<()> {
        if effect.kind != EffectKind::Execute(StageKind::Push) { return Ok(()); }
        match durable(&self.push_lock_path(effect), b"") {
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            result => result,
        }
    }
    pub(super) fn lock_push(&self, binding: &Binding) -> std::io::Result<File> {
        crate::git::try_process_lock(&self.push_lock_path(&binding.effect))
    }
    pub(super) fn push_send_recorded(&self, binding: &Binding) -> Result<bool> {
        Ok(self.push_phase_path(binding, "send").try_exists()?)
    }
    fn push_process_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect)))
    }
    /// A receiver can keep processing after the local client disconnects. A successful
    /// client exit or an explicit ref refusal is evidence that the receive has finished.
    pub(super) fn push_receive_finished(&self, binding: &Binding, identity: &CodingLanding) -> Result<bool> {
        #[derive(Deserialize)]
        struct Receipt { binding: Binding, identity: CodingLanding, exit: Option<i32>, log_digest: String }
        let raw = match self.bytes(&self.push_process_path(binding)) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let fact: Receipt = serde_json::from_slice(&raw)?;
        let output = self.bytes(&self.remote_log(binding))?;
        if fact.binding != *binding || fact.identity != *identity || fact.log_digest != digest(&output) {
            return Err(Error::Refused("the previous push's process receipt could not be verified"));
        }
        let refused = String::from_utf8_lossy(&output).lines().any(|line| {
            let mut fields = line.split('\t');
            fields.next() == Some("!")
                && fields.next().and_then(|refs| refs.split_once(':')).is_some_and(|(_, target)| target == identity.target)
                && fields.next().is_some_and(|reason| reason.starts_with("[rejected]") || reason.starts_with("[remote rejected]"))
        });
        Ok(fact.exit == Some(0) || (fact.exit.is_some() && refused))
    }
    /// Only an authenticated pre-send record without send intent proves that replay is safe.
    /// Missing legacy records, unreadable records and any send intent retain push uncertainty.
    fn reconcile_unsent_push(&self, binding: &Binding) -> Result<Outcome> {
        if self.push_send_recorded(binding)? {
            return Err(Error::Refused("push send was admitted; whether it landed is unknown"));
        }
        let raw = self.bytes(&self.push_phase_path(binding, "waiting"))?;
        let recorded: Binding = serde_json::from_slice(&raw)?;
        if recorded != *binding { return Err(Error::Refused("foreign push pre-send record")); }
        Ok(Outcome::Interrupted)
    }
    /// Which base a folded commit was made on: a push leases the target against exactly that.
    pub(super) fn fold_index(&self, binding: &Binding, folded: &str) -> PathBuf {
        self.root.join("receipts").join(format!("fold-{}-{folded}.json", digest(&(&binding.workspace, &binding.effect.item))))
    }
    /// Group process gates compare against the durable fold base, not the runner's startup base.
    /// Keep Binding unchanged: historical receipts and admission still belong to that binding.
    pub(super) fn process_base(&self, binding: &Binding, candidate: &str) -> Result<String> {
        if binding.effect.kind != EffectKind::Execute(StageKind::Run) || binding.effect.members.is_empty() {
            return Ok(binding.base.clone());
        }
        let index: serde_json::Value = serde_json::from_slice(&self.bytes(&self.fold_index(binding, candidate))?)?;
        if index["folded"] != candidate || index["item"] != binding.effect.item {
            return Err(Error::Refused("gate fold record is for another candidate or group"));
        }
        index["base"].as_str().filter(|base| sha(base)).map(str::to_string)
            .ok_or(Error::Refused("gate has no valid fold base"))
    }
    pub(super) fn landing(&self) -> Result<&CodingLanding> {
        self.coding.as_ref().and_then(|c| c.landing.as_ref()).ok_or(Error::Refused("landing verbs need a landing identity"))
    }
    pub(super) fn write_fact(&self, binding: &Binding, label: &str, candidate: &str, detail: serde_json::Value) -> Result<()> {
        durable(&self.verb_path(binding), &serde_json::to_vec(&VerbFact {
            binding: binding.clone(), label: label.into(), candidate: candidate.into(), detail })?)
    }
    /// A settlement based on a fresh target read replaces the fact a crash left, atomically.
    pub(super) fn settle_push_fact(&self, binding: &Binding, landed: bool, settled_by: &str) -> Result<Outcome> {
        let candidate = binding.effect.candidate.clone().ok_or(Error::Refused("missing push candidate"))?;
        let label = if landed { "passed" } else { "base_moved" };
        let fact = VerbFact { binding: binding.clone(), label: label.into(), candidate,
            detail: serde_json::json!({ "settled_by": settled_by, "landed": landed }) };
        let raw = serde_json::to_vec(&fact)?;
        let path = self.verb_path(binding);
        let temporary = path.with_extension("settling");
        let _ = std::fs::remove_file(&temporary);
        durable(&temporary, &raw)?;
        std::fs::rename(&temporary, &path)?;
        std::fs::File::open(path.parent().ok_or(Error::Refused("missing parent"))?)?.sync_all()?;
        Self::verb_outcome(binding, &StageKind::Push, fact, &raw)
    }
    /// The local repository's configuration must not redirect or extend the transport.
    async fn refuse_redirected_transport(&self, binding: &Binding) -> Result<()> {
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if super::coding_landing::redirects_transport(&config) {
            return Err(Error::Refused("local landing config redirects transport"));
        }
        Ok(())
    }
    /// Git in `dir`, recorded like `git_recorded`, but a refusal is an answer and not an error.
    pub(super) async fn try_git(&self, binding: &Binding, dir: &Path, args: &[&str]) -> Result<(bool, String, String)> {
        let result = crate::git::git(dir, args).await;
        let raw = match &result {
            Ok(output) => serde_json::json!({ "args": args, "ok": output.ok, "stdout": output.stdout, "stderr": output.stderr }),
            Err(error) => serde_json::json!({ "args": args, "error": error.to_string() }),
        };
        let path = self.root.join("receipts").join(format!("{}.git.jsonl", binding.effect.id));
        let mut file = OpenOptions::new().append(true).create(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 { return Err(Error::Refused("aliased or nonregular Git log")); }
        use std::io::Write;
        writeln!(file, "{raw}")?;
        file.sync_all()?;
        let output = result.map_err(|_| Error::Refused("git failed; inspect raw receipt"))?;
        Ok((output.ok, output.stdout.trim().into(), output.stderr))
    }

    pub(super) async fn execute_verb(&self, binding: &Binding, kind: &StageKind,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome>
    {
        match kind {
            // A wait that errs has failed for a reason and spends an attempt; only a cut-short wait is `Interrupted`.
            StageKind::AwaitBase => Ok(self.await_base(binding, cancel).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "await-base failed"); Outcome::SafeFailure })),
            StageKind::RerunCi => self.rerun_ci(binding, cancel).await,
            StageKind::Fold => Ok(self.fold(binding, cancel).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "fold infrastructure failed"); Outcome::SafeFailure })),
            StageKind::Push => self.push(binding, cancel).await,
            // Putting a branch on the remote again is safe, so a push that failed for any reason is tried again.
            StageKind::PushCandidate => Ok(self.push_candidate(binding, cancel).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "candidate branch push failed"); Outcome::SafeFailure })),
            StageKind::AwaitCi => Ok(self.await_ci(binding, cancel).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "await-ci failed"); Outcome::SafeFailure })),
            // An unreadable diff has failed for a reason: it spends an attempt, then ends where a person can see it.
            StageKind::Route => Ok(self.route_paths(binding).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "route failed"); Outcome::SafeFailure })),
            _ => Err(Error::Refused("not a landing verb")),
        }
    }

    /// Read the paths the ticket changed since it began and answer the label its pinned rules give them.
    /// It only reads, so a crash before its fact is written reads again as a fresh attempt. A route no definition can take
    /// yet (`pull_request`) answers `rejected`, which ends the run.
    async fn route_paths(&self, binding: &Binding) -> Result<Outcome> {
        let text = binding.effect.operation.arguments.get(1).ok_or(Error::Refused("missing route rules"))?;
        if let Some(rules) = crate::review_agent::ReviewRoute::read(text) {
            let tree = self.tree(binding);
            self.verify_candidate(binding, &tree).await?;
            let candidate = binding.effect.candidate.as_deref().filter(|c| sha(c)).ok_or(Error::Refused("missing candidate"))?;
            let stat = self.git_recorded(binding, &tree, &["diff", "--numstat", "--no-renames", "--no-color", "--ignore-submodules=none", "--no-ext-diff",
                "--no-textconv", "-z", &binding.base, candidate]).await?;
            let label = rules.route(&stat).ok_or(Error::Refused("unreadable review diff"))?;
            self.write_fact(binding, label, candidate, serde_json::json!({ "numstat": stat }))?;
            return self.reconcile_verb(binding, &StageKind::Route).await;
        }
        let rules = crate::path_routes::PathRules::read(text).ok_or(Error::Refused("unsound path rules"))?;
        let tree = self.tree(binding);
        self.verify_candidate(binding, &tree).await?;
        let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing candidate"))?;
        let changed = self.git_recorded(binding, &tree, &["-c", "core.quotepath=off", "diff", "--name-only", "--no-renames", "-z", &binding.base, &candidate]).await?;
        let paths: Vec<String> = changed.split('\0').filter(|p| !p.is_empty()).map(String::from).collect();
        let route = rules.route_of(&paths);
        let label = match route { farcooler_core::charter_landing::Route::PullRequest => "rejected", other => crate::path_routes::label(other) };
        self.write_fact(binding, label, &candidate, serde_json::json!({ "route": crate::path_routes::label(route), "paths": paths }))?;
        self.reconcile_verb(binding, &StageKind::Route).await
    }

    /// Read the settled fact: never runs a command. With no fact, a read is read again and a fold is
    /// made again (the step runs anew); a push stays unknown unless its pre-send record proves it never sent.
    pub(super) async fn reconcile_verb(&self, binding: &Binding, kind: &StageKind) -> Result<Outcome> {
        let raw = match self.bytes(&self.verb_path(binding)) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return match kind {
                    StageKind::RerunCi => self.reconcile_ci_rerun(binding).await,
                    StageKind::Push => self.reconcile_unsent_push(binding),
                    // A wait cut short only looks again; a fold or a route cut short is made anew and spends an attempt.
                    StageKind::AwaitBase | StageKind::AwaitCi | StageKind::AwaitPr => Ok(Outcome::Interrupted),
                    _ => Ok(Outcome::SafeFailure),
                };
            }
            Err(error) => return Err(error),
        };
        let fact: VerbFact = serde_json::from_slice(&raw)?;
        if fact.binding != *binding || !sha(&fact.candidate) { return Err(Error::Refused("foreign landing verb settlement")); }
        // A recorded landing is only believed while the remote still has the commit.
        if matches!(kind, StageKind::Push) && fact.label == "passed" {
            let identity = self.landing()?;
            let log = self.root.join("receipts").join(format!("{}.reconcile.fetch.output", digest(&binding.effect)));
            if fact.candidate != binding.effect.candidate.clone().unwrap_or_default()
                || !self.on_target(identity, &fact.candidate, &log).await?
            { return Err(Error::Refused("remote landing SHA changed")); }
        }
        Self::verb_outcome(binding, kind, fact, &raw)
    }

    fn verb_outcome(binding: &Binding, kind: &StageKind, fact: VerbFact, raw: &[u8]) -> Result<Outcome> {
        let manifest = digest(&(binding, &fact.candidate, raw, &fact.label));
        let receipt = if fact.label == "passed" { digest(&("coding-receipt/v1", &manifest)) }
            else { format!("{}.verb.json", digest(&binding.effect)) };
        let evidence = Evidence { candidate: fact.candidate, manifest, receipt, policy_pin: binding.effect.policy_pin.clone() };
        if *kind == StageKind::AwaitCi && (binding.effect.ci_retry.is_some()
            || fact.detail.get("run").is_some_and(|r| !r.is_null())
            || fact.detail.get("failure").is_some_and(|f| !f.is_null())) {
            return Ok(Outcome::CiResult { label: fact.label, evidence,
                run: fact.detail.get("run").filter(|r| !r.is_null()).map(|r| serde_json::from_value(r.clone())).transpose()?,
                failure: fact.detail.get("failure").filter(|f| !f.is_null()).map(|f| serde_json::from_value(f.clone())).transpose()? });
        }
        if *kind == StageKind::Fold && fact.label == "conflict" {
            return Ok(Outcome::FoldConflict { evidence,
                members: fact.detail.get("members").cloned().map(serde_json::from_value).transpose()?,
                files: serde_json::from_value(fact.detail.get("files").cloned().unwrap_or(serde_json::json!([])))? });
        }
        if fact.label != "passed" {
            if let Some(reason) = fact.detail["why"].as_str().filter(|text| farcooler_workflow::valid_rework_reason(text)) {
                return Ok(Outcome::LabeledWithReason { label: fact.label, evidence, reason: reason.into() });
            }
        }
        Ok(match fact.label.as_str() {
            "passed" => Outcome::Passed(evidence),
            "rejected" => match serde_json::from_value::<farcooler_workflow::GroupFailure>(fact.detail["failure"].clone()) {
                Ok(failure) => Outcome::RejectedWithDetails(evidence, failure),
                Err(_) => Outcome::Rejected(evidence),
            },
            other => Outcome::Labeled(other.into(), evidence),
        })
    }

    /// Wait for the landing target's head, and for CI to finish on it if the step says `green`. A
    /// pending base keeps waiting unless a previous landing group owns its CI: then fold and gate
    /// speculatively, retaining that group as a dependency of the push (ov-1209). A red base is taken
    /// as it is so a fix for red main can land (ov-1076). The head becomes the group's candidate.
    async fn await_base(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let (every, patience) = wait_times(&binding.effect.operation).ok_or(Error::Refused("unsound wait times"))?;
        let green = binding.effect.operation.arguments.get(1).map(String::as_str) == Some("green");
        let deadline = binding.effect.created_ms.saturating_add(i64::try_from(patience).unwrap_or(i64::MAX));
        // A resumed read keeps the head it first observed, even if main moved during downtime.
        // Interrupted preserves `run`; a new stage attempt or back edge has a new run.
        let head_path = self.root.join("receipts").join(format!("base-{}.json",
            digest(&(&binding.workspace, &binding.effect.item, &binding.effect.run))));
        let mut pinned: Option<String> = match self.bytes(&head_path) {
            Ok(raw) => Some(serde_json::from_slice(&raw)?),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if pinned.as_ref().is_some_and(|s| !sha(s)) { return Err(Error::Refused("invalid saved base SHA")); }
        let mut last = pinned.clone().unwrap_or_else(|| binding.base.clone());
        // When this look first found no CI run on the head. A resumed wait starts the grace over, which only waits longer.
        let mut unrun_since: Option<i64> = None;
        for poll in 0u32.. {
            if *cancel.borrow() { return Ok(Outcome::Interrupted); }
            let head = match &pinned {
                Some(head) => Some(head.clone()),
                None => self.fetch_base(binding, poll, &cancel).await?,
            };
            if let Some(head) = head {
                if pinned.is_none() {
                    durable(&head_path, &serde_json::to_vec(&head)?)?;
                    pinned = Some(head.clone());
                }
                last = head.clone();
                let predecessor = self.pending_landing(&head, &binding.effect.resource)?;
                let (mut ci, mut verdict_from) = (None, None);
                if green && predecessor.is_none() {
                    let observed = self.ci_poll(binding, &head, poll, &cancel).await?;
                    ci = Some(observed.as_ref().map_or(CiStatus::Unknown, |o| o.status));
                    if observed.is_none() {
                        let since = *unrun_since.get_or_insert_with(|| self.clock_ms());
                        let grace = self.coding.as_ref().and_then(|c| c.ci.unrun.as_ref()).map_or(CiUnrun::DEFAULT_GRACE_MS, |u| u.grace_ms);
                        if self.clock_ms().saturating_sub(since) >= i64::try_from(grace).unwrap_or(i64::MAX) {
                            match self.verdict_behind(binding, &head, poll, &cancel).await? {
                                Behind::Verdict(status, from) => { ci = Some(status); verdict_from = Some(from); }
                                Behind::Waiting => {}
                                Behind::None(why) => {
                                    self.write_fact(binding, "timeout", &head, serde_json::json!({ "polls": poll + 1, "why": why }))?;
                                    return self.reconcile_verb(binding, &StageKind::AwaitBase).await;
                                }
                            }
                        }
                    } else { unrun_since = None; }
                }
                if matches!(ci, None | Some(CiStatus::Passed | CiStatus::Failed)) {
                    let mut detail = serde_json::json!({ "polls": poll + 1, "predecessor": predecessor });
                    if ci == Some(CiStatus::Failed) { detail["red"] = true.into(); }
                    if let Some(from) = verdict_from { detail["ci_from"] = from.into(); }
                    self.write_fact(binding, "passed", &head, detail)?;
                    return self.reconcile_verb(binding, &StageKind::AwaitBase).await;
                }
            } else if super::coding::landing_url(self.landing()?).is_some() {
                // Waiting won't change a sign-in the remote turned down: the run ends now, saying why.
                let log = self.bytes(&self.poll_log(binding, poll)).unwrap_or_default();
                if let Some(why) = sign_in_refusal(&String::from_utf8_lossy(&log)) {
                    self.write_fact(binding, "rejected", &last, serde_json::json!({ "polls": poll + 1, "why": why }))?;
                    return self.reconcile_verb(binding, &StageKind::AwaitBase).await;
                }
            }
            if self.clock_ms() >= deadline {
                self.write_fact(binding, "timeout", &last, serde_json::json!({ "polls": poll + 1 }))?;
                return self.reconcile_verb(binding, &StageKind::AwaitBase).await;
            }
            if pause(every, &cancel).await { return Ok(Outcome::Interrupted); }
        }
        unreachable!("the poll count is unbounded")
    }
    /// The head has had no CI run for the grace period. Walk back along the target's first-parent history over commits
    /// that touch only paths CI ignores: the nearest ancestor with a run speaks for the head, since nothing CI reads
    /// differs between them. Otherwise say plainly why there is no verdict, so a person is asked instead of the step
    /// polling for its whole patience (ov-1243).
    async fn verdict_behind(&self, binding: &Binding, head: &str, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Behind> {
        const WALK: usize = 20;
        let unrun = self.coding.as_ref().and_then(|c| c.ci.unrun.clone()).unwrap_or(CiUnrun { grace_ms: CiUnrun::DEFAULT_GRACE_MS, ignored: Vec::new() });
        let none = |why: String| Ok(Behind::None(why));
        let repo = self.repository();
        let mut commit = head.to_string();
        for step in 0..WALK as u32 {
            let changed = self.git_recorded(binding, &repo, &["-c", "core.quotepath=off", "diff-tree", "--no-commit-id", "--name-only",
                "-r", "--no-renames", "-z", "-m", "--first-parent", &commit]).await?;
            let paths: Vec<&str> = changed.split('\0').filter(|p| !p.is_empty()).collect();
            if paths.is_empty() || !paths.iter().all(|p| unrun.ignores(p)) {
                let short = &commit[..commit.len().min(10)];
                return none(if commit == head {
                    format!("CI has no run on the target's head {short}, and that commit changes files CI does not ignore, so no run is coming. Ask the orchestrator to push a commit CI will run on.")
                } else {
                    format!("CI has no run on the target's head, nor on any commit behind it back to {short}, which changes files CI does not ignore. Ask the orchestrator to push a commit CI will run on.")
                });
            }
            let parent = self.git_recorded(binding, &repo, &["rev-parse", "--verify", "--quiet", &format!("{commit}^")]).await.unwrap_or_default();
            let parent = parent.trim();
            if !sha(parent) { return none("CI has no run anywhere in the target's history that reaches the head.".into()); }
            // Each look writes receipts named for its poll, and none may be written twice: an ancestor's gets a number no wait reaches.
            let numbered = 0x4000_0000u32.saturating_add(poll.saturating_mul(32)).saturating_add(step);
            if let Some(observed) = self.ci_poll(binding, parent, numbered, cancel).await? {
                return Ok(match observed.status {
                    CiStatus::Passed | CiStatus::Failed => Behind::Verdict(observed.status, parent.to_string()),
                    _ => Behind::Waiting,
                });
            }
            commit = parent.to_string();
        }
        none(format!("CI has no run on the target's head, nor on the {WALK} commits behind it that touch only files CI ignores. Ask the orchestrator to push a commit CI will run on."))
    }
    /// Only a push still being judged by its own group can lend its head to the next gate.
    /// Read all boards: the landing resource is shared across workspaces too.
    fn pending_landing(&self, head: &str, resource: &str) -> Result<Option<(String, String)>> {
        for workspace in self.store.workflow_workspaces()? {
            if let Some(state) = self.store.workflow_state(&workspace)? {
                if let Some((id, _)) = state.items.iter().find(|(_, i)| i.group_of.as_ref().is_some_and(|g|
                    i.members.iter().any(|m| state.items.get(&m.item)
                        .and_then(|t| t.definition.groups.get(&g.name)).is_some_and(|d| d.exclusive == resource)))
                    && i.pushed.as_deref() == Some(head)
                    && !matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled)) {
                    return Ok(Some((workspace, id.clone())));
                }
            }
        }
        Ok(None)
    }
    /// Read the base step's retained dependency even if CI ended during the fold or gate.
    /// Looking only at live groups at push time would miss a failure in between.
    fn base_predecessor(&self, binding: &Binding, base: &str) -> Result<Option<(String, String)>> {
        for stored in self.store.workflow_outbox(&binding.workspace)?.into_iter().rev() {
            if stored.effect.item != binding.effect.item || stored.effect.fence >= binding.effect.fence
                || stored.effect.kind != EffectKind::Execute(StageKind::AwaitBase) { continue; }
            let path = self.root.join("receipts").join(format!("{}.verb.json", digest(&stored.effect)));
            let fact: VerbFact = serde_json::from_slice(&self.bytes(&path)?)?;
            if fact.binding.effect != stored.effect || fact.candidate != base || fact.label != "passed" {
                return Err(Error::Refused("fold has no matching base settlement"));
            }
            return fact.detail.get("predecessor").filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(Error::from)).transpose();
        }
        Ok(None) // Older admitted folds did not retain this dependency.
    }
    /// A speculative gate cannot publish until the group that lent it its base has its answer: `true` when it
    /// landed green, `false` when it failed or was cancelled after its push. That group's own pinned CI step
    /// decides, including every additional workflow and any permitted rerun.
    async fn wait_landing(&self, binding: &Binding, predecessor: &(String, String), base: &str,
        cancel: &tokio::sync::watch::Receiver<bool>) -> Result<bool>
    {
        let state = self.store.workflow_state(&binding.workspace)?.ok_or(Error::Refused("missing board"))?;
        let item = state.items.get(&binding.effect.item).ok_or(Error::Refused("missing group"))?;
        let (every, patience) = item.definition.stages.iter().find(|s| s.kind == StageKind::AwaitBase)
            .and_then(|s| wait_times(&s.operation)).ok_or(Error::Refused("missing base wait"))?;
        let deadline = binding.effect.created_ms.saturating_add(i64::try_from(patience).unwrap_or(i64::MAX));
        loop {
            if *cancel.borrow() { return Err(Error::Refused("push wait interrupted")); }
            let state = self.store.workflow_state(&predecessor.0)?.ok_or(Error::Refused("missing predecessor board"))?;
            let phase = if let Some(item) = state.items.get(&predecessor.1) {
                if item.pushed.as_deref() != Some(base) { return Err(Error::Refused("predecessor changed")); }
                &item.phase
            } else {
                &state.archive.get(&predecessor.1).filter(|a| a.group.as_ref()
                    .is_some_and(|g| g.pushed.as_deref() == Some(base)))
                    .ok_or(Error::Refused("missing predecessor"))?.phase
            };
            match phase {
                Phase::Done => return Ok(true),
                Phase::Failed | Phase::Cancelled => return Ok(false),
                _ => {}
            }
            if self.clock_ms() >= deadline { return Err(Error::Refused("previous landing wait timed out")); }
            if pause(every, cancel).await { return Err(Error::Refused("push wait interrupted")); }
        }
    }
    fn poll_log(&self, binding: &Binding, poll: u32) -> PathBuf {
        self.root.join("receipts").join(format!("{}.poll-{poll}.fetch.output", digest(&binding.effect)))
    }
    /// Bring the target's head into the local repository, under a private ref, and read it there.
    /// Another lander's commits are not here until this runs. A fetch that fails is a poll with no answer.
    async fn fetch_base(&self, binding: &Binding, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<String>> {
        let identity = self.landing()?;
        self.refuse_redirected_transport(binding).await?;
        let log = self.poll_log(binding, poll);
        if let Some(url) = super::coding::landing_url(identity) { return self.fetch_target(identity, url, &log).await; }
        self.fetch_local_target(identity, &log, Duration::from_secs(60), cancel).await
    }

    pub(super) async fn fetch_local_target(&self, identity: &CodingLanding, log: &Path, wait: Duration,
        cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<String>>
    {
        let local = Self::base_ref(identity);
        let (program, environment) = self.git_environment().await?;
        let argv = vec![program, "fetch".into(), "--quiet".into(), "--no-tags".into(), "--no-recurse-submodules".into(),
            identity.repository.to_string_lossy().into_owned(), format!("+{}:{local}", identity.target)];
        durable(log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, log, wait, cancel.clone()).await?;
        if exit != Some(0) { return Ok(None); }
        let head = adapters::git(&self.repository(), &["rev-parse", &local]).await?;
        Ok(sha(&head).then_some(head))
    }

    /// Wait for CI on the pushed commit. One effect that looks again and again: a passing run is
    /// `passed`, a failing one `rejected`, a canceled one `cancelled`, and no answer in time `timeout`.
    async fn await_ci(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let (every, patience) = wait_times(&binding.effect.operation).ok_or(Error::Refused("unsound wait times"))?;
        let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing CI candidate"))?;
        let deadline = binding.effect.created_ms.saturating_add(i64::try_from(patience).unwrap_or(i64::MAX));
        for poll in 0u32.. {
            if *cancel.borrow() { return Ok(Outcome::Interrupted); }
            let CiObservation { mut status, run, retry_pending } = self.ci_poll(binding, &candidate, poll, &cancel)
                .await?.unwrap_or(CiObservation { status: CiStatus::Unknown, run: None, retry_pending: false });
            if let Some(retry) = &binding.effect.ci_retry {
                if !run.as_ref().is_some_and(|r| r.id == retry.run.id && r.attempt > retry.run.attempt) {
                    status = CiStatus::Unknown;
                }
            }
            // An active controller or rerun owns the retry budget. Keep looking
            // even if the original CI wait has used up its patience.
            let label = match (retry_pending, status) {
                (false, CiStatus::Passed) => "passed",
                (false, CiStatus::Failed) => "rejected",
                (false, CiStatus::Superseded) => "cancelled",
                (false, _) if self.clock_ms() >= deadline => "timeout",
                _ => {
                    if pause(every, &cancel).await { return Ok(Outcome::Interrupted); }
                    continue;
                }
            };
            let mut detail = serde_json::json!({ "polls": poll + 1, "run": run });
            if label != "passed" {
                detail["failure"] = serde_json::to_value(self.ci_failure(binding, label, run.as_ref(), poll).await?)?;
            }
            self.write_fact(binding, label, &candidate, detail)?;
            return self.reconcile_verb(binding, &StageKind::AwaitCi).await;
        }
        unreachable!("the poll count is unbounded")
    }

    async fn ci_failure(&self, binding: &Binding, label: &str, run: Option<&CiRun>, poll: u32) -> Result<farcooler_workflow::GroupFailure> {
        let path = self.root.join("receipts").join(format!("{}.poll-{poll}.provider.json", digest(&binding.effect)));
        let receipt: Option<CodingCiReceipt> = match self.bytes(&path) {
            Ok(raw) => Some(serde_json::from_slice(&raw)?),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let conclusion = receipt.as_ref().and_then(|r| r.run["conclusion"].as_str());
        let relevant = match label {
            "cancelled" => conclusion == Some("cancelled"),
            "rejected" => conclusion.is_some_and(|c| c != "success" && c != "cancelled"),
            _ => true,
        };
        let run = run.filter(|_| relevant);
        let subject = run.map_or_else(|| "a required CI workflow".to_string(), |r| format!("CI run {}, attempt {}", r.id, r.attempt));
        let reason = match label {
            "cancelled" if run.is_none() => "A required CI workflow was cancelled.".into(),
            "cancelled" => format!("{subject} was cancelled."),
            "timeout" => format!("Timed out waiting for a final result from {subject}."),
            _ if run.is_none() => "A required CI workflow failed.".into(),
            _ => format!("{subject} failed."),
        };
        if label == "rejected" && let Some(mut failure) = receipt.and_then(|r| r.failure) {
            failure.job = failure.job.chars().filter(|c| !c.is_control()).take(256).collect();
            failure.first_line = failure.first_line.chars().filter(|c| !c.is_control()).take(1000).collect();
            if failure.first_line.trim().is_empty() { failure.first_line = reason.clone(); }
            failure.split = false;
            if !failure.job.is_empty() { return Ok(failure); }
        }
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        if let Some(run) = run
            && (label == "cancelled" || (label == "rejected" && run.failed_jobs.is_empty()))
            && config.ci.command.environment.get("CI_GH").is_some_and(|p| Path::new(p).is_absolute())
        {
            let path = format!("repos/{}/actions/runs/{}/attempts/{}/jobs?per_page=100", config.ci.repository, run.id, run.attempt);
            let filter = r#"[.[].jobs[] | select(.conclusion == "cancelled" and (.steps | length) == 0 and (.runner_name // "") == "") | .name][0] // empty"#;
            if let Ok((Some(0), raw)) = self.ci_gh(binding, "cancelled-job",
                &["api", "--paginate", "--slurp", &path, "--jq", filter]).await
            {
                let job: String = String::from_utf8_lossy(&raw).trim().chars().filter(|c| !c.is_control()).take(256).collect();
                if !job.is_empty() {
                    return Ok(farcooler_workflow::GroupFailure {
                        job, first_line: format!("{reason} This job never acquired a runner."), split: false,
                    });
                }
            }
        }
        Ok(farcooler_workflow::GroupFailure {
            job: run.and_then(|r| r.failed_jobs.first()).map(|job| job.chars().filter(|c| !c.is_control()).take(256).collect::<String>())
                .filter(|job| !job.is_empty()).unwrap_or_else(|| "CI".into()),
            first_line: reason,
            split: false,
        })
    }
    /// One look at CI on `sha`, over every workflow the step waits on. The configured workflow must have a run; each
    /// other one a CI wait names (ov-1071) counts once the push started it. GitHub starts every workflow a push
    /// triggers together, so one with no run beside the configured one's is one this push didn't trigger (a path filter).
    /// The run facts are the configured workflow's: that is the run a CI policy reruns. A retry in any
    /// required workflow can defer settlement. `None` means the provider answered and found no configured run.
    async fn ci_poll(&self, binding: &Binding, sha: &str, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<CiObservation>> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        let also = match &binding.effect.kind {
            EffectKind::Execute(StageKind::AwaitCi) => match binding.effect.operation.arguments.get(4) {
                Some(text) => super::coding::also_workflows(text).ok_or(Error::Refused("unsound CI workflows"))?,
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        let Some(mut observed) = self.ci_look(binding, sha, poll, &config.ci.workflow, 0, cancel).await? else { return Ok(None) };
        let mut all = vec![observed.status];
        for (index, name) in also.iter().enumerate() {
            if let Some(other) = self.ci_look(binding, sha, poll, name, index + 1, cancel).await? {
                all.push(other.status);
                observed.retry_pending |= other.retry_pending;
            }
        }
        observed.status = combined(&all);
        Ok(Some(observed))
    }
    /// One look at one workflow's newest run on `sha`: the configured provider hook runs once, told which workflow in
    /// `FARCOOLER_CI_WORKFLOW`, and writes a receipt. A hook that fails couldn't say (`Unknown`); one that succeeds and
    /// writes nothing found no run (`None`). A receipt about something else is an error. Only the configured workflow
    /// (`index` 0) follows a CI retry's exact run; the others look for their newest run on `sha`.
    async fn ci_look(&self, binding: &Binding, sha: &str, poll: u32, workflow: &str, index: usize,
        cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<CiObservation>> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        // The configured workflow keeps the names it always had.
        let stem = match index { 0 => format!("{}.poll-{poll}", digest(&binding.effect)), n => format!("{}.poll-{poll}.ci-{n}", digest(&binding.effect)) };
        let (receipt, log) = (self.root.join("receipts").join(format!("{stem}.provider.json")),
            self.root.join("receipts").join(format!("{stem}.output")));
        let mut command = config.ci.command.clone();
        self.pin_context(&mut command, binding, sha, &receipt)?;
        command.environment.insert("FARCOOLER_CI_WORKFLOW".into(), workflow.into());
        if index != 0 { command.environment.remove("FARCOOLER_CI_RUN"); }
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &command.argv, &command.environment, &log,
            Duration::from_millis(command.timeout_ms), cancel.clone()).await?;
        if exit != Some(0) {
            return Ok(Some(CiObservation { status: CiStatus::Unknown, run: None, retry_pending: false }));
        }
        let raw = match self.bytes(&receipt) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let provided: CodingCiReceipt = serde_json::from_slice(&raw)?;
        if provided.binding != digest(binding) || provided.repository != config.ci.repository {
            return Err(Error::Refused("foreign CI provider identity"));
        }
        let run = crate::ci_watch::parse_run(&serde_json::to_vec(&provided.run)?).ok_or(Error::Refused("invalid CI receipt"))?;
        if run.id == 0 || run.head_sha != sha || run.name != workflow || run.event != config.ci.event {
            return Err(Error::Refused("foreign CI run provenance"));
        }
        let facts = provided.run["run_attempt"].as_u64().and_then(|n| u32::try_from(n).ok())
            .map(|attempt| CiRun { id: run.id, attempt, failed_jobs: provided.failed_jobs.unwrap_or_default(),
                // A step the run's facts can't hold (an overlong name) is dropped: its job then never reads as setup-only.
                failed_steps: provided.failed_steps.unwrap_or_default().into_iter().take(256)
                    .filter(FailedStep::valid).collect() });
        if facts.as_ref().is_some_and(|r| !r.valid()) { return Err(Error::Refused("invalid CI run facts")); }
        let attempt = facts.as_ref().map(|r| u64::from(r.attempt));
        let pending = config.ci.command.environment.get("CI_GH").is_some_and(|p| Path::new(p).is_absolute())
            && crate::ci_watch::retry_pending(&run, attempt, self.clock_ms().div_euclid(1000), |path| async move {
                use crate::ci_watch::RetryRead;
                let endpoint = format!("repos/{}/{path}", config.ci.repository);
                match self.ci_gh(binding, "retry-look", &["api", "-X", "GET", &endpoint]).await {
                    Ok((Some(0), raw)) => RetryRead::Data(raw),
                    Ok((Some(_), raw)) => RetryRead::error(&raw),
                    _ => RetryRead::Unavailable,
                }
            }).await;
        let status = if pending { CiStatus::Queued }
            else { crate::ci_watch::summarize(&format!("sha:{sha}"), &[(run, None)]).status };
        Ok(Some(CiObservation { status, run: facts, retry_pending: pending }))
    }

    /// Apply every member's commits, in order, onto the base in a fresh tree. The tree is made new
    /// each time, so a group that starts over after a split or a moved base keeps nothing of its last try.
    /// A commit that does not apply is the label `conflict`.
    async fn fold(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let base = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("fold needs the base to build on"))?;
        let members = &binding.effect.members;
        if members.is_empty() || members.iter().any(|m| !sha(&m.candidate)) { return Err(Error::Refused("fold needs pinned members")); }
        let (repo, mut tree) = (self.repository(), self.tree(binding));
        let branch = format!("wf-{}", &digest(&(&binding.workspace, &binding.effect.item))[4..]);
        // Until none is left: a tree under its name before ov-1081 goes too, so this one is made under the new name.
        while tree.try_exists()? {
            self.try_git(binding, &repo, &["worktree", "remove", "--force", &tree.to_string_lossy()]).await?;
            if tree.try_exists()? { std::fs::remove_dir_all(&tree)?; }
            tree = self.tree(binding);
        }
        let tree_path = tree.to_string_lossy().into_owned();
        self.try_git(binding, &repo, &["worktree", "prune"]).await?;
        if self.try_git(binding, &repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await?.0 {
            self.git_recorded(binding, &repo, &["branch", "-D", &branch]).await?;
        }
        // Git holds an "initializing" lock through worktree add's checkout.
        // Materialize the files after registration so a checkout timeout
        // leaves a worktree the next fold attempt can remove.
        self.git_recorded(binding, &repo, &["worktree", "add", "--no-checkout", "-b", &branch, &tree_path, &base]).await?;
        self.git_recorded(binding, &tree, &["read-tree", "-u", "-m", "HEAD"]).await?;
        spotlight::exclude_builds(&tree).await;
        for member in members {
            if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
            if let Some(conflict) = self.pick_member(Some(binding), &tree, member).await? {
                // Diagnose the whole batch on main, retaining only successful members. This
                // catches both conflicts with main and conflicts between tickets in one pass.
                let mut conflicts = Vec::new();
                let mut files = BTreeSet::new();
                self.git_recorded(binding, &tree, &["reset", "--hard", &base]).await?;
                for suspect in members {
                    if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
                    let checkpoint = self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
                    if let Some(detail) = self.pick_member(Some(binding), &tree, suspect).await? {
                        files.extend(detail.files);
                        conflicts.push(suspect.item.clone());
                        // Aborting a pick only undoes its last commit; discard every earlier
                        // commit from this member before testing the next member.
                        self.git_recorded(binding, &tree, &["reset", "--hard", &checkpoint]).await?;
                    }
                }
                self.git_recorded(binding, &tree, &["reset", "--hard", &base]).await?;
                self.write_fact(binding, "conflict", &base, serde_json::json!({
                    "members": conflicts, "files": files, "first_conflict": conflict }))?;
                return self.reconcile_verb(binding, &StageKind::Fold).await;
            }
        }
        if !self.git_recorded(binding, &tree, &["status", "--porcelain", "--untracked-files=all"]).await?.is_empty() {
            return Err(Error::Refused("fold left the tree dirty"));
        }
        let folded = self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
        if !sha(&folded) { return Err(Error::Refused("invalid folded SHA")); }
        let index = serde_json::to_vec(&serde_json::json!({ "item": binding.effect.item, "base": base, "folded": folded }))?;
        match durable(&self.fold_index(binding, &folded), &index) {
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists
                && self.bytes(&self.fold_index(binding, &folded))? == index => {}
            other => other?,
        }
        self.write_fact(binding, "passed", &folded, serde_json::json!({ "base": base, "members": members.len() }))?;
        self.reconcile_verb(binding, &StageKind::Fold).await
    }

    /// Apply one pinned member, skipping changes already on the base. Abort a conflict before
    /// returning so diagnosis and the next member always start from a settled tree.
    pub(super) async fn pick_member(&self, binding: Option<&Binding>, tree: &Path,
        member: &farcooler_workflow::Member) -> Result<Option<PickConflict>>
    {
        // Git omits commits already in HEAD's ancestry and marks equivalent patches with `-`.
        // Independent floor increases can have identical patches. Use the source SHA recorded by
        // cherry-pick -x to recognize shared floor commits, even after adjusting their delta.
        let listed = self.pick_read(binding, tree, &["cherry", "HEAD", &member.candidate]).await?;
        for (sign, commit) in listed.lines().filter_map(|line| line.split_once(' ')) {
            if !matches!(sign, "+" | "-") || !sha(commit) { continue; }
            let mut args = vec!["diff-tree", "-r", "--no-commit-id", "--name-only", "--diff-filter=M", commit, "--"];
            args.extend(SWIFT_FLOORS);
            let changed = self.pick_read(binding, tree, &args).await?;
            let floors: Vec<_> = changed.lines().collect();
            if !floors.is_empty() {
                let origin = format!("--grep=^(cherry picked from commit {commit})$");
                if !self.pick_read(binding, tree, &["log", "-1", "--format=%H", "--basic-regexp", &origin, "HEAD"]).await?.is_empty() {
                    continue;
                }
            }
            if sign == "-" {
                self.restore_collapsed_floors(binding, tree, commit, &floors, FloorPick::Skipped).await?;
                continue;
            }
            let pick = |extra: &[&'static str]| -> Vec<&'static str> {
                COMMIT_IDENTITY.iter().copied().chain(extra.iter().copied()).collect()
            };
            let mut args = pick(&["cherry-pick", "-x"]); args.push(commit);
            if self.pick_git(binding, tree, &args).await?.0 {
                self.restore_collapsed_floors(binding, tree, commit, &floors, FloorPick::Applied).await?;
                continue;
            }
            let unmerged = self.pick_read(binding, tree, &["diff", "--name-only", "--diff-filter=U"]).await?;
            let clean = self.pick_git(binding, tree, &["diff", "--cached", "--quiet"]).await?.0
                && self.pick_git(binding, tree, &["diff", "--quiet"]).await?.0;
            if unmerged.is_empty() && clean && self.pick_git(binding, tree, &pick(&["cherry-pick", "--skip"])).await?.0 {
                self.restore_collapsed_floors(binding, tree, commit, &floors, FloorPick::Skipped).await?;
                continue;
            }
            // Only confirmed unmerged paths diagnose a member; infrastructure failures retry the fold.
            if unmerged.is_empty() { return Err(Error::Refused("fold cherry-pick failed without conflicts")); }
            let files: Vec<_> = unmerged.lines().collect();
            if self.resolve_baselines(binding, tree, &files, &[]).await? {
                let (finish, floor_pick) = if self.pick_git(binding, tree, &["diff", "--cached", "--quiet"]).await?.0 {
                    ("--skip", FloorPick::Skipped)
                } else {
                    ("--continue", FloorPick::Applied)
                };
                self.pick_read(binding, tree, &pick(&["cherry-pick", finish])).await?;
                self.restore_collapsed_floors(binding, tree, commit, &floors, floor_pick).await?;
                continue;
            }
            self.pick_read(binding, tree, &pick(&["cherry-pick", "--abort"])).await?;
            // A copy of this change on the tree under other context (a later commit of the same ticket
            // touched the same file) is not a conflict: the change is already there.
            if self.change_is_in_tree(binding, tree, commit).await? { continue; }
            return Ok(Some(PickConflict { member: member.item.clone(), commit: commit.into(),
                files: unmerged.lines().map(str::to_owned).collect() }));
        }
        Ok(None)
    }

    /// Each independent pick owes its increase, even when Git produced no commit.
    async fn restore_collapsed_floors(&self, binding: Option<&Binding>, tree: &Path, commit: &str,
        paths: &[&str], pick: FloorPick) -> Result<()>
    {
        if paths.is_empty() { return Ok(()); }
        let flag = match pick {
            FloorPick::Applied => "--after-pick",
            FloorPick::Skipped => "--skipped-pick",
        };
        if self.resolve_baselines(binding, tree, paths, &[flag, commit]).await? {
            let mut args = COMMIT_IDENTITY.to_vec();
            let origin = format!("(cherry picked from commit {commit})");
            match pick {
                FloorPick::Applied => args.extend(["commit", "--amend", "--no-edit"]),
                FloorPick::Skipped => args.extend(["commit", "-m", "Preserve Swift test floor increase", "-m", &origin]),
            }
            self.pick_read(binding, tree, &args).await?;
        }
        Ok(())
    }

    /// Resolve only known, regular baseline files using bundled source. A floor repair may
    /// replace a subset of the requested floors. A conflict must be resolved in full.
    async fn resolve_baselines(&self, binding: Option<&Binding>, tree: &Path, paths: &[&str], repair: &[&str]) -> Result<bool> {
        if !paths.iter().all(|path| SWIFT_FLOORS.contains(path)
            || matches!(*path, "scripts/daemon-layers.json" | "scripts/daemon-layers-counts.txt")) {
            return Ok(false);
        }
        let root = tree.canonicalize()?;
        for path in paths {
            let parent = root.join(path).parent().ok_or(Error::Refused("baseline file has no parent"))?.to_path_buf();
            if parent.canonicalize()? != parent { return Ok(false); }
        }
        let mut command = std::process::Command::new("/usr/bin/python3");
        command.env_clear();
        crate::git_guard::apply(&mut command, &crate::git_guard::fixed());
        command.args(["-I", "-c", include_str!("../../../../scripts/fold-baselines.py"),
            include_str!("../../../../scripts/daemon-layers.py")]);
        command.args(repair).args(paths).current_dir(&root).env("PATH", "/usr/bin:/bin").env("GIT_OPTIONAL_LOCKS", "0");
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output()).await
            .map_err(|_| Error::Refused("fold baseline resolver timed out"))??;
        if !output.status.success() {
            tracing::warn!(error = %String::from_utf8_lossy(&output.stderr), "fold baseline resolver failed");
            return Err(Error::Refused("fold baseline resolver failed"));
        }
        let replacements: BTreeMap<String, String> = serde_json::from_slice(&output.stdout)?;
        if replacements.is_empty() { return Ok(false); }
        if (repair.is_empty() && replacements.len() != paths.len())
            || replacements.keys().any(|path| !paths.contains(&path.as_str())) {
            return Err(Error::Refused("unexpected baseline resolutions"));
        }
        for (path, text) in &replacements {
            let destination = root.join(path);
            let parent = destination.parent().ok_or(Error::Refused("baseline file has no parent"))?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            std::io::Write::write_all(&mut file, text.as_bytes())?;
            file.persist(destination).map_err(|error| error.error)?;
        }
        let mut add = vec!["add", "--"];
        add.extend(replacements.keys().map(String::as_str));
        self.pick_read(binding, tree, &add).await?;
        Ok(true)
    }

    async fn pick_git(&self, binding: Option<&Binding>, tree: &Path, args: &[&str]) -> Result<(bool, String)> {
        if let Some(binding) = binding {
            let (ok, stdout, _) = self.try_git(binding, tree, args).await?;
            return Ok((ok, stdout));
        }
        // A linked worktree shares the repository's recorded conflict resolutions.
        let mut probe = vec!["-c", "rerere.enabled=false"];
        probe.extend_from_slice(args);
        let output = crate::git::git(tree, &probe).await.map_err(|_| Error::Refused("queue Git check failed"))?;
        Ok((output.ok, output.stdout.trim().into()))
    }

    async fn pick_read(&self, binding: Option<&Binding>, tree: &Path, args: &[&str]) -> Result<String> {
        let (ok, stdout) = self.pick_git(binding, tree, args).await?;
        if !ok { return Err(Error::Refused("Git refused while applying landing commits")); }
        Ok(stdout)
    }

    /// Whether the tree already holds a commit with exactly this commit's lines changed, whatever its sha
    /// and context: the commits since the two last met are compared by their zero-context change.
    async fn change_is_in_tree(&self, binding: Option<&Binding>, tree: &Path, commit: &str) -> Result<bool> {
        let change = |commit: String| async move {
            let diff = self.pick_read(binding, tree, &["diff-tree", "-p", "-U0", "--no-commit-id", "-r", &commit]).await?;
            Ok::<_, Error>(diff.lines().filter(|l| !l.starts_with("index ") && !l.starts_with("@@")).collect::<Vec<_>>().join("\n"))
        };
        let wanted = change(commit.to_string()).await?;
        if wanted.is_empty() { return Ok(false); }
        let fork = self.pick_read(binding, tree, &["merge-base", "HEAD", commit]).await?;
        let range = format!("{fork}..HEAD");
        for held in self.pick_read(binding, tree, &["rev-list", &range]).await?.lines().filter(|c| !c.is_empty()) {
            if change(held.to_string()).await? == wanted { return Ok(true); }
        }
        Ok(false)
    }

    /// Push the folded commit to the target, once, and only while the target is still at the base the
    /// gate ran on. A target that moved, or a push the remote refused, is `base_moved`: nothing landed,
    /// so starting over from the new base is safe. A timeout or a lost connection is not: that is uncertain.
    /// The push is a plain fast-forward, never forced, and a candidate that doesn't build on the base in the
    /// private landing repository is `rejected` before anything is sent.
    async fn push(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        // Never re-enter a send. A recovered pre-send effect is rescheduled with a fresh fence.
        if self.verb_path(binding).try_exists()? || self.push_phase_path(binding, "waiting").try_exists()?
            || self.push_phase_path(binding, "send").try_exists()? {
            return self.reconcile_verb(binding, &StageKind::Push).await;
        }
        durable(&self.push_phase_path(binding, "waiting"), &serde_json::to_vec(binding)?)?;
        let identity = self.landing()?.clone();
        let url = super::coding::landing_url(&identity).map(str::to_string);
        let url = url.as_deref();
        let read_log = |tag: &str| self.root.join("receipts").join(format!("{}.target-{tag}.output", digest(&binding.effect)));
        // Everything up to the send changes nothing outside: a failure here is retried, not uncertain.
        let checked = async {
            self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.validate()?;
            let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing push candidate"))?;
            // The tree is exactly what the gate judged.
            if self.verify_tree(binding).await? != candidate { return Err(Error::Refused("pushed candidate is not the gated tree")); }
            let index: serde_json::Value = serde_json::from_slice(&self.bytes(&self.fold_index(binding, &candidate))?)?;
            let base = index["base"].as_str().filter(|b| sha(b)).ok_or(Error::Refused("push has no fold record"))?.to_string();
            if index["folded"] != candidate || index["item"] != binding.effect.item {
                return Err(Error::Refused("fold record is for another commit"));
            }
            if url.is_none() && adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
                return Err(Error::Refused("landing transport is not bare"));
            }
            self.refuse_redirected_transport(binding).await?;
            if url.is_some() { self.stage_candidate(binding, &candidate).await?; }
            let builds = self.builds_on(&base, &candidate).await?;
            let before = self.target_head(&identity, &read_log("before")).await?;
            let predecessor = self.base_predecessor(binding, &base)?;
            Ok((candidate, base, builds, before, predecessor))
        }.await;
        let (candidate, base, builds, before, predecessor) = match checked {
            Ok(checked) => checked,
            Err(error) => {
                tracing::warn!(effect = %binding.effect.id, %error, "push refused before anything was sent; it will be tried again");
                return Ok(Outcome::SafeFailure);
            }
        };
        if !builds {
            // Its history, read where no agent can rewrite it, doesn't hold the base: sending it would drop commits.
            self.write_fact(binding, "rejected", &candidate, serde_json::json!({ "base": base, "sent": false,
                "why": "The landing candidate doesn't build on the target's head, so nothing was sent." }))?;
            return self.reconcile_verb(binding, &StageKind::Push).await;
        }
        let Some(before) = before else {
            // A remote that turned this runner's sign-in down won't let it in on a later attempt either.
            if let Some(why) = url.and(sign_in_refusal(&String::from_utf8_lossy(&self.bytes(&read_log("before"))?))) {
                self.write_fact(binding, "rejected", &candidate, serde_json::json!({ "base": base, "sent": false, "why": why }))?;
                return self.reconcile_verb(binding, &StageKind::Push).await;
            }
            tracing::warn!(effect = %binding.effect.id, "the landing remote did not answer before the push; it will be tried again");
            return Ok(Outcome::SafeFailure);
        };
        if before != base {
            // Another lander got there after the gate; nothing was sent.
            self.write_fact(binding, "base_moved", &candidate, serde_json::json!({ "base": base, "target": before, "sent": false }))?;
            return self.reconcile_verb(binding, &StageKind::Push).await;
        }
        // A previous landing whose CI went red is still the target's head, and the gate ran on top of it: pushing
        // can't make the target worse, and a fix for it lands now. Starting over would fold and gate on the same
        // red head again, and the group's own CI decides either way (ov-1291).
        let mut red_base = false;
        if let Some(predecessor) = predecessor {
            match self.wait_landing(binding, &predecessor, &base, &cancel).await {
                Ok(passed) => red_base = !passed,
                Err(error) => {
                    tracing::warn!(%error, "previous landing has not passed; nothing was sent");
                    return Ok(if *cancel.borrow() { Outcome::Interrupted } else { Outcome::SafeFailure });
                }
            }
        }
        if *cancel.borrow() { return Ok(Outcome::Interrupted); }
        // Hold the writer lock before publishing send intent. A local receive-pack keeps it even
        // when the runner or push process dies. A network receiver cannot inherit a local lock.
        let writer = self.lock_push(binding)?;
        // Fsync intent before invoking either transport. A crash from here on can never retry the send.
        durable(&self.push_phase_path(binding, "send"), &serde_json::to_vec(binding)?)?;
        let log = self.remote_log(binding);
        let (argv, environment, exit) = if let Some(url) = url {
            self.push_remote(url, &identity, &candidate, &log, cancel).await?
        } else {
            let (program, environment) = self.git_environment().await?;
            let argv = vec![program, "push".into(), "--porcelain".into(),
                "--no-follow-tags".into(), "--recurse-submodules=no".into(), "--signed=false".into(),
                identity.repository.to_string_lossy().into_owned(), super::coding_landing::push_target(&candidate, &identity.target)];
            durable(&log, b"")?;
            let exit = crate::effect_runner::configured_with_git_lock(&self.repository(), &argv, &environment, &log,
                Duration::from_secs(60), cancel, &writer).await?;
            (argv, environment, exit)
        };
        let output = self.bytes(&log)?;
        durable(&self.push_process_path(binding),
            &serde_json::to_vec(&serde_json::json!({ "binding": binding, "identity": identity, "argv": argv,
                "environment": environment, "exit": exit, "log_digest": digest(&output) }))?)?;
        let after = self.target_head(&identity, &read_log("after")).await.ok().flatten();
        let porcelain = String::from_utf8_lossy(&output).into_owned();
        // A remote that refused the push while the target still sits at the base did not refuse it for a moved
        // base: a protected branch, a hook, or no right to push. Starting over would be refused the same way.
        let refused = match classify_push(exit, &porcelain, after.as_deref(), &candidate) {
            PushVerdict::Refused if after.as_deref() == Some(base.as_str()) => Some((refusal(&porcelain), true)),
            // Over a network, a client that exited with no word on the ref never got as far as sending it: it
            // could not sign in, or reach the host, or find the repository. The target says nothing came.
            PushVerdict::Unknown if url.is_some() && exit.is_some_and(|e| e != 0) && after.as_deref() == Some(base.as_str())
                && !porcelain.lines().any(|l| l.split('\t').nth(1).is_some_and(|r| r.ends_with(&format!(":{}", identity.target)))) => Some((unsent(&porcelain), false)),
            _ => None,
        };
        if let Some((why, sent)) = refused {
            tracing::warn!(effect = %binding.effect.id, why, "the landing remote refused the push");
            self.write_fact(binding, "rejected", &candidate, serde_json::json!({ "base": base, "target": after, "sent": sent,
                "why": why, "log": digest(&output) }))?;
            return self.reconcile_verb(binding, &StageKind::Push).await;
        }
        match classify_push(exit, &porcelain, after.as_deref(), &candidate) {
            PushVerdict::Landed => {
                let mut detail = serde_json::json!({ "base": base, "log": digest(&output) });
                if red_base { detail["red_base"] = true.into(); }
                self.write_fact(binding, "passed", &candidate, detail)?;
                // The local copy of the target follows, when it was where this push started. Best effort: the remote is the record.
                // Only for a landing on this machine: a network landing never writes a ref in the shared repository, whose
                // main belongs to its owner (ov-1058 review).
                let local = if url.is_none() { adapters::git(&self.repository(), &["rev-parse", self.landing_ref()]).await.ok() } else { None };
                if local.as_deref() == Some(base.as_str()) {
                    let _ = self.try_git(binding, &self.repository(), &["update-ref", "-m", &format!("workflow:{}", digest(binding)),
                        self.landing_ref(), &candidate, &base]).await;
                }
            }
            PushVerdict::Refused => self.write_fact(binding, "base_moved", &candidate,
                serde_json::json!({ "base": base, "target": after, "sent": true, "log": digest(&output) }))?,
            PushVerdict::Unknown => return Err(Error::Refused("push outcome unknown; inspect receipt")),
        }
        self.reconcile_verb(binding, &StageKind::Push).await
    }
}

/// What the base step found behind a head CI never ran on (ov-1243).
enum Behind {
    /// An ancestor reachable over commits CI ignores has this verdict.
    Verdict(CiStatus, String),
    /// That ancestor's run is still going.
    Waiting,
    /// No ancestor can speak for the head; the sentence says why.
    None(String),
}

/// The sentence a push the remote refused at the ref ends its run with.
fn refusal(porcelain: &str) -> &'static str {
    let said = porcelain.to_ascii_lowercase();
    if said.contains("protected branch") { "The landing remote's branch protection refused the push. Nothing landed." }
    else if said.contains("hook declined") { "A hook on the landing remote refused the push. Nothing landed." }
    else { "The landing remote refused the push, and the target hasn't moved. Nothing landed." }
}
/// The sentence a push that never reached the ref ends its run with.
fn unsent(log: &str) -> &'static str {
    sign_in_refusal(log).unwrap_or("The push couldn't reach the landing remote, so nothing was sent.")
}
/// The sentence for a call the landing remote turned away for good: this runner's sign-in, an unknown host key,
/// or no such repository. `None` for anything that may pass on a later look, such as an unreachable host.
fn sign_in_refusal(log: &str) -> Option<&'static str> {
    let said = log.to_ascii_lowercase();
    Some(if said.contains("permission denied") || said.contains("authentication failed") || said.contains("could not read username")
        || said.contains("returned error: 401") || said.contains("returned error: 403") {
        "The landing remote turned down this runner's sign-in, so nothing was sent. Check its git and ssh credentials."
    } else if said.contains("host key verification failed") {
        "This runner doesn't know the landing remote's host key, so nothing was sent. Connect once with ssh to trust it."
    } else if said.contains("repository not found") || said.contains("does not appear to be a git repository") {
        "The landing remote has no such repository, or this runner can't see it. Nothing was sent."
    } else { return None })
}
/// Whether `candidate` is on the target of the repository at `repository`. A remote that cannot be read is
/// an error, never an answer of "no": only an object the remote lacks, or one off the target, is absent.
pub(super) async fn on_remote(repository: &Path, target: &str, candidate: &str) -> Result<bool> {
    let git = async |args: &[&str]| {
        crate::git::git(repository, args).await.map_err(|_| Error::Refused("the landing remote cannot be read"))
    };
    let target_commit = format!("{target}^{{commit}}");
    let read = git(&["--no-replace-objects", "rev-parse", "--verify", "--quiet", &target_commit]).await?;
    let head = read.stdout.trim();
    if !read.ok || !sha(head) { return Err(Error::Refused("the landing target cannot be read")); }
    let candidate_commit = format!("{candidate}^{{commit}}");
    let known = git(&["--no-replace-objects", "rev-parse", "--verify", "--quiet", &candidate_commit]).await?;
    if known.code == Some(1) && known.stderr.is_empty() { return Ok(false); }
    if !known.ok { return Err(Error::Refused("the candidate cannot be read on the landing remote")); }
    match git(&["--no-replace-objects", "merge-base", "--is-ancestor", candidate, head]).await?.code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::Refused("the landing target's history cannot be read")),
    }
}

/// Sleep `every` milliseconds. True when a cancel came first.
pub(super) async fn pause(every: u64, cancel: &tokio::sync::watch::Receiver<bool>) -> bool {
    let mut watching = cancel.clone();
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(every)) => *cancel.borrow(),
        cancelled = watching.wait_for(|c| *c) => cancelled.is_ok(),
    }
}

/// Where CI stands over several workflows' runs (ov-1071): anything red is red, then anything a newer push superseded,
/// then all green is green; anything else is still going.
pub(super) fn combined(all: &[CiStatus]) -> CiStatus {
    if all.contains(&CiStatus::Failed) { CiStatus::Failed }
    else if all.contains(&CiStatus::Superseded) { CiStatus::Superseded }
    else if !all.is_empty() && all.iter().all(|s| *s == CiStatus::Passed) { CiStatus::Passed }
    else { CiStatus::Unknown }
}
