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
                && i.definition.start_policy.as_ref().is_some_and(|p| p.auto_queue)
                && published.as_ref().is_none_or(|p| queue_publication_is_current(&state, i, &p.pin)))
            {
                if self.automatic_queue_deferred(workspace, key, now_ms) { continue; }
                match self.check_waiting_join(workspace, key, item, state.grouping.as_ref(), &mut heads).await
                    .and_then(|check| self.apply_queue_check(workspace, key, check, now_ms))
                {
                    Ok(true) => { ready.insert((workspace.clone(), key.clone())); }
                    Ok(false) => {}
                    Err(error) => tracing::debug!(%workspace, item = %key, %error, "waiting candidate could not be checked"),
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
                return Ok(QueueCheck::Held);
            }
            self.check_waiting_join(workspace, key, item, state.grouping.as_ref(), &mut BTreeMap::new()).await
        }.await;
        if let Err(error) = &result { tracing::debug!(%workspace, item = %key, %error, "queue held for waiting candidate"); }
        result.unwrap_or(QueueCheck::Held)
    }

    async fn check_waiting_join(&self, workspace: &str, key: &str, item: &Item, grouping: Option<&Grouping>,
        heads: &mut BTreeMap<String, Option<String>>) -> Result<QueueCheck>
    {
        let StageKind::Join { group, .. } = &item.definition.stages[item.stage].kind else {
            return Ok(QueueCheck::Held);
        };
        // Queue uses the published group when it still defines this join's group name.
        let config = if let Some(grouping) = grouping.filter(|p| self.service_mode && p.definition.groups.contains_key(group)) {
            let bundle = self.store.workflow_bundle(&grouping.policy_pin)?.ok_or(Error::Refused("missing group bundle"))?;
            bundle.adapter.map(|AdapterConfiguration::Coding(config)| config)
        } else {
            self.item_configuration(workspace, key, item)?
        };
        let Some(config) = config else { return Ok(QueueCheck::Ready) };
        let Some(candidate) = item.candidate.as_deref().filter(|c| coding::sha(c)) else { return Ok(QueueCheck::Held) };
        let target = digest(&(&config.repository, &config.landing_ref, &config.landing));
        let mut exec = self.exec.clone();
        exec.coding = Some(config);
        if !heads.contains_key(&target) {
            let head = exec.waiting_landing_head(&target).await;
            heads.insert(target.clone(), head.as_ref().ok().cloned());
            head?;
        }
        let Some(head) = heads[&target].as_deref().filter(|head| coding::sha(head)) else { return Ok(QueueCheck::Held) };
        let member = Member { item: key.into(), candidate: candidate.into() };
        let Some(conflict) = exec.check_member(head, &member).await? else { return Ok(QueueCheck::Ready) };
        Ok(QueueCheck::Conflict { run: item.run.clone(), pin: item.pin.clone(), candidate: member.candidate,
            base: head.into(), paths: conflict.files })
    }

    pub(super) fn apply_queue_check(&self, workspace: &str, key: &str, check: QueueCheck, now_ms: i64) -> Result<bool> {
        let (run, pin, candidate, base, paths) = match check {
            QueueCheck::Ready => return Ok(true),
            QueueCheck::Held => return Ok(false),
            QueueCheck::Conflict { run, pin, candidate, base, paths } => (run, pin, candidate, base, paths),
        };
        let Some(state) = self.state(workspace)? else { return Ok(false) };
        if !state.items.get(key).is_some_and(|i| i.phase == Phase::WaitingJoin && i.run == run
            && i.pin == pin && i.candidate.as_ref() == Some(&candidate))
        {
            return Ok(false);
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
        Ok(false)
    }
}

pub(super) enum QueueCheck {
    Ready,
    Held,
    Conflict { run: String, pin: String, candidate: String, base: String, paths: Vec<String> },
}

/// Do not queue an old candidate or start the old group rules while publication migration waits.
fn queue_publication_is_current(state: &State, item: &Item, pin: &str) -> bool {
    item.policy_pin == pin && state.grouping.as_ref().is_none_or(|g| g.policy_pin == pin)
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

    /// Run the same picks as a landing fold in a detached worktree with its own index and HEAD.
    async fn check_member(&self, head: &str, member: &Member) -> Result<Option<PickConflict>> {
        if head == member.candidate { return Ok(None); }
        let repository = self.repository();
        let fork = adapters::git(&repository, &["merge-base", head, &member.candidate]).await?;
        // A linear series already based on this head applies in order. A merge
        // commit needs the actual pick sequence because the fold skips merges.
        if fork == head && adapters::git(&repository, &["rev-list", "--merges", "--max-count=1",
            &format!("{head}..{}", member.candidate)]).await?.is_empty()
        {
            return Ok(None);
        }
        let scratch = tempfile::Builder::new().prefix("queue-check-").tempdir_in(&self.root)?;
        let tree = scratch.path().join("tree");
        let path = tree.to_str().ok_or(Error::Refused("invalid queue check worktree path"))?;
        let created = adapters::git(&repository, &["worktree", "add", "--detach", path, head]).await;
        let result = match created {
            Ok(_) => tokio::time::timeout(FOLD_CHECK_WAIT, self.pick_member(None, &tree, member)).await
                .unwrap_or(Err(Error::Refused("waiting candidate check timed out"))),
            Err(error) => Err(error),
        };
        if tree.try_exists()? {
            adapters::git(&repository, &["worktree", "remove", "--force", path]).await?;
        }
        result
    }
}
