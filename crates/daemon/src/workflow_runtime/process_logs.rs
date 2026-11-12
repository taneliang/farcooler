//! Process evidence is local to the engine, alongside the receipts that retain it (ov-1127).
use super::*;

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
            file.seek(SeekFrom::Start(metadata.len().saturating_sub(crate::effect_runner::LOG_LIMIT as u64)))?;
            let mut bytes = Vec::new();
            file.take(crate::effect_runner::LOG_LIMIT as u64).read_to_end(&mut bytes)?;
            let text = String::from_utf8_lossy(&bytes);
            let tail = text.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            logs.push(serde_json::json!({"item":effect.item, "run":effect.run, "effect":effect.id,
                "stage":effect.kind, "log_path":path, "exit":exit, "tail":tail}));
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
