//! What an agent's review gate may pin besides its identity (ov-1071): words of the workflow's own for the reviewer,
//! and the model it runs with by the paths the ticket changed.
//!
//! The options are the step's third argument, so a launch reads them from its effect alone and a recovery rebuilds the
//! same command. A step that pins none reviews as before: the engine's brief, with the model the run picked at Start.
use farcooler_core::charter_landing::glob_matches;
use serde::{Deserialize, Serialize};

/// The most model rules one gate carries.
pub const MAX_MODEL_RULES: usize = 32;
/// The longest a reviewer's prompt may be: what a build's may.
pub const MAX_PROMPT_BYTES: usize = 4096;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewOptions {
    /// Said to the reviewer after the engine's brief: how this workspace reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// `(glob, model)`, first match wins per file. A ticket that changes a file a rule names runs with that rule's
    /// model; the earliest rule any file matches decides, so the list is in order of care. No match keeps the run's pick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<(String, String)>,
}
impl ReviewOptions {
    /// The text a step pins as its third argument.
    pub fn argument(&self) -> String { serde_json::to_string(self).expect("options serialize") }
    /// Options a step may pin: readable and bounded, with plain globs and models. Whether each model is one the step
    /// lists is the definition's to check.
    pub fn read(argument: &str) -> Option<Self> {
        let options: Self = serde_json::from_str(argument).ok()?;
        let sound = options.models.len() <= MAX_MODEL_RULES
            && options.prompt.as_ref().is_none_or(|p| !p.trim().is_empty() && p.len() <= MAX_PROMPT_BYTES && !p.contains('\0'))
            && options.models.iter().all(|(glob, model)| !glob.is_empty() && glob.len() <= 256 && !glob.starts_with('/')
                && farcooler_workflow::valid_model(model));
        (sound && options != Self::default()).then_some(options)
    }
    /// The model a ticket that changes `paths` reviews with: the earliest rule that any of its files takes, or `None`.
    pub fn model_for(&self, paths: &[String]) -> Option<&str> {
        let first = |path: &String| self.models.iter().position(|(glob, _)| glob_matches(glob, path));
        paths.iter().filter_map(first).min().map(|i| self.models[i].1.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> ReviewOptions {
        ReviewOptions { prompt: Some("Read the lane rules.".into()), models: vec![
            ("crates/workflow/**".into(), "claude:opus".into()), ("crates/daemon/src/auth/**".into(), "claude:opus".into()),
            ("apps/**".into(), "claude:sonnet".into())] }
    }
    fn paths(list: &[&str]) -> Vec<String> { list.iter().map(|p| p.to_string()).collect() }

    #[test]
    fn the_earliest_rule_any_changed_file_takes_decides_the_model() {
        let o = options();
        assert_eq!(o.model_for(&paths(&["apps/web/a.ts", "crates/workflow/src/x.rs"])), Some("claude:opus"));
        assert_eq!(o.model_for(&paths(&["apps/web/a.ts", "docs/b.md"])), Some("claude:sonnet"));
        assert_eq!(o.model_for(&paths(&["docs/b.md"])), None, "no rule keeps the run's pick");
        assert_eq!(o.model_for(&[]), None);
    }
    #[test]
    fn the_pinned_text_round_trips_and_a_bad_one_is_refused() {
        let o = options();
        assert_eq!(ReviewOptions::read(&o.argument()), Some(o));
        let many = ReviewOptions { prompt: None, models: (0..=MAX_MODEL_RULES).map(|i| (format!("d{i}/**"), "claude".into())).collect() };
        let long = ReviewOptions { prompt: Some("x".repeat(MAX_PROMPT_BYTES + 1)), models: vec![] };
        for bad in ["", "agent", "{}", r#"{"prompt":"  "}"#, r#"{"models":[["","claude"]]}"#, r#"{"models":[["/abs/**","claude"]]}"#,
            r#"{"models":[["a/**","claude opus"]]}"#, r#"{"prompt":"p","extra":1}"#, &many.argument(), &long.argument()] {
            assert_eq!(ReviewOptions::read(bad), None, "{bad}");
        }
    }
}
