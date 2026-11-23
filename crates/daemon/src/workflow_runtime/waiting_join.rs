//! Check whether the landing fold can apply a waiting candidate to the current target.
use super::{adapters, coding, Error, Exec, Result, Runtime};
use super::coding_verbs::PickConflict;
use farcooler_protocol::workflow_commands::{Action, MAX_BYTES};
use farcooler_protocol::workflow_config::AdapterConfiguration;
use farcooler_workflow::{digest, valid_id, Command, Grant, Grouping, Item, JoinRefresh, Member, Phase, StageKind, State};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const TARGET_WAIT: Duration = Duration::from_secs(5);
const FOLD_CHECK_WAIT: Duration = Duration::from_secs(20);

impl Runtime {
    pub(super) async fn wire_queue_check(&self, workspace: &str, principal: &str, action: &Action) -> Result<QueueCheck> {
        let Action::Queue { mutation, item, run, pin, .. } = action else { return Ok(QueueCheck::Ready) };
        if !valid_id(&mutation.id) || mutation.id.len() > 100 { return Ok(QueueCheck::Ready); }
        let payload = serde_json::to_string(&(principal, action))?;
        if payload.len() > MAX_BYTES { return Err(Error::Refused("oversized command")); }
        let id = format!("wire:{principal}:{}", mutation.id);
        if self.store.workflow_wire_receipt(workspace, &id, &payload)?.is_some() { return Ok(QueueCheck::Ready); }
        Ok(self.check_queue(workspace, item, mutation.revision, Some(mutation.epoch), Some((run, pin))).await)
    }

    /// Evaluate the waiting tickets before automatic queueing, sharing each target read within this pass.
    pub(super) async fn check_auto_queues(&self, workspaces: &[String], now_ms: i64) -> Result<BTreeSet<(String, String)>> {
        let mut heads = BTreeMap::new();
        let mut ready = BTreeSet::new();
        for workspace in workspaces {
            if self.shutdown.as_ref().is_some_and(|s| *s.borrow()) { break; }
            if self.service_mode && self.store.workflow_board_deleted(workspace)? { continue; }
            let Some(state) = self.state(workspace)? else { continue };
            let published = if self.service_mode { self.store.workflow_configuration(workspace)? } else { None };
            for (key, item) in state.items.iter().filter(|(_, i)| i.phase == Phase::WaitingJoin
                && (i.queue_hold.is_some() || i.definition.start_policy.as_ref().is_some_and(|p| p.auto_queue)))
            {
                if self.automatic_queue_deferred(workspace, key, now_ms) { continue; }
                let check = if published.as_ref().is_some_and(|p| !queue_publication_is_current(&state, item, &p.pin)) {
                    QueueCheck::Held(PUBLICATION_HOLD.into())
                } else {
                    self.check_waiting_join(workspace, key, item, state.grouping.as_ref(), &mut heads).await
                        .unwrap_or_else(|error| held(workspace, key, "Could not read the landing configuration", error))
                };
                if let QueueCheck::Held(reason) = &check {
                    self.record_queue_hold(workspace, key, item, reason, now_ms)?;
                } else if self.apply_queue_check(workspace, key, check, now_ms)?.is_none() {
                    ready.insert((workspace.clone(), key.clone()));
                }
            }
        }
        Ok(ready)
    }

    pub(super) async fn check_queue(&self, workspace: &str, key: &str, revision: u64,
        epoch: Option<u64>, run: Option<(&str, &str)>) -> QueueCheck
    {
        let result = async {
            let Some(state) = self.state(workspace)? else { return Ok(QueueCheck::Ready) };
            if revision != state.revision || epoch.is_some_and(|epoch| epoch != state.epoch) { return Ok(QueueCheck::Ready); }
            let Some(item) = state.items.get(key).filter(|i| i.phase == Phase::WaitingJoin
                && run.is_none_or(|(run, pin)| i.run == run && i.pin == pin)) else { return Ok(QueueCheck::Ready) };
            if self.service_mode && self.store.workflow_configuration(workspace)?
                .is_some_and(|p| !queue_publication_is_current(&state, item, &p.pin))
            {
                return Ok(QueueCheck::Held(PUBLICATION_HOLD.into()));
            }
            self.check_waiting_join(workspace, key, item, state.grouping.as_ref(), &mut BTreeMap::new()).await
        }.await;
        result.unwrap_or_else(|error| held(workspace, key, "Could not read the landing configuration", error))
    }

    async fn check_waiting_join(&self, workspace: &str, key: &str, item: &Item, grouping: Option<&Grouping>,
        heads: &mut BTreeMap<String, std::result::Result<String, String>>) -> Result<QueueCheck>
    {
        let StageKind::Join { group, .. } = &item.definition.stages[item.stage].kind else {
            return Ok(QueueCheck::Held("The waiting ticket has no landing join step.".into()));
        };
        // Queue uses the published group when it still defines this join's group name.
        let config = if let Some(grouping) = grouping.filter(|p| self.service_mode && p.definition.groups.contains_key(group)) {
            let bundle = self.store.workflow_bundle(&grouping.policy_pin)?.ok_or(Error::Refused("missing group bundle"))?;
            bundle.adapter.map(|AdapterConfiguration::Coding(config)| config)
        } else {
            self.item_configuration(workspace, key, item)?
        };
        let Some(config) = config else { return Ok(QueueCheck::Ready) };
        let Some(candidate) = item.candidate.as_deref().filter(|c| coding::sha(c)) else { return Ok(QueueCheck::Held("The reviewed ticket has no valid candidate commit.".into())) };
        let target = digest(&(&config.repository, &config.landing_ref, &config.landing));
        let mut exec = self.exec.clone();
        exec.coding = Some(config);
        if !heads.contains_key(&target) {
            let head = exec.waiting_landing_head(&target).await
                .map_err(|error| describe_hold(workspace, key, "Could not read or fetch the landing target head", error));
            heads.insert(target.clone(), head);
        }
        let head = match &heads[&target] {
            Ok(head) if coding::sha(head) => head,
            Ok(_) => return Ok(QueueCheck::Held("The landing target has no valid head commit.".into())),
            Err(reason) => return Ok(QueueCheck::Held(reason.clone())),
        };
        let member = Member { item: key.into(), candidate: candidate.into() };
        let conflict = match exec.check_member(head, &member).await {
            Ok(Some(conflict)) => conflict,
            Ok(None) => return Ok(QueueCheck::Ready),
            Err(error) => return Ok(held(workspace, key, "Could not check the reviewed candidate against the landing head", error)),
        };
        Ok(QueueCheck::Conflict { run: item.run.clone(), pin: item.pin.clone(), candidate: member.candidate,
            base: head.into(), paths: conflict.files })
    }

    pub(super) fn record_queue_hold(&self, workspace: &str, key: &str, checked: &Item,
        reason: &str, now_ms: i64) -> Result<()>
    {
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        let Some(item) = state.items.get(key).filter(|i| i.phase == Phase::WaitingJoin
            && i.run == checked.run && i.pin == checked.pin && i.candidate == checked.candidate
            && i.policy_pin == checked.policy_pin) else { return Ok(()) };
        if item.queue_hold.as_ref().is_none_or(|hold| hold.reason != reason || (!hold.notified
            && now_ms.saturating_sub(hold.since_ms) >= farcooler_workflow::QUEUE_HOLD_NOTICE_MS))
        {
            let receipt = self.apply(super::Request {
                id: format!("runtime:{}", uuid::Uuid::now_v7()), workspace: workspace.into(),
                expected_revision: state.revision, sequence: state.source_sequences.get("runtime").copied().unwrap_or(0) + 1,
                command: Command::HoldQueue { item: key.into(), reason: reason.into() },
            }, "runtime", Grant::Controller, now_ms)?;
            if let Err(error) = receipt.result {
                tracing::warn!(%workspace, item = %key, ?error, "could not record landing preflight hold");
            }
        }
        self.defer_automatic_queue(workspace, key, now_ms);
        Ok(())
    }

    pub(super) fn apply_queue_check(&self, workspace: &str, key: &str, check: QueueCheck, now_ms: i64) -> Result<Option<String>> {
        let (run, pin, candidate, base, paths) = match check {
            QueueCheck::Ready => return Ok(None),
            QueueCheck::Held(reason) => return Ok(Some(reason)),
            QueueCheck::Conflict { run, pin, candidate, base, paths } => (run, pin, candidate, base, paths),
        };
        let Some(state) = self.state(workspace)? else { return Ok(Some("The workspace changed during the landing check.".into())) };
        if !state.items.get(key).is_some_and(|i| i.phase == Phase::WaitingJoin && i.run == run
            && i.pin == pin && i.candidate.as_ref() == Some(&candidate))
        {
            return Ok(Some("The reviewed candidate changed during the landing check.".into()));
        }
        let receipt = self.apply(super::Request {
            id: format!("runtime:{}", uuid::Uuid::now_v7()), workspace: workspace.into(),
            expected_revision: state.revision, sequence: state.source_sequences.get("runtime").copied().unwrap_or(0) + 1,
            command: Command::RefreshJoin { item: key.into(), previous: candidate, base, result: JoinRefresh::Conflict { paths } },
        }, "runtime", Grant::Controller, now_ms)?;
        if let Err(error) = receipt.result {
            self.defer_automatic_queue(workspace, key, now_ms);
            tracing::debug!(%workspace, item = %key, ?error, "conflicting waiting candidate could not be sent back");
        }
        Ok(Some("The reviewed candidate conflicts with the landing head.".into()))
    }
}

pub(super) enum QueueCheck {
    Ready,
    Held(String),
    Conflict { run: String, pin: String, candidate: String, base: String, paths: Vec<String> },
}

const PUBLICATION_HOLD: &str = "Waiting for the ticket and its landing group to adopt the current workflow publication.";

fn held(workspace: &str, key: &str, step: &str, error: Error) -> QueueCheck {
    QueueCheck::Held(describe_hold(workspace, key, step, error))
}

fn describe_hold(workspace: &str, key: &str, step: &str, error: Error) -> String {
    tracing::debug!(%workspace, item = %key, %error, step, "landing preflight held");
    // Keep paths, remote credentials, and arbitrary Git output off the wire.
    match error {
        Error::Refused(reason) => format!("{step} ({reason})."),
        Error::Io(error) => format!("{step} ({}).", error.kind()),
        _ => format!("{step}."),
    }
}

/// Do not queue an old candidate or start the old group rules while publication migration waits.
fn queue_publication_is_current(state: &State, item: &Item, pin: &str) -> bool {
    item.policy_pin == pin && state.grouping.as_ref().is_none_or(|g| g.policy_pin == pin)
}

impl Exec {
    async fn waiting_landing_head(&self, target: &str) -> Result<String> {
        let repository = self.repository();
        let Some(landing) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else {
            return queue_git(&repository, &["rev-parse", "--verify", self.landing_ref()], "local landing ref could not be resolved").await;
        };
        coding::validate_landing_identity(landing)?;
        let log = |op| self.root.join("receipts").join(format!("{}.waiting-main-{op}.output", adapters::folder_name(target)));
        let head = if coding::landing_url(landing).is_some() {
            let log = log("read");
            let _ = std::fs::remove_file(&log);
            self.start_head_within(&log, TARGET_WAIT).await?.ok_or(Error::Refused("missing network landing head"))?
        } else {
            queue_git(&landing.repository, &["rev-parse", "--verify", &landing.target], "local landing target could not be resolved").await?
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

    /// Run the same picks as a landing fold in a detached worktree with its own index and HEAD.
    async fn check_member(&self, head: &str, member: &Member) -> Result<Option<PickConflict>> {
        if head == member.candidate { return Ok(None); }
        let repository = self.repository();
        queue_git(&repository, &["cat-file", "-e", &format!("{}^{{commit}}", member.candidate)],
            "reviewed candidate commit is unavailable in the runner repository").await?;
        let fork = queue_git(&repository, &["merge-base", head, &member.candidate],
            "candidate and target have no readable common ancestor").await?;
        // A linear series already based on this head applies in order. A merge
        // commit needs the actual pick sequence because the fold skips merges.
        if fork == head && queue_git(&repository, &["rev-list", "--merges", "--max-count=1",
            &format!("{head}..{}", member.candidate)], "candidate history could not be read").await?.is_empty()
        {
            return Ok(None);
        }
        let scratch = tempfile::Builder::new().prefix("queue-check-").tempdir_in(&self.root)?;
        let tree = scratch.path().join("tree");
        let path = tree.to_str().ok_or(Error::Refused("invalid queue check worktree path"))?;
        let created = queue_git(&repository, &["worktree", "add", "--detach", path, head], "temporary check worktree could not be created").await;
        let result = match created {
            Ok(_) => tokio::time::timeout(FOLD_CHECK_WAIT, self.pick_member(None, &tree, member)).await
                .unwrap_or(Err(Error::Refused("temporary landing fold exceeded 20 seconds"))),
            Err(error) => Err(error),
        };
        if tree.try_exists()? {
            queue_git(&repository, &["worktree", "remove", "--force", path], "temporary check worktree could not be removed").await?;
        }
        result
    }
}

async fn queue_git(repository: &std::path::Path, args: &[&str], refused: &'static str) -> Result<String> {
    match crate::git::git(repository, args).await {
        Ok(output) if output.ok => Ok(output.stdout.trim().into()),
        Ok(output) => {
            tracing::debug!(operation = args[0], stderr = %output.stderr, "landing preflight Git refused");
            Err(Error::Refused(refused))
        }
        Err(_) => Err(Error::Refused("Git invocation failed or timed out")),
    }
}
