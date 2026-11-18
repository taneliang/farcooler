//! Process evidence is local to the engine, alongside the receipts that retain it (ov-1127).
use super::*;

const TAIL_BYTES: usize = 8 * 1024;
const TAIL_LINES: usize = 40;
// Count JSON escaping too: control bytes can grow sixfold on the wire. Leave room for
// state and log paths in the 1 MiB control envelope, while retaining the newest evidence.
const REPLY_TAIL_BYTES: usize = 256 * 1024;

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
    pub(super) fn failed_process_logs(&self, workspace: &str) -> Result<Vec<serde_json::Value>> {
        let state = self.state(workspace)?;
        let Some(live) = state.as_ref().map(|s| &s.items) else { return Ok(Vec::new()) };
        let outbox = self.store.workflow_outbox(workspace)?;
        // The outbox is ordered by revision, then id, so a later effect of the same stage replaces an earlier one.
        let mut last = std::collections::BTreeMap::new();
        for (n, stored) in outbox.iter().enumerate() {
            if live.contains_key(&stored.effect.item) { last.insert((stored.effect.item.as_str(), stage_key(&stored.effect.kind)), n); }
        }
        let mut chosen: Vec<_> = last.into_values().collect();
        chosen.sort_unstable();
        let mut logs = Vec::new();
        for n in chosen {
            if let Some(log) = self.process_log_entry(state.as_ref(), &outbox[n].effect, false)? { logs.push(log); }
        }
        budget_tails(&mut logs)?;
        Ok(logs)
    }

    /// Every retained process log of one item, in every run, failed or not, live, archived or removed:
    /// what `workflow.get`'s reply leaves out, read on request (ov-1298). Budgeted like the reply.
    pub(super) fn item_process_logs(&self, workspace: &str, item: &str) -> Result<serde_json::Value> {
        let state = self.state(workspace)?;
        let mut logs = Vec::new();
        for stored in self.store.workflow_outbox(workspace)?.iter().filter(|s| s.effect.item == item) {
            if let Some(log) = self.process_log_entry(state.as_ref(), &stored.effect, true)? { logs.push(log); }
        }
        budget_tails(&mut logs)?;
        let live = state.as_ref().is_some_and(|s| s.items.contains_key(item));
        let archived = state.as_ref().is_some_and(|s| s.archive.contains_key(item));
        Ok(serde_json::json!({"schema":1, "item":item, "live":live, "archived":archived, "process_logs":logs}))
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
            matches!(j.outcome, Outcome::Rejected(_) | Outcome::SafeFailure | Outcome::SafeFailureBecause(_)
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
    pub(super) fn prune_process_logs(&self, workspaces: &[String]) -> Result<()> {
        let mut active = BTreeSet::new();
        let mut fixture_receipts = std::collections::BTreeMap::new();
        for workspace in workspaces {
            for stored in self.store.workflow_outbox(workspace)? {
                fixture_receipts.insert(stored.effect.id.clone(), digest(&stored.effect));
                if stored.active || matches!(stored.dispatch_state.as_str(), "claimed" | "admitted" | "uncertain") {
                    active.insert(digest(&stored.effect));
                    active.insert(stored.effect.id);
                }
            }
        }
        let receipts = self.root.join("receipts");
        for entry in std::fs::read_dir(&receipts)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".output").or_else(|| name.strip_suffix(".log")) else { continue };
            // Other adapters have their own *.fetch.output etc. This owns only process sidecars.
            if !key.starts_with("wf1:") || key.len() != 68 || !key[4..].bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
            let journal_key = if name.ends_with(".log") { fixture_receipts.get(key).map_or(key, String::as_str) } else { key };
            if active.contains(key) || receipts.join(format!("{key}.process.json")).try_exists()?
                || receipts.join(format!("{journal_key}.json")).try_exists()? { continue; }
            if entry.file_type()?.is_file() { std::fs::remove_file(entry.path())?; }
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
            state.archive.insert(id, farcooler_workflow::Archived { closed: None, phase: Phase::Cancelled, candidate: item.candidate, group: None });
        }
        runtime.store.workflow_state_for_tests(&state).unwrap();
        assert_eq!(runtime.state(workspace).unwrap().unwrap().archive.len(), count);
    }

    fn frame_len(reply: &serde_json::Value) -> usize {
        use farcooler_protocol::v1 as pb;
        let envelope = pb::WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(pb::wire_envelope::Body::Response(pb::Response {
                request_id: farcooler_protocol::ids::new_id(),
                outcome: Some(pb::response::Outcome::Result(pb::Result {
                    value: Some(pb::result::Value::WorkflowReply(pb::WorkflowReply { schema: 1, json: reply.to_string() })),
                })),
            })),
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
        assert_eq!(reply["process_logs"][0]["tail"], "why it failed");
        let get = receipt(&rpc.handle(request(workspace, &Action::Get)).await);
        assert!(get["process_logs"].as_array().unwrap().iter().all(|log| log["item"] == "item-001"));
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
        runtime.prune_process_logs(&["w".into()]).unwrap();
        assert!(path.exists(), "retained receipts retain their logs");
        let journal = path.with_extension("json");
        let kept: serde_json::Value = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
        assert_eq!(kept["log_path"], log["log_path"]);
        let fixture_log = runtime.root.join("receipts").join(format!("{}.log", log["effect"].as_str().unwrap()));
        std::fs::write(&fixture_log, "old fixture output").unwrap();
        std::fs::remove_file(fact).unwrap();
        runtime.prune_process_logs(&["w".into()]).unwrap();
        assert!(path.exists() && fixture_log.exists(), "the journal still retains both log names");
        std::fs::remove_file(journal).unwrap();
        runtime.prune_process_logs(&["w".into()]).unwrap();
        assert!(!path.exists() && !fixture_log.exists(), "logs go when their receipts go");
    }
}
