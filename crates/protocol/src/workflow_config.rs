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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CodingHarness {
    Command(CodingCommand),
    /// Existing native launch policy, never prepared/installed by this runtime.
    Native { preset: String, settings: Option<LaunchFile>, project_hooks: Option<LaunchFile>, plugin_dir: Option<PathBuf>,
        prompt: String, process: CodingCommand },
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
}

/// Explicit local transport identity; no named remote lookup or network URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingLanding {
    pub job: String,
    pub repository: PathBuf,
    pub target: String,
    pub config_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingConfig {
    pub repository: PathBuf,
    pub base: String,
    pub landing_ref: String,
    #[serde(default)]
    pub landing: Option<CodingLanding>,
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
pub enum NoticePoint { WaitingApproval, WaitingReview, Blocked, Uncertain, Done, Failed, Cancelled, StageChanged }
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
