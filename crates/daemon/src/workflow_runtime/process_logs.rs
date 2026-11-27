//! Process evidence is local to the engine, alongside the receipts that retain it (ov-1127).
use super::*;

const TAIL_BYTES: usize = 8 * 1024;
const TAIL_LINES: usize = 40;
// Count JSON escaping too: control bytes can grow sixfold on the wire. Leave room for
// state and log paths in the 1 MiB control envelope, while retaining the newest evidence.
const REPLY_TAIL_BYTES: usize = 256 * 1024;
pub(super) const LOG_PREVIEW_BYTES: usize = 288 * 1024;

impl Exec {
    pub(super) fn effect_log(&self, effect: &Effect) -> Option<PathBuf> {
        let receipts = self.root.join("receipts");
        [receipts.join(format!("{}.output", digest(effect))), receipts.join(format!("{}.log", effect.id))]
            .into_iter().find(|path| path.is_file())
    }

    /// Read only engine-derived paths, never the path text in a receipt. Old receipts did not name
    /// their log, but used the same names, so their failures can be inspected too.
    ///
    /// `workflow.get` carries tails for live items only, and for each live item only its last effect
    /// per stage: the reply is read on every plan load, and the outbox keeps every finished run (ov-1298).
    /// An archived or removed item's tails, and a live item's older ones, are `workflow.get`'s `Logs`.
    pub(super) fn failed_process_logs(&self, workspace: &str, state: Option<&State>) -> Result<Vec<serde_json::Value>> {
        let Some(live) = state.map(|s| &s.items) else { return Ok(Vec::new()) };
        let outbox = self.store.workflow_item_effects(workspace, live.keys().map(String::as_str))?;
        // The outbox is ordered by revision, then id, so a later effect of the same stage replaces an earlier one.
        let mut last = std::collections::BTreeMap::new();
        for (n, stored) in outbox.iter().enumerate() {
            last.insert((stored.effect.item.as_str(), stage_key(&stored.effect.kind)), n);
        }
        let mut chosen: Vec<_> = last.into_values().collect();
        chosen.sort_unstable();
        let mut logs = Vec::new();
        for n in chosen {
            if let Some(log) = self.process_log_entry(state, &outbox[n].effect, false)? { logs.push(log); }
        }
        budget_tails(&mut logs)?;
        Ok(logs)
    }

    /// Recent retained logs of one item, across runs, including archived or removed items.
    /// Use the paged logs history for the rest.
    pub(super) fn item_process_logs(&self, workspace: &str, item: &str) -> Result<serde_json::Value> {
        let state = self.state(workspace)?;
        let mut logs = self.retained_process_logs(workspace, state.as_ref(), Some(item))?;
        budget_tails(&mut logs)?;
        let live = state.as_ref().is_some_and(|s| s.items.contains_key(item));
        let archived = state.as_ref().is_some_and(|s| s.archive.contains_key(item));
        let mut reply = serde_json::json!({"schema":1, "item":item, "live":live, "archived":archived});
        super::wire_history::recent(reply.as_object_mut().expect("a JSON object"),
            "process_logs", logs, super::wire_history::ListOrder::OldestFirst, LOG_PREVIEW_BYTES)?;
        Ok(reply)
    }

    pub(super) fn retained_process_logs(&self, workspace: &str, state: Option<&State>, item: Option<&str>)
        -> Result<Vec<serde_json::Value>>
    {
        let mut logs = Vec::new();
        let effects = match item {
            Some(item) => self.store.workflow_item_effects(workspace, [item])?,
            None => self.store.workflow_outbox(workspace)?,
        };
        for stored in effects {
            if let Some(log) = self.process_log_entry(state, &stored.effect, true)? {
                logs.push(log);
            }
        }
        Ok(logs)
    }

    /// One effect's log entry: `None` without a log, or, unless `all`, when the effect did not fail.
    fn process_log_entry(&self, state: Option<&State>, effect: &Effect, all: bool) -> Result<Option<serde_json::Value>> {
        let Some(path) = self.effect_log(effect) else { return Ok(None) };
        let journal = self.bytes(&self.journal_path(effect)).ok()
            .and_then(|raw| serde_json::from_slice::<Journal>(&raw).ok());
        let fact_path = self.root.join("receipts").join(format!("{}.process.json", digest(effect)));
        let fact = self.bytes(&fact_path).ok()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok());
        let exit = fact.as_ref().and_then(|f| f["exit"].as_i64());
        let failed_run = state.and_then(|s| s.items.get(&effect.item)).is_some_and(|i|
            i.run == effect.run && matches!(i.phase, Phase::Failed | Phase::Uncertain));
        let failed = failed_run || exit.is_some_and(|code| code != 0) || journal.is_some_and(|j|
            matches!(j.outcome, Outcome::ReviewRejected { .. } | Outcome::Rejected(_) | Outcome::SafeFailure | Outcome::SafeFailureBecause(_)
                | Outcome::Interrupted | Outcome::Uncertain));
        if !failed && !all { return Ok(None); }
        // Old daemons left unbounded logs. Inspect their tail without reading the entire file.
        use std::io::{Read, Seek, SeekFrom};
        let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 { return Ok(None); }
        let offset = metadata.len().saturating_sub(TAIL_BYTES as u64);
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(TAIL_BYTES as u64).read_to_end(&mut bytes)?;
        // Seeking may split a UTF-8 character. Skip its continuation bytes rather than
        // manufacture a replacement character for an otherwise valid log.
        let mut first = 0;
        if offset > 0 {
            while first < bytes.len() && bytes[first] & 0xc0 == 0x80 { first += 1; }
        }
        let text = String::from_utf8_lossy(&bytes[first..]);
        let mut lines = text.lines().rev();
        let mut tail = lines.by_ref().take(TAIL_LINES).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        let mut truncated = offset > 0 || lines.next().is_some();
        // Lossy decoding can expand invalid input, so enforce the final byte bound too.
        let mut cut = tail.len().saturating_sub(TAIL_BYTES);
        while !tail.is_char_boundary(cut) { cut += 1; }
        if cut > 0 { tail = tail[cut..].to_owned(); truncated = true; }
        let mut log = serde_json::json!({"item":effect.item, "run":effect.run, "effect":effect.id,
            "stage":effect.kind, "log_path":path, "exit":exit, "tail":tail, "tail_truncated":truncated});
        if all { log["failed"] = failed.into(); }
        Ok(Some(log))
    }

    /// Logs have exactly the retention of their process/journal receipts. Never age out evidence
    /// independently or delete an admitted process's output before it has written its receipt.
    pub(super) fn prune_process_logs(&self) -> Result<()> {
        let receipts = self.root.join("receipts");
        let mut candidates = Vec::new();
        // Pick candidates before reading the outbox. A process admitted after that
        // read can create a new sidecar while this background pass is running.
        for entry in std::fs::read_dir(&receipts)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".output").or_else(|| name.strip_suffix(".log")) else { continue };
            // Other adapters have their own *.fetch.output etc. This owns only process sidecars.
            if !key.starts_with("wf1:") || key.len() != 68 || !key[4..].bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
            let fixture = name.ends_with(".log");
            if receipts.join(format!("{key}.process.json")).try_exists()?
                || !fixture && receipts.join(format!("{key}.json")).try_exists()? { continue; }
            if entry.file_type()?.is_file() { candidates.push((entry.path(), key.to_owned(), fixture)); }
        }
        if candidates.is_empty() { return Ok(()); }

        let mut active = BTreeSet::new();
        let mut fixture_receipts = std::collections::BTreeMap::new();
        let has_fixtures = candidates.iter().any(|(_, _, fixture)| *fixture);
        for workspace in self.store.workflow_workspaces()? {
            for stored in self.store.workflow_outbox(&workspace)? {
                if has_fixtures {
                    fixture_receipts.insert(stored.effect.id.clone(), digest(&stored.effect));
                }
                if stored.active || matches!(stored.dispatch_state.as_str(), "claimed" | "admitted" | "uncertain") {
                    active.insert(digest(&stored.effect));
                    active.insert(stored.effect.id);
                }
            }
        }
        for (path, key, fixture) in candidates {
            let journal_key = if fixture { fixture_receipts.get(&key).map_or(key.as_str(), String::as_str) } else { &key };
            // An effect could have written its receipt and settled since candidate selection.
            if active.contains(&key) || receipts.join(format!("{key}.process.json")).try_exists()?
                || receipts.join(format!("{journal_key}.json")).try_exists()? { continue; }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

/// Which stage an effect is, for keeping only the last of each: a decision's input differs every time.
fn stage_key(kind: &EffectKind) -> String {
    match kind {
        EffectKind::Decide { policy, .. } => format!("decide:{}", serde_json::to_string(policy).unwrap_or_default()),
        EffectKind::Execute(stage) => format!("execute:{stage:?}"),
        EffectKind::Stop { operation } => format!("stop:{operation}"),
    }
}

/// The outbox is ordered by revision, then id: omit oldest tails first, keeping every path
/// available for inspecting the complete retained process output.
fn budget_tails(logs: &mut [serde_json::Value]) -> Result<()> {
    let mut budget = 0;
    for log in logs.iter() { budget += serde_json::to_vec(&log["tail"])?.len(); }
    for log in logs.iter_mut() {
        if budget <= REPLY_TAIL_BYTES { break; }
        budget -= serde_json::to_vec(&log["tail"])?.len() - 2; // empty JSON string still costs two bytes
        log["tail"] = "".into();
        log["tail_truncated"] = true.into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{enqueue, finish};

    async fn failed_logs(count: usize, output: &str) -> (tempfile::TempDir, Runtime) {
        failed_logs_in("w", count, output).await
    }

    async fn failed_logs_in(workspace: &str, count: usize, output: &str) -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let controller = runtime.controller();
        for n in 0..count {
            let receipt = runtime.submit_now(&controller, Request {
                id: format!("enqueue-{n}"), workspace: workspace.into(), expected_revision: n as u64,
                sequence: n as u64 + 1,
                command: Command::Enqueue { item: format!("item-{n:03}"), definition: coding_definition(false),
                    policy_pin: "policy/v1".into(), resource: Exec::item_resource(workspace, &format!("item-{n:03}")),
                    dependencies: BTreeSet::new(), initial_candidate: None },
            }, 1).unwrap();
            assert!(receipt.result.is_ok(), "{:?}", receipt.result);
        }
        for stored in runtime.store.workflow_outbox(workspace).unwrap() {
            let key = digest(&stored.effect);
            std::fs::write(runtime.root.join("receipts").join(format!("{key}.output")), output).unwrap();
            std::fs::write(runtime.root.join("receipts").join(format!("{key}.process.json")), br#"{"exit":1}"#).unwrap();
        }
        (dir, runtime)
    }

    /// Move the first `count` items to the archive, as the reducer does once a finished item has been seen.
    fn archive_first(runtime: &Runtime, workspace: &str, count: usize) {
        let mut state = runtime.state(workspace).unwrap().unwrap();
        for n in 0..count {
            let id = format!("item-{n:03}");
            let item = state.items.remove(&id).unwrap();
            state.archive.insert(id, farcooler_workflow::Archived { start_load: item.start_load, last_run: None, history: Vec::new(), closed: None, phase: Phase::Cancelled, candidate: item.candidate, group: None });
        }
        runtime.store.workflow_state_for_tests(&state).unwrap();
        assert_eq!(runtime.state(workspace).unwrap().unwrap().archive.len(), count);
    }

    fn frame_len(reply: &serde_json::Value) -> usize {
        use farcooler_protocol::v1 as pb;
        response_frame_len(pb::Response {
            request_id: farcooler_protocol::ids::new_id(),
            outcome: Some(pb::response::Outcome::Result(pb::Result {
                value: Some(pb::result::Value::WorkflowReply(pb::WorkflowReply { schema: 1, json: reply.to_string() })),
            })),
        })
    }

    fn response_frame_len(response: farcooler_protocol::v1::Response) -> usize {
        use farcooler_protocol::v1 as pb;
        let envelope = pb::WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(pb::wire_envelope::Body::Response(response)),
        };
        farcooler_protocol::framing::encode(&envelope).expect("the reply must fit the protocol cap").len()
    }

    #[tokio::test]
    async fn workflow_get_with_a_long_history_carries_live_tails_only() {
        // Nov 16: 351 entries, mostly finished runs, put the plan's reply past the 1 MiB frame cap (ov-1298).
        let output = (0..400).map(|n| format!("failure evidence line {n} {}\n", "x".repeat(200))).collect::<String>();
        let (_dir, runtime) = failed_logs(420, &output).await;
        archive_first(&runtime, "w", 400);
        let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        let bytes = frame_len(&reply);
        eprintln!("workflow get with 400 archived and 20 live runs: {bytes} wire bytes");
        assert!(bytes < farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES / 2, "{bytes} bytes is not under half the frame cap");
        let logs = reply["process_logs"].as_array().unwrap();
        let items: BTreeSet<_> = logs.iter().map(|log| log["item"].as_str().unwrap().to_owned()).collect();
        assert_eq!(items, (400..420).map(|n| format!("item-{n:03}")).collect::<BTreeSet<_>>(), "live items only");
        for log in logs {
            let tail = log["tail"].as_str().unwrap();
            assert!(tail.lines().last().unwrap().starts_with("failure evidence line 399 "), "the newest line stays");
            assert!(tail.len() > 7 * 1024, "a live tail is whole, {} bytes", tail.len());
        }
    }

    #[tokio::test]
    async fn engine_reads_a_large_board_with_the_shipped_request() {
        use super::super::admission_tests::{factory, held, receipt, request};
        use farcooler_protocol::{v1::{Plan, Scope}, workflow_commands::{Action, HistorySection, Mutation}};
        use farcooler_transport::{Handler, Peer};

        let (_service_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let workspace_key = workspace.to_string();
        let output = "x".repeat(TAIL_BYTES);
        let (_runtime_dir, mut runtime) = failed_logs_in(&workspace_key, 88, &output).await;
        let mut state = runtime.state(&workspace_key).unwrap().unwrap();
        let tracker = Definition::ticket_at("todo").unwrap();
        let mut ticket = state.items.values().next().unwrap().clone();
        ticket.definition = tracker.clone();
        ticket.pin = tracker.pin();
        ticket.phase = Phase::WaitingApproval;
        ticket.pending = None;
        let mut group = ticket.clone();
        let definition = Definition::coding();
        definition.validate_group().unwrap();
        group.pin = definition.pin();
        group.definition = definition;
        group.phase = Phase::Failed;
        group.group_of = Some(GroupOf { name: "landing".into(), pin: group.pin.clone() });
        group.brief = Some("a failed landing's brief ".repeat(160));
        group.handed = vec!["item-000".into()];
        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
        let last_run = RunRecord { phase: Phase::Done, run: "earlier-run".into(), stop_run: None,
            pin: "earlier-pin".into(), fence: 1, candidate: Some("a".repeat(40)), tree: None,
            models: Default::default(), reason: Some("Prior result. ".repeat(30)), rework_history: Vec::new(),
            sent_back: Vec::new(), back_edges: Default::default() };

        for (groups, tickets) in [(114, 58), (350, 200)] {
            for n in 0..groups {
                group.since_ms = 2 * n;
                let id = format!("group-{n}-failed");
                group.run = digest(&(&workspace_key, &id));
                group.resource = Exec::item_resource(&workspace_key, &id);
                state.items.insert(id, group.clone());
                state.archive.insert(format!("group-{n}-landed"), Archived {
                    phase: Phase::Done,
                    group: Some(farcooler_workflow::ArchivedGroup {
                        name: "landing".into(), tickets: vec!["item-000".into()],
                        pushed: last_run.candidate.clone(), since_ms: 2 * n + 1,
                    }),
                    candidate: last_run.candidate.clone(), start_load: None, closed: None,
                    last_run: Some(last_run.clone()), history: Vec::new(),
                });
                state.archive.insert(format!("closed-{n:04}"), Archived {
                    phase: if n == 0 { Phase::Cancelled } else { Phase::Done },
                    group: None, candidate: None, start_load: None, closed: None,
                    last_run: Some(last_run.clone()), history: Vec::new(),
                });
            }
            for n in 0..tickets {
                state.items.insert(format!("ticket-{n:03}"), ticket.clone());
            }
            assert!(serde_json::to_vec(&state).unwrap().len() + 88 * output.len()
                > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES,
                "the unbounded board and process log tails must exceed the control frame");
            runtime.store.workflow_state_for_tests(&state).unwrap();
            let legacy = farcooler_client::plan_json::workflow_request(workspace);
            for flag in [None, Some("workflow_full_v1"), Some("workflow_summary_v1")] {
                let mut read = legacy.clone();
                read.accepts.extend(flag.map(String::from));
                let response = rpc.handle(read).await;
                let raw = receipt(&response);
                let bytes = response_frame_len(response);
                eprintln!("{groups} finished and {groups} failed groups, {tickets} tickets at rest, {flag:?}: {bytes} bytes");
                assert_eq!(raw["landing_groups_omitted"], 2 * groups - 12);
                assert_eq!(raw["state"]["revision"], state.revision);
                assert_eq!(raw["state"]["epoch"], state.epoch);
                assert_eq!(raw["state"]["source_sequences"], serde_json::json!(state.source_sequences));
                assert_eq!(raw["state"]["items"]["ticket-057"]["pin"], ticket.pin);
                assert_eq!(raw["state"]["items"]["ticket-057"]["run"], ticket.run);
                assert_eq!(raw["state"]["items"]["ticket-057"]["phase"], "WaitingApproval");
                assert!(raw["process_logs_omitted"].as_u64().unwrap() > 0);
                assert!(raw["archive_omitted"].as_u64().unwrap() > 0);
                assert!(raw["state"]["archive"].get("closed-0000").is_none());
                assert!(raw["state"]["archive"].get("group-0-landed").is_none());
                if flag.is_some() {
                    let newest = format!("group-{}-failed", groups - 1);
                    let row = &raw["state"]["items"][&newest];
                    assert_eq!(row["run"], state.items[&newest].run);
                    assert_eq!(row["pin"], state.items[&newest].pin);
                    assert_eq!(row["phase"], "Failed");
                    assert!(raw["state"]["items"].get("group-0-failed").is_none());
                }
                // An app from before paging reads one reply: unmarked, or marked
                // `workflow_full_v1` as the last shipped Engine view sends it.
                if flag != Some("workflow_summary_v1") {
                    assert!(raw["items_compact"].is_null(), "an older Engine view needs whole rows");
                    assert!(raw["items_omitted"].is_null(), "no live ticket may be left out");
                    let view = farcooler_client::engine_view::from_workflow_reply(
                        workspace, &raw, None, farcooler_client::workflow_presentation::Style::Apple).unwrap();
                    let view = serde_json::to_value(view).unwrap();
                    assert_eq!(view["landings"].as_array().unwrap().len(), 12);
                    let landed = format!("group-{}-landed", groups - 1);
                    let failed = format!("group-{}-failed", groups - 1);
                    assert_eq!(view["landings"][0]["id"], landed);
                    assert_eq!(view["landings"][0]["members"][0]["id"], "item-000");
                    assert_eq!(view["landings"][1]["id"], failed);
                    let plan = farcooler_client::plan_json::with_workflow_state(&Plan::default(), workspace, Some(async { Ok(raw.clone()) })).await.unwrap();
                    // These are the shared Plan models consumed by iPhone, Android and web.
                    for style in ["apple", "material"] {
                        let input = serde_json::json!({"schema": 1, "plan": plan, "context": {"style": style}});
                        let rendered: serde_json::Value = serde_json::from_str(
                            &farcooler_client::plan_view::json(&input.to_string())).unwrap();
                        assert!(rendered["error"].is_null(), "{rendered}");
                        let landings = rendered["view"]["landingGroups"].as_array().unwrap();
                        assert_eq!(landings.len(), 6);
                        assert!(landings.iter().any(|row| row["id"] == failed), "{landings:?}");
                    }
                }
            }

            let mut pages = farcooler_client::plan_json::WorkflowPlanPages::new(workspace);
            let plan = loop {
                let response = rpc.handle(pages.request()).await;
                response_frame_len(response.clone());
                let Some(farcooler_protocol::v1::response::Outcome::Result(reply)) = response.outcome else {
                    panic!("Expected a Plan workflow reply");
                };
                if let Some(raw) = pages.push(reply.value.unwrap()).unwrap() {
                    break farcooler_client::plan_json::with_workflow_state(
                        &Plan::default(), workspace, Some(async { Ok(raw) }),
                    ).await.unwrap();
                }
            };
            let input = serde_json::json!({"schema": 1, "plan": plan, "context": {"style": "material"}});
            let rendered: serde_json::Value = serde_json::from_str(
                &farcooler_client::plan_view::json(&input.to_string()),
            ).unwrap();
            assert!(rendered["error"].is_null(), "{rendered}");
            let landings = rendered["view"]["landingGroups"].as_array().unwrap();
            assert!(landings.len() <= 12);
            let newest = format!("group-{}-failed", groups - 1);
            assert!(landings.iter().any(|row| row["id"] == newest));
        }

        let archived = receipt(&rpc.handle(request(workspace, &Action::ArchiveItem { item: "CLOSED-0000".into() })).await);
        assert_eq!(archived["item"]["key"], "closed-0000");
        assert_eq!(archived["item"]["run"], last_run.run);
        assert_eq!(archived["item"]["pin"], last_run.pin);
        assert_eq!(archived["item"]["phase"], "Cancelled");
        let page = receipt(&rpc.handle(request(workspace, &Action::History {
            section: HistorySection::Archive, offset: 0, item: Some("group-0-landed".into()),
        })).await);
        let retained: serde_json::Value = serde_json::from_str(page["chunk"].as_str().unwrap()).unwrap();
        assert_eq!(retained["group-0-landed"]["group"]["tickets"], serde_json::json!(["item-000"]));
        assert_eq!(retained["group-0-landed"]["phase"], "Done");

        let mut offset = 0;
        let mut encoded = String::new();
        let mut digest = None;
        loop {
            let read = request(workspace, &Action::History { section: HistorySection::Groups, offset, item: None });
            let page = receipt(&rpc.handle(read).await);
            frame_len(&page);
            assert_eq!(page["offset"], offset);
            let first = digest.get_or_insert_with(|| page["digest"].clone());
            assert_eq!(*first, page["digest"]);
            encoded.push_str(page["chunk"].as_str().unwrap());
            let Some(next) = page["next_offset"].as_u64() else { break };
            assert!(next > offset);
            offset = next;
        }
        assert!(offset > 0, "the retained groups must span multiple frames");
        let groups: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(groups.as_object().unwrap().len(), 350);
        assert_eq!(groups["group-0-failed"]["brief"], group.brief.unwrap());

        let page = receipt(&rpc.handle(request(workspace, &Action::History {
            section: HistorySection::Logs, offset: 0, item: Some("item-000".into()),
        })).await);
        assert!(page["next_offset"].is_null());
        let logs: serde_json::Value = serde_json::from_str(page["chunk"].as_str().unwrap()).unwrap();
        assert_eq!(logs[0]["tail"], output);

        // Bind a cancellation from the summary and a targeted read of an older group.
        let mut read = request(workspace, &Action::Get);
        read.accepts.push("workflow_summary_v1".into());
        let snapshot = receipt(&rpc.handle(read).await);
        let state = &snapshot["state"];
        let item = "group-0-failed";
        assert!(state["items"].get(item).is_none());
        let page = receipt(&rpc.handle(request(workspace, &Action::History {
            section: HistorySection::Groups, offset: 0, item: Some(item.into()),
        })).await);
        assert!(page["next_offset"].is_null());
        let targeted: serde_json::Value = serde_json::from_str(page["chunk"].as_str().unwrap()).unwrap();
        assert_eq!(targeted.as_object().unwrap().len(), 1);
        assert_eq!(targeted[item], groups[item]);
        let row = &targeted[item];
        let cancel = Action::Cancel {
            mutation: Mutation {
                id: "cancel-old-group".into(),
                revision: state["revision"].as_u64().unwrap(),
                epoch: state["epoch"].as_u64().unwrap(),
                sequence: state["source_sequences"].as_object().unwrap()
                    .values().filter_map(serde_json::Value::as_u64).max().unwrap_or(0) + 1,
            },
            item: item.into(),
            run: row["run"].as_str().unwrap().into(),
            pin: row["pin"].as_str().unwrap().into(),
        };
        let reply = receipt(&held(&mut runtime, &rpc, workspace, cancel, false, || {}).await);
        assert!(reply["receipt"]["result"].get("Ok").is_some(), "{reply}");
        let next = runtime.state(&workspace_key).unwrap().unwrap();
        let phase = next.items.get(item).map(|row| &row.phase).or_else(|| next.archive.get(item).map(|row| &row.phase));
        assert_eq!(phase, Some(&Phase::Cancelled));
    }

    #[tokio::test]
    async fn failed_group_identities_cannot_overflow_workflow_get() {
        use super::super::admission_tests::{factory, receipt};
        use farcooler_protocol::v1::Scope;
        use farcooler_transport::{Handler, Peer};

        let (_service_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let key = workspace.to_string();
        let (_runtime_dir, runtime) = failed_logs_in(&key, 1, "").await;
        let mut state = runtime.state(&key).unwrap().unwrap();
        let live = state.items.get_mut("item-000").unwrap();
        live.phase = Phase::Failed;
        live.pending = None;
        let mut group = live.clone();
        group.definition = Definition::coding();
        group.pin = group.definition.pin();
        group.group_of = Some(GroupOf { name: "landing".into(), pin: group.pin.clone() });
        // The detached groups' guards and the sequence map together used to
        // overflow the frame even after compacting all item rows.
        state.source_sequences.extend((0..7_500).map(|n| (format!("wire:{}-{n:04}", "x".repeat(102)), 1)));
        let mut identities = serde_json::Map::new();
        let count = farcooler_workflow::MAX_ITEMS - state.items.len();
        for n in 0..count {
            let id = format!("group-{n:020}-{}", "g".repeat(100));
            group.since_ms = n as i64;
            group.run = digest(&(&key, &id));
            group.resource = Exec::item_resource(&key, &id);
            identities.insert(id.clone(), serde_json::json!({"run":group.run, "pin":group.pin, "phase":group.phase}));
            state.items.insert(id, group.clone());
        }
        let unbounded_guards = serde_json::json!({"source_sequences":state.source_sequences, "items":identities});
        assert!(serde_json::to_vec(&unbounded_guards).unwrap().len()
            > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES);
        runtime.store.workflow_state_for_tests(&state).unwrap();

        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
        for flag in ["workflow_summary_v1", "workflow_full_v1"] {
            let mut read = farcooler_client::plan_json::workflow_request(workspace);
            read.accepts.push(flag.into());
            let response = rpc.handle(read).await;
            let raw = receipt(&response);
            response_frame_len(response);
            assert_eq!(raw["state"]["source_sequences"], serde_json::json!(state.source_sequences));
            assert_eq!(raw["source_sequences_omitted"], 0);
            assert_eq!(raw["state"]["revision"], state.revision);
            assert_eq!(raw["state"]["epoch"], state.epoch);
            let live = &state.items["item-000"];
            assert_eq!(raw["state"]["items"]["item-000"]["run"], live.run);
            assert_eq!(raw["state"]["items"]["item-000"]["pin"], live.pin);
            assert_eq!(raw["state"]["items"]["item-000"]["phase"], serde_json::json!(live.phase));
            let omitted = raw["landing_groups_omitted"].as_u64().unwrap() as usize;
            assert!(omitted >= count - 12);
            assert_eq!(raw["state"]["items"].as_object().unwrap().len(), 1 + count - omitted);
        }
    }

    #[tokio::test]
    async fn engine_and_scripts_read_a_board_whose_source_sequences_alone_exceed_the_frame() {
        use super::super::admission_tests::{factory, held, receipt, request};
        use farcooler_client::workflow_presentation::Style;
        use farcooler_protocol::{v1::Scope, workflow_commands::{Action, HistorySection, Mutation}};
        use farcooler_transport::{Handler, Peer};

        let (_service_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let key = workspace.to_string();
        let (_runtime_dir, mut runtime) = failed_logs_in(&key, 1, "").await;
        let mut state = runtime.state(&key).unwrap().unwrap();
        let live = state.items.get_mut("item-000").unwrap();
        live.phase = Phase::Failed;
        live.pending = None;
        state.source_sequences.extend((0..700)
            .map(|n| (format!("wire:member:former-member-{n:018}"), 99)));
        state.source_sequences.insert("wire:owner".into(), 41);
        assert!(serde_json::to_vec(&state.source_sequences).unwrap().len() > 16 * 1024);
        runtime.store.workflow_state_for_tests(&state).unwrap();

        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
        let response = rpc.handle(farcooler_client::plan_json::workflow_request(workspace)).await;
        let legacy = receipt(&response);
        response_frame_len(response);
        assert_eq!(legacy["state"]["source_sequences"], serde_json::json!(state.source_sequences));
        assert_eq!(legacy["source_sequences_omitted"], 0);

        state.source_sequences.extend((0..30_000)
            .map(|n| (format!("wire:member:former-member-{n:018}"), 99)));
        assert!(serde_json::to_vec(&state.source_sequences).unwrap().len()
            > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES);
        runtime.store.workflow_state_for_tests(&state).unwrap();

        let response = rpc.handle(farcooler_client::plan_json::workflow_request(workspace)).await;
        let legacy = receipt(&response);
        response_frame_len(response);
        assert_eq!(legacy["state"]["source_sequences"], serde_json::json!({"wire:owner":41}));
        assert_eq!(legacy["source_sequences_omitted"], state.source_sequences.len() - 1);
        for style in [Style::Apple, Style::Material, Style::Web] {
            farcooler_client::engine_view::from_workflow_reply(workspace, &legacy, None, style).unwrap();
        }

        let mut snapshot = serde_json::Value::Null;
        for view in [None, Some("workflow_summary_v1"), Some("workflow_full_v1")] {
            let mut pages = 0;
            snapshot = farcooler_client::plan_json::read_workflow_view(workspace, view, async |read| {
                if let Some(view) = view { assert!(read.accepts.iter().any(|entry| entry == view)); }
                let response = rpc.handle(read).await;
                response_frame_len(response.clone());
                if receipt(&response)["workflow_page"].is_object() { pages += 1; }
                let Some(farcooler_protocol::v1::response::Outcome::Result(reply)) = response.outcome else {
                    panic!("Expected a workflow reply");
                };
                Ok(reply.value.unwrap())
            }).await.unwrap();
            assert!(pages > 1, "the complete map must span multiple frames");
            let wire = &snapshot["state"];
            assert_eq!(wire["revision"], state.revision);
            assert_eq!(wire["epoch"], state.epoch);
            assert_eq!(wire["source_sequences"], serde_json::json!(state.source_sequences));
            assert_eq!(snapshot["source_sequences_omitted"], 0);
            let live = &state.items["item-000"];
            assert_eq!(wire["items"]["item-000"]["run"], live.run);
            assert_eq!(wire["items"]["item-000"]["pin"], live.pin);
            assert_eq!(wire["items"]["item-000"]["phase"], serde_json::json!(live.phase));
            assert_eq!(wire["items"]["item-000"].get("definition").is_none(), view == Some("workflow_summary_v1"));
            if view.is_none() {
                for style in [Style::Apple, Style::Material, Style::Web] {
                    farcooler_client::engine_view::from_workflow_reply(workspace, &snapshot, None, style).unwrap();
                }
            }
        }

        let mut offset = 0;
        let mut digest = None;
        let mut encoded = String::new();
        loop {
            let read = request(workspace, &Action::History { section: HistorySection::SourceSequences, offset, item: None });
            let response = rpc.handle(read).await;
            let page = receipt(&response);
            response_frame_len(response);
            assert_eq!(page["offset"], offset);
            let first = digest.get_or_insert_with(|| page["digest"].as_str().unwrap().to_owned());
            assert_eq!(page["digest"], *first);
            encoded.push_str(page["chunk"].as_str().unwrap());
            let Some(next) = page["next_offset"].as_u64() else { break };
            assert!(next > offset);
            offset = next;
        }
        assert!(offset > 0, "the complete map must span multiple frames");
        let sequences: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(sequences, serde_json::json!(state.source_sequences));

        let wire = &snapshot["state"];
        let caller = snapshot["context"]["capabilities"]["decision_principal"].as_str().unwrap();
        let sequence = wire["source_sequences"][format!("wire:{caller}")].as_u64().unwrap() + 1;
        let row = &wire["items"]["item-000"];
        let cancel = Action::Cancel {
            mutation: Mutation { id: "cancel-from-paged-sequences".into(),
                revision: wire["revision"].as_u64().unwrap(), epoch: wire["epoch"].as_u64().unwrap(), sequence },
            item: "item-000".into(), run: row["run"].as_str().unwrap().into(), pin: row["pin"].as_str().unwrap().into(),
        };
        let response = held(&mut runtime, &rpc, workspace, cancel, false, || {}).await;
        assert!(receipt(&response).pointer("/receipt/result/Ok").is_some());
        state.source_sequences.insert("wire:owner".into(), sequence);
        assert_eq!(runtime.state(&key).unwrap().unwrap().source_sequences, state.source_sequences);
    }

    #[tokio::test]
    async fn engine_and_scripts_read_boards_that_exceed_the_frame_without_previews() {
        use super::super::admission_tests::{factory, receipt, request};
        use farcooler_protocol::{v1::{Plan, Scope}, workflow_commands::{Action, HistorySection}};
        use farcooler_transport::{Handler, Peer};

        let (_service_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let key = workspace.to_string();
        let (_runtime_dir, runtime) = failed_logs_in(&key, 1, "").await;
        let mut state = runtime.state(&key).unwrap().unwrap();
        let mut running = state.items.pop_first().unwrap().1;
        running.phase = Phase::Failed;
        running.pending = None;
        let reason = "Please revisit this build decision. ".repeat(100);
        let send_back = farcooler_workflow::SendBack { id: "back-1".into(), revision: state.revision,
            run: running.run.clone(), from: "review".into(), to: "build".into(), reason: reason.clone() };
        let mut tracker = running.clone();
        tracker.definition = Definition::ticket_at("todo").unwrap();
        tracker.pin = tracker.definition.pin();
        tracker.policy_pin = tracker.pin.clone();
        tracker.phase = Phase::WaitingApproval;
        let closed = "closed-dependency";
        tracker.dependencies.insert(closed.into());
        state.archive.insert(closed.into(), Archived { phase: Phase::Done, candidate: None,
            group: None, start_load: None, closed: None, last_run: None, history: Vec::new() });
        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);

        let max = farcooler_workflow::MAX_ITEMS;
        for (count, all_runs) in [(max, false), (max, true), (150, true)] {
            state.items.clear();
            let last = format!("ticket-{:04}", count - 1);
            for n in 0..count {
                let id = format!("ticket-{n:04}");
                let mut item = if all_runs || id == last { running.clone() } else { tracker.clone() };
                item.run = digest(&(&key, &id));
                item.resource = Exec::item_resource(&key, &id);
                if all_runs && (count < max || id == last) {
                    item.sent_back = (0..3).map(|i| farcooler_workflow::SendBack {
                        id: format!("back-{i}"), run: item.run.clone(), ..send_back.clone()
                    }).collect();
                } else if !all_runs && n == 0 {
                    item.follows = Some(last.clone());
                } else if !all_runs && id == last {
                    item.followers.push("ticket-0000".into());
                }
                state.items.insert(id, item);
            }
            assert!(serde_json::to_vec(&state.items).unwrap().len() > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES,
                "the live items alone must overflow the frame");
            runtime.store.workflow_state_for_tests(&state).unwrap();

            let legacy = farcooler_client::plan_json::workflow_request(workspace);
            for flag in [None, Some("workflow_full_v1"), Some("workflow_summary_v1")] {
                let mut read = legacy.clone();
                read.accepts.extend(flag.map(String::from));
                let response = rpc.handle(read).await;
                let raw = receipt(&response);
                let bytes = response_frame_len(response);
                eprintln!("{count} live items, all runs: {all_runs}, {flag:?}: {bytes} bytes");
                assert_eq!(raw["state"]["revision"], state.revision);
                assert_eq!(raw["state"]["epoch"], state.epoch);
                assert_eq!(raw["state"]["source_sequences"], serde_json::json!(state.source_sequences));
                if !all_runs {
                    // The bare Done row fits the archive preview and survives dependency
                    // compaction. If the final fallback removes it, dispatch needs the
                    // omission count to look it up before opening a pane.
                    if flag == Some("workflow_summary_v1") {
                        assert_eq!(raw["state"]["archive"][closed]["phase"], "Done");
                        assert_eq!(raw["archive_omitted"], 0);
                    } else {
                        assert!(raw["state"]["archive"].get(closed).is_none());
                        assert_eq!(raw["archive_omitted"], 1);
                    }
                }
                let items = raw["state"]["items"].as_object().unwrap();
                for (id, item) in &state.items {
                    match items.get(id) {
                        Some(row) => {
                            assert_eq!(row["run"], item.run);
                            assert_eq!(row["pin"], item.pin);
                            assert_eq!(row["phase"], serde_json::json!(item.phase));
                        }
                        None => assert!(flag.is_none() && count == max, "{id} missing from {flag:?}"),
                    }
                }
                if flag == Some("workflow_full_v1") && !all_runs {
                    assert_eq!(raw["items_compact"], true);
                    assert_eq!(items["ticket-0000"]["definition"]["source"], farcooler_workflow::TICKET_SOURCE);
                    assert_eq!(items["ticket-0000"]["follows"], last);
                }
                if flag.is_none() {
                    if !all_runs {
                        assert!(items.contains_key(&last), "keep the active run before at-rest tickets");
                    }
                    if count < max {
                        assert_eq!(items[&last]["sent_back"].as_array().unwrap().len(), 3);
                        assert_eq!(raw["item_details_omitted"], true);
                    } else {
                        assert_eq!(raw["items_omitted"], count - items.len());
                        assert!(items.len() < count);
                        if all_runs {
                            assert!(!items.contains_key(&last), "the legacy preview can also omit active runs");
                        }
                    }
                    farcooler_client::engine_view::from_workflow_reply(
                        workspace, &raw, None, farcooler_client::workflow_presentation::Style::Apple).unwrap();
                    farcooler_client::plan_json::with_workflow_state(&Plan::default(), workspace, Some(async { Ok(raw.clone()) })).await.unwrap();
                }
            }

            // The reader used by the Mac's Engine command and every Session must
            // assemble all live rows before rendering or creating action guards.
            let mut pages = 0;
            let raw = farcooler_client::plan_json::read_workflow(workspace, async |read| {
                let response = rpc.handle(read).await;
                response_frame_len(response.clone());
                if receipt(&response)["workflow_page"].is_object() { pages += 1; }
                let Some(farcooler_protocol::v1::response::Outcome::Result(reply)) = response.outcome else {
                    panic!("Expected a workflow reply");
                };
                Ok(reply.value.unwrap())
            }).await.unwrap();
            assert!(pages > 1, "the live snapshot needs more than one frame");
            assert_eq!(raw["state"]["items"].as_object().unwrap().len(), count);
            assert_eq!(raw["state"]["archive"][closed]["phase"], "Done");
            for (id, item) in &state.items {
                let row = &raw["state"]["items"][id];
                assert_eq!(row["run"], item.run);
                assert_eq!(row["pin"], item.pin);
                assert_eq!(row["phase"], serde_json::json!(item.phase));
                assert_eq!(row["definition"], serde_json::to_value(&item.definition).unwrap());
            }

            use farcooler_client::workflow_presentation::{ActionKind, Presentation, Style};
            let view = farcooler_client::engine_view::from_workflow_reply(workspace, &raw, None, Style::Apple).unwrap();
            let view = serde_json::to_value(view).unwrap();
            let tickets: std::collections::BTreeMap<_, _> = view["stations"].as_array().unwrap().iter()
                .flat_map(|station| station["tickets"].as_array().unwrap())
                .map(|ticket| (ticket["id"].as_str().unwrap(), ticket)).collect();
            assert_eq!(tickets.len(), count);
            assert_eq!(tickets[last.as_str()]["run"], state.items[&last].run);
            if all_runs {
                assert_eq!(tickets[last.as_str()]["sendBacks"]["history"][0]["reason"], reason);
            } else {
                assert_eq!(tickets["ticket-0000"]["run"], state.items[&last].run);
            }
            let plan = farcooler_client::plan_json::with_workflow_state(&Plan::default(), workspace, Some(async { Ok(raw) }))
                .await.unwrap();
            let presentation: Presentation = serde_json::from_value(plan["workflow"].clone()).unwrap();
            assert_eq!(presentation.runs.len(), if all_runs { count } else { 1 });
            for run in presentation.runs {
                assert_eq!(run.run, state.items[&run.item].run);
                assert!(run.actions.iter().any(|action| action.kind == ActionKind::Cancel), "{}", run.item);
            }
        }
        let archived = receipt(&rpc.handle(request(workspace, &Action::ArchiveItem { item: closed.into() })).await);
        assert_eq!(archived["item"]["key"], closed);
        assert_eq!(archived["item"]["phase"], "Done");
        let page = receipt(&rpc.handle(request(workspace, &Action::History {
            section: HistorySection::Items, offset: 0, item: Some("ticket-0000".into()),
        })).await);
        let retained: serde_json::Value = serde_json::from_str(page["chunk"].as_str().unwrap()).unwrap();
        assert_eq!(retained["ticket-0000"]["sent_back"][0]["reason"], reason);
    }

    #[tokio::test]
    async fn an_archived_items_tail_is_read_on_request() {
        let (_dir, runtime) = failed_logs(3, "first\nwhy it failed\n").await;
        archive_first(&runtime, "w", 2);
        let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        assert!(reply["process_logs"].as_array().unwrap().iter().all(|log| log["item"] == "item-002"));
        let logs = runtime.wire_command("w", "owner",
            farcooler_protocol::workflow_commands::Action::Logs { item: "item-000".into() }).unwrap();
        assert_eq!(logs["item"], "item-000");
        assert_eq!(logs["archived"], true);
        assert_eq!(logs["live"], false);
        let log = &logs["process_logs"][0];
        assert_eq!(log["tail"], "first\nwhy it failed");
        assert_eq!(log["failed"], true);
        assert_eq!(log["exit"], 1);
        assert!(Path::new(log["log_path"].as_str().unwrap()).is_file());
        // A name the board never held reads as no logs, not a refusal.
        let none = runtime.wire_command("w", "owner",
            farcooler_protocol::workflow_commands::Action::Logs { item: "never".into() }).unwrap();
        assert_eq!(none["process_logs"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn an_archived_items_tail_is_read_over_rpc() {
        use super::super::admission_tests::{factory, request, receipt};
        use farcooler_transport::{Handler, Peer};
        use farcooler_protocol::{v1::Scope, workflow_commands::Action};
        let (_svc_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let ws = workspace.to_string();
        let (_dir, runtime) = failed_logs_in(&ws, 2, "why it failed\n").await;
        archive_first(&runtime, &ws, 1);
        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
        let reply = receipt(&rpc.handle(request(workspace, &Action::Logs { item: "item-000".into() })).await);
        assert_eq!(reply["archived"], true);
        assert_eq!(reply["process_logs_omitted"], 0);
        assert_eq!(reply["process_logs"][0]["tail"], "why it failed");
        let get = receipt(&rpc.handle(request(workspace, &Action::Get)).await);
        assert!(get["process_logs"].as_array().unwrap().iter().all(|log| log["item"] == "item-001"));
    }

    fn manager_json_recipe(cli: &str) -> String {
        let plugin = crate::skill_install::render(crate::skill_install::Harness::Claude, cli);
        let skill = plugin.iter().find(|file| file.relative == "skills/manager/SKILL.md").unwrap();
        skill.contents.lines().find(|line| line.starts_with("workflow_json=")).unwrap().to_owned()
    }

    #[test]
    fn workflow_get_manager_recipe_preserves_captured_json_escapes() {
        // Exact JSON string excerpts at the failures from the two ov-1340 captures.
        // In the manager's original command, echo "$o" emitted a raw ESC, collapsed
        // the backslash before '(', or left a backslash followed by a raw ESC.
        let cases = [
            (r#"o\u001b[33;37Hrequired"#, "o\x1b[33;37Hrequired"),
            (r#"rmtree\\(|cleanup_target"#, "rmtree\\(|cleanup_target"),
            (r#"=codex\u001b\\\u001b[?2026h\u001b[?25l"#, "=codex\x1b\\\x1b[?2026h\x1b[?25l"),
        ];
        let recipe = format!(
            "fixture_cli() {{ printf '%s\\n' \"$1\"; }}\n{}",
            manager_json_recipe("fixture_cli \"$1\""),
        );
        for (encoded_tail, tail) in cases {
            let reply = format!(r#"{{"tail":"{encoded_tail}"}}"#);
            // Enable backslash interpretation in Bash echo, as in the manager's zsh shell.
            let output = std::process::Command::new("/bin/bash")
                .args(["-O", "xpg_echo", "-c", &recipe, "workflow-json", &reply])
                .output().unwrap();
            assert!(output.status.success());
            let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(parsed["tail"], tail);
            assert_eq!(output.stdout, format!("{reply}\n").as_bytes());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn workflow_get_original_manager_capture_reproduces_malformed_bytes() {
        // Replay the logged caller with the exact JSON escapes around the failures.
        let script = r#"
fixture_cli() { printf '%s\n' "$1"; }
o=$(fixture_cli "$1" workflow get "$workspace" --json)
echo "$o"
"#;
        let cases = [
            (r#"{"tail":"o\u001b[33;37Hrequired"}"#, b"{\"tail\":\"o\x1b[33;37Hrequired\"}\n".as_slice()),
            (r#"{"tail":"rmtree\\(|cleanup_target"}"#, b"{\"tail\":\"rmtree\\(|cleanup_target\"}\n".as_slice()),
            (r#"{"tail":"=codex\u001b\\\u001b[?2026h\u001b[?25l"}"#, b"{\"tail\":\"=codex\x1b\\\x1b[?2026h\x1b[?25l\"}\n".as_slice()),
        ];
        for (reply, broken) in cases {
            let output = std::process::Command::new("/bin/zsh")
                .args(["-f", "-c", script, "workflow-json", reply])
                .output().unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, broken);
            assert!(serde_json::from_slice::<serde_json::Value>(&output.stdout).is_err());
        }
    }

    #[test]
    fn workflow_get_manager_recipe_propagates_a_failed_read_without_json() {
        let recipe = format!(
            "fixture_cli() {{ printf '%s\\n' 'Workflow read timed out' >&2; return 1; }}\n{}",
            manager_json_recipe("fixture_cli"),
        );
        let output = std::process::Command::new("/bin/bash")
            .args(["-c", &recipe])
            .output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"Workflow read timed out\n");
    }

    // Like the workflow.get load test, this uses the real CLI binary. Build it with
    // cargo build -p farcooler-cli before running this test with --ignored.
    #[tokio::test]
    #[ignore]
    async fn workflow_get_cli_json_while_pane_output_changes() {
        use super::super::admission_tests::factory;
        use crate::test_support::{PANE_PATIENCE, shell_ready, short_fixture};
        use farcooler_protocol::v1::Scope;
        use farcooler_transport::{HandshakeConfig, Peer, UnixListenerServer};
        use tokio::process::Command;

        let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/farcooler")
            .canonicalize().expect("build the CLI first");
        let (dir, svc, repo) = short_fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id.to_string();
        let (_runtime_dir, runtime) = failed_logs_in(&workspace, 20, &"\x1b[31m".repeat(1600)).await;
        let effect = runtime.store.workflow_outbox(&workspace).unwrap().pop().unwrap().effect;
        let log = runtime.exec.effect_log(&effect).unwrap();
        std::fs::write(&log, "").unwrap();

        let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
        let terminal = svc.create_terminal(worktree.id, "changing output", "shell").await.unwrap().id;
        shell_ready(&svc, terminal).await;
        let pane = svc.tmux.list_tagged_panes().await.unwrap().into_iter()
            .find(|pane| pane.terminal_id == terminal).unwrap().pane_id;
        let quoted_log = crate::service::shell_quote(log.to_str().unwrap());
        svc.tmux.pipe_pane_start(&pane, &format!("cat >> {quoted_log}")).await.unwrap();
        let script = r#"n=0; while :; do printf '\033[31m%s\033[0m =codex\033\\\033[?2026h\033[?25l\n' "sample-$n é rmtree\\(|cleanup_target \""; n=$((n+1)); sleep 0.01; done"#;
        svc.tmux.send_keys(&pane, &format!("{script}\n")).await.unwrap();
        let first_sample = "\x1b[31msample-0 é rmtree\\(|cleanup_target \"\x1b[0m =codex\x1b\\\x1b[?2026h\x1b[?25l".as_bytes();
        tokio::time::timeout(PANE_PATIENCE, async {
            while !std::fs::read(&log).unwrap().windows(first_sample.len()).any(|bytes| bytes == first_sample) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("pane started producing captured output");

        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
        let home = dir.path().join("state");
        let listener = UnixListenerServer::bind(home.join("farcoolerd.sock")).unwrap();
        let server = tokio::spawn(async move {
            listener.serve(move |_| Some((HandshakeConfig { daemon_version: "test".into() }, rpc.clone()))).await
        });
        // Exercise the manager's documented shell recipe as well as direct stdout.
        let recipe = manager_json_recipe("\"$1\"");
        let mut tails = BTreeSet::new();
        for n in 0..16 {
            let mut command = if n % 2 == 0 {
                let mut command = Command::new(&cli);
                command.args(["workflow", "get", &workspace, "--json"]);
                command
            } else {
                // Enable the same escape interpretation that zsh gives echo by default.
                let mut command = Command::new("/bin/bash");
                command.args(["-O", "xpg_echo", "-c", &recipe, "workflow-json"])
                    .arg(&cli).env("workspace", &workspace);
                command
            };
            let output = command.env("FARCOOLER_HOME", &home).output().await.unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert!(output.stdout.len() > 200 * 1024, "exercise a multi-chunk reply");
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let tail = json["process_logs"].as_array().unwrap().iter()
                .find(|entry| entry["effect"] == effect.id).unwrap()["tail"].as_str().unwrap();
            assert!(tail.contains("\x1b[31msample-"));
            assert!(tail.contains("é rmtree\\(|cleanup_target \"\x1b[0m =codex\x1b\\\x1b[?2026h\x1b[?25l"));
            tails.insert(tail.to_owned());
        }
        assert!(tails.len() > 1, "the captured pane output changed during the reads");
        server.abort();
    }

    fn get_with_bounded_logs(runtime: &Runtime, count: usize) -> serde_json::Value {
        let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        use farcooler_protocol::v1 as pb;
        let envelope = pb::WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(pb::wire_envelope::Body::Response(pb::Response {
                request_id: farcooler_protocol::ids::new_id(),
                outcome: Some(pb::response::Outcome::Result(pb::Result {
                    value: Some(pb::result::Value::WorkflowReply(pb::WorkflowReply {
                        schema: 1, json: reply.to_string(),
                    })),
                })),
            })),
        };
        let frame = farcooler_protocol::framing::encode(&envelope).expect("workflow get must fit the protocol cap");
        eprintln!("workflow get with {count} failures: {} wire bytes", frame.len());
        let budgets = reply["back_edge_budgets"].as_object().unwrap();
        assert_eq!(budgets.len(), count);
        let logs = reply["process_logs"].as_array().unwrap();
        assert_eq!(logs.len(), count);
        for log in logs {
            assert!(Path::new(log["log_path"].as_str().unwrap()).is_file());
            assert!(log["tail"].as_str().unwrap().len() <= 8 * 1024);
        }
        let budget: usize = logs.iter().map(|log| serde_json::to_vec(&log["tail"]).unwrap().len()).sum();
        assert!(budget <= 256 * 1024, "encoded tails use {budget} bytes");
        reply
    }

    #[tokio::test]
    async fn workflow_get_keeps_short_tails_whole() {
        let (_dir, runtime) = failed_logs(1, "first\nlast\n").await;
        let reply = get_with_bounded_logs(&runtime, 1);
        assert_eq!(reply["process_logs"][0]["tail"], "first\nlast");
        assert_eq!(reply["process_logs"][0]["tail_truncated"], false);
    }

    #[tokio::test]
    async fn workflow_get_keeps_only_the_last_forty_lines() {
        let output = (0..60).map(|n| format!("line {n}\n")).collect::<String>();
        let (_dir, runtime) = failed_logs(1, &output).await;
        let reply = get_with_bounded_logs(&runtime, 1);
        let log = &reply["process_logs"][0];
        let tail = log["tail"].as_str().unwrap();
        assert_eq!(tail.lines().count(), 40);
        assert!(tail.starts_with("line 20\n"));
        assert!(tail.ends_with("line 59"));
        assert_eq!(log["tail_truncated"], true);
    }

    #[tokio::test]
    async fn workflow_get_budgets_plain_text_tails_without_shortening_newest_logs() {
        let output = "x".repeat(8 * 1024);
        let (_dir, runtime) = failed_logs(48, &output).await;
        let reply = get_with_bounded_logs(&runtime, 48);
        let logs = reply["process_logs"].as_array().unwrap();
        // JSON string quotes consume budget too: 32 complete tails exceed 256 KiB.
        assert_eq!(logs.iter().filter(|log| log["tail"] == "").count(), 17);
        for log in &logs[17..] { assert_eq!(log["tail"], output); }
        assert_eq!(logs.last().unwrap()["tail_truncated"], false);
    }

    #[tokio::test]
    async fn workflow_get_bounds_a_two_mib_single_line_failure() {
        // Three-byte characters force the byte cutoff to land inside a UTF-8 code point.
        let output = "xx".to_owned() + &"界".repeat((crate::effect_runner::LOG_LIMIT - 5) / 3) + "END";
        assert_eq!(output.len(), crate::effect_runner::LOG_LIMIT);
        let (_dir, runtime) = failed_logs(1, &output).await;
        let reply = get_with_bounded_logs(&runtime, 1);
        let log = &reply["process_logs"][0];
        assert_eq!(log["tail_truncated"], true);
        assert!(log["tail"].as_str().unwrap().ends_with("END"));
        assert!(!log["tail"].as_str().unwrap().contains('�'));
    }

    #[tokio::test]
    async fn workflow_get_bounds_captured_output_from_a_failed_process() {
        let (_dir, runtime) = failed_logs(1, "").await;
        let effect = runtime.store.workflow_outbox("w").unwrap().remove(0).effect;
        let path = runtime.effect_log(&effect).unwrap();
        // Exercise the actual merged-stream capture before workflow get: more than
        // 2 MiB without a newline, followed by failure evidence on stderr.
        let argv = ["/bin/sh", "-c",
            "dd if=/dev/zero bs=1024 count=2048 2>/dev/null | tr '\\000' x; printf END >&2; exit 1"]
            .map(String::from);
        let environment = [("PATH".into(), "/usr/bin:/bin".into())].into_iter().collect();
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let exit = crate::effect_runner::configured(&runtime.root, &argv, &environment,
            &path, Duration::from_secs(30), cancelled).await.unwrap();
        assert_eq!(exit, Some(1));
        let retained = std::fs::read_to_string(&path).unwrap();
        assert_eq!(retained.len(), crate::effect_runner::LOG_LIMIT);
        assert!(retained.ends_with("END"));
        let reply = get_with_bounded_logs(&runtime, 1);
        let log = &reply["process_logs"][0];
        assert_eq!(log["tail"], retained[retained.len() - 8 * 1024..]);
        assert_eq!(log["tail_truncated"], true);
        assert_eq!(log["exit"], 1);
        assert_eq!(Path::new(log["log_path"].as_str().unwrap()), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), retained);
    }

    #[tokio::test]
    async fn workflow_get_bounds_legacy_logs_without_changing_retained_evidence() {
        // Old daemons did not cap files. Exercise every multibyte UTF-8 width at
        // the seek boundary, and leave the original evidence intact for inspection.
        for character in ["é", "界", "🦀"] {
            let output = character.repeat(3 * 1024 * 1024 / character.len()) + "END";
            let (_dir, runtime) = failed_logs(1, &output).await;
            let reply = get_with_bounded_logs(&runtime, 1);
            let log = &reply["process_logs"][0];
            let tail = log["tail"].as_str().unwrap();
            assert!(tail.ends_with("END"));
            assert!(!tail.contains('�'));
            assert_eq!(log["tail_truncated"], true);
            assert_eq!(std::fs::read_to_string(log["log_path"].as_str().unwrap()).unwrap(), output);
        }
    }

    #[tokio::test]
    async fn workflow_get_bounds_lossy_utf8_expansion() {
        let (_dir, runtime) = failed_logs(1, "").await;
        let effect = runtime.store.workflow_outbox("w").unwrap().remove(0).effect;
        let path = runtime.effect_log(&effect).unwrap();
        std::fs::write(path, vec![0xff; 8 * 1024]).unwrap();
        let reply = get_with_bounded_logs(&runtime, 1);
        assert_eq!(reply["process_logs"][0]["tail_truncated"], true);
        assert!(reply["process_logs"][0]["tail"].as_str().unwrap().contains('�'));
    }

    #[tokio::test]
    async fn workflow_get_drops_oldest_tails_to_bound_many_failures() {
        // Each source is small, but together they exceed the control envelope cap.
        // Control characters also ensure the budget counts JSON escaping.
        let output = "\u{0001}".repeat(32 * 1024);
        let (_dir, runtime) = failed_logs(48, &output).await;
        let reply = get_with_bounded_logs(&runtime, 48);
        let logs = reply["process_logs"].as_array().unwrap();
        assert_eq!(logs.first().unwrap()["item"], "item-000");
        assert_eq!(logs.last().unwrap()["item"], "item-047");
        let mut saw_tail = false;
        for log in logs {
            if log["tail"] == "" {
                assert!(!saw_tail, "only the oldest tails should be omitted");
                assert_eq!(log["tail_truncated"], true);
            } else { saw_tail = true; }
        }
        assert_eq!(logs[0]["tail"], "");
        assert!(saw_tail, "keep the newest evidence");
    }

    #[tokio::test]
    async fn pruning_preserves_output_from_an_admitted_effect_without_a_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        enqueue(&mut runtime, "w", "one", coding_definition(false));
        let state = runtime.state("w").unwrap().unwrap();
        let effect = runtime.store.workflow_outbox("w").unwrap().into_iter()
            .find(|row| row.active).unwrap().effect;
        let claim = runtime.store.workflow_claim_effect("w", &effect.id, "test-owner",
            state.revision, state.epoch, 2, 7).unwrap().unwrap();
        assert!(runtime.store.workflow_admit_effect(&claim, 2).unwrap());

        let receipts = runtime.root.join("receipts");
        let output = receipts.join(format!("{}.output", digest(&effect)));
        let legacy = receipts.join(format!("{}.log", effect.id));
        for path in [&output, &legacy] {
            std::fs::write(path, "still writing").unwrap();
        }
        runtime.prune_process_logs().unwrap();
        assert!(output.exists() && legacy.exists());
    }

    #[tokio::test]
    async fn idle_runtime_prunes_orphan_logs_created_after_startup() {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let receipts = runtime.root.join("receipts");
        let startup = receipts.join(format!("{}.output", digest(&"startup")));
        std::fs::write(&startup, "startup orphan").unwrap();

        tokio::time::pause();
        runtime.pump(now()).await.unwrap();
        assert!(!startup.exists());

        let orphan = receipts.join(format!("{}.output", digest(&"orphan")));
        let legacy_orphan = receipts.join(format!("{}.log", digest(&"legacy")));
        let process = receipts.join(format!("{}.output", digest(&"process")));
        let journal = receipts.join(format!("{}.output", digest(&"journal")));
        for file in [&orphan, &legacy_orphan, &process, &journal] {
            std::fs::write(file, "output after startup").unwrap();
        }
        std::fs::write(process.with_extension("process.json"), "{}").unwrap();
        std::fs::write(journal.with_extension("json"), "{}").unwrap();

        let handle = runtime.command_handle(&runtime.controller()).unwrap();
        let (stop, signal) = tokio::sync::watch::channel(false);
        let worker = tokio::spawn(async move { runtime.run(signal).await });
        assert!(handle.state("w").await.unwrap().is_none());
        assert!(orphan.exists(), "a read must not trigger another log scan");
        tokio::time::advance(super::super::build_cleanup::CLEANUP_MAINTENANCE_INTERVAL).await;
        let pruned = super::super::wait_until(
            tokio::time::Instant::now() + Duration::from_secs(10), Duration::from_millis(10),
            || !orphan.exists() && !legacy_orphan.exists(),
        ).await;
        stop.send_replace(true);
        worker.await.unwrap().unwrap();
        assert!(pruned, "idle maintenance must remove sidecars left after startup");
        assert!(process.exists() && journal.exists(), "either receipt retains its output");
    }

    #[tokio::test]
    async fn workflow_failed_gate_keeps_both_streams_and_get_points_to_its_tail() {
        let (_dir, mut runtime) = super::super::coding_tests::configured(false, "check-fail").await;
        let definition = runtime.configured_definition().unwrap();
        enqueue(&mut runtime, "w", "one", definition);
        assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Failed);
        let reply = runtime.wire_command("w", "owner", farcooler_protocol::workflow_commands::Action::Get).unwrap();
        let log = &reply["process_logs"][0];
        assert_eq!(log["item"], "one");
        assert_eq!(log["exit"], 1);
        let path = PathBuf::from(log["log_path"].as_str().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("gate stdout"));
        assert!(text.contains("gate stderr"));
        assert_eq!(log["tail"].as_str().unwrap().lines().count(), 40);
        assert_eq!(log["tail_truncated"], true);
        assert!(log["tail"].as_str().unwrap().contains("gate stderr"));
        let fact = path.with_extension("process.json");
        let receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&fact).unwrap()).unwrap();
        assert_eq!(receipt["log_path"], log["log_path"]);
        runtime.prune_process_logs().unwrap();
        assert!(path.exists(), "retained receipts retain their logs");
        let journal = path.with_extension("json");
        let kept: serde_json::Value = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
        assert_eq!(kept["log_path"], log["log_path"]);
        let fixture_log = runtime.root.join("receipts").join(format!("{}.log", log["effect"].as_str().unwrap()));
        std::fs::write(&fixture_log, "old fixture output").unwrap();
        std::fs::remove_file(fact).unwrap();
        runtime.prune_process_logs().unwrap();
        assert!(path.exists() && fixture_log.exists(), "the journal still retains both log names");
        std::fs::remove_file(journal).unwrap();
        runtime.prune_process_logs().unwrap();
        assert!(!path.exists() && !fixture_log.exists(), "logs go when their receipts go");
    }
}
