//! A read-only decision call may be tried twice. Admission records each attempt before spawn;
//! recovery consumes a retained receipt, or logs Missing so the reducer spends that attempt.
use super::*;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

impl Exec {
    fn hook_receipt(&self, effect: &Effect) -> PathBuf {
        self.root.join("receipts").join(format!("{}.decision.json", effect.id))
    }
    fn hook_result(&self, binding: &Binding) -> HookResult {
        match self.bytes(&self.hook_receipt(&binding.effect)) {
            Ok(bytes) if bytes.len() <= 16_384 => match String::from_utf8(bytes.clone()) {
                Ok(raw) => HookResult::Receipt { manifest: digest(&("hook-receipt/v1", binding, &binding.effect.kind, bytes)), raw },
                Err(_) => HookResult::Invalid,
            },
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => HookResult::Missing,
            _ => HookResult::Invalid,
        }
    }
    pub(super) fn recover_hook(&self, binding: &Binding, now_ms: i64) -> Result<Outcome> {
        if let Some(journal) = self.read_journal(binding)? { return Ok(journal.outcome); }
        let result = self.hook_result(binding);
        let expired = self.store.workflow_state(&binding.workspace)?.and_then(|s| s.items.get(&binding.effect.item)
            .and_then(|i| i.deciding.as_ref()).map(|c| now_ms >= c.deadline_ms)).unwrap_or(false);
        Ok(Outcome::Hook(if result == HookResult::Missing && expired { HookResult::Timeout } else { result }))
    }
    pub(super) async fn execute_hook(&self, binding: &Binding, mut cancelled: Option<tokio::sync::watch::Receiver<bool>>) -> Result<Outcome> {
        let EffectKind::Decide { policy, input } = &binding.effect.kind else { return Err(Error::Refused("not a rule")) };
        // Execute the verified bytes from a private immutable copy: changing the source between the
        // digest check and exec must not change the program this published call runs.
        let bytes = match self.bytes(Path::new(&policy.script)) {
            Ok(bytes) if digest(&bytes) == policy.digest => bytes,
            _ => return Ok(Outcome::Hook(HookResult::Invalid)),
        };
        let executable = self.root.join("receipts").join(format!("{}.script", binding.effect.id));
        durable(&executable, &bytes)?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o500))?;
        let state = self.store.workflow_state(&binding.workspace)?.ok_or(Error::Refused("missing rule state"))?;
        let Some(open) = state.items.get(&binding.effect.item).and_then(|i| i.deciding.as_ref()) else {
            return Ok(Outcome::Hook(HookResult::Missing));
        };
        let remaining = open.deadline_ms.saturating_sub(self.clock_ms());
        if remaining <= 0 { return Ok(Outcome::Hook(HookResult::Timeout)); }
        let log = self.root.join("receipts").join(format!("{}.log", binding.effect.id));
        let log = OpenOptions::new().write(true).create_new(true).mode(0o600).open(log)?;
        let checkout = self.root.join(format!("hook-tree-{}", binding.effect.id));
        let checkout_name = checkout.to_str().ok_or(Error::Refused("invalid rule checkout path"))?;
        adapters::git(&self.repository(), &["worktree", "add", "--detach", checkout_name, &binding.base]).await?;
        let result: Result<Outcome> = async {
            let mut command = tokio::process::Command::new(&executable);
            command.env_clear().env("FARCOOLER_RECEIPT", self.hook_receipt(&binding.effect))
                .current_dir(&checkout).stdin(Stdio::piped()).stdout(log.try_clone()?).stderr(log)
                .kill_on_drop(true).process_group(0);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(_) => return Ok(Outcome::Hook(HookResult::Missing)),
            };
            let pid = child.id();
            let document = serde_json::to_vec(input)?;
            let mut stdin = child.stdin.take().ok_or(Error::Refused("missing rule stdin"))?;
            let wait = async {
                // A broken pipe is an exit, and still may have left a valid receipt.
                let _ = stdin.write_all(&document).await;
                drop(stdin);
                child.wait().await
            };
            let remaining = open.deadline_ms.saturating_sub(self.clock_ms()).max(1);
            let interrupted = tokio::select! {
                completed = tokio::time::timeout(Duration::from_millis(remaining as u64), wait) =>
                    completed.is_err().then_some(HookResult::Timeout),
                _ = async {
                    match &mut cancelled {
                        Some(signal) => { if !*signal.borrow() { let _ = signal.changed().await; } }
                        None => std::future::pending::<()>().await,
                    }
                } => Some(HookResult::Missing),
            };
            if let Some(result) = interrupted {
                if let Some(pid) = pid { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Ok(Outcome::Hook(result));
            }
            // A receipt, not exit status or stdout, is the decision.
            Ok(Outcome::Hook(self.hook_result(binding)))
        }.await;
        adapters::git(&self.repository(), &["worktree", "remove", "--force", checkout_name]).await?;
        result
    }
}
