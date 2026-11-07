//! The pull request verbs of an item: push its branch, open its one pull request, take it out of draft,
//! and wait for a person to merge it (ov-957).
//!
//! The engine reaches GitHub only through the configured `gh`, run with the environment the
//! configuration pinned and nothing inherited. Every verb here is an Ensure verb or a Read: it reads
//! the fact first and acts only if the fact is absent, so a crash at any point is repaired by running
//! the step again. Pull requests are found by their head branch, whose name the engine derives from the
//! item, so there is never a second one: more than one is a refusal, not a choice.
use super::*;
use super::coding::{sha, wait_times};
use super::coding_verbs::pause;

/// What `gh pr list` says about one pull request.
#[derive(Clone, Debug, Deserialize)]
struct Pr {
    number: u64,
    state: String,
    #[serde(rename = "isDraft", default)]
    draft: bool,
    #[serde(rename = "headRefName")]
    head: String,
    #[serde(rename = "headRefOid", default)]
    head_sha: String,
    #[serde(rename = "mergeCommit", default)]
    merge: Option<Oid>,
    #[serde(rename = "latestReviews", default)]
    reviews: Vec<Review>,
}
#[derive(Clone, Debug, Deserialize)]
struct Oid { oid: String }
#[derive(Clone, Debug, Deserialize)]
struct Review { state: String, #[serde(default)] commit: Option<Oid> }

const FIELDS: &str = "number,state,isDraft,headRefName,headRefOid,mergeCommit,latestReviews";

pub(super) fn is_pr_verb(kind: &StageKind) -> bool {
    matches!(kind, StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr)
}

impl Exec {
    fn pull_request(&self) -> Result<&CodingPullRequest> {
        self.coding.as_ref().and_then(|c| c.pull_request.as_ref()).ok_or(Error::Refused("pull request verbs need a pull request identity"))
    }
    /// The head branch of an item's pull request. The same item always has the same one.
    fn head_branch(&self, binding: &Binding) -> String {
        format!("farcooler/{}", &digest(&(&binding.workspace, &binding.effect.item))[4..20])
    }
    fn candidate_of(&self, binding: &Binding) -> Result<String> {
        binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing pull request candidate"))
    }

    pub(super) async fn execute_pr_verb(&self, binding: &Binding, kind: &StageKind,
        cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome>
    {
        self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.validate()?;
        match kind {
            StageKind::PushBranch => self.push_branch(binding, cancel).await,
            StageKind::OpenPr => self.open_pr(binding, cancel).await,
            StageKind::PrReady => self.pr_ready(binding, cancel).await,
            StageKind::AwaitPr => self.await_pr(binding, cancel).await,
            _ => Err(Error::Refused("not a pull request verb")),
        }
    }

    /// Run `gh` with the pinned environment and read what it printed. `tag` names the call's log.
    async fn gh(&self, binding: &Binding, tag: &str, args: &[&str], cancel: &tokio::sync::watch::Receiver<bool>)
        -> Result<(Option<i32>, Vec<u8>)>
    {
        let pr = self.pull_request()?;
        let mut argv = pr.gh.argv.clone();
        argv.extend(args.iter().map(|a| a.to_string()));
        let log = self.root.join("receipts").join(format!("{}.gh-{tag}.output", digest(&binding.effect)));
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &pr.gh.environment, &log,
            Duration::from_millis(pr.gh.timeout_ms), cancel.clone()).await?;
        Ok((exit, self.bytes(&log)?))
    }
    /// Every pull request, open or not, whose head is this item's branch. `None` when GitHub gave no answer.
    async fn list(&self, binding: &Binding, tag: &str, cancel: &tokio::sync::watch::Receiver<bool>) -> Result<Option<Vec<Pr>>> {
        let (repo, head) = (self.pull_request()?.repository.clone(), self.head_branch(binding));
        let (exit, out) = self.gh(binding, tag, &["pr", "list", "--repo", &repo, "--head", &head, "--state", "all",
            "--limit", "100", "--json", FIELDS], cancel).await?;
        if exit != Some(0) { return Ok(None); }
        let Ok(all) = serde_json::from_slice::<Vec<Pr>>(&out) else { return Ok(None) };
        let ours: Vec<Pr> = all.into_iter().filter(|p| p.head == head).collect();
        // The head branch is this item's alone, so a second pull request is something to refuse, never to pick between.
        if ours.len() > 1 { return Err(Error::Refused("more than one pull request has this head branch")); }
        Ok(Some(ours))
    }
    fn finish(&self, binding: &Binding, label: &str, candidate: &str, detail: serde_json::Value) -> Result<()> {
        self.write_fact(binding, label, candidate, detail)
    }
    async fn settle(&self, binding: &Binding, kind: &StageKind) -> Result<Outcome> { self.reconcile_verb(binding, kind).await }

    /// Put the candidate on the remote as the head branch. A branch that already has it is left alone.
    async fn push_branch(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let identity = self.landing()?.clone();
        let candidate = self.candidate_of(binding)?;
        if self.verify_tree(binding).await? != candidate { return Err(Error::Refused("pushed candidate is not the item's tree")); }
        if adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
            return Err(Error::Refused("branch transport is not bare"));
        }
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if config.lines().any(|l| l.starts_with("url.") || l.starts_with("include.") || l.starts_with("includeif.")) {
            return Err(Error::Refused("local config redirects transport"));
        }
        let reference = format!("refs/heads/{}", self.head_branch(binding));
        let before = adapters::git(&identity.repository, &["rev-parse", "--verify", "--quiet", &reference]).await.ok();
        if before.as_deref() != Some(candidate.as_str()) {
            let (program, environment) = self.git_environment().await?;
            let argv = vec![program, "push".into(), "--porcelain".into(), "--no-follow-tags".into(),
                "--recurse-submodules=no".into(), "--signed=false".into(),
                // Only over what was seen: an empty expectation means the branch is not there yet.
                format!("--force-with-lease={reference}:{}", before.as_deref().unwrap_or("")),
                identity.repository.to_string_lossy().into_owned(), format!("{candidate}:{reference}")];
            let log = self.root.join("receipts").join(format!("{}.branch-push.output", digest(&binding.effect)));
            durable(&log, b"")?;
            crate::effect_runner::configured(&self.repository(), &argv, &environment, &log, Duration::from_secs(60), cancel).await?;
        }
        let after = adapters::git(&identity.repository, &["rev-parse", "--verify", "--quiet", &reference]).await.ok();
        // Pushing a branch is safe to do again, so a push that did not land is a failure to try again.
        if after.as_deref() != Some(candidate.as_str()) { return Ok(Outcome::SafeFailure); }
        self.finish(binding, "passed", &candidate, serde_json::json!({ "branch": reference, "pushed": before.as_deref() != Some(candidate.as_str()) }))?;
        self.settle(binding, &StageKind::PushBranch).await
    }

    /// Open the pull request for the head branch, unless it is already open. The pull request is
    /// looked up by its head branch both before and after creating it, so it exists once.
    async fn open_pr(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let candidate = self.candidate_of(binding)?;
        let (repo, base, head) = {
            let pr = self.pull_request()?;
            (pr.repository.clone(), pr.base.clone(), self.head_branch(binding))
        };
        let draft = binding.effect.operation.arguments.get(1).map(String::as_str) == Some("draft");
        let Some(found) = self.list(binding, "list-0", &cancel).await? else { return Ok(Outcome::SafeFailure) };
        let existing = match found.into_iter().next() {
            Some(existing) => existing,
            None => {
                let identity = self.landing()?;
                let remote = adapters::git(&identity.repository, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{head}")]).await.ok();
                if remote.as_deref() != Some(candidate.as_str()) { return Err(Error::Refused("the head branch does not hold the candidate")); }
                let title = self.git_recorded(binding, &self.tree(binding), &["log", "-1", "--format=%s", &candidate]).await?;
                let body = self.git_recorded(binding, &self.tree(binding), &["log", "-1", "--format=%b", &candidate]).await?;
                let title: String = title.chars().take(200).collect();
                let body: String = body.chars().take(8000).collect();
                let mut args = vec!["pr", "create", "--repo", &repo, "--head", &head, "--base", &base, "--title", &title, "--body", &body];
                if draft { args.push("--draft"); }
                // A create that failed may still have made the pull request, so the answer is the lookup that follows.
                self.gh(binding, "create", &args, &cancel).await?;
                let Some(after) = self.list(binding, "list-1", &cancel).await? else { return Ok(Outcome::SafeFailure) };
                match after.into_iter().next() {
                    Some(created) => created,
                    None => return Ok(Outcome::SafeFailure),
                }
            }
        };
        let label = if existing.state == "OPEN" { "passed" } else { "closed" };
        self.finish(binding, label, &candidate, serde_json::json!({ "number": existing.number, "head": head, "draft": existing.draft }))?;
        self.settle(binding, &StageKind::OpenPr).await
    }

    /// Take the pull request out of draft. One that is already ready is left alone.
    async fn pr_ready(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let candidate = self.candidate_of(binding)?;
        let repo = self.pull_request()?.repository.clone();
        let Some(found) = self.list(binding, "list-0", &cancel).await? else { return Ok(Outcome::SafeFailure) };
        let Some(existing) = found.into_iter().next() else { return Err(Error::Refused("there is no pull request to make ready")) };
        if existing.state != "OPEN" {
            self.finish(binding, "closed", &candidate, serde_json::json!({ "number": existing.number }))?;
            return self.settle(binding, &StageKind::PrReady).await;
        }
        if existing.draft {
            self.gh(binding, "ready", &["pr", "ready", &existing.number.to_string(), "--repo", &repo], &cancel).await?;
            let Some(after) = self.list(binding, "list-1", &cancel).await? else { return Ok(Outcome::SafeFailure) };
            if after.first().is_none_or(|p| p.draft) { return Ok(Outcome::SafeFailure); }
        }
        self.finish(binding, "passed", &candidate, serde_json::json!({ "number": existing.number }))?;
        self.settle(binding, &StageKind::PrReady).await
    }

    /// Wait for the pull request to be merged, and take the commit GitHub made as the new candidate. A
    /// squash never makes the branch's commit an ancestor of the target, so the branch's SHA is not the answer.
    /// A change requested of the commit that is still the head, or a pull request closed unmerged, are
    /// answers too. Nothing is merged here: a person merges.
    async fn await_pr(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        let (every, patience) = wait_times(&binding.effect.operation).ok_or(Error::Refused("unsound wait times"))?;
        let candidate = self.candidate_of(binding)?;
        let deadline = binding.effect.created_ms.saturating_add(i64::try_from(patience).unwrap_or(i64::MAX));
        for poll in 0u32.. {
            if *cancel.borrow() { return Ok(Outcome::SafeFailure); }
            if let Some(found) = self.list(binding, &format!("poll-{poll}"), &cancel).await? {
                if let Some(pr) = found.first() {
                    let polls = serde_json::json!({ "polls": poll + 1, "number": pr.number });
                    match pr.state.as_str() {
                        "MERGED" => {
                            // What was merged must be what was pushed. Anyone may push to a branch.
                            if pr.head_sha != candidate { return Err(Error::Refused("the merged pull request is not the pushed candidate")); }
                            let merged = pr.merge.as_ref().map(|m| m.oid.clone()).filter(|m| sha(m))
                                .ok_or(Error::Refused("a merged pull request names no merge commit"))?;
                            self.finish(binding, "passed", &merged, polls)?;
                            return self.settle(binding, &StageKind::AwaitPr).await;
                        }
                        "CLOSED" => {
                            self.finish(binding, "closed", &candidate, polls)?;
                            return self.settle(binding, &StageKind::AwaitPr).await;
                        }
                        // A review of an older commit does not count: a rework pushes a new head, and the
                        // decision GitHub shows stays until the reviewer looks again.
                        _ if pr.head_sha == candidate && pr.reviews.iter().any(|r| r.state == "CHANGES_REQUESTED"
                            && r.commit.as_ref().is_some_and(|c| c.oid == candidate)) => {
                            self.finish(binding, "changes_requested", &candidate, polls)?;
                            return self.settle(binding, &StageKind::AwaitPr).await;
                        }
                        _ => {}
                    }
                }
            }
            if self.clock_ms() >= deadline {
                self.finish(binding, "timeout", &candidate, serde_json::json!({ "polls": poll + 1 }))?;
                return self.settle(binding, &StageKind::AwaitPr).await;
            }
            if pause(every, &cancel).await { return Ok(Outcome::SafeFailure); }
        }
        unreachable!("the poll count is unbounded")
    }
}
