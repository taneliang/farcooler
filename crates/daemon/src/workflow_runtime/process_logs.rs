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
    pub(super) fn failed_process_logs(&self, workspace: &str) -> Result<Vec<serde_json::Value>> {
        let mut logs = Vec::new();
        let state = self.state(workspace)?;
        for stored in self.store.workflow_outbox(workspace)? {
            let effect = &stored.effect;
            let Some(path) = self.effect_log(effect) else { continue };
            let journal = self.bytes(&self.journal_path(effect)).ok()
                .and_then(|raw| serde_json::from_slice::<Journal>(&raw).ok());
            let fact_path = self.root.join("receipts").join(format!("{}.process.json", digest(effect)));
            let fact = self.bytes(&fact_path).ok()
                .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok());
            let exit = fact.as_ref().and_then(|f| f["exit"].as_i64());
            let failed_run = state.as_ref().and_then(|s| s.items.get(&effect.item)).is_some_and(|i|
                i.run == effect.run && matches!(i.phase, Phase::Failed | Phase::Uncertain));
            let failed = failed_run || exit.is_some_and(|code| code != 0) || journal.is_some_and(|j|
                matches!(j.outcome, Outcome::Rejected(_) | Outcome::SafeFailure | Outcome::SafeFailureBecause(_)
                    | Outcome::Interrupted | Outcome::Uncertain));
            if !failed { continue; }
            // Old daemons left unbounded logs. Inspect their tail without reading the entire file.
            use std::io::{Read, Seek, SeekFrom};
            let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.nlink() != 1 { continue; }
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
            logs.push(serde_json::json!({"item":effect.item, "run":effect.run, "effect":effect.id,
                "stage":effect.kind, "log_path":path, "exit":exit, "tail":tail, "tail_truncated":truncated}));
        }
        let mut budget = 0;
        for log in &logs { budget += serde_json::to_vec(&log["tail"])?.len(); }
        // The outbox is ordered by revision, then id: omit oldest tails first, keeping
        // every path available for inspecting the complete retained process output.
        for log in &mut logs {
            if budget <= REPLY_TAIL_BYTES { break; }
            budget -= serde_json::to_vec(&log["tail"])?.len() - 2; // empty JSON string still costs two bytes
            log["tail"] = "".into();
            log["tail_truncated"] = true.into();
        }
        Ok(logs)
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{enqueue, finish};

    async fn failed_logs(count: usize, output: &str) -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let controller = runtime.controller();
        for n in 0..count {
            let receipt = runtime.submit(&controller, Request {
                id: format!("enqueue-{n}"), workspace: "w".into(), expected_revision: n as u64,
                sequence: n as u64 + 1,
                command: Command::Enqueue { item: format!("item-{n:03}"), definition: coding_definition(false),
                    policy_pin: "policy/v1".into(), resource: Exec::item_resource("w", &format!("item-{n:03}")),
                    dependencies: BTreeSet::new(), initial_candidate: None },
            }, 1).unwrap();
            assert!(receipt.result.is_ok(), "{:?}", receipt.result);
        }
        for stored in runtime.store.workflow_outbox("w").unwrap() {
            let key = digest(&stored.effect);
            std::fs::write(runtime.root.join("receipts").join(format!("{key}.output")), output).unwrap();
            std::fs::write(runtime.root.join("receipts").join(format!("{key}.process.json")), br#"{"exit":1}"#).unwrap();
        }
        (dir, runtime)
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
