//! Only the RPC admission seam supplies principal; the wire vocabulary has none.
use super::*;
use farcooler_protocol::workflow_commands::{Action, MAX_BYTES};
use farcooler_store::{workflow::WorkflowActor, workflow_publication::Write};

/// `workflow.get` for anyone but the owner (R-111, M1): every value in the adapter's command environments is
/// replaced by `HIDDEN`, and the names stay, so a member sees what a gate is given without what it is given.
/// The definitions are left whole: their pins are checked by the apps, and they carry no environment values.
pub(super) fn hide_environment(reply: &mut serde_json::Value) {
    fn walk(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => for (name, field) in fields.iter_mut() {
                match (name.as_str(), field) {
                    ("environment", serde_json::Value::Object(environment)) =>
                        for v in environment.values_mut() { *v = HIDDEN.into(); },
                    (_, field) => walk(field),
                }
            },
            serde_json::Value::Array(items) => items.iter_mut().for_each(walk),
            _ => {}
        }
    }
    if let Some(adapter) = reply.pointer_mut("/configuration/adapter") { walk(adapter); }
}
/// What a hidden environment value reads as.
pub(super) use crate::workflow_summary::HIDDEN;

/// Runner-prepared inputs. Neither field is accepted from the wire.
#[derive(Default)]
pub(super) struct Prepared { pub tree: Option<RunTree>, pub close_verified: bool }

/// The CLI's small view is a projection only: no durable item or archive is changed.
fn summarize(mut value:serde_json::Value) -> serde_json::Value {
    if let Some(config) = value.get_mut("configuration").and_then(|v| v.as_object_mut()) {
        config.remove("definition"); config.remove("adapter");
    }
    if let Some(state) = value.get_mut("state").and_then(|v| v.as_object_mut()) {
        state.remove("seen");
        if let Some(archive) = state.get_mut("archive").and_then(|v| v.as_object_mut()) {
            archive.retain(|_, item| item.get("group").is_none_or(|g| g.is_null()));
        }
        if let Some(items) = state.get_mut("items").and_then(|v| v.as_object_mut()) {
            items.retain(|_, item| !(item.get("group_of").is_some_and(|g| !g.is_null())
                && matches!(item["phase"].as_str(), Some("Done" | "Failed" | "Cancelled"))));
            for item in items.values_mut().filter_map(|v| v.as_object_mut()) {
                item.remove("definition");
                item.entry("members").or_insert_with(|| serde_json::json!([]));
                for field in ["grouped_in", "tree"] { item.entry(field).or_insert(serde_json::Value::Null); }
            }
        }
    }
    value.as_object_mut().unwrap().remove("notice_deliveries");
    value.as_object_mut().unwrap().remove("notices");
    value
}

impl Exec {
    /// Reads committed records and local stop receipts only; never executes git or drives effects.
    fn read_get(&self, workspace:&str) -> Result<serde_json::Value> {
        let deliveries=self.store.workflow_notice_deliveries(workspace)?;
        let notices:Vec<_>=deliveries.iter().map(|d|farcooler_store::workflow::WorkflowNotice {
            id:d.notice.id.clone(),revision:d.notice.revision,body:format!("{}:{:?}",d.notice.item,d.notice.point)
        }).collect();
        let state = self.execution_state(workspace)?;
        // Why a run's named tree stopped it, for the run it stopped (ov-979).
        let mut stops = serde_json::Map::new();
        for (item, live) in state.iter().flat_map(|s| &s.items) {
            let Ok(bytes) = std::fs::read(self.tree_stop_path(workspace, item)) else { continue };
            if let Ok(stop) = serde_json::from_slice::<named_tree::TreeStop>(&bytes) {
                if stop.run == live.run { stops.insert(item.clone(), serde_json::json!(stop)); }
            }
        }
        let budgets: std::collections::BTreeMap<_, _> = state.iter().flat_map(|s| &s.items)
            .map(|(id, item)| (id.clone(), farcooler_workflow::back_edge_budgets(item))).collect();
        Ok(serde_json::json!({"schema":1,"configuration": self.store.workflow_configuration(workspace)?,
            "build_cleanup":self.store.workflow_cleanups(workspace)?, "back_edge_budgets":budgets, "process_logs":self.failed_process_logs(workspace)?, "places":self.places().view(workspace), "state":state, "notices":notices,"notice_deliveries":deliveries, "stops":stops}))
    }
}
impl CommandHandle {
    /// A read has no receipt or reducer input, so it need not join the single writer's inbox.
    /// SQLite, file reads and JSON construction run on a blocking worker; authority is rechecked
    /// after the wait before any reply (including owner-only configuration values) is released.
    pub(crate) async fn read_get(&self, admission:Admission, full:bool) -> Result<serde_json::Value> {
        if self.is_closed() { return Err(Error::Refused("runtime inbox closed")); }
        admission.recheck().and_then(|()| admission.permits(&Action::Get)).map_err(Error::Domain)?;
        let reader = self.reader.clone();
        let workspace = admission.workspace();
        let mut value = tokio::task::spawn_blocking(move || {
            let value = reader.read_get(&workspace)?;
            Ok::<_, Error>(if full { value } else { summarize(value) })
        }).await.map_err(|_| Error::Refused("workflow read worker failed"))??;
        admission.recheck().and_then(|()| admission.permits(&Action::Get)).map_err(Error::Domain)?;
        if self.is_closed() { return Err(Error::Refused("runtime inbox closed")); }
        let epoch = value.pointer("/state/epoch").and_then(|v| v.as_u64()).unwrap_or(1);
        value["context"] = admission.context(epoch);
        if !admission.is_owner() { hide_environment(&mut value); }
        Ok(value)
    }
    pub(crate) async fn wire_command(&self, admission: Admission, action: Action) -> Result<serde_json::Value> {
        let (send, receive) = oneshot::channel();
        self.send.send(Inbox::Wire(Box::new(admission), Box::new(action), send)).await
            .map_err(|_| Error::Refused("runtime inbox closed"))?;
        receive.await.map_err(|_| Error::Refused("runtime reply closed"))?
    }
}
impl Runtime {
    #[cfg(test)]
    pub(super) fn wire_command(&self, workspace: &str, principal: &str, action: Action) -> Result<serde_json::Value> {
        self.wire_command_with(workspace, principal, &[principal.to_string()], None, action, Prepared::default())
    }
    /// `prepared.tree` is what `prepare_start` resolved for a Start that names a worktree (ov-967). `actor` is who sent
    /// it, as admission read them, written into the receipt (ov-1001). `deciders` are the decider names the caller
    /// holds (`Admission::deciders`): a decision is made as the one its gate waits on, if the caller holds it, and
    /// otherwise as `principal`, which the reducer then refuses (ov-1002).
    pub(super) fn wire_command_with(&self, workspace: &str, principal: &str, deciders: &[String], actor: Option<&WorkflowActor>, action: Action,
        prepared: Prepared) -> Result<serde_json::Value>
    {
        let Prepared { tree, close_verified } = prepared;
        let payload = serde_json::to_string(&(principal, &action))?;
        if payload.len() > MAX_BYTES { return Err(Error::Refused("oversized command")); }
        // A proposal is filed and answered by the daemon before the engine is involved (`workflow_drafts`).
        if matches!(action, Action::Propose { .. } | Action::Drafts { .. }) { return Err(Error::Refused("a proposal is not an engine command")); }
        if matches!(action, Action::Get) {
            return self.exec.read_get(workspace);
        }
        let mutation = match &action {
            Action::Publish { mutation, .. } | Action::Enqueue { mutation, .. }
            | Action::Review { mutation, .. } | Action::Approve { mutation, .. } | Action::Cancel { mutation, .. } | Action::Retry { mutation, .. }
            | Action::ResolveCancellation { mutation, .. } | Action::Queue { mutation, .. } | Action::Leave { mutation, .. } | Action::Settle { mutation, .. }
            | Action::Track { mutation, .. } | Action::Move { mutation, .. } | Action::Start { mutation, .. }
            | Action::SendBack { mutation, .. } | Action::Close { mutation, .. } | Action::Reopen { mutation, .. } => mutation,
            Action::Get | Action::Propose { .. } | Action::Drafts { .. } => unreachable!(),
        };
        if !valid_id(&mutation.id) || mutation.id.len() > 100 { return Err(Error::Refused("invalid command id")); }
        let id = format!("wire:{principal}:{}", mutation.id);
        // Durable lookup precedes revision, epoch, run and configuration checks.
        if let Some(mut receipt) = self.store.workflow_wire_receipt(workspace, &id, &payload)? {
            // The id is the principal's, not the person's: `orchestrator` outlives a lead change, and a handle can
            // be issued again. Someone else replaying it learns who sent it, never their device or pane (R-111, L3).
            if let (Some(sender), Some(asker)) = (receipt.actor.as_mut(), actor)
                && (sender.person != asker.person || sender.device != asker.device || sender.pane != asker.pane)
            {
                (sender.device, sender.pane) = (None, None);
            }
            return Ok(serde_json::json!({"schema":1,"receipt":receipt}));
        }
        let state = self.state(workspace)?.unwrap_or_else(|| State::empty(workspace.into(), 1));
        let stale = state.epoch != mutation.epoch || state.revision != mutation.revision;
        let controller = self.controller();
        self.authorize(controller.instance)?;
        let session = self.decision_session(&controller, principal)?;
        let input = |command, grant| Input { schema:1,id:id.clone(),workspace:workspace.into(),
            expected_revision:mutation.revision,epoch:mutation.epoch,source:format!("wire:{principal}"),
            source_sequence:mutation.sequence,now_ms:now(),grant,command,busy_resources:BTreeSet::new() };
        let receipt = if stale {
            self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
        } else {
            match &action {
                Action::Publish { expected_pin, definition, adapter, notifications, .. } => {
                    // A valid pure definition can still name an unsupported fixture adapter.
                    self.validate_publication(definition, adapter.as_deref())?;
                    self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Publish {
                        expected_pin:expected_pin.as_deref(),definition,adapter:adapter.as_deref(),notifications:notifications.as_ref(),revision:state.revision })?
                }
                Action::Enqueue { item, pin, .. } => {
                    let configuration = self.store.workflow_configuration(workspace)?;
                    match configuration.filter(|c| &c.pin == pin) {
                        None => self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?,
                        Some(configuration) => {
                            self.validate_publication(&configuration.definition, configuration.adapter.as_ref())?;
                            self.refuse_removed_decider(&configuration.definition)?;
                            let initial_candidate = matches!(configuration.definition.stages[0].kind, StageKind::Tracker).then(|| pin.clone());
                            let input = input(Command::Enqueue { item:item.clone(),definition:configuration.definition,
                                policy_pin:pin.clone(),resource:Exec::item_resource(workspace, item),dependencies:BTreeSet::new(),initial_candidate },Grant::Controller);
                            self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                        }
                    }
                }
                Action::Approve { item, run, pin, candidate, .. } => {
                    let decider = crate::workflow_deciders::decide_as(crate::workflow_deciders::waiting_on(&state, item), deciders, &session.principal);
                    let input = input(Command::Approve { item:item.clone(),run:run.clone(),pin:pin.clone(),candidate:candidate.clone() },Grant::Decision { principal:decider.into() });
                    self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                }
                Action::Review { item, run, pin, evidence, passed, reason, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin && i.last_evidence.as_ref() == Some(evidence)) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        let decider = crate::workflow_deciders::decide_as(crate::workflow_deciders::waiting_on(&state, item), deciders, &session.principal);
                        let input = input(Command::Review { item:item.clone(),run:run.clone(),evidence:evidence.clone(),passed:*passed,reason:reason.clone() },Grant::Decision { principal:decider.into() });
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Cancel { item, run, pin, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        let input = input(Command::Cancel { item:item.clone() , reason: None },Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::ResolveCancellation { item, run, pin, reason, .. } => {
                    // The owner's recorded reconciliation closes an already fenced cancellation. It never
                    // supplies a candidate or a successful external result, and an active owner blocks it.
                    let live = state.items.get(item).filter(|i| &i.run == run && &i.pin == pin);
                    let effects = self.store.workflow_outbox(workspace)?;
                    let busy = self.activities.lock().map_err(|_| Error::Refused("execution observations unavailable"))?
                        .keys().any(|(board, effect)| board == workspace
                            && effects.iter().any(|e| &e.effect.id == effect && &e.effect.item == item));
                    let busy = busy || effects.iter().any(|e| e.effect.item == *item && e.active
                        && e.dispatch_state == "claimed" && e.lease_until.is_some_and(|t| t > self.clock_ms()));
                    let refusal = match live {
                        _ if principal != "owner" => Some(Refusal::Unauthorized),
                        None => Some(Refusal::Stale),
                        Some(_) if !farcooler_workflow::valid_rework_reason(reason) => Some(Refusal::Invalid),
                        Some(i) if i.phase != Phase::Uncertain || i.stopping.is_none() || i.pending.is_none()
                            || !i.run.starts_with("cancel:") || !i.members.is_empty()
                            || !matches!(i.definition.stages[i.stage].kind, StageKind::Provision | StageKind::Build
                                | StageKind::Check | StageKind::Run | StageKind::Review(_)) => Some(Refusal::Phase),
                        Some(_) if busy => Some(Refusal::Phase),
                        _ => None,
                    };
                    if let Some(refusal) = refusal {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(refusal))?
                    } else {
                        let live = live.unwrap();
                        let input = input(Command::Resolve { item:item.clone(),effect:live.pending.clone().unwrap(),
                            run:run.clone(),fence:live.fence,outcome:Outcome::Stopped {
                                operation:live.stopping.clone().unwrap(),
                                receipt:digest(&("owner-confirmed-cancellation/v1", principal, reason, &payload)),
                            } },Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Retry { item, run, pin, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        let input = input(Command::Retry { item:item.clone() },Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Queue { item, run, pin, .. } | Action::Leave { item, run, pin, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        let command = if matches!(action, Action::Queue { .. }) { Command::Queue { item:item.clone() } }
                            else { Command::Leave { item:item.clone() } };
                        let input = input(command,Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Settle { .. } => unreachable!("settling reads the remote, so it is answered before this"),
                Action::Track { item, column, .. } => {
                    let Some(definition) = Definition::ticket_at(column.as_deref().unwrap_or("backlog")) else {
                        return Ok(serde_json::json!({"schema":1,"receipt":self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Invalid))?}));
                    };
                    self.validate_publication(&definition, None)?;
                    let policy_pin = self.store.workflow_ticket_bundle(&definition)?;
                    let input = input(Command::Enqueue { item:item.clone(),initial_candidate:Some(definition.pin()),definition,
                        policy_pin,resource:Exec::item_resource(workspace, item),dependencies:BTreeSet::new() },Grant::Controller);
                    self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                }
                Action::Move { item, run, pin, to, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        let input = input(Command::Move { item:item.clone(),to:to.clone() },Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Start { item, run, pin, start_pin, models, worktree, with, .. } => {
                    let configuration = self.store.workflow_configuration(workspace)?.filter(|c| &c.pin == start_pin);
                    match configuration {
                        Some(configuration) if state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) => {
                            self.validate_publication(&configuration.definition, configuration.adapter.as_ref())?;
                            self.refuse_removed_decider(&configuration.definition)?;
                            if worktree.as_ref().map(|w| (&w.name, &w.branch)) != tree.as_ref().map(|t| (&t.name, &t.branch)) {
                                return Err(Error::Refused("a named worktree is prepared by the runner before Start"));
                            }
                            let initial_candidate = matches!(configuration.definition.stages[0].kind, StageKind::Tracker).then(|| start_pin.clone());
                            let input = input(Command::Start { item:item.clone(),definition:configuration.definition,
                                policy_pin:start_pin.clone(),resource:Exec::item_resource(workspace, item),dependencies:BTreeSet::new(),initial_candidate,models:models.clone(),tree:tree.clone().map(Box::new),with:with.clone() },Grant::Controller);
                            self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                        }
                        _ => self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?,
                    }
                }
                Action::Close { item, run, pin, resolution, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else {
                        // A landing proof is produced only by the asynchronous runtime path, never the wire.
                        let verified = close_verified;
                        let continues = matches!(resolution, farcooler_workflow::CloseResolution::Continued { item: other }
                            if other != item && state.items.get(other).is_some_and(|i| !matches!(i.phase, Phase::Done | Phase::Cancelled)));
                        if !verified && !continues {
                            self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Evidence))?
                        } else {
                            let input = input(Command::Close { item:item.clone(),resolution:resolution.clone() },Grant::Controller);
                            self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                        }
                    }
                }
                Action::SendBack { item, run, pin, reason, .. } => {
                    if !state.items.get(item).is_some_and(|i| &i.run == run && &i.pin == pin) {
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Stale))?
                    } else if matches!(state.items.get(item), Some(i) if i.phase == Phase::Uncertain && !self.build_has_ended(workspace, i)?) {
                        // An Uncertain build goes back only once its agent has ended (ov-1090).
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Phase))?
                    } else {
                        let input = input(Command::SendBack { item:item.clone(),reason:reason.clone() },Grant::Controller);
                        self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                    }
                }
                Action::Reopen { item, column, .. } => {
                    let Some(definition) = Definition::ticket_at(column.as_deref().unwrap_or("todo")) else {
                        return Ok(serde_json::json!({"schema":1,"receipt":self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Refuse(Refusal::Invalid))?}));
                    };
                    self.validate_publication(&definition, None)?;
                    let policy_pin = self.store.workflow_ticket_bundle(&definition)?;
                    let input = input(Command::Reopen { item:item.clone(),definition,policy_pin,resource:Exec::item_resource(workspace, item) },Grant::Controller);
                    self.store.workflow_wire_write_by(workspace,&id,&payload,actor,Write::Apply(&input))?
                }
                Action::Get | Action::Propose { .. } | Action::Drafts { .. } => unreachable!(),
            }
        };
        // A board tracks its cards once it has a published workflow, so the cards already on it come under the engine now (ov-962).
        if matches!(action, Action::Publish { .. }) && receipt.result.is_ok() {
            let found = self.store.untracked_tickets(None).unwrap_or_default();
            let mine: Vec<_> = found.into_iter().filter(|(w, _, _)| w.to_string() == workspace)
                .map(|(w, k, s)| (w.to_string(), k, crate::workflow_service::column_of(s).to_string())).collect();
            let _ = self.track_tickets(&mine);
        }
        if let Some(wake) = &self.notice_wake { wake.notify_one(); }
        Ok(serde_json::json!({"schema":1,"receipt":receipt}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    fn m(id:&str,revision:u64,sequence:u64) -> Mutation { Mutation { id:id.into(),revision,epoch:1,sequence } }
    fn ok(value:&serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(),"{value}"); }
    #[test]
    fn workflow_get_summary_hides_definitions_and_finished_groups_without_changing_tickets() {
        let input = serde_json::json!({"configuration":{"pin":"config", "definition":{"stages":[]}, "adapter":{}},
            "state":{"revision":17,"epoch":2,"source_sequences":{"wire:owner":9},"seen":["old"],
                "items":{"ticket":{"definition":{},"phase":"Done","run":"r","pin":"p"},
                    "active":{"definition":{},"group_of":{"name":"landing"},"phase":"Running"},
                    "finished":{"definition":{},"group_of":{"name":"landing"},"phase":"Failed"}},
                "archive":{"old":{"phase":"Done","group":{}}}},"notice_deliveries":[{"body":"large"}]});
        let summary = summarize(input.clone());
        assert!(summary.pointer("/state/items/finished").is_none());
        assert!(summary.pointer("/state/items/active").is_some());
        assert_eq!(summary["state"]["items"]["ticket"]["run"], "r");
        assert!(summary.pointer("/state/items/ticket/definition").is_none());
        assert!(summary.pointer("/configuration/definition").is_none());
        assert!(summary.pointer("/state/archive/old").is_none());
        assert_eq!(summary["state"]["revision"], input["state"]["revision"]);
    }
    // Opt-in measurement through the actual CLI and socket, with fifteen CPU-active build adapters.
    // Build target/debug/farcooler first. The scratch runner never reaches an installed daemon.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore]
    async fn workflow_get_cli_p95_with_fifteen_builds() {
        use super::super::{coding_tests::configured, concurrency_tests::enqueue_item};
        use farcooler_transport::{UnixListenerServer, HandshakeConfig, Peer};
        use farcooler_protocol::v1::Scope;
        use std::sync::Arc;
        let cli = std::env::current_dir().unwrap().join("../../target/debug/farcooler").canonicalize().unwrap();
        let (dir, mut runtime) = configured(false, "pass").await;
        let script = dir.path().join("adapter.py");
        let original = std::fs::read_to_string(&script).unwrap();
        let busy = "if stage == 'build':\n    import hashlib, time\n    started = pathlib.Path(os.environ['STARTED']).parent / 'builds'\n    started.mkdir(exist_ok=True)\n    (started / os.environ['FARCOOLER_WORKFLOW_RUN']).write_text(str(os.getpid()))\n    until = time.monotonic() + 110\n    while time.monotonic() < until: hashlib.sha256(b'x' * 1048576).digest()\n";
        std::fs::write(&script, original.replacen("if stage == 'build':\n", busy, 1)).unwrap();
        if let CodingHarness::Command(build) = &mut runtime.coding.as_mut().unwrap().harness { build.timeout_ms = 120_000; }
        runtime.exec.coding = runtime.coding.clone();
        runtime.set_effect_cap(15);
        let home = dir.path().join("home");
        let service = Arc::new(crate::service::Service::open_in(home.clone()).await.unwrap());
        let root = service.store.create_repository_root(service.host_id, dir.path().to_str().unwrap(), 0).unwrap();
        let repo = service.store.create_repository(service.host_id, root.id, "load", "/fixture/.git", "").unwrap();
        let workspace = service.store.ensure_main_workspace(repo.id).unwrap().id;
        let definition = runtime.configured_definition().unwrap();
        for n in 0..15 { enqueue_item(&mut runtime, &workspace.to_string(), &format!("load-{n}"), definition.clone()); }
        let handle = runtime.command_handle(&runtime.controller()).unwrap();
        let (stop, signal) = tokio::sync::watch::channel(false);
        let worker = tokio::spawn(async move { runtime.run(signal).await });
        let server = UnixListenerServer::bind(home.join("farcoolerd.sock")).unwrap();
        let serve = tokio::spawn(async move { server.serve(move |_| Some((HandshakeConfig { daemon_version:"load-fixture".into() },
            crate::rpc::RpcFactory::new(service.clone(), crate::watch::Watcher::new(service.clone()), Arc::new(tokio::sync::Notify::new()),
                Peer { client_id:None, scope:Scope::Control }).with_workflow_fixture(handle.clone())))).await });
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if std::fs::read_dir(dir.path().join("builds")).map(|d| d.count()).unwrap_or(0) == 15 { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.expect("fifteen real build processes started");
        let full = tokio::process::Command::new(&cli).env("FARCOOLER_HOME", &home)
            .args(["workflow", "get", &workspace.to_string(), "--full", "--json"]).output().await.unwrap();
        assert!(full.status.success(), "{}", String::from_utf8_lossy(&full.stderr));
        let full_value:serde_json::Value = serde_json::from_slice(&full.stdout).unwrap();
        assert!(full_value.pointer("/state/items/load-0/definition/stages").is_some());
        let full_bytes = full.stdout.len();
        let mut samples = Vec::new();
        let mut bytes = 0;
        for _ in 0..40 {
            let start = std::time::Instant::now();
            let output = tokio::process::Command::new(&cli).env("FARCOOLER_HOME", &home)
                .args(["workflow", "get", &workspace.to_string(), "--json"]).output().await.unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let value:serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let items = value["state"]["items"].as_object().unwrap();
            assert_eq!(items.len(), 15);
            assert!(items.values().all(|i| i["phase"] == "Running" && i["stage"] == 1), "all fifteen builds remain active");
            assert!(value.pointer("/state/items/load-0/definition").is_none());
            bytes = output.stdout.len();
            samples.push(start.elapsed().as_secs_f64());
        }
        stop.send_replace(true);
        worker.await.unwrap().unwrap();
        serve.abort(); let _ = serve.await;
        samples.sort_by(f64::total_cmp);
        println!("ov-1159 builds=15 samples=40 bytes={bytes} full_bytes={full_bytes} median={:.4}s p95={:.4}s max={:.4}s", samples[19], samples[37], samples[39]);
        assert!(samples[37] < 2.0, "CLI p95 must be under two seconds");
    }
    #[tokio::test]
    async fn workflow_get_summary_does_not_wait_for_the_effect_inbox() {
        use super::super::admission_tests::{factory, request, receipt};
        use farcooler_transport::{Handler, Peer};
        use farcooler_protocol::v1::Scope;
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let ws = workspace.to_string();
        runtime.wire_command(&ws, "owner", Action::Track {
            mutation: farcooler_protocol::workflow_commands::Mutation { id:"track".into(), revision:0, epoch:1, sequence:1 },
            item:"ov-1".into(), column:None,
        }).unwrap();
        let rpc = factory(&svc, &runtime, Peer { client_id:None, scope:Scope::Control }, None);
        // Leave the actor inbox deliberately unconsumed: a read cannot depend on any effect's progress.
        let mut req = request(workspace, &Action::Get);
        req.accepts.push("workflow_summary_v1".into());
        let reply = tokio::time::timeout(Duration::from_secs(2), rpc.handle(req)).await
            .expect("workflow get waited on the effect inbox");
        let summary = receipt(&reply);
        let full = runtime.wire_command(&ws, "owner", Action::Get).unwrap();
        for field in ["revision", "epoch", "source_sequences"] {
            assert_eq!(summary["state"][field], full["state"][field]);
        }
        for field in ["run", "pin", "phase", "stage", "members", "grouped_in", "candidate", "tree"] {
            let expected = if field == "members" { serde_json::json!([]) } else { full["state"]["items"]["ov-1"][field].clone() };
            assert!(summary["state"]["items"]["ov-1"].get(field).is_some(), "{field} stays present");
            assert_eq!(summary["state"]["items"]["ov-1"][field], expected);
        }
        assert!(summary.pointer("/state/items/ov-1/definition").is_none());
        let mut req = request(workspace, &Action::Get);
        req.accepts.push("workflow_full_v1".into());
        let reply = tokio::time::timeout(Duration::from_secs(2), rpc.handle(req)).await.unwrap();
        assert_eq!(receipt(&reply)["state"], full["state"]);
        // Shipped apps send no accepts marker and still need the original full schema.
        let reply = tokio::time::timeout(Duration::from_secs(2), rpc.handle(request(workspace, &Action::Get)))
            .await.expect("unmarked workflow get waited on the effect inbox");
        assert_eq!(receipt(&reply)["state"], full["state"]);
        assert!(receipt(&reply).pointer("/state/items/ov-1/definition").is_some());
    }
    #[tokio::test]
    async fn workflow_owner_can_resolve_a_lost_provision_cancellation_then_reopen() {
        lost_cancellation(false).await;
    }
    #[tokio::test]
    async fn workflow_owner_can_resolve_a_lost_review_cancellation_then_reopen() {
        lost_cancellation(true).await;
    }
    async fn lost_cancellation(review: bool) {
        let (_dir, mut runtime) = super::super::coding_tests::configured(review, "pass").await;
        let definition = runtime.configured_definition().unwrap();
        super::super::tests::enqueue(&mut runtime, "w", "one", definition);
        // The admitted Provision or review lost its settlement before a restart could read it.
        if review { for at in 2..6 { runtime.pump(at).await.unwrap(); } }
        let original = super::super::coding_tests::admitted(&runtime).await;
        runtime.pump(200).await.unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::Uncertain);
        let resolve = |state: &State, id: &str, reason: &str| Action::ResolveCancellation {
            mutation:m(id,state.revision,state.revision + 100),item:"one".into(),run:state.items["one"].run.clone(),
            pin:state.items["one"].pin.clone(),reason:reason.into(),
        };
        let why = "The step's settlement was lost. I checked the worktree and all owned processes have ended.";
        assert!(runtime.wire_command("w","owner",resolve(&state,"before-cancel",why)).unwrap()
            .pointer("/receipt/result/Err").is_some());
        ok(&runtime.wire_command("w","owner",Action::Cancel { mutation:m("cancel",state.revision,state.revision + 100),
            item:"one".into(),run:state.items["one"].run.clone(),pin:state.items["one"].pin.clone() }).unwrap());
        runtime.pump(201).await.unwrap();
        // Reconciliation is done by the service in production; reopen it with the same durable state.
        let (root, store) = (runtime.root.clone(), runtime.store.clone());
        drop(runtime);
        let runtime = Runtime::open_service(&root, store).unwrap();
        let state = runtime.state("w").unwrap().unwrap();
        assert_eq!(state.items["one"].phase, Phase::Uncertain);
        assert!(runtime.execution_state("w").unwrap().unwrap().items["one"].execution.as_ref().unwrap()
            .text.contains("cancellation receipt could not be verified"));
        let action = resolve(&state,"resolve",why);
        assert!(super::super::admission::person_only(&action));
        assert!(runtime.wire_command("w","orchestrator",resolve(&state,"agent",why)).unwrap()
            .pointer("/receipt/result/Err").is_some());
        let key = ("w".into(), original.effect.id.clone());
        runtime.activities.lock().unwrap().insert(key.clone(), Default::default());
        assert!(runtime.wire_command("w","owner",resolve(&state,"busy",why)).unwrap()
            .pointer("/receipt/result/Err").is_some());
        runtime.activities.lock().unwrap().remove(&key);
        assert!(runtime.wire_command("w","owner",resolve(&state,"blank"," ")).unwrap()
            .pointer("/receipt/result/Err").is_some());
        let mut stale = resolve(&state,"stale",why);
        if let Action::ResolveCancellation { run, .. } = &mut stale { *run = "old".into(); }
        assert!(runtime.wire_command("w","owner",stale).unwrap().pointer("/receipt/result/Err").is_some());
        let receipt = runtime.wire_command("w","owner",action.clone()).unwrap(); ok(&receipt);
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Cancelled);
        assert_eq!(runtime.wire_command("w","owner",action).unwrap(),receipt);
        let state = runtime.state("w").unwrap().unwrap();
        ok(&runtime.wire_command("w","owner",Action::Reopen { mutation:m("reopen",state.revision,state.revision + 100),
            item:"one".into(),column:None }).unwrap());
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::WaitingApproval);
    }

    #[tokio::test]
    async fn workflow_wire_pins_policy_and_replays_after_restart_without_reexecution() {
        let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("fixture");
        let mut runtime = Runtime::create(&root).await.unwrap();
        let mut definition = coding_definition(false);
        definition.stages[0].progression = Progression::AwaitApproval { principal:"owner".into() };
        let pin = definition.pin();
        let publication = Action::Publish { notifications:None, adapter:None, mutation:m("publish",0,1),expected_pin:None,definition };
        ok(&runtime.wire_command("ws","owner",publication.clone()).unwrap());
        let enqueue = Action::Enqueue { mutation:m("enqueue",0,1),item:"a".into(),pin:pin.clone() };
        let receipt = runtime.wire_command("ws","owner",enqueue.clone()).unwrap(); ok(&receipt);
        runtime.pump(10).await.unwrap();
        let state = runtime.state("ws").unwrap().unwrap();
        assert_eq!(state.items["a"].phase,Phase::WaitingApproval);
        let new_definition = coding_definition(true);
        ok(&runtime.wire_command("ws","owner",Action::Publish { notifications:None, adapter:None, mutation:m("publish2",state.revision,2),expected_pin:Some(pin.clone()),definition:new_definition }).unwrap());
        assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].pin,pin);
        assert_eq!(runtime.wire_command("ws","owner",enqueue.clone()).unwrap(),receipt);
        assert!(runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("enqueue",0,1),item:"different".into(),pin:pin.clone() }).is_err());
        drop(runtime);
        let runtime = Runtime::open(&root).unwrap();
        assert_eq!(runtime.wire_command("ws","owner",enqueue).unwrap(),receipt);
        ok(&runtime.wire_command("ws","owner",publication).unwrap());
        assert_eq!(runtime.state("ws").unwrap().unwrap().items.len(),1);
    }
    #[tokio::test]
    async fn workflow_wire_stale_epoch_pin_run_and_wrong_decider_refuse() {
        let dir = tempfile::tempdir().unwrap(); let mut runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
        let definition = Definition::tracker("owner"); let pin = definition.pin();
        ok(&runtime.wire_command("ws","owner",Action::Publish { notifications:None, adapter:None, mutation:m("p",0,1),expected_pin:None,definition }).unwrap());
        ok(&runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap());
        runtime.pump(10).await.unwrap();
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        let approve = |mutation,run,pin| Action::Approve { mutation,item:"a".into(),run,pin,candidate:item.candidate.clone() };
        let refused = runtime.wire_command("ws","orchestrator",approve(m("wrong-person",state.revision,1),item.run.clone(),pin.clone())).unwrap();
        assert!(refused.to_string().contains("Unauthorized"),"{refused}");
        let mut stale = m("epoch",state.revision,2); stale.epoch = 99;
        assert!(runtime.wire_command("ws","owner",approve(stale,item.run.clone(),pin.clone())).unwrap().to_string().contains("Stale"));
        for (id,run,pin) in [("run","old".into(),pin.clone()),("pin",item.run.clone(),"old".into())] {
            let refused = runtime.wire_command("ws","owner",approve(m(id,state.revision,2),run,pin)).unwrap();
            assert!(refused.pointer("/receipt/result/Err").is_some(),"{refused}");
        }
        ok(&runtime.wire_command("ws","owner",approve(m("approve",state.revision,2),item.run.clone(),pin.clone())).unwrap());
        runtime.pump(20).await.unwrap();
        assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].phase,Phase::Done);
    }
    #[tokio::test]
    async fn workflow_wire_tracks_moves_and_starts_a_ticket_through_the_engine() {
        let dir = tempfile::tempdir().unwrap(); let runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
        let track = Action::Track { mutation:m("track",0,1),item:"a".into(),column:None };
        let receipt = runtime.wire_command("ws","orchestrator",track.clone()).unwrap(); ok(&receipt);
        assert_eq!(runtime.wire_command("ws","orchestrator",track).unwrap(),receipt,"a retried track is the same receipt");
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        assert!(item.definition.is_ticket());
        let mv = |id,rev,run:&str,pin:&str,to:&str| Action::Move { mutation:m(id,rev,2),item:"a".into(),run:run.into(),pin:pin.into(),to:to.into() };
        let stale = runtime.wire_command("ws","orchestrator",mv("old",state.revision,"old",&item.pin,"doing")).unwrap();
        assert!(stale.to_string().contains("Stale"),"{stale}");
        ok(&runtime.wire_command("ws","orchestrator",mv("doing",state.revision,&item.run,&item.pin,"doing")).unwrap());
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        assert_eq!(item.definition.stages[item.stage].id,"doing");
        // Starting needs the published definition's pin; a pin that is not the published one is stale.
        let definition = coding_definition(false); let start_pin = definition.pin();
        ok(&runtime.wire_command("ws","owner",Action::Publish { notifications:None,adapter:None,mutation:m("publish",state.revision,3),expected_pin:None,definition }).unwrap());
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        let start = |id,pin:&str| Action::Start { mutation:m(id,state.revision,4),item:"a".into(),run:item.run.clone(),pin:item.pin.clone(),start_pin:pin.into(),models:Default::default(),worktree:None,with:vec![] };
        let stale = runtime.wire_command("ws","orchestrator",start("wrong","wf1:other")).unwrap();
        assert!(stale.to_string().contains("Stale"),"{stale}");
        ok(&runtime.wire_command("ws","orchestrator",start("start",&start_pin)).unwrap());
        let state = runtime.state("ws").unwrap().unwrap();
        assert_eq!(state.items["a"].policy_pin,start_pin);
        assert!(!state.items["a"].definition.is_ticket());
    }
    #[tokio::test]
    async fn workflow_wire_retry_needs_the_exact_run_and_a_failed_item() {
        let dir = tempfile::tempdir().unwrap(); let mut runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
        let definition = Definition::tracker("owner"); let pin = definition.pin();
        ok(&runtime.wire_command("ws","owner",Action::Publish { notifications:None, adapter:None, mutation:m("p",0,1),expected_pin:None,definition }).unwrap());
        ok(&runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap());
        runtime.pump(10).await.unwrap();
        let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
        let retry = |id,run:String| Action::Retry { mutation:m(id,state.revision,2),item:"a".into(),run,pin:pin.clone() };
        // An old run id is stale; the current run is refused because the item has not failed.
        let stale = runtime.wire_command("ws","owner",retry("old","old".into())).unwrap();
        assert!(stale.to_string().contains("Stale"),"{stale}");
        let waiting = runtime.wire_command("ws","owner",retry("now",item.run.clone())).unwrap();
        assert!(waiting.to_string().contains("Phase"),"{waiting}");
        assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].retries,0);
        // Queue and leave need a ticket at a join step or in a waiting group; this one is neither.
        for (id,action) in [("queue",Action::Queue { mutation:m("queue",state.revision,3),item:"a".into(),run:item.run.clone(),pin:pin.clone() }),
                            ("leave",Action::Leave { mutation:m("leave",state.revision,4),item:"a".into(),run:item.run.clone(),pin:pin.clone() })] {
            let refused = runtime.wire_command("ws","owner",action).unwrap();
            assert!(refused.to_string().contains("Phase"),"{id}: {refused}");
        }
    }
    #[tokio::test]
    async fn workflow_wire_auto_and_every_stage_approval_preserve_policy() {
        for every in [false,true] {
            let dir = tempfile::tempdir().unwrap(); let mut runtime = Runtime::create(&dir.path().join("fixture")).await.unwrap();
            let mut definition = coding_definition(false);
            if every { for stage in &mut definition.stages { stage.progression=Progression::AwaitApproval { principal:"orchestrator".into() }; } }
            let pin=definition.pin();
            ok(&runtime.wire_command("ws","owner",Action::Publish { notifications:None, adapter:None, mutation:m("p",0,1),expected_pin:None,definition }).unwrap());
            ok(&runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap());
            let mut approvals=0;
            for time in 1..40 {
                runtime.pump(time).await.unwrap();
                let state=runtime.state("ws").unwrap().unwrap(); let item=&state.items["a"];
                if item.phase==Phase::Done { break; }
                if item.phase==Phase::WaitingApproval {
                    assert!(every); approvals+=1;
                    ok(&runtime.wire_command("ws","orchestrator",Action::Approve { mutation:m(&format!("a{approvals}"),state.revision,approvals),item:"a".into(),run:item.run.clone(),pin:pin.clone(),candidate:item.candidate.clone() }).unwrap());
                }
            }
            assert_eq!(runtime.state("ws").unwrap().unwrap().items["a"].phase,Phase::Done);
            assert_eq!(approvals,if every {5} else {0});
        }
    }
}

#[cfg(test)]
mod semantic_review_tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    #[tokio::test]
    async fn workflow_wire_semantic_review_requires_exact_observation_and_principal() {
        for passed in [false,true] {
            let dir=tempfile::tempdir().unwrap();let mut runtime=Runtime::create(&dir.path().join("fixture")).await.unwrap();
            let mut definition=coding_definition(true);
            definition.stages.iter_mut().find(|s| matches!(s.kind,StageKind::Review(_))).unwrap().kind=StageKind::Review(Reviewer::Person("owner".into()));
            let pin=definition.pin();
            let m=|id:&str,revision,sequence| Mutation { id:id.into(),revision,epoch:1,sequence };
            runtime.wire_command("ws","owner",Action::Publish { notifications:None, adapter:None, mutation:m("p",0,1),expected_pin:None,definition }).unwrap();
            runtime.wire_command("ws","owner",Action::Enqueue { mutation:m("e",0,1),item:"a".into(),pin:pin.clone() }).unwrap();
            for time in 1..15 {
                runtime.pump(time).await.unwrap();
                if runtime.state("ws").unwrap().unwrap().items["a"].phase==Phase::WaitingReview {break;}
            }
            let state=runtime.state("ws").unwrap().unwrap();let item=&state.items["a"];
            assert_eq!(item.phase,Phase::WaitingReview);
            let evidence=item.last_evidence.clone().unwrap();
            let review=|id:&str,sequence,run,pin,evidence| Action::Review { mutation:m(id,state.revision,sequence),item:"a".into(),run,pin,evidence,passed,reason:None };
            let wrong=runtime.wire_command("ws","orchestrator",review("wrong",1,item.run.clone(),pin.clone(),evidence.clone())).unwrap();
            assert!(wrong.to_string().contains("Unauthorized"),"{wrong}");
            let mut old=evidence.clone();old.receipt="unseen".into();
            for (id,run,pin,evidence) in [("old-evidence",item.run.clone(),pin.clone(),old),("old-run","stale".into(),pin.clone(),evidence.clone()),("old-pin",item.run.clone(),"stale".into(),evidence.clone())] {
                assert!(runtime.wire_command("ws","owner",review(id,2,run,pin,evidence)).unwrap().to_string().contains("Stale"));
            }
            let accepted=runtime.wire_command("ws","owner",review("review",2,item.run.clone(),pin.clone(),evidence.clone())).unwrap();
            assert!(accepted.pointer("/receipt/result/Ok").is_some(),"{accepted}");
            assert_eq!(runtime.wire_command("ws","owner",review("review",2,item.run.clone(),pin,evidence)).unwrap(),accepted);
            let next=runtime.state("ws").unwrap().unwrap();
            assert_eq!(next.items["a"].iteration,if passed {0} else {1});
            assert_eq!(next.items["a"].stage,if passed {4} else {1});
        }
    }
}

#[cfg(test)]
mod reasoned_tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;
    #[tokio::test]
    async fn workflow_reasoned_wire_durable_replay_and_refusals() {
        let dir=tempfile::tempdir().unwrap();let root=dir.path().join("fixture");
        let mut runtime=Runtime::create(&root).await.unwrap();
        let mut definition=coding_definition(true);
        definition.stages.iter_mut().find(|s|matches!(s.kind,StageKind::Review(_))).unwrap().kind=StageKind::Review(Reviewer::Person("owner".into()));
        let pin=definition.pin();
        let m=|id:&str,revision,sequence|Mutation{id:id.into(),revision,epoch:1,sequence};
        runtime.wire_command("ws","owner",Action::Publish{notifications:None,adapter:None,mutation:m("p",0,1),expected_pin:None,definition}).unwrap();
        runtime.wire_command("ws","owner",Action::Enqueue{mutation:m("e",0,1),item:"a".into(),pin:pin.clone()}).unwrap();
        for t in 1..15 {runtime.pump(t).await.unwrap();if runtime.state("ws").unwrap().unwrap().items["a"].phase==Phase::WaitingReview {break;}}
        let state=runtime.state("ws").unwrap().unwrap();let item=&state.items["a"];
        assert_eq!(item.phase,Phase::WaitingReview);
        let reason="  请修复 <script>\n $(never execute)  ";
        let action=|id:&str,reason:&str|Action::Review{mutation:m(id,state.revision,2),item:"a".into(),run:item.run.clone(),pin:pin.clone(),evidence:item.last_evidence.clone().unwrap(),passed:false,reason:Some(reason.into())};
        for (id,principal,text,expected) in [("bad","owner","\0","Invalid"),("blank","owner"," \n","Invalid"),("wrong","orchestrator",reason,"Unauthorized")] {
            let request=action(id,text);let refused=runtime.wire_command("ws",principal,request.clone()).unwrap();
            assert!(refused.to_string().contains(expected),"{refused}");
            assert_eq!(runtime.wire_command("ws",principal,request).unwrap(),refused);
        }
        for field in ["revision","epoch","run","pin","evidence"] {
            let mut request=action(field,reason);
            if let Action::Review{mutation,run,pin,evidence,..}=&mut request {
                match field {"revision"=>mutation.revision+=1,"epoch"=>mutation.epoch+=1,"run"=>*run="unseen".into(),
                    "pin"=>*pin="unseen".into(),_=>evidence.receipt="unseen".into()}
            }
            assert!(runtime.wire_command("ws","owner",request).unwrap().to_string().contains("Stale"));
        }
        let request=action("accepted",reason);let receipt=runtime.wire_command("ws","owner",request.clone()).unwrap();
        assert!(receipt.pointer("/receipt/result/Ok").is_some(),"{receipt}");
        let next=runtime.state("ws").unwrap().unwrap();let events=runtime.events("ws").unwrap();
        assert_eq!(next.items["a"].rework_history[0].reason,reason);
        let mut replay=State::empty("ws".into(),1);
        for e in &events {replay=step(&replay,e).unwrap().state;}
        assert_eq!(replay,next);
        drop(runtime);let mut runtime=Runtime::open(&root).unwrap();
        assert_eq!(runtime.state("ws").unwrap().unwrap(),next);
        assert_eq!(runtime.wire_command("ws","owner",request).unwrap(),receipt);
        assert_eq!(runtime.events("ws").unwrap(),events);
        assert!(runtime.wire_command("ws","owner",action("accepted","changed reason")).is_err());
        for iteration in 1..=2 {
            for t in 20..40 {runtime.pump(t).await.unwrap();if runtime.state("ws").unwrap().unwrap().items["a"].phase==Phase::WaitingReview {break;}}
            let current=runtime.state("ws").unwrap().unwrap();let i=&current.items["a"];
            assert_eq!(i.phase,Phase::WaitingReview);assert_eq!(i.iteration,iteration);
            let request=Action::Review{mutation:m(&format!("rework-{iteration}"),current.revision,u64::from(iteration)+2),
                item:"a".into(),run:i.run.clone(),pin:i.pin.clone(),evidence:i.last_evidence.clone().unwrap(),passed:false,reason:Some(reason.into())};
            let reply=runtime.wire_command("ws","owner",request.clone()).unwrap();
            if iteration==1 {assert!(reply.pointer("/receipt/result/Ok").is_some(),"{reply}");}
            else {
                assert_eq!(reply.pointer("/receipt/result/Err").and_then(|v|v.as_str()),Some("Limit"));
                assert_eq!(runtime.state("ws").unwrap().unwrap(),current);
                drop(runtime);runtime=Runtime::open(&root).unwrap();
                assert_eq!(runtime.wire_command("ws","owner",request).unwrap(),reply);
                assert_eq!(runtime.state("ws").unwrap().unwrap(),current);
            }
        }
    }
}

#[cfg(test)]
mod cli_payload_tests {
    use super::*;
    use farcooler_protocol::workflow_commands::Mutation;

    #[tokio::test]
    #[ignore = "requires private ov-941 production CLI payload fixture"]
    async fn workflow_cli_payload_receipt_boundary() {
        let root=std::path::PathBuf::from(std::env::var_os("FARCOOLER_OV941_FIXTURE").expect("private fixture root"));
        let ws="00000000-0000-0000-0000-000000000941";
        if std::env::var("FARCOOLER_OV941_PHASE").as_deref()==Ok("seed") {
            let mut runtime=Runtime::create(&root.join("runtime")).await.unwrap();
            let mut definition=coding_definition(true);
            definition.stages.iter_mut().find(|s|matches!(s.kind,StageKind::Review(_))).unwrap().kind=StageKind::Review(Reviewer::Person("owner".into()));
            let pin=definition.pin();
            let m=|id:&str| Mutation{id:id.into(),revision:0,epoch:1,sequence:1};
            runtime.wire_command(ws,"owner",Action::Publish{notifications:None,adapter:None,mutation:m("p"),expected_pin:None,definition}).unwrap();
            runtime.wire_command(ws,"owner",Action::Enqueue{mutation:m("e"),item:"a".into(),pin}).unwrap();
            for t in 1..15 {runtime.pump(t).await.unwrap();if runtime.state(ws).unwrap().unwrap().items["a"].phase==Phase::WaitingReview {break;}}
            let state=runtime.state(ws).unwrap().unwrap();let item=&state.items["a"];
            assert_eq!(item.phase,Phase::WaitingReview);
            std::fs::write(root.join("observed.json"),serde_json::to_vec(&serde_json::json!({
                "workspace":ws,"revision":state.revision,"epoch":state.epoch,"run":item.run,"pin":item.pin})).unwrap()).unwrap();
            std::fs::write(root.join("evidence.json"),serde_json::to_vec(item.last_evidence.as_ref().unwrap()).unwrap()).unwrap();
            return;
        }
        assert_eq!(std::env::var("FARCOOLER_OV941_PHASE").as_deref(),Ok("consume"));
        let bytes=std::fs::read(root.join("cli-payload.json")).unwrap();
        let resubmitted=std::fs::read(root.join("cli-resubmitted.json")).unwrap();
        assert_eq!(bytes,resubmitted);
        let parse=|bytes:&[u8]|farcooler_protocol::workflow_commands::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
        let runtime=Runtime::open(&root.join("runtime")).unwrap();
        let before=runtime.state(ws).unwrap().unwrap();let before_events=runtime.events(ws).unwrap();
        // Discard the answer at the caller: retain it here solely as an assertion oracle.
        let lost_answer=runtime.wire_command(ws,"owner",parse(&bytes)).unwrap();
        let receipt:farcooler_store::workflow::Receipt=serde_json::from_value(lost_answer["receipt"].clone()).unwrap();
        assert!(receipt.result.is_ok(),"{receipt:?}");
        let after=runtime.state(ws).unwrap().unwrap();let events=runtime.events(ws).unwrap();
        assert_eq!(events.len(),before_events.len()+1);
        assert_eq!(after.items["a"].iteration,before.items["a"].iteration+1);
        assert_eq!(after.items["a"].rework_history.len(),1);
        assert_eq!(after.items["a"].rework_history[0].reason,"  请修复 <script>\n $(never execute)  ");
        assert_eq!(runtime.wire_command(ws,"owner",parse(&resubmitted)).unwrap(),lost_answer);
        drop(runtime);
        let runtime=Runtime::open(&root.join("runtime")).unwrap();
        assert_eq!(runtime.wire_command(ws,"owner",parse(&resubmitted)).unwrap(),lost_answer);
        for field in ["reason","evidence","run","pin","revision","epoch","sequence"] {
            let mut altered=parse(&bytes);
            let Action::Review{mutation,reason,evidence,run,pin,..}=&mut altered else {panic!("review")};
            match field {
                "reason"=>*reason=Some("changed".into()),"evidence"=>evidence.receipt="changed".into(),
                "run"=>*run="changed".into(),"pin"=>*pin="changed".into(),
                "revision"=>mutation.revision+=1,"epoch"=>mutation.epoch+=1,_=>mutation.sequence+=1,
            }
            assert!(matches!(runtime.wire_command(ws,"owner",altered),Err(Error::Store(WorkflowError::IdentityConflict))),"same identity changed {field}");
            assert_eq!(runtime.state(ws).unwrap().unwrap(),after);
            assert_eq!(runtime.events(ws).unwrap(),events);
        }
        assert_eq!(runtime.events(ws).unwrap(),events);
        assert_eq!(runtime.state(ws).unwrap().unwrap(),after);
        std::fs::write(root.join("receipt.json"),serde_json::to_vec(&lost_answer).unwrap()).unwrap();
    }
}
