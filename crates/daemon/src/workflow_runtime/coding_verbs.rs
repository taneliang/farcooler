//! The landing verbs of a group: wait for the base, fold, push, and wait for CI (ov-956).
//! `run` is the ordinary gate command and lives with the other processes in coding_execute.
//!
//! Each verb ends by writing one fact (its label, its candidate and what it saw) and then settles
//! from that fact, so the first run and a recovery after a crash take the same road. What a verb may
//! do with no fact after a crash depends on its class: a read is read again and a fold is made again
//! (`SafeFailure` runs the step anew), but a push is never repeated and stays uncertain.
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
    matches!(kind, StageKind::AwaitBase | StageKind::Fold | StageKind::Push | StageKind::AwaitCi | StageKind::Route)
        || super::coding_pr::is_pr_verb(kind)
}
const COMMIT_IDENTITY: [&str; 8] = ["-c", "user.name=Far Cooler", "-c", "user.email=landing@farcooler.invalid",
    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];

impl Exec {
    pub(super) fn verb_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.verb.json", digest(&binding.effect)))
    }
    /// Which base a folded commit was made on: a push leases the target against exactly that.
    fn fold_index(&self, binding: &Binding, folded: &str) -> PathBuf {
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
    async fn try_git(&self, binding: &Binding, dir: &Path, args: &[&str]) -> Result<(bool, String, String)> {
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
            StageKind::Fold => self.fold(binding, cancel).await,
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
        let rules = binding.effect.operation.arguments.get(1).and_then(|a| crate::path_routes::PathRules::read(a))
            .ok_or(Error::Refused("unsound path rules"))?;
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
    /// made again (the step runs anew); a push is never repeated, so its outcome stays unknown.
    pub(super) async fn reconcile_verb(&self, binding: &Binding, kind: &StageKind) -> Result<Outcome> {
        let raw = match self.bytes(&self.verb_path(binding)) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return match kind {
                    StageKind::Push => Err(Error::Refused("push has no settlement; whether it landed is unknown")),
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
            if fact.candidate != binding.effect.candidate.clone().unwrap_or_default()
                || !on_remote(&identity.repository, &identity.target, &fact.candidate).await?
            { return Err(Error::Refused("remote landing SHA changed")); }
        }
        let manifest = digest(&(binding, &fact.candidate, &raw, &fact.label));
        let evidence = Evidence { candidate: fact.candidate, manifest: manifest.clone(),
            receipt: digest(&("coding-receipt/v1", manifest)), policy_pin: binding.effect.policy_pin.clone() };
        Ok(match fact.label.as_str() {
            "passed" => Outcome::Passed(evidence),
            "rejected" => Outcome::Rejected(evidence),
            other => Outcome::Labeled(other.into(), evidence),
        })
    }

    /// Wait for the landing target's head, and for CI to pass on it if the step says `green`. A red
    /// base keeps waiting. The head becomes the group's candidate: the base everything else builds on.
    async fn await_base(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let (every, patience) = wait_times(&binding.effect.operation).ok_or(Error::Refused("unsound wait times"))?;
        let green = binding.effect.operation.arguments.get(1).map(String::as_str) == Some("green");
        let deadline = binding.effect.created_ms.saturating_add(i64::try_from(patience).unwrap_or(i64::MAX));
        let mut last = binding.base.clone();
        for poll in 0u32.. {
            if *cancel.borrow() { return Ok(Outcome::Interrupted); }
            if let Some(head) = self.fetch_base(binding, poll, &cancel).await? {
                last = head.clone();
                if !green || self.ci_poll(binding, &head, poll, &cancel).await? == CiStatus::Passed {
                    self.write_fact(binding, "passed", &head, serde_json::json!({ "polls": poll + 1 }))?;
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
    /// Bring the target's head into the local repository, under a private ref, and read it there.
    /// Another lander's commits are not here until this runs. A fetch that fails is a poll with no answer.
    async fn fetch_base(&self, binding: &Binding, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<String>> {
        let identity = self.landing()?;
        self.refuse_redirected_transport(binding).await?;
        let local = format!("refs/farcooler/base/{}", &digest(&(&identity.repository, &identity.target))[4..20]);
        let (program, environment) = self.git_environment().await?;
        let argv = vec![program, "fetch".into(), "--quiet".into(), "--no-tags".into(), "--no-recurse-submodules".into(),
            identity.repository.to_string_lossy().into_owned(), format!("+{}:{local}", identity.target)];
        let log = self.root.join("receipts").join(format!("{}.poll-{poll}.fetch.output", digest(&binding.effect)));
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
            let label = match self.ci_poll(binding, &candidate, poll, &cancel).await? {
                CiStatus::Passed => "passed",
                CiStatus::Failed => "rejected",
                CiStatus::Superseded => "cancelled",
                _ if self.clock_ms() >= deadline => "timeout",
                _ => {
                    if pause(every, &cancel).await { return Ok(Outcome::Interrupted); }
                    continue;
                }
            };
            self.write_fact(binding, label, &candidate, serde_json::json!({ "polls": poll + 1 }))?;
            return self.reconcile_verb(binding, &StageKind::AwaitCi).await;
        }
        unreachable!("the poll count is unbounded")
    }
    /// One look at CI on `sha`: the configured provider hook runs once and writes a receipt. A hook
    /// that fails, or writes nothing, has no answer yet. A receipt about something else is an error.
    async fn ci_poll(&self, binding: &Binding, sha: &str, poll: u32, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<CiStatus> {
        let config = self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?;
        let stem = format!("{}.poll-{poll}", digest(&binding.effect));
        let (receipt, log) = (self.root.join("receipts").join(format!("{stem}.provider.json")),
            self.root.join("receipts").join(format!("{stem}.output")));
        let mut command = config.ci.command.clone();
        self.pin_context(&mut command, binding, sha, &receipt);
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &command.argv, &command.environment, &log,
            Duration::from_millis(command.timeout_ms), cancel.clone()).await?;
        if exit != Some(0) { return Ok(CiStatus::Unknown); }
        let raw = match self.bytes(&receipt) {
            Ok(raw) => raw,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(CiStatus::Unknown),
            Err(error) => return Err(error),
        };
        let provided: CodingCiReceipt = serde_json::from_slice(&raw)?;
        if provided.binding != digest(binding) || provided.repository != config.ci.repository {
            return Err(Error::Refused("foreign CI provider identity"));
        }
        let run = crate::ci_watch::parse_run(&serde_json::to_vec(&provided.run)?).ok_or(Error::Refused("invalid CI receipt"))?;
        if run.id == 0 || run.head_sha != sha || run.name != config.ci.workflow || run.event != config.ci.event {
            return Err(Error::Refused("foreign CI run provenance"));
        }
        Ok(crate::ci_watch::summarize(&format!("sha:{sha}"), &[(run, None)]).status)
    }

    /// Apply every member's commits, in order, onto the base in a fresh tree. The tree is made new
    /// each time, so a group that starts over after a split or a moved base keeps nothing of its last try.
    /// A commit that does not apply is the label `conflict`.
    async fn fold(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let base = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("fold needs the base to build on"))?;
        let members = &binding.effect.members;
        if members.is_empty() || members.iter().any(|m| !sha(&m.candidate)) { return Err(Error::Refused("fold needs pinned members")); }
        let (repo, tree) = (self.repository(), self.tree(binding));
        let tree_path = tree.to_string_lossy().into_owned();
        let branch = format!("wf-{}", &digest(&(&binding.workspace, &binding.effect.item))[4..]);
        if tree.try_exists()? {
            self.try_git(binding, &repo, &["worktree", "remove", "--force", &tree_path]).await?;
            if tree.try_exists()? { std::fs::remove_dir_all(&tree)?; }
        }
        self.try_git(binding, &repo, &["worktree", "prune"]).await?;
        if self.try_git(binding, &repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await?.0 {
            self.git_recorded(binding, &repo, &["branch", "-D", &branch]).await?;
        }
        self.git_recorded(binding, &repo, &["worktree", "add", "-b", &branch, &tree_path, &base]).await?;
        for member in members {
            if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
            let range = format!("{base}..{}", member.candidate);
            let listed = self.git_recorded(binding, &tree, &["rev-list", "--reverse", &range]).await?;
            for commit in listed.lines().filter(|c| !c.is_empty()) {
                let pick = |extra: &'static [&'static str]| -> Vec<&str> {
                    COMMIT_IDENTITY.iter().copied().chain(extra.iter().copied()).collect()
                };
                let mut args = pick(&["cherry-pick", "-x"]); args.push(commit);
                if self.try_git(binding, &tree, &args).await?.0 { continue; }
                let unmerged = self.try_git(binding, &tree, &["diff", "--name-only", "--diff-filter=U"]).await?.1;
                let clean = self.try_git(binding, &tree, &["diff", "--cached", "--quiet"]).await?.0
                    && self.try_git(binding, &tree, &["diff", "--quiet"]).await?.0;
                // Already on the base: an empty pick.
                if unmerged.trim().is_empty() && clean && self.try_git(binding, &tree, &pick(&["cherry-pick", "--skip"])).await?.0 {
                    continue;
                }
                self.try_git(binding, &tree, &["cherry-pick", "--abort"]).await?;
                self.write_fact(binding, "conflict", &base, serde_json::json!({
                    "member": member.item, "commit": commit, "files": unmerged.lines().collect::<Vec<_>>() }))?;
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

    /// Push the folded commit to the target, once, and only while the target is still at the base the
    /// gate ran on. A target that moved, or a push the remote refused, is `base_moved`: nothing landed,
    /// so starting over from the new base is safe. A timeout or a lost connection is not: that is uncertain.
    async fn push(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let identity = self.landing()?.clone();
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
            self.git_recorded(binding, &self.tree(binding), &["merge-base", "--is-ancestor", &base, &candidate]).await?;
            if adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
                return Err(Error::Refused("landing transport is not bare"));
            }
            self.refuse_redirected_transport(binding).await?;
            let before = adapters::git(&identity.repository, &["rev-parse", &identity.target]).await?;
            Ok((candidate, base, before))
        }.await;
        let Ok((candidate, base, before)) = checked else {
            tracing::warn!(effect = %binding.effect.id, "push refused before anything was sent; it will be tried again");
            return Ok(Outcome::SafeFailure);
        };
        if before != base {
            // Another lander got there after the gate; nothing was sent.
            self.write_fact(binding, "base_moved", &candidate, serde_json::json!({ "base": base, "target": before, "sent": false }))?;
            return self.reconcile_verb(binding, &StageKind::Push).await;
        }
        let (program, environment) = self.git_environment().await?;
        let argv = vec![program, "push".into(), "--porcelain".into(),
            "--no-follow-tags".into(), "--recurse-submodules=no".into(), "--signed=false".into(),
            format!("--force-with-lease={}:{base}", identity.target),
            identity.repository.to_string_lossy().into_owned(), crate::landing_job::push_target(&candidate, &identity.target)];
        let log = self.remote_log(binding);
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, &log, Duration::from_secs(60), cancel).await?;
        let output = self.bytes(&log)?;
        durable(&self.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect))),
            &serde_json::to_vec(&serde_json::json!({ "binding": binding, "identity": identity, "argv": argv,
                "environment": environment, "exit": exit, "log_digest": digest(&output) }))?)?;
        let after = adapters::git(&identity.repository, &["rev-parse", &identity.target]).await.ok();
        match classify_push(exit, &String::from_utf8_lossy(&output), after.as_deref(), &candidate) {
            PushVerdict::Landed => {
                self.write_fact(binding, "passed", &candidate, serde_json::json!({ "base": base, "log": digest(&output) }))?;
                // The local copy of the target follows, when it was where this push started. Best effort: the remote is the record.
                let local = adapters::git(&self.repository(), &["rev-parse", self.landing_ref()]).await.ok();
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
