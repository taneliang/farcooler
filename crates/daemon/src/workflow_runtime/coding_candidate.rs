//! A landing group's candidate on its own branch of the landing remote (ov-1269).
//!
//! The group pushes its folded commit to `landing/<id>` as soon as the fold succeeds, so CI runs on it while the
//! local gate does. The CI wait after the gate judges that commit, and the push then moves the target to the very
//! same commit: CI's verdict on the branch is the target's, and ci.yml skips the second run on main.
//!
//! The branch is the engine's alone: its name is a digest of the workspace and the group item, and nothing else
//! writes it. Pushing it again is safe, so the step is an Ensure verb: a failure of any kind runs it again. Each
//! branch is written down before its first push and deleted once its group has ended, whatever it ended as.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use farcooler_workflow::{digest, Outcome, Phase, StageKind};
use serde::{Deserialize, Serialize};
use super::{durable, Binding, CodingConfigExt, CodingLanding, Error, Exec, Result};
use super::coding::sha;

/// One branch this runner put on the landing remote, kept until the branch is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateBranch { pub workspace: String, pub item: String, pub branch: String }

/// What the landing remote said about a branch. `Unknown` is never read as absent.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum OnRemote { Unknown, Absent, At(String) }

/// How long a deletion that failed waits before the sweep tries it again.
const DELETE_BACKOFF: Duration = Duration::from_secs(600);

/// Deletions in flight or waiting out a failure, by ledger path: the sweep runs on every dispatch pass.
fn deleting() -> &'static std::sync::Mutex<std::collections::BTreeMap<PathBuf, Option<std::time::Instant>>> {
    static DELETING: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<PathBuf, Option<std::time::Instant>>>> =
        std::sync::OnceLock::new();
    DELETING.get_or_init(Default::default)
}

impl Exec {
    /// The branch a group's candidate goes to. The same group item always has the same one.
    pub(super) fn candidate_branch(workspace: &str, item: &str) -> String {
        format!("landing/{}", &digest(&("landing-candidate/v1", workspace, item))[4..20])
    }
    fn candidate_ledgers(&self) -> PathBuf { self.root.join("candidate-branches") }
    fn candidate_ledger(&self, workspace: &str, item: &str) -> PathBuf {
        self.candidate_ledgers().join(format!("{}.json", &digest(&("landing-candidate/v1", workspace, item))[4..20]))
    }

    /// Push the folded candidate to the group's branch, and read it back from the remote.
    pub(super) async fn push_candidate(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        if self.verb_path(binding).try_exists()? { return self.reconcile_verb(binding, &StageKind::PushCandidate).await; }
        self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.validate()?;
        let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing candidate"))?;
        // What CI judges is exactly the tree the fold made and the gate will run on.
        if self.verify_tree(binding).await? != candidate { return Err(Error::Refused("the branch candidate is not the folded tree")); }
        let identity = self.landing()?.clone();
        let url = super::coding::landing_url(&identity).map(str::to_string);
        if url.is_none() && super::adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
            return Err(Error::Refused("landing transport is not bare"));
        }
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if config.lines().any(|l| l.starts_with("url.") || l.starts_with("include.") || l.starts_with("includeif.")) {
            return Err(Error::Refused("local landing config redirects transport"));
        }
        let branch = Self::candidate_branch(&binding.workspace, &binding.effect.item);
        let reference = format!("refs/heads/{branch}");
        // Written before the push, so a crash in between still leaves the branch known, and deleted later.
        self.remember_candidate_branch(&CandidateBranch { workspace: binding.workspace.clone(), item: binding.effect.item.clone(), branch })?;
        let log = |tag: &str| self.root.join("receipts").join(format!("{}.candidate-{tag}.output", digest(&binding.effect)));
        // A forced update: the branch is this group's, and a refold after a moved base replaces its last candidate.
        let refspec = format!("+{candidate}:{reference}");
        match url.as_deref() {
            Some(url) => {
                self.stage_candidate(binding, &candidate).await?;
                self.remote_git(url, &["push".into(), "--porcelain".into(), "--no-follow-tags".into(), "--recurse-submodules=no".into(),
                    "--signed=false".into(), url.into(), refspec], &log("push"), Duration::from_secs(120), cancel.clone()).await?;
            }
            None => {
                let (program, environment) = self.git_environment().await?;
                let argv = vec![program, "push".into(), "--porcelain".into(), "--no-follow-tags".into(), "--recurse-submodules=no".into(),
                    "--signed=false".into(), identity.repository.to_string_lossy().into_owned(), refspec];
                let _ = std::fs::remove_file(log("push"));
                durable(&log("push"), b"")?;
                crate::effect_runner::configured(&self.repository(), &argv, &environment, &log("push"), Duration::from_secs(60), cancel.clone()).await?;
            }
        }
        if *cancel.borrow() { return Ok(Outcome::Interrupted); }
        // The remote is the record: only a branch that holds the candidate passes.
        if self.branch_on_remote(&identity, &reference, &log("after")).await? != OnRemote::At(candidate.clone()) {
            tracing::warn!(effect = %binding.effect.id, "the candidate branch doesn't hold the candidate yet; it will be pushed again");
            return Ok(Outcome::SafeFailure);
        }
        self.write_fact(binding, "passed", &candidate, serde_json::json!({ "branch": reference }))?;
        self.reconcile_verb(binding, &StageKind::PushCandidate).await
    }

    /// What the landing remote holds at `reference`, asked of the remote itself.
    pub(super) async fn branch_on_remote(&self, identity: &CodingLanding, reference: &str, log: &Path) -> Result<OnRemote> {
        let Some(url) = super::coding::landing_url(identity) else {
            let read = crate::git::git(&identity.repository, &["for-each-ref", "--format=%(objectname)", reference]).await
                .map_err(|_| Error::Refused("the landing repository cannot be read"))?;
            if !read.ok { return Ok(OnRemote::Unknown); }
            let head = read.stdout.trim();
            return Ok(if head.is_empty() { OnRemote::Absent } else if sha(head) { OnRemote::At(head.into()) } else { OnRemote::Unknown });
        };
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        let (_, _, exit) = self.remote_git(url, &["ls-remote".into(), "--refs".into(), url.into(), reference.into()],
            log, Duration::from_secs(60), cancel).await?;
        if exit != Some(0) { return Ok(OnRemote::Unknown); }
        // Notices on stderr share the log: only a line that is exactly `<sha> TAB <ref>` is an answer.
        Ok(String::from_utf8_lossy(&self.bytes(log)?).lines().find_map(|l| l.split_once('\t')
            .filter(|(head, name)| *name == reference && sha(head)).map(|(head, _)| OnRemote::At(head.to_string())))
            .unwrap_or(OnRemote::Absent))
    }

    fn remember_candidate_branch(&self, branch: &CandidateBranch) -> Result<()> {
        let dir = self.candidate_ledgers();
        match std::fs::create_dir(&dir) {
            Ok(()) => std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        match durable(&self.candidate_ledger(&branch.workspace, &branch.item), &serde_json::to_vec(branch)?) {
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            other => other,
        }
    }

    /// The branches this runner pushed whose group has ended. A group that failed may be retried, and then
    /// pushes its branch again and writes it down again.
    pub(super) fn ended_candidate_branches(&self) -> Result<Vec<(PathBuf, CandidateBranch)>> {
        let mut ended = Vec::new();
        let entries = match std::fs::read_dir(self.candidate_ledgers()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(ended),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") { continue; }
            let Ok(branch) = serde_json::from_slice::<CandidateBranch>(&self.bytes(&path)?) else {
                tracing::warn!(path = %path.display(), "an unreadable candidate branch record is left alone");
                continue;
            };
            if branch.branch != Self::candidate_branch(&branch.workspace, &branch.item)
                || path != self.candidate_ledger(&branch.workspace, &branch.item) { continue; }
            if self.group_ended(&branch)? { ended.push((path, branch)); }
        }
        Ok(ended)
    }
    fn group_ended(&self, branch: &CandidateBranch) -> Result<bool> {
        let Some(state) = self.state(&branch.workspace)? else { return Ok(true) };
        Ok(state.items.get(&branch.item).is_none_or(|i| matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled)))
    }

    /// Delete one ended group's branch from the landing remote. True once the remote no longer has it.
    pub(super) async fn delete_candidate_branch(&self, branch: &CandidateBranch) -> Result<bool> {
        let identity = self.landing()?.clone();
        let reference = format!("refs/heads/{}", branch.branch);
        let stem = format!("candidate-delete-{}", &digest(&("landing-candidate/v1", &branch.workspace, &branch.item))[4..20]);
        let log = |tag: &str| self.root.join("receipts").join(format!("{stem}.{tag}.output"));
        if self.branch_on_remote(&identity, &reference, &log("before")).await? == OnRemote::Absent { return Ok(true); }
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        let refspec = format!(":{reference}");
        match super::coding::landing_url(&identity) {
            Some(url) => {
                self.remote_git(url, &["push".into(), "--porcelain".into(), url.into(), refspec], &log("push"), Duration::from_secs(60), cancel).await?;
            }
            None => {
                let (program, environment) = self.git_environment().await?;
                let argv = vec![program, "push".into(), "--porcelain".into(), identity.repository.to_string_lossy().into_owned(), refspec];
                let _ = std::fs::remove_file(log("push"));
                durable(&log("push"), b"")?;
                crate::effect_runner::configured(&self.repository(), &argv, &environment, &log("push"), Duration::from_secs(60), cancel).await?;
            }
        }
        Ok(self.branch_on_remote(&identity, &reference, &log("after")).await? == OnRemote::Absent)
    }

    /// Start deleting the branch of every group that has ended, off the dispatch loop: a slow remote never holds it
    /// up. A deletion that fails waits `DELETE_BACKOFF`, then the next pass tries again. Its record goes only once
    /// the remote no longer has the branch and the group is still over.
    pub(super) fn sweep_candidate_branches(&self) {
        if self.coding.as_ref().and_then(|c| c.landing.as_ref()).is_none() { return; }
        let ended = match self.ended_candidate_branches() {
            Ok(ended) => ended,
            Err(error) => { tracing::warn!(%error, "candidate branches could not be listed"); return; }
        };
        for (path, branch) in ended {
            {
                let mut deleting = deleting().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match deleting.get(&path) {
                    Some(None) => continue,
                    Some(Some(retry)) if *retry > std::time::Instant::now() => continue,
                    _ => { deleting.insert(path.clone(), None); }
                }
            }
            let exec = self.clone();
            tokio::spawn(async move {
                let gone = match exec.delete_candidate_branch(&branch).await {
                    Ok(gone) => gone,
                    Err(error) => { tracing::warn!(%error, branch = %branch.branch, "a candidate branch could not be deleted"); false }
                };
                if gone && exec.group_ended(&branch).unwrap_or(false) { let _ = std::fs::remove_file(&path); }
                let mut deleting = deleting().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if gone { deleting.remove(&path); } else { deleting.insert(path, Some(std::time::Instant::now() + DELETE_BACKOFF)); }
            });
        }
    }
}
