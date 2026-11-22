//! Versioned trusted-local coding configuration, shared by CLI and daemon.
use std::{collections::BTreeMap, path::PathBuf};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCommand {
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchFile { pub path: PathBuf, pub digest: String }

// One per configuration, read once per publication: the size of the native arm costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CodingHarness {
    Command(CodingCommand),
    /// Existing native launch policy, never prepared/installed by this runtime.
    Native { preset: String, settings: Option<LaunchFile>, project_hooks: Option<LaunchFile>, plugin_dir: Option<PathBuf>,
        prompt: String, process: CodingCommand,
        /// Codex's hooks, from a file the runner keeps outside the repository, as Claude's `settings` are (ov-1088).
        /// The engine copies it into `.codex/hooks.json` in each worktree it launches Codex in, so no repository
        /// commits a file that names this machine's paths. Left out of the pinned bytes when absent, so
        /// configurations written before it keep their pin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        codex_hooks: Option<LaunchFile> },
}

/// The hooks file a native agent runs with, and how it reaches the agent (ov-1206).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AgentHooks<'a> {
    /// Claude's `--settings`, which carry its hooks.
    Settings(&'a LaunchFile),
    /// Codex's hooks, which the engine copies into the worktree (ov-1088).
    Codex(&'a LaunchFile),
    /// The repository's own hooks file, for any agent but Claude.
    Project(&'a LaunchFile),
}
impl CodingHarness {
    /// The hooks `agent` (a model's harness: `claude`, `codex`, …) runs with under this harness: what the engine
    /// verifies and hands it at launch, and so what a change of model can start running. `None` for a command
    /// harness, which runs its command whatever the model.
    pub fn agent_hooks(&self, agent: &str) -> Option<AgentHooks<'_>> {
        let CodingHarness::Native { settings, project_hooks, codex_hooks, .. } = self else { return None };
        match agent {
            "claude" => settings.as_ref().map(AgentHooks::Settings),
            "codex" => codex_hooks.as_ref().map(AgentHooks::Codex).or(project_hooks.as_ref().map(AgentHooks::Project)),
            _ => project_hooks.as_ref().map(AgentHooks::Project),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingReviewer { pub identity: String, pub command: CodingCommand }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCi {
    pub repository: String,
    pub workflow: String,
    pub event: String,
    pub command: CodingCommand,
    /// What the base step does when the target's head has no CI run at all (ov-1243). Left out of the pinned
    /// bytes when absent, so configurations written before it keep their pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unrun: Option<CiUnrun>,
}

/// How long a target head may sit without any CI run before the base step stops waiting for one, and which
/// paths a commit may touch for the step to take the verdict of the nearest ancestor that has one: the paths the
/// CI workflow ignores on push, so that a commit touching only them never starts a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiUnrun {
    /// Milliseconds with no run on the head before the step acts. Left out, ten minutes.
    #[serde(default = "CiUnrun::default_grace_ms")]
    pub grace_ms: u64,
    /// Exact paths, or a directory written with a trailing `/`, that CI ignores. Empty: no commit is skippable.
    #[serde(default)]
    pub ignored: Vec<String>,
}
impl CiUnrun {
    pub const DEFAULT_GRACE_MS: u64 = 600_000;
    fn default_grace_ms() -> u64 { Self::DEFAULT_GRACE_MS }
    /// Whether CI ignores `path`.
    pub fn ignores(&self, path: &str) -> bool {
        self.ignored.iter().any(|i| if i.ends_with('/') { path.starts_with(i.as_str()) } else { path == i })
    }
}

/// Where code lands: an explicit transport identity, never a named remote looked up in a config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingLanding {
    pub job: String,
    /// A local bare repository's path, or an `ssh://` or `https://` URL the engine fetches from and pushes
    /// to with the runner user's own git and ssh setup (ov-1026). Written as a path so pins made before
    /// remotes keep their bytes.
    pub repository: PathBuf,
    pub target: String,
    /// The digest of a local bare repository's config, which fixes where it pushes. Empty for a URL.
    pub config_digest: String,
}

/// Where the engine opens pull requests, and the `gh` it opens them with (ov-957). `gh` is one absolute
/// path; the engine adds the arguments, and the environment is exactly what is written here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingPullRequest {
    /// `owner/name` on the host `gh` is signed in to.
    pub repository: String,
    /// The branch pull requests are opened against.
    pub base: String,
    /// Where head branches are pushed and read back: the repository the pull requests live in, as a
    /// bare repository path, an `https://` or an `ssh://` URL. Left out, it is the landing transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    pub gh: CodingCommand,
}

/// A full-review gate approved in adapter configuration, independent of route labels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityReview {
    pub step: String,
    pub models: Vec<String>,
    /// Exact review options, including the prompt and any path-dependent model rules.
    pub options: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingConfig {
    /// Trusted full-review sequence. Definitions may skip it only after a light diff classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_reviews: Vec<SecurityReview>,
    pub repository: PathBuf,
    pub base: String,
    pub landing_ref: String,
    #[serde(default)]
    pub landing: Option<CodingLanding>,
    /// Left out of the pinned bytes when absent, so configurations written before pull requests keep their pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<CodingPullRequest>,
    pub harness: CodingHarness,
    pub checks: Vec<CodingCommand>,
    pub reviewer: Option<CodingReviewer>,
    pub ci: CodingCi,
}

impl CodingConfig { pub fn pin(&self) -> String { farcooler_workflow::digest(self) } }
/// Adapter versions are closed; publication never conveys an authority grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version", content = "config", deny_unknown_fields)]
pub enum AdapterConfiguration { #[serde(rename = "coding/v1")] Coding(CodingConfig) }
impl AdapterConfiguration {
    pub fn pin(&self) -> String { farcooler_workflow::digest(self) }
}

/// Finite informational key points. Never an execution or decision grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticePoint { WaitingApproval, WaitingReview, Blocked, Uncertain, Done, Failed, Cancelled, StageChanged, PublicationRestarted }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationPolicy { pub points: std::collections::BTreeSet<NoticePoint> }
// Omission permanently selects this v1 default for legacy immutable bundles.
// New defaults require a new version; never retarget already admitted items.
impl Default for NotificationPolicy {
    fn default() -> Self {
        use NoticePoint::*;
        Self { points: [WaitingApproval, WaitingReview, Uncertain, Done, Failed, Cancelled].into() }
    }
}
impl NotificationPolicy {
    pub fn pin(&self) -> String { farcooler_workflow::digest(&("workflow-notifications/v1", self)) }
}

#[cfg(test)]
mod security_review_tests {
    use super::*;

    #[test]
    fn legacy_adapter_bytes_keep_their_pin_and_security_policy_changes_it() {
        let command = serde_json::json!({"argv":["/bin/true"],"environment":{},"timeout_ms":1000});
        let legacy = serde_json::json!({"repository":"/repo","base":"a".repeat(40),"landing_ref":"refs/heads/main",
            "landing":null,"harness":{"Command":command},"checks":[],"reviewer":null,
            "ci":{"repository":"owner/repo","workflow":"CI","event":"push","command":command}});
        let mut config: CodingConfig = serde_json::from_value(legacy.clone()).unwrap();
        assert!(config.security_reviews.is_empty());
        assert_eq!(serde_json::to_value(&config).unwrap(), legacy);
        let legacy_pin = config.pin();
        config.security_reviews.push(SecurityReview { step: "full".into(), models: vec!["claude:opus".into()], options: None });
        assert_ne!(config.pin(), legacy_pin);
        let pinned = config.pin();
        assert_eq!(serde_json::from_slice::<CodingConfig>(&serde_json::to_vec(&config).unwrap()).unwrap().pin(), pinned);
        config.security_reviews[0].models = vec!["claude:sonnet".into()];
        assert_ne!(config.pin(), pinned);
    }
}
