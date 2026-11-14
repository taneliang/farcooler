//! The task travels with the launch. Pin the whole brief once per effect so a changed card cannot change
//! the command recovery expects, and no agent needs a working CLI or a responsive runner to learn its job.
use super::*;

#[derive(Serialize, Deserialize)]
struct Brief { binding: String, text: String }

impl Exec {
    pub(super) fn ticket_prompt(&self, binding: &Binding, prompt: &str) -> Result<String> {
        let path = self.root.join("receipts").join(format!("{}.brief.json", digest(&binding.effect)));
        match self.bytes(&path) {
            Ok(raw) => {
                let brief: Brief = serde_json::from_slice(&raw)?;
                if brief.binding != digest(binding) { return Err(Error::Refused("foreign ticket brief")); }
                return Ok(brief.text);
            }
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        let reviewing = matches!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent)));
        let instructions = if reviewing { Self::review_prompt(binding) }
            else { Self::briefed(prompt, &binding.effect, binding.iteration) };
        let exe = std::env::current_exe()?;
        let found = crate::service::cli_binary(Some(&exe));
        // Even a missing sibling is named by its full channel path, never a PATH-dependent command.
        let cli = if Path::new(&found).is_absolute() { PathBuf::from(found) }
            else { exe.parent().ok_or(Error::Refused("missing daemon directory"))?.join(farcooler_protocol::CHANNEL.cli_binary_name()) };
        let cli = crate::service::shell_quote(&cli.to_string_lossy());
        let mut text = instructions.replace("`farcooler ", &format!("`{cli} "));
        let workspace = Uuid::parse_str(&binding.workspace).ok();
        for key in std::iter::once(&binding.effect.item).chain(binding.effect.with.iter()) {
            let tasks = self.store.tasks_with_key(None, key).map_err(Error::Domain)?;
            let task = tasks.iter().find(|t| workspace.is_none_or(|w| t.workspace_id == w));
            let Some(task) = task else {
                // Standalone workflows can name items that aren't board tickets; recovery must still be
                // able to reconstruct and cancel those older effects.
                continue;
            };
            text.push_str(&format!("\n\nTicket {}\nTitle\n{}\n\nIntent\n{}\n\nAcceptance\n",
                task.key, task.title, task.intent));
            for line in &task.acceptance { text.push_str(&line.text); text.push('\n'); }
            text.push_str("\nConstraints\n");
            for line in &task.constraints { text.push_str(line); text.push('\n'); }
        }
        if text.ends_with('\n') { text.push_str("End of ticket brief."); }
        durable(&path, &serde_json::to_vec(&Brief { binding: digest(binding), text: text.clone() })?)?;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_store::models::{Actor, AcceptanceItem, TaskUpdate};

    #[tokio::test]
    async fn ov_1217_build_and_review_carry_every_ticket_line_and_pin_the_snapshot() {
        let (_dir, mut runtime) = super::super::coding_tests::configured(false, "pass").await;
        let host = Uuid::now_v7();
        let root = runtime.store.create_repository_root(host, "/brief-test", 0).unwrap();
        let repo = runtime.store.create_repository(host, root.id, "brief", "/brief-test/.git", "").unwrap();
        let workspace = runtime.store.ensure_main_workspace(repo.id).unwrap();
        let task = runtime.store.create_task(workspace.id, "A title with `quotes`", Actor::User).unwrap();
        let acceptance = vec!["First line, exactly.", "Second line\nwith a continuation.", "修复 every item"];
        let update = TaskUpdate { title: task.title.clone(), intent: "Intent\nkeeps its line breaks.".into(),
            acceptance: acceptance.iter().map(|text| AcceptanceItem { id: Uuid::now_v7(), text: (*text).into(), met: false }).collect(),
            constraints: vec!["No stash, reset, amend, cargo fmt".into()], labels: vec![], worktree_id: None };
        runtime.store.update_task(task.id, task.resource_version, &update).unwrap();
        let effect: Effect = serde_json::from_value(serde_json::json!({ "id": "brief-build", "item": task.key,
            "run": "r", "epoch": 1, "fence": 1, "resource": "x", "definition_pin": "d", "policy_pin": "p",
            "candidate": null, "kind": { "Execute": "Build" },
            "operation": { "adapter_version": "a", "arguments": [], "environment": {} }, "created_ms": 0 })).unwrap();
        let mut binding = Binding::new(&workspace.id.to_string(), &effect, 1, 0, "base");
        let companion = runtime.store.create_task(workspace.id, "Companion ticket", Actor::User).unwrap();
        binding.effect.with = vec![companion.key.clone()];
        for review in [false, true] {
            if review {
                binding.effect.id = "brief-review".into();
                binding.effect.kind = EffectKind::Execute(StageKind::Review(Reviewer::Agent));
            }
            let brief = runtime.ticket_prompt(&binding, "Run tests and commit.").unwrap();
            for text in std::iter::once(update.title.as_str()).chain(std::iter::once(update.intent.as_str()))
                .chain(acceptance.iter().copied()).chain(update.constraints.iter().map(String::as_str)) {
                assert!(brief.contains(text), "missing {text:?}: {brief}");
            }
            assert!(brief.contains("Companion ticket"));
            assert!(!brief.contains("`farcooler "), "all CLI commands have an installed path: {brief}");
            if review {
                assert!(brief.contains("Don't change, commit or stage anything here."));
                let exe = std::env::current_exe().unwrap();
                let cli = crate::service::cli_binary(Some(&exe));
                let cli = if Path::new(&cli).is_absolute() { PathBuf::from(cli) }
                    else { exe.parent().unwrap().join(farcooler_protocol::CHANNEL.cli_binary_name()) };
                assert!(brief.contains(&format!("`{} workflow verdict", crate::service::shell_quote(&cli.to_string_lossy()))));
            }
            assert_eq!(runtime.ticket_prompt(&binding, "changed workflow words").unwrap(), brief);
        }
        let card = runtime.store.get_task(task.id).unwrap();
        let mut changed = update; changed.intent = "Changed after launch".into();
        runtime.store.update_task(task.id, card.resource_version, &changed).unwrap();
        assert!(!runtime.ticket_prompt(&binding, "changed").unwrap().contains("Changed after launch"));
        // Large cards travel through a private prompt file, without cutting any acceptance text or overflowing tmux.
        let card = runtime.store.get_task(task.id).unwrap();
        changed.intent = "Long ticket's intent. ".repeat(2000);
        runtime.store.update_task(task.id, card.resource_version, &changed).unwrap();
        binding.effect.id = "long-build".into();
        binding.effect.kind = EffectKind::Execute(StageKind::Build);
        runtime.coding.as_mut().unwrap().harness = CodingHarness::Native { preset: "claude:opus".into(),
            settings: None, project_hooks: None, codex_hooks: None, plugin_dir: None, prompt: "Run tests and commit.".into(),
            process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
        let command = runtime.coding_command(&binding).unwrap();
        assert!(command.argv[2].len() < crate::service::MAX_INLINE_LAUNCH_BYTES);
        let path = runtime.root.join("receipts").join(format!("{}.launch-brief", digest(&binding.effect)));
        let delivered = std::fs::read_to_string(path).unwrap();
        assert!(delivered.contains(&changed.intent));
        for line in acceptance { assert!(delivered.contains(line)); }
        assert_eq!(runtime.coding_command(&binding).unwrap(), command);
    }
}
