//! Keep the builder's branch and the candidate the landing group will pin on the same base.
use super::*;
use farcooler_protocol::workflow_commands::{Action, MAX_BYTES};
use std::collections::BTreeMap;

const TARGET_WAIT: Duration = Duration::from_secs(5);
const REBASE_WAIT: Duration = Duration::from_secs(20);

impl Runtime {
    pub(super) async fn wire_queue_held(&self, workspace: &str, principal: &str, action: &Action) -> Result<bool> {
        let Action::Queue { mutation, item, run, pin, .. } = action else { return Ok(false) };
        if !valid_id(&mutation.id) || mutation.id.len() > 100 { return Ok(false); }
        let payload = serde_json::to_string(&(principal, action))?;
        if payload.len() > MAX_BYTES { return Err(Error::Refused("oversized command")); }
        let id = format!("wire:{principal}:{}", mutation.id);
        if self.store.workflow_wire_receipt(workspace, &id, &payload)?.is_some() { return Ok(false); }
        Ok(!self.prepare_queue(workspace, item, mutation.revision, Some(mutation.epoch), Some((run, pin)), now()).await)
    }

    /// Evaluate the waiting tickets before automatic queueing, sharing each target read within this pass.
    pub(super) async fn refresh_waiting_joins(&self, workspaces: &[String], now_ms: i64) -> Result<BTreeSet<(String, String)>> {
        let mut heads = BTreeMap::new();
        let mut ready = BTreeSet::new();
        for workspace in workspaces {
            if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { break; }
            if self.service_mode && self.store.workflow_board_deleted(workspace)? { continue; }
            let Some(state) = self.state(workspace)? else { continue };
            for (key, item) in state.items.iter().filter(|(_, i)| i.phase == Phase::WaitingJoin) {
                match self.refresh_waiting_join(workspace, key, item, now_ms, &mut heads).await {
                    Ok(true) => { ready.insert((workspace.clone(), key.clone())); }
                    Ok(false) => {}
                    Err(error) => tracing::debug!(%workspace, item = %key, %error, "waiting branch could not be refreshed"),
                }
            }
        }
        Ok(ready)
    }

    pub(super) async fn prepare_queue(&self, workspace: &str, key: &str, revision: u64,
        epoch: Option<u64>, run: Option<(&str, &str)>, now_ms: i64) -> bool
    {
        let result = async {
            let Some(state) = self.state(workspace)? else { return Ok(true) };
            if revision != state.revision || epoch.is_some_and(|epoch| epoch != state.epoch) { return Ok(true); }
            let Some(item) = state.items.get(key).filter(|i| i.phase == Phase::WaitingJoin
                && run.is_none_or(|(run, pin)| i.run == run && i.pin == pin)) else { return Ok(true) };
            self.refresh_waiting_join(workspace, key, item, now_ms, &mut BTreeMap::new()).await
        }.await;
        if let Err(error) = &result { tracing::debug!(%workspace, item = %key, %error, "queue held for waiting branch"); }
        result.unwrap_or(false)
    }

    async fn refresh_waiting_join(&self, workspace: &str, key: &str, item: &Item, now_ms: i64,
        heads: &mut BTreeMap<String, Option<String>>) -> Result<bool>
    {
        let Some(config) = self.item_configuration(workspace, key, item)? else { return Ok(true) };
        let Some(previous) = item.candidate.as_deref().filter(|c| coding::sha(c)) else { return Ok(false) };
        let target = digest(&(&config.repository, &config.landing_ref, &config.landing));
        let mut exec = self.exec.clone();
        exec.coding = Some(config);
        if !heads.contains_key(&target) {
            let head = exec.waiting_landing_head(&target).await;
            heads.insert(target.clone(), head.as_ref().ok().cloned());
            head?;
        }
        let Some(head) = heads[&target].as_deref() else { return Ok(false) };
        let Some((base, result)) = exec.rebase_waiting_branch(workspace, key, item, previous, head).await? else {
            return Ok(true);
        };
        let ready = base == head && matches!(result, JoinRefresh::Rebased { .. });
        let receipt = self.internal(workspace, Command::RefreshJoin {
            item: key.into(), previous: previous.into(), base, result,
        }, Grant::Controller, now_ms)?;
        Ok(receipt.result.is_ok() && ready)
    }
}

impl Exec {
    async fn waiting_landing_head(&self, target: &str) -> Result<String> {
        let repository = self.repository();
        let Some(landing) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else {
            return adapters::git(&repository, &["rev-parse", "--verify", self.landing_ref()]).await;
        };
        coding::validate_landing_identity(landing)?;
        let log = |op| self.root.join("receipts").join(format!("{}.waiting-main-{op}.output", adapters::folder_name(target)));
        let head = if coding::landing_url(landing).is_some() {
            let log = log("read");
            let _ = std::fs::remove_file(&log);
            self.start_head_within(&log, TARGET_WAIT).await?.ok_or(Error::Refused("missing network landing head"))?
        } else {
            adapters::git(&landing.repository, &["rev-parse", "--verify", &landing.target]).await?
        };
        if coding::sha(&head) && adapters::git(&repository, &["cat-file", "-e", &format!("{head}^{{commit}}")]).await.is_ok() {
            return Ok(head);
        }
        let log = log("fetch");
        let _ = std::fs::remove_file(&log);
        let (_keep, cancel) = tokio::sync::watch::channel(false);
        let fetched = if let Some(url) = coding::landing_url(landing) {
            self.fetch_target_within(landing, url, &log, TARGET_WAIT).await?
        } else {
            let config = adapters::git(&repository, &["config", "--list"]).await?;
            if super::coding_landing::redirects_transport(&config) {
                return Err(Error::Refused("local landing config redirects transport"));
            }
            self.fetch_local_target(landing, &log, TARGET_WAIT, &cancel).await?
        };
        fetched.ok_or(Error::Refused("landing head could not be fetched"))
    }

    async fn rebase_waiting_branch(&self, workspace: &str, key: &str, item: &Item, previous: &str, base: &str)
        -> Result<Option<(String, JoinRefresh)>>
    {
        let repository = self.repository();
        if !coding::sha(base) { return Err(Error::Refused("invalid landing head")); }
        if ancestor(&repository, base, previous).await? { return Ok(None); }

        let (tree, branch) = match &item.tree {
            Some(tree) => (PathBuf::from(&tree.path), format!("refs/heads/{}", tree.branch)),
            None => (self.item_tree(workspace, key), format!("refs/heads/wf-{}", &digest(&(workspace, key))[4..])),
        };
        let canonical = tree.canonicalize()?;
        let common = adapters::git(&tree, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
        let top = adapters::git(&tree, &["rev-parse", "--show-toplevel"]).await?;
        if canonical == repository || Path::new(&top).canonicalize()? != canonical
            || Path::new(&common).canonicalize()? != repository.join(".git").canonicalize()?
            || branch == self.landing_ref()
        {
            return Err(Error::Refused("waiting worktree is not owned by this item"));
        }

        let action = format!("farcooler-rebase-{}", &digest(&(workspace, key, &item.run, previous))[4..]);
        let git_dir = adapters::git(&tree, &["rev-parse", "--absolute-git-dir"]).await?;
        let git_dir = Path::new(&git_dir);
        let rebase = git_dir.join("rebase-merge");
        if rebase.try_exists()? {
            return settle_rebase_conflict(&tree, &rebase, &action, &branch, previous).await;
        }
        for name in ["rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "sequencer", "BISECT_START"] {
            if git_dir.join(name).try_exists()? { return Err(Error::Refused("waiting worktree has a Git operation in progress")); }
        }
        if adapters::git(&tree, &["symbolic-ref", "HEAD"]).await? != branch
            || !adapters::git(&tree, &["status", "--porcelain", "--untracked-files=all"]).await?.is_empty()
        {
            return Err(Error::Refused("waiting worktree branch changed or is dirty"));
        }
        let head = adapters::git(&tree, &["rev-parse", "HEAD"]).await?;
        if head != previous {
            // Git may have finished just before the runner exited or its store write failed.
            // Only accept a reflog entry belonging to this waiting candidate.
            let reflog = adapters::git(&tree, &["reflog", "-1", "--format=%H %gs", &branch]).await?;
            let onto = reflog.strip_prefix(&format!("{head} {action} (finish): {branch} onto "));
            if let Some(onto) = onto.filter(|onto| coding::sha(onto))
                && ancestor(&tree, onto, &head).await?
            {
                return Ok(Some((onto.into(), JoinRefresh::Rebased { candidate: head })));
            }
            return Err(Error::Refused("waiting branch no longer matches its candidate"));
        }

        let signer = rebase_signer(&tree, previous, base).await?;
        if !run_rebase(&tree, &action, base, signer).await? {
            if !rebase.try_exists()? { return Err(Error::Refused("waiting branch rebase did not start")); }
            return settle_rebase_conflict(&tree, &rebase, &action, &branch, previous).await;
        }
        let candidate = adapters::git(&tree, &["rev-parse", "HEAD"]).await?;
        if !coding::sha(&candidate) || !ancestor(&tree, base, &candidate).await? {
            return Err(Error::Refused("rebased candidate does not contain its base"));
        }
        Ok(Some((base.into(), JoinRefresh::Rebased { candidate })))
    }
}

async fn ancestor(tree: &Path, base: &str, candidate: &str) -> Result<bool> {
    let output = crate::git::git(tree, &["merge-base", base, candidate]).await
        .map_err(|_| Error::Refused("could not compare landing base and waiting candidate"))?;
    if !output.ok { return Err(Error::Refused("landing base and waiting candidate have no common history")); }
    Ok(output.stdout.trim() == base)
}

async fn settle_rebase_conflict(tree: &Path, rebase: &Path, action: &str, branch: &str, previous: &str)
    -> Result<Option<(String, JoinRefresh)>>
{
    // An interrupted automatic rebase can be recovered by the same waiting run.
    let reflog = adapters::git(tree, &["reflog", "-1", "--format=%gs", "HEAD"]).await?;
    let onto = std::fs::read_to_string(rebase.join("onto"))?;
    let base = onto.trim();
    if !reflog.starts_with(&format!("{action} (")) || !coding::sha(base)
        || std::fs::read_to_string(rebase.join("orig-head"))?.trim() != previous
        || std::fs::read_to_string(rebase.join("head-name"))?.trim() != branch
    {
        return Err(Error::Refused("waiting worktree has another rebase in progress"));
    }
    let paths = crate::git::git_bytes(tree, &["diff", "--name-only", "--diff-filter=U", "-z"]).await
        .map_err(|_| Error::Refused("could not read conflicting paths"))?;
    if !paths.ok { return Err(Error::Refused("could not read conflicting paths")); }
    adapters::git(tree, &["rebase", "--abort"]).await?;
    if adapters::git(tree, &["rev-parse", "HEAD"]).await? != previous {
        return Err(Error::Refused("failed rebase did not restore the waiting branch"));
    }
    if paths.stdout.is_empty() { return Err(Error::Refused("waiting branch rebase failed without conflicts")); }
    let paths = paths.stdout.split(|byte| *byte == 0).filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned()).collect();
    Ok(Some((base.into(), JoinRefresh::Conflict { paths })))
}

/// Rewrites need new signatures. Honor the repository's signing setting and explicitly sign
/// tickets that carry signed commits even when signing is off by default.
async fn rebase_signer(tree: &Path, previous: &str, base: &str) -> Result<Option<(&'static str, &'static str, PathBuf)>> {
    let required = adapters::git(tree, &["config", "--type=bool", "--get", "commit.gpgsign"]).await.is_ok_and(|value| value == "true");
    if !required {
        let commits = adapters::git(tree, &["rev-list", &format!("{base}..{previous}")]).await?;
        let mut signed = false;
        for commit in commits.lines() {
            let raw = adapters::git(tree, &["cat-file", "commit", commit]).await?;
            let headers = raw.split("\n\n").next().unwrap_or_default();
            if headers.lines().any(|line| line.starts_with("gpgsig ") || line.starts_with("gpgsig-sha256 ")) {
                signed = true;
                break;
            }
        }
        if !signed { return Ok(None); }
    }
    let format = adapters::git(tree, &["config", "--get", "gpg.format"]).await.unwrap_or_else(|_| "openpgp".into());
    let (format, config, program) = match format.as_str() {
        "openpgp" => ("openpgp", "gpg.openpgp.program", "gpg"),
        "ssh" => ("ssh", "gpg.ssh.program", "ssh-keygen"),
        "x509" => ("x509", "gpg.x509.program", "gpgsm"),
        _ => return Err(Error::Refused("unsupported rebase signature format")),
    };
    let program = farcooler_core::programs::find(program).ok_or(Error::Refused("rebase signing program unavailable"))?;
    Ok(Some((format, config, program)))
}

async fn run_rebase(tree: &Path, action: &str, base: &str, signer: Option<(&str, &str, PathBuf)>) -> Result<bool> {
    let err = |_| Error::Refused("could not start waiting branch rebase");
    let mut pins = crate::git::guard_pins(tree).await.map_err(err)?;
    pins.extend([("rerere.enabled".into(), "false".into()), ("core.logAllRefUpdates".into(), "true".into()),
        ("notes.rewrite.rebase".into(), "false".into())]);
    let config = adapters::git(tree, &["config", "--list"]).await?;
    pins.extend(config.lines().filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.starts_with("merge.") && key.ends_with(".driver"))
        .map(|(key, _)| (key.into(), "/usr/bin/false".into())));
    if adapters::git(tree, &["var", "GIT_COMMITTER_IDENT"]).await.is_err() {
        pins.extend([("user.name".into(), "Far Cooler".into()), ("user.email".into(), "landing@farcooler.invalid".into())]);
    }
    let git = crate::git_launch::git_launch().map_err(err)?;
    let (launch, sign) = if let Some((format, config, program)) = signer {
        pins.extend([("gpg.format".into(), format.into()), (config.into(), program.as_os_str().into()),
            ("gpg.program".into(), program.as_os_str().into()),
            ("gpg.ssh.defaultKeyCommand".into(), "/usr/bin/false".into())]);
        // Allow only the usual signer resolved from the runner's PATH. Repository config
        // cannot nominate an executable or loosen the existing Git allowlist.
        let mut allowed = git.sandbox.as_ref().map(|sandbox| sandbox.allowed().to_vec()).unwrap_or_else(|| vec![git.program.clone()]);
        allowed.push(program);
        let sandbox = crate::git_sandbox::Sandbox::new(&git.program, crate::git_sandbox::allowlist(&allowed));
        (crate::git_launch::Launch { program: git.program.clone(), sandbox }, "--gpg-sign")
    } else {
        (crate::git_launch::Launch { program: git.program.clone(), sandbox: git.sandbox.clone() }, "--no-gpg-sign")
    };
    let mut command = launch.command().map_err(err)?;
    crate::git_guard::prepare(&mut command, Some(&pins), tree);
    let child = tokio::process::Command::from(command).current_dir(tree)
        .env("GIT_REFLOG_ACTION", action).env("GIT_EDITOR", "/usr/bin/false").env("GIT_SEQUENCE_EDITOR", "/usr/bin/false")
        .args(["rebase", "--merge", "--rebase-merges", "--no-fork-point", "--no-autostash", "--no-update-refs", sign, base])
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .kill_on_drop(true).spawn()?;
    let output = tokio::time::timeout(REBASE_WAIT, child.wait_with_output()).await
        .map_err(|_| Error::Refused("waiting branch rebase timed out"))??;
    if !output.status.success() {
        tracing::debug!(tree = %tree.display(), stderr = %String::from_utf8_lossy(&output.stderr), "waiting branch rebase stopped");
    }
    Ok(output.status.success())
}
