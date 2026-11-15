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
    /// A pinned check index to run alongside this review. Both must pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<usize>,
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
        ReviewOptions { prompt: Some("Read the lane rules.".into()), check: None, models: vec![
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
        let many = ReviewOptions { prompt: None, check: None, models: (0..=MAX_MODEL_RULES).map(|i| (format!("d{i}/**"), "claude".into())).collect() };
        let long = ReviewOptions { prompt: Some("x".repeat(MAX_PROMPT_BYTES + 1)), check: None, models: vec![] };
        for bad in ["", "agent", "{}", r#"{"prompt":"  "}"#, r#"{"models":[["","claude"]]}"#, r#"{"models":[["/abs/**","claude"]]}"#,
            r#"{"models":[["a/**","claude opus"]]}"#, r#"{"prompt":"p","extra":1}"#, &many.argument(), &long.argument()] {
            assert_eq!(ReviewOptions::read(bad), None, "{bad}");
        }
    }
}

/// Far Cooler's mandatory review floor. Protect whole implementation trees, including tests:
/// a filename heuristic cannot tell a UI change from credential handling or a trust-boundary change.
/// Definition globs may extend this policy, but an omission must never weaken it (ov-1194).
const SECURITY_REVIEW_PATHS: &[&str] = &[
    "apps/**", "crates/**", "services/**", "proto/**", "scripts/**",
    ".github/**", ".cargo/**", ".config/**", ".claude/**", ".codex/**", ".agents/**",
    "Cargo.toml", "Cargo.lock", "Cross.toml", "typeshare.toml", ".gitmodules", ".gitattributes",
];

/// A Route step may choose `light` or `full` before entering distinct review steps (ov-1194).
/// Security rules take precedence over both size and report paths. Binary changes always get full review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRoute {
    pub under_lines: u64,
    pub report_paths: Vec<String>,
    pub full_paths: Vec<String>,
}
impl ReviewRoute {
    pub fn read(text: &str) -> Option<Self> {
        let rules: Self = serde_json::from_str(text).ok()?;
        let valid = |paths: &[String]| paths.len() <= MAX_MODEL_RULES && paths.iter().all(|p|
            !p.is_empty() && p.len() <= 256 && !p.starts_with('/') && !p.contains('\0'));
        (rules.under_lines > 0 && !rules.full_paths.is_empty() && valid(&rules.report_paths) && valid(&rules.full_paths)).then_some(rules)
    }
    /// Numstat with NUL delimiters, no renames: each record is additions, deletions, path.
    /// Anything unreadable is an error rather than permission to reduce review.
    pub fn route(&self, numstat: &str) -> Option<&'static str> {
        if !numstat.is_empty() && !numstat.ends_with('\0') { return None; }
        let mut lines = 0u64;
        let mut reports = true;
        let mut full = false;
        for record in numstat.split('\0').filter(|r| !r.is_empty()) {
            let mut fields = record.splitn(3, '\t');
            let (added, removed, path) = (fields.next()?, fields.next()?, fields.next()?);
            if path.is_empty() { return None; }
            full |= path.contains('\u{fffd}') || SECURITY_REVIEW_PATHS.iter().any(|g| glob_matches(g, path))
                || self.full_paths.iter().any(|g| glob_matches(g, path));
            reports &= self.report_paths.iter().any(|g| glob_matches(g, path));
            if added == "-" || removed == "-" { full = true; continue; }
            lines = lines.checked_add(added.parse::<u64>().ok()?)?.checked_add(removed.parse::<u64>().ok()?)?;
        }
        Some(if !full && (lines < self.under_lines || reports) { "light" } else { "full" })
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    #[test]
    fn repository_security_policy_cannot_be_omitted_or_overridden_by_report_rules() {
        // Both an incomplete definition and the published example must preserve the security floor.
        let docs = include_str!("../../../docs/light-review.md");
        let documented = docs.split("```json\n").nth(1).unwrap().split("\n```").next().unwrap();
        let omitted = r#"{"under_lines":100,"report_paths":["**"],"full_paths":["unrelated/**"]}"#;
        for argument in [omitted, documented] {
            let rules = ReviewRoute::read(argument).unwrap();
            for path in [
                "apps/shared/AgentKit/Tests/AgentKitTests/AccountKeychainWriteTests.swift",
                "apps/shared/AgentKit/Sources/AgentKit/AccountKeychain.swift",
                "apps/android/app/src/main/java/com/farcooler/Account.kt",
                "crates/core/src/auth.rs", "crates/cli/src/main.rs", "crates/daemon/src/review_agent.rs",
                "crates/workflow/src/lib.rs", "services/relay/src/index.ts", "services/web/src/auth.ts",
                "proto/farcooler.proto", "scripts/release.sh", ".github/workflows/ci.yml",
                ".cargo/config.toml", ".codex/hooks.json", "Cargo.toml", "Cargo.lock",
            ] {
                assert_eq!(rules.route(&format!("1\t0\t{path}\0")), Some("full"), "{argument}: {path}");
            }
            assert_eq!(rules.route("1\t0\tdocs/guide.md\0"), Some("light"));
            assert_eq!(rules.route("1\t0\tREADME.md\0"), Some("light"));
        }
    }

    #[test]
    fn light_review_size_reports_security_and_binary_precedence() {
        let r = ReviewRoute { under_lines: 10, report_paths: vec!["reports/**".into()], full_paths: vec!["private/**".into(), "**/auth*".into(), "apps/Security.swift".into()] };
        for (stat, expected) in [("4\t5\tnotes/ui.txt\0", "light"), ("5\t5\tnotes/ui.txt\0", "full"),
            ("100\t0\treports/findings.md\0", "light"), ("100\t0\treports/a.md\x000\t1\tnotes/ui.txt\0", "full"),
            ("1\t0\tcrates/daemon/src/auth.rs\0", "full"), ("1\t0\tprivate/a\0", "full"),
            ("1\t0\treports/auth.md\0", "full"), ("1\t0\tapps/Security.swift\0", "full"), ("-\t-\treports/image.png\0", "full"), ("", "light")] {
            assert_eq!(r.route(stat), Some(expected), "{stat:?}");
        }
        for bad in ["bad", "1\t0\ta", "1\t0\t\0", "1\tx\ta\0", "18446744073709551615\t1\ta\0"] { assert_eq!(r.route(bad), None); }
        assert!(ReviewRoute::read(r#"{"under_lines":10,"report_paths":["reports/**"],"full_paths":["auth/**"]}"#).is_some());
        for bad in [r#"{"under_lines":10,"report_paths":[],"full_paths":[]}"#, r#"{"under_lines":0,"report_paths":[],"full_paths":["auth/**"]}"#,
            r#"{"under_lines":10,"report_paths":["/abs"],"full_paths":["auth/**"]}"#] { assert!(ReviewRoute::read(bad).is_none()); }
    }
}
