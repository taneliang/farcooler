//! The range a review sees, plus the agent's brief and changed-path model selection (ov-1071).
use super::*;
use super::coding::sha;
use crate::review_agent::ReviewOptions;

/// The model a review chose by its paths, written once before the reviewer starts, so every later read of the
/// launch (a settlement, a recovery) names the same one.
#[derive(Debug, Serialize, Deserialize)]
struct ChosenModel { binding: String, model: Option<String>, paths: usize }

#[derive(Serialize, Deserialize)]
struct ReviewBase { binding: String, base: String }

impl Exec {
    fn review_base_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.review-base.json", digest(&binding.effect)))
    }

    pub(super) fn review_base(&self, binding: &Binding) -> Result<Option<String>> {
        match self.bytes(&self.review_base_path(binding)) {
            Ok(raw) => {
                let pin: ReviewBase = serde_json::from_slice(&raw)?;
                if pin.binding != digest(binding) || !sha(&pin.base) {
                    return Err(Error::Refused("invalid review base receipt"));
                }
                Ok(Some(pin.base))
            }
            // Reviews launched before this pin existed still reconcile against their original binding.
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Pin the common ancestor of the candidate and the landing target before launch. The target may
    /// have advanced again after the rebase, so its tip is only an input to this calculation.
    pub(super) async fn pin_review_base(&self, binding: &Binding) -> Result<()> {
        if self.review_base(binding)?.is_some() { return Ok(()); }
        let candidate = binding.effect.candidate.as_deref().filter(|c| sha(c))
            .ok_or(Error::Refused("missing review candidate"))?;
        let head = self.landing_head_in_repository(&binding.effect.id).await?;
        let base = self.git_recorded(binding, &self.repository(),
            &["--no-replace-objects", "merge-base", "--all", &head, candidate]).await?;
        if !sha(&base) { return Err(Error::Refused("review needs a single common ancestor")); }
        let receipts = self.root.join("receipts");
        let mut draft = tempfile::NamedTempFile::new_in(&receipts)?;
        serde_json::to_writer(draft.as_file_mut(), &ReviewBase { binding: digest(binding), base })?;
        draft.as_file().sync_all()?;
        draft.persist_noclobber(self.review_base_path(binding)).map_err(|error| error.error)?;
        std::fs::File::open(receipts)?.sync_all()?;
        Ok(())
    }

    /// A deterministic child operation owns its own process, log and receipt. It is derived only from
    /// the admitted review's pinned options, and never admitted as a separate engine effect.
    pub(super) fn review_check(&self, binding: &Binding) -> Option<Binding> {
        let index = Self::review_options(binding)?.check?;
        let mut check = binding.clone();
        check.effect.id = format!("{}-check", binding.effect.id);
        check.effect.kind = EffectKind::Execute(StageKind::Run);
        check.effect.operation = self.coding.as_ref()?.operation("coding-check/v1", &index.to_string());
        Some(check)
    }

    /// The options an agent's review gate pinned, if it pinned any.
    pub(super) fn review_options(binding: &Binding) -> Option<ReviewOptions> {
        if !matches!(binding.effect.kind, EffectKind::Execute(StageKind::Review(Reviewer::Agent))) { return None; }
        binding.effect.operation.arguments.get(2).and_then(|a| ReviewOptions::read(a))
    }
    fn chosen_model_path(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.review-model.json", digest(&binding.effect)))
    }
    /// The model this step's agent runs with: for a review whose options choose by path, the one chosen before it
    /// started; otherwise the run's pick at Start (ov-966). `None` is the configured preset.
    pub(super) fn run_model(&self, binding: &Binding) -> Option<String> {
        if Self::review_options(binding).is_some_and(|o| !o.models.is_empty()) {
            let chosen = self.bytes(&self.chosen_model_path(binding)).ok()
                .and_then(|raw| serde_json::from_slice::<ChosenModel>(&raw).ok())
                .filter(|c| c.binding == digest(binding));
            if let Some(chosen) = chosen { return chosen.model; }
        }
        binding.effect.model.clone()
    }
    /// Before a review whose options choose by path starts: read the paths the ticket changed, from its base to the
    /// candidate under review, and write down the model they earn. The earliest rule any file takes wins; with none,
    /// the run's pick stands. Written once: a retry of the same effect keeps the first answer.
    pub(super) async fn choose_review_model(&self, binding: &Binding, tree: &Path) -> Result<()> {
        let Some(options) = Self::review_options(binding).filter(|o| !o.models.is_empty()) else { return Ok(()) };
        let path = self.chosen_model_path(binding);
        if path.try_exists()? { return Ok(()); }
        let candidate = binding.effect.candidate.clone().filter(|c| sha(c)).ok_or(Error::Refused("missing candidate"))?;
        let base = self.review_base(binding)?.unwrap_or_else(|| binding.base.clone());
        let changed = self.git_recorded(binding, tree, &["-c", "core.quotepath=off", "diff", "--name-only", "--no-renames", "-z", &base, &candidate]).await?;
        let paths: Vec<String> = changed.split('\0').filter(|p| !p.is_empty()).map(String::from).collect();
        let model = options.model_for(&paths).map(String::from).or_else(|| binding.effect.model.clone());
        durable(&path, &serde_json::to_vec(&ChosenModel { binding: digest(binding), model, paths: paths.len() })?)
    }
    /// An agent gate's brief, and the workflow's own words for its reviewer after it.
    pub(super) fn review_prompt(&self, binding: &Binding) -> Result<String> {
        let mut brief = Self::reviewing(&binding.effect.item);
        if let Some(base) = self.review_base(binding)? {
            let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing review candidate"))?;
            brief.push_str(&format!("\n\nThe engine gave you review range `{base}..{candidate}`. Include this range in a rejection summary."));
        }
        Ok(match Self::review_options(binding).and_then(|o| o.prompt) {
            Some(prompt) => format!("{brief}\n\n{prompt}"),
            None => brief,
        })
    }
}
