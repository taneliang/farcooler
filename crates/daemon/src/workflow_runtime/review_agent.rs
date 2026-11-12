//! An agent's review gate, as its step's options shape it (ov-1071): the workflow's words for the reviewer, and the
//! model it runs with by the paths the ticket changed.
use super::*;
use super::coding::sha;
use crate::review_agent::ReviewOptions;

/// The model a review chose by its paths, written once before the reviewer starts, so every later read of the
/// launch (a settlement, a recovery) names the same one.
#[derive(Debug, Serialize, Deserialize)]
struct ChosenModel { binding: String, model: Option<String>, paths: usize }

impl Exec {
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
        let changed = self.git_recorded(binding, tree, &["-c", "core.quotepath=off", "diff", "--name-only", "--no-renames", "-z", &binding.base, &candidate]).await?;
        let paths: Vec<String> = changed.split('\0').filter(|p| !p.is_empty()).map(String::from).collect();
        let model = options.model_for(&paths).map(String::from).or_else(|| binding.effect.model.clone());
        durable(&path, &serde_json::to_vec(&ChosenModel { binding: digest(binding), model, paths: paths.len() })?)
    }
    /// An agent gate's brief, and the workflow's own words for its reviewer after it.
    pub(super) fn review_prompt(binding: &Binding) -> String {
        let brief = Self::reviewing(&binding.effect.item);
        match Self::review_options(binding).and_then(|o| o.prompt) {
            Some(prompt) => format!("{brief}\n\n{prompt}"),
            None => brief,
        }
    }
}
