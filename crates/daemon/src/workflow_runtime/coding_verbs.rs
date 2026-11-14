//! The landing verbs of a group: wait for the base, fold, push, and wait for CI (ov-956).
//! `run` is the ordinary gate command and lives with the other processes in coding_execute.
//!
//! Each verb ends by writing one fact (its label, its candidate and what it saw) and then settles
//! from that fact, so the first run and a recovery after a crash take the same road. What a verb may
//! do with no fact after a crash depends on its class: a read is read again and a fold is made again
//! (`SafeFailure` runs the step anew), but an attempted push is never repeated and stays uncertain.
//! A durable pre-send record lets a push interrupted while waiting for predecessor CI resume safely.
use super::*;
use super::coding::{sha, wait_times};
use farcooler_store::board_ci::CiStatus;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerbFact {
    binding: Binding,
    label: String,
    candidate: String,
    detail: serde_json::Value,
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
    matches!(kind, StageKind::AwaitBase | StageKind::Fold | StageKind::Push | StageKind::AwaitCi | StageKind::RerunCi | StageKind::Route)
        || super::coding_pr::is_pr_verb(kind)
}
const COMMIT_IDENTITY: [&str; 8] = ["-c", "user.name=Far Cooler", "-c", "user.email=landing@farcooler.invalid",
    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];

impl Exec {
    pub(super) fn verb_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.verb.json", digest(&binding.effect)))
    }
    fn push_phase_path(&self, binding: &Binding, phase: &str) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push-{phase}.json", digest(&binding.effect)))
    }
    /// Only an authenticated pre-send record without send intent proves that replay is safe.
    /// Missing legacy records, unreadable records and any send intent retain push uncertainty.
    fn reconcile_unsent_push(&self, binding: &Binding) -> Result<Outcome> {
        if self.push_phase_path(binding, "send").try_exists()? {
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
    pub(super) fn landing(&self) -> Result<&CodingLanding> {
        self.coding.as_ref().and_then(|c| c.landing.as_ref()).ok_or(Error::Refused("landing verbs need a landing identity"))
    }
    pub(super) fn write_fact(&self, binding: &Binding, label: &str, candidate: &str, detail: serde_json::Value) -> Result<()> {
        durable(&self.verb_path(binding), &serde_json::to_vec(&VerbFact {
            binding: binding.clone(), label: label.into(), candidate: candidate.into(), detail })?)
    }
    /// A person's settlement replaces the fact a crash left, atomically; nothing else ever does.
    pub(super) fn settle_fact(&self, binding: &Binding, label: &str, candidate: &str, detail: serde_json::Value) -> Result<()> {
        let path = self.verb_path(binding);
        let temporary = path.with_extension("settling");
        let _ = std::fs::remove_file(&temporary);
        durable(&temporary, &serde_json::to_vec(&VerbFact { binding: binding.clone(), label: label.into(), candidate: candidate.into(), detail })?)?;
        std::fs::rename(&temporary, &path)?;
        std::fs::File::open(path.parent().ok_or(Error::Refused("missing parent"))?)?.sync_all()?;
        Ok(())
    }
    /// The local repository's configuration must not redirect or extend the transport.
    async fn refuse_redirected_transport(&self, binding: &Binding) -> Result<()> {
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if config.lines().any(|l| l.starts_with("url.") || l.starts_with("include.") || l.starts_with("includeif.")) {
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
        let manifest = digest(&(binding, &fact.candidate, &raw, &fact.label));
        let evidence = Evidence { candidate: fact.candidate, manifest: manifest.clone(),
            receipt: digest(&("coding-receipt/v1", manifest)), policy_pin: binding.effect.policy_pin.clone() };
        if *kind == StageKind::AwaitCi && (binding.effect.ci_retry.is_some()
            || fact.detail.get("run").is_some_and(|r| !r.is_null())) {
            return Ok(Outcome::CiResult { label: fact.label, evidence,
                run: fact.detail.get("run").filter(|r| !r.is_null()).map(|r| serde_json::from_value(r.clone())).transpose()?,
                failure: fact.detail.get("failure").filter(|f| !f.is_null()).map(|f| serde_json::from_value(f.clone())).transpose()? });
        }
        if *kind == StageKind::Fold && fact.label == "conflict" {
            return Ok(Outcome::FoldConflict { evidence,
                members: fact.detail.get("members").cloned().map(serde_json::from_value).transpose()?,
                files: serde_json::from_value(fact.detail.get("files").cloned().unwrap_or(serde_json::json!([])))? });
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
                let ci = if green && predecessor.is_none() { Some(self.ci_poll(binding, &head, poll, &cancel).await?.0) } else { None };
                if matches!(ci, None | Some(CiStatus::Passed | CiStatus::Failed)) {
                    let mut detail = serde_json::json!({ "polls": poll + 1, "predecessor": predecessor });
                    if ci == Some(CiStatus::Failed) { detail["red"] = true.into(); }
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
    /// A speculative gate cannot publish until the group that lent it its base succeeds. Its
    /// own pinned CI step decides, including every additional workflow and any permitted rerun.
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
        let local = Self::base_ref(identity);
        let (program, environment) = self.git_environment().await?;
        let argv = vec![program, "fetch".into(), "--quiet".into(), "--no-tags".into(), "--no-recurse-submodules".into(),
            identity.repository.to_string_lossy().into_owned(), format!("+{}:{local}", identity.target)];
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, &log,
            Duration::from_secs(60), cancel.clone()).await?;
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
            let (mut status, run) = self.ci_poll(binding, &candidate, poll, &cancel).await?;
            if let Some(retry) = &binding.effect.ci_retry {
                if !run.as_ref().is_some_and(|r| r.id == retry.run.id && r.attempt > retry.run.attempt) {
                    status = CiStatus::Unknown;
                }
            }
            let label = match status {
                CiStatus::Passed => "passed",
                CiStatus::Failed => "rejected",
                CiStatus::Superseded => "cancelled",
                _ if self.clock_ms() >= deadline => "timeout",
                _ => {
                    if pause(every, &cancel).await { return Ok(Outcome::Interrupted); }
                    continue;
                }
            };
            let mut detail = serde_json::json!({ "polls": poll + 1, "run": run });
            if label == "rejected" {
                let path = self.root.join("receipts").join(format!("{}.poll-{poll}.provider.json", digest(&binding.effect)));
                let receipt: CodingCiReceipt = serde_json::from_slice(&self.bytes(&path)?)?;
                if let Some(mut failure) = receipt.failure {
                    failure.job = failure.job.chars().filter(|c| !c.is_control()).take(256).collect();
                    failure.first_line = failure.first_line.chars().filter(|c| !c.is_control()).take(1000).collect();
                    failure.split = false;
                    if !failure.job.is_empty() { detail["failure"] = serde_json::to_value(failure)?; }
                }
            }
            self.write_fact(binding, label, &candidate, detail)?;
            return self.reconcile_verb(binding, &StageKind::AwaitCi).await;
        }
        unreachable!("the poll count is unbounded")
    }
    /// One look at CI on `sha`, over every workflow the step waits on. The configured workflow must have a run; each
    /// other one a CI wait names (ov-1071) counts once the push started it. GitHub starts every workflow a push
    /// triggers together, so one with no run beside the configured one's is one this push didn't trigger (a path filter).
    /// Anything red is red; anything still going waits; all green is green. The run facts are the configured
    /// workflow's: that is the run a CI retry reruns.
    async fn ci_poll(&self, binding: &Binding, sha: &str, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<(CiStatus, Option<CiRun>)> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        let also = match &binding.effect.kind {
            EffectKind::Execute(StageKind::AwaitCi) => match binding.effect.operation.arguments.get(4) {
                Some(text) => super::coding::also_workflows(text).ok_or(Error::Refused("unsound CI workflows"))?,
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        let Some((first, run)) = self.ci_look(binding, sha, poll, &config.ci.workflow, 0, cancel).await? else { return Ok((CiStatus::Unknown, None)) };
        let mut all = vec![first];
        for (index, name) in also.iter().enumerate() {
            if let Some((status, _)) = self.ci_look(binding, sha, poll, name, index + 1, cancel).await? { all.push(status); }
        }
        Ok((combined(&all), run))
    }
    /// One look at one workflow's newest run on `sha`: the configured provider hook runs once, told which workflow in
    /// `FARCOOLER_CI_WORKFLOW`, and writes a receipt. A hook that fails couldn't say (`Unknown`); one that succeeds and
    /// writes nothing found no run (`None`). A receipt about something else is an error. Only the configured workflow
    /// (`index` 0) follows a CI retry's exact run; the others look for their newest run on `sha`.
    async fn ci_look(&self, binding: &Binding, sha: &str, poll: u32, workflow: &str, index: usize,
        cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<(CiStatus, Option<CiRun>)>> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        // The configured workflow keeps the names it always had.
        let stem = match index { 0 => format!("{}.poll-{poll}", digest(&binding.effect)), n => format!("{}.poll-{poll}.ci-{n}", digest(&binding.effect)) };
        let (receipt, log) = (self.root.join("receipts").join(format!("{stem}.provider.json")),
            self.root.join("receipts").join(format!("{stem}.output")));
        let mut command = config.ci.command.clone();
        self.pin_context(&mut command, binding, sha, &receipt);
        command.environment.insert("FARCOOLER_CI_WORKFLOW".into(), workflow.into());
        if index != 0 { command.environment.remove("FARCOOLER_CI_RUN"); }
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &command.argv, &command.environment, &log,
            Duration::from_millis(command.timeout_ms), cancel.clone()).await?;
        if exit != Some(0) { return Ok(Some((CiStatus::Unknown, None))); }
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
            .map(|attempt| CiRun { id: run.id, attempt, failed_jobs: provided.failed_jobs.unwrap_or_default() });
        if facts.as_ref().is_some_and(|r| !r.valid()) { return Err(Error::Refused("invalid CI run facts")); }
        Ok(Some((crate::ci_watch::summarize(&format!("sha:{sha}"), &[(run, None)]).status, facts)))
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
        self.git_recorded(binding, &repo, &["worktree", "add", "-b", &branch, &tree_path, &base]).await?;
        spotlight::exclude_builds(&tree).await;
        for member in members {
            if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
            if let Some(conflict) = self.pick_member(binding, &tree, &base, member).await? {
                // Diagnose the whole batch on main, retaining only successful members. This
                // catches both conflicts with main and conflicts between tickets in one pass.
                let mut conflicts = Vec::new();
                let mut files = BTreeSet::new();
                self.git_recorded(binding, &tree, &["reset", "--hard", &base]).await?;
                for suspect in members {
                    if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
                    let checkpoint = self.git_recorded(binding, &tree, &["rev-parse", "HEAD"]).await?;
                    if let Some(detail) = self.pick_member(binding, &tree, &base, suspect).await? {
                        if let Some(paths) = detail.get("files").and_then(|v| v.as_array()) {
                            files.extend(paths.iter().filter_map(|v| v.as_str().map(String::from)));
                        }
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

    /// Apply one pinned member; an empty pick is already on the base. Abort a conflict before
    /// returning so diagnosis and the next member always start from a settled tree.
    async fn pick_member(&self, binding: &Binding, tree: &Path, base: &str,
        member: &farcooler_workflow::Member) -> Result<Option<serde_json::Value>>
    {
        let range = format!("{base}..{}", member.candidate);
        let listed = self.git_recorded(binding, tree, &["rev-list", "--reverse", &range]).await?;
        for commit in listed.lines().filter(|c| !c.is_empty()) {
            let pick = |extra: &'static [&'static str]| -> Vec<&str> {
                COMMIT_IDENTITY.iter().copied().chain(extra.iter().copied()).collect()
            };
            let mut args = pick(&["cherry-pick", "-x"]); args.push(commit);
            if self.try_git(binding, tree, &args).await?.0 { continue; }
            let unmerged = self.git_recorded(binding, tree, &["diff", "--name-only", "--diff-filter=U"]).await?;
            let clean = self.try_git(binding, tree, &["diff", "--cached", "--quiet"]).await?.0
                && self.try_git(binding, tree, &["diff", "--quiet"]).await?.0;
            if unmerged.trim().is_empty() && clean && self.try_git(binding, tree, &pick(&["cherry-pick", "--skip"])).await?.0 {
                continue;
            }
            // Only confirmed unmerged paths diagnose a member; infrastructure failures retry the fold.
            if unmerged.trim().is_empty() { return Err(Error::Refused("fold cherry-pick failed without conflicts")); }
            self.git_recorded(binding, tree, &pick(&["cherry-pick", "--abort"])).await?;
            return Ok(Some(serde_json::json!({ "member": member.item, "commit": commit,
                "files": unmerged.lines().collect::<Vec<_>>() })));
        }
        Ok(None)
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
        if let Some(predecessor) = predecessor {
            match self.wait_landing(binding, &predecessor, &base, &cancel).await {
                Ok(true) => {}
                Ok(false) => {
                    self.write_fact(binding, "base_moved", &candidate,
                        serde_json::json!({ "base": base, "sent": false, "why": "Previous landing CI failed" }))?;
                    return self.reconcile_verb(binding, &StageKind::Push).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "previous landing has not passed; nothing was sent");
                    return Ok(if *cancel.borrow() { Outcome::Interrupted } else { Outcome::SafeFailure });
                }
            }
        }
        if *cancel.borrow() { return Ok(Outcome::Interrupted); }
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
            let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, &log, Duration::from_secs(60), cancel).await?;
            (argv, environment, exit)
        };
        let output = self.bytes(&log)?;
        durable(&self.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect))),
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
                self.write_fact(binding, "passed", &candidate, serde_json::json!({ "base": base, "log": digest(&output) }))?;
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
    let read = crate::git::git(repository, &["rev-parse", "--verify", "--quiet", target]).await
        .map_err(|_| Error::Refused("the landing remote cannot be read"))?;
    if !read.ok { return Err(Error::Refused("the landing target cannot be read")); }
    let known = crate::git::git(repository, &["cat-file", "-e", &format!("{candidate}^{{commit}}")]).await
        .map_err(|_| Error::Refused("the landing remote cannot be read"))?;
    if !known.ok { return Ok(false); }
    Ok(crate::git::git(repository, &["merge-base", "--is-ancestor", candidate, target]).await
        .map_err(|_| Error::Refused("the landing remote cannot be read"))?.ok)
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
