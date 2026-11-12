//! Rerunning CI is a single-send verb. Recovery reads the provider's attempt and never resends.
use super::*;

impl Exec {
    async fn ci_gh(&self, binding: &Binding, tag: &str, args: &[&str]) -> Result<(Option<i32>, Vec<u8>)> {
        let ci = &self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.ci;
        let gh = ci.command.environment.get("CI_GH").filter(|p| Path::new(p).is_absolute())
            .ok_or(Error::Refused("missing pinned CI_GH"))?;
        let argv: Vec<String> = std::iter::once(gh.clone()).chain(args.iter().map(|a| a.to_string())).collect();
        let log = self.root.join("receipts").join(format!("{}.ci-{tag}-{}.output", digest(&binding.effect), Uuid::now_v7()));
        durable(&log, b"")?;
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        let exit = crate::effect_runner::configured(&self.repository(), &argv, &ci.command.environment, &log,
            Duration::from_millis(ci.command.timeout_ms), cancel).await?;
        Ok((exit, self.bytes(&log)?))
    }
    /// Only a newer attempt of the exact run and commit confirms that the single send took effect.
    async fn ci_attempt_advanced(&self, binding: &Binding) -> Result<bool> {
        let retry = binding.effect.ci_retry.as_ref().filter(|r| r.pending && r.run.valid())
            .ok_or(Error::Refused("missing CI rerun facts"))?;
        let ci = &self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.ci;
        let endpoint = format!("repos/{}/actions/runs/{}", ci.repository, retry.run.id);
        let (exit, raw) = self.ci_gh(binding, "read", &["api", "-X", "GET", &endpoint]).await?;
        if exit != Some(0) { return Ok(false); }
        let value: serde_json::Value = serde_json::from_slice(&raw)?;
        let run = crate::ci_watch::parse_run(&raw).ok_or(Error::Refused("invalid CI rerun observation"))?;
        if run.id != retry.run.id || Some(&run.head_sha) != binding.effect.candidate.as_ref()
            || run.name != ci.workflow || run.event != ci.event
        { return Err(Error::Refused("foreign CI rerun observation")); }
        Ok(value["run_attempt"].as_u64().is_some_and(|a| a > u64::from(retry.run.attempt)))
    }
    pub(super) async fn reconcile_ci_rerun(&self, binding: &Binding) -> Result<Outcome> {
        if !self.ci_attempt_advanced(binding).await? { return Ok(Outcome::Uncertain); }
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing CI candidate"))?;
        self.write_fact(binding, "passed", candidate, serde_json::json!({ "rerun": binding.effect.ci_retry }))?;
        Box::pin(self.reconcile_verb(binding, &StageKind::RerunCi)).await
    }
    pub(super) async fn rerun_ci(&self, binding: &Binding, cancel: tokio::sync::watch::Receiver<bool>) -> Result<Outcome> {
        if self.verb_path(binding).try_exists()? { return self.reconcile_verb(binding, &StageKind::RerunCi).await; }
        let retry = binding.effect.ci_retry.as_ref().filter(|r| r.pending && r.run.valid())
            .ok_or(Error::Refused("missing CI rerun facts"))?;
        let intent = self.root.join("receipts").join(format!("{}.ci-send.json", digest(&binding.effect)));
        // Create-new and fsync before sending: even re-entry into this function never sends twice.
        if intent.try_exists()? { return self.reconcile_ci_rerun(binding).await; }
        if *cancel.borrow() { return Ok(Outcome::Uncertain); }
        if self.ci_attempt_advanced(binding).await? {
            return self.reconcile_ci_rerun(binding).await;
        }
        durable(&intent, &serde_json::to_vec(binding)?)?;
        let ci = &self.coding.as_ref().ok_or(Error::Refused("missing coding config"))?.ci;
        let (exit, _) = self.ci_gh(binding, "send", &["run", "rerun", &retry.run.id.to_string(), "--failed", "--repo", &ci.repository]).await?;
        if exit != Some(0) { return self.reconcile_ci_rerun(binding).await; }
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing CI candidate"))?;
        self.write_fact(binding, "passed", candidate, serde_json::json!({ "rerun": retry }))?;
        self.reconcile_verb(binding, &StageKind::RerunCi).await
    }
}
