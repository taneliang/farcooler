//! Explicit, trusted-local retained landing transport identity. No network remote.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LandingFact {
    binding: Binding,
    identity: CodingLanding,
    candidate: String,
    log_digest: String,
}
impl Runtime {
    fn remote_fact(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.landing.json", digest(&binding.effect)))
    }
    fn remote_log(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push.output", digest(&binding.effect)))
    }
    pub(super) async fn land_remote(&self, binding: &Binding, cancelled: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let Some(identity) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else { return Ok(()) };
        self.coding.as_ref().unwrap().validate()?;
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing landing candidate"))?;
        if adapters::git(&identity.repository, &["rev-parse", &identity.target]).await? != binding.base {
            return Err(Error::Refused("remote landing base moved"));
        }
        if adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
            return Err(Error::Refused("landing transport is not bare"));
        }
        self.git_recorded(binding, &self.tree(binding), &["merge-base", "--is-ancestor", &binding.base, candidate]).await?;
        // Disable global/system configuration and refuse URL rewriting/includes
        // in the selected local repository. No named-remote or provider lookup.
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if config.lines().any(|l| l.starts_with("url.") || l.starts_with("include.") || l.starts_with("includeif.")) {
            return Err(Error::Refused("local landing config redirects transport"));
        }
        let git = crate::git_launch::git_launch().map_err(|_| Error::Refused("git unavailable"))?;
        let mut command = std::process::Command::new(&git.program);
        command.env_clear();
        let mut pins = crate::git::guard_pins(&self.repository()).await.map_err(|_| Error::Refused("landing guard pins unavailable"))?;
        pins.push(("protocol.file.allow".into(), "always".into()));
        crate::git_guard::apply(&mut command, &pins);
        command.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null");
        let environment = command.get_envs().filter_map(|(k,v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())))
            .collect();
        let target = crate::landing_job::push_target(candidate, &identity.target);
        let argv = vec![git.program.to_string_lossy().into_owned(), "push".into(), "--porcelain".into(),
            // Local push policy must not expand this one-ref operation to tags,
            // submodule remotes or signing machinery outside its pinned identity.
            "--no-follow-tags".into(), "--recurse-submodules=no".into(), "--signed=false".into(),
            format!("--force-with-lease={}:{}", identity.target, binding.base),
            identity.repository.to_string_lossy().into_owned(), target];
        let log = self.remote_log(binding);
        durable(&log, b"")?;
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, &log,
            Duration::from_secs(60), cancelled).await?;
        durable(&self.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect))),
            &serde_json::to_vec(&serde_json::json!({"binding": binding, "identity": identity, "argv": argv,
                "environment": environment, "exit": exit, "log_digest": digest(&self.bytes(&log)?)}))?)?;
        if exit != Some(0) { return Err(Error::Refused("local landing push failed; inspect receipt")); }
        if adapters::git(&identity.repository, &["rev-parse", &identity.target]).await? != candidate {
            return Err(Error::Refused("remote accepted SHA unknown"));
        }
        durable(&self.remote_fact(binding), &serde_json::to_vec(&LandingFact { binding: binding.clone(),
            identity: identity.clone(), candidate: candidate.into(), log_digest: digest(&self.bytes(&log)?) })?)
    }
    pub(super) async fn verify_remote_landing(&self, binding: &Binding) -> Result<()> {
        let Some(identity) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else { return Ok(()) };
        self.coding.as_ref().unwrap().validate()?;
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing remote candidate"))?;
        // CI's own binding differs. It verifies current identity/SHA while Land
        // alone consumes the exact original push settlement. No push on recovery.
        if matches!(binding.effect.kind, EffectKind::Execute(StageKind::Land)) {
            let fact: LandingFact = serde_json::from_slice(&self.bytes(&self.remote_fact(binding))?)?;
            if fact.binding != *binding || fact.identity != *identity || fact.candidate != candidate
                || fact.log_digest != digest(&self.bytes(&self.remote_log(binding))?) {
                return Err(Error::Refused("foreign local landing settlement"));
            }
        }
        if adapters::git(&identity.repository, &["rev-parse", &identity.target]).await? != candidate {
            return Err(Error::Refused("remote landing SHA changed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    pub(super) async fn remote() -> (tempfile::TempDir, Runtime) {
        let (dir, mut runtime) = configured(false, "pass").await;
        let bare = dir.path().join("private-remote.git"); std::fs::create_dir(&bare).unwrap();
        adapters::git(&bare, &["init", "--bare", "--quiet"]).await.unwrap();
        let repo = runtime.repository();
        // Initial private remote population is test setup, never an external push.
        let status = tokio::process::Command::new("/usr/bin/git").current_dir(&repo)
            .args(["-c", "core.hooksPath=/dev/null", "push", bare.to_str().unwrap(), "HEAD:refs/heads/main"])
            .status().await.unwrap(); assert!(status.success());
        let config = runtime.coding.as_mut().unwrap();
        config.landing = Some(CodingLanding { job: "landing-928".into(), repository: bare.canonicalize().unwrap(),
            target: "refs/heads/main".into(), config_digest: digest(&std::fs::read(bare.join("config")).unwrap()) });
        let root = runtime.root.clone(); std::fs::write(root.join("coding.json"), serde_json::to_vec(config).unwrap()).unwrap();
        drop(runtime);
        (dir, Runtime::open(&root).unwrap())
    }
    #[tokio::test]
    async fn workflow_coding_retained_private_remote_exact_sha_and_repeated_bases() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap();
        for item in ["one", "two"] {
            enqueue(&mut runtime, "w", item, definition.clone());
            let state = finish(&mut runtime, "w").await;
            assert_eq!(state.items[item].phase, Phase::Done);
            let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
            assert_eq!(adapters::git(&identity.repository, &["rev-parse", &identity.target]).await.unwrap(),
                state.items[item].candidate.clone().unwrap());
        }
    }
    #[tokio::test]
    async fn workflow_coding_retained_remote_crash_receipt_and_changed_base_refusal() {
        for crash in [true, false] {
            let (_dir, mut runtime) = remote().await;
            let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
            for time in 2..6 { runtime.pump(time).await.unwrap(); }
            let binding = admitted(&runtime).await;
            let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
            let candidate = binding.effect.candidate.as_ref().unwrap();
            if !crash {
                // Visible exact candidate alone is not this operation's push receipt.
                let status = tokio::process::Command::new("/usr/bin/git").current_dir(runtime.repository())
                    .args(["push", identity.repository.to_str().unwrap(), &crate::landing_job::push_target(candidate, &identity.target)])
                    .status().await.unwrap(); assert!(status.success());
            }
            let (_hold, cancelled) = tokio::sync::watch::channel(false);
            let result = runtime.execute_coding("w", &binding, cancelled).await;
            assert_eq!(result.is_ok(), crash);
            let root = runtime.root.clone(); drop(runtime);
            let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
            if crash { assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done); }
            else { assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain); }
        }
    }
    #[tokio::test]
    async fn workflow_coding_remote_missing_settlement_after_acceptance_never_pushes_again() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..6 { runtime.pump(time).await.unwrap(); }
        let binding = admitted(&runtime).await;
        let (_hold, cancelled) = tokio::sync::watch::channel(false);
        runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        std::fs::remove_file(runtime.remote_fact(&binding)).unwrap();
        let before = std::fs::read(runtime.remote_log(&binding)).unwrap();
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        assert_eq!(runtime.pump(201).await.unwrap(), 0);
        assert_eq!(std::fs::read(runtime.remote_log(&binding)).unwrap(), before);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use super::tests::remote;
    use super::super::{coding_tests::admitted, tests::enqueue};
    #[tokio::test]
    async fn workflow_coding_remote_rejected_update_records_exit_without_acceptance() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..6 { runtime.pump(time).await.unwrap(); }
        let binding = admitted(&runtime).await;
        let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
        let lock = identity.repository.join("refs/heads/main.lock"); std::fs::write(&lock, b"private test lock").unwrap();
        let (_hold, cancelled) = tokio::sync::watch::channel(false);
        assert!(runtime.execute_coding("w", &binding, cancelled).await.is_err());
        let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.root.join("receipts")
            .join(format!("{}.push-process.json", digest(&binding.effect)))).unwrap()).unwrap();
        assert_ne!(fact["exit"], 0);
        assert_eq!(adapters::git(&identity.repository, &["rev-parse", &identity.target]).await.unwrap(), binding.base);
        assert!(!runtime.remote_fact(&binding).exists());
        std::fs::remove_file(lock).unwrap();
        assert!(runtime.reconcile_coding(&binding).await.is_err(), "recovery must not retry the rejected push");
    }
}

#[cfg(test)]
#[path = "coding_security_tests.rs"]
mod security_tests;
