//! Build-output release is separate from the stage's execution: a skip must never fail a run.
use crate::{digest, Definition, Phase, RunTree, StageKind, State};
use serde::{Deserialize, Serialize};

pub const DEFAULT_BUILD_FOLDERS: &[&str] = &[
    "target", "apps/android/app/build", "apps/android/build", "apps/ios/build",
    "apps/macos/build", "apps/shared/build", ".build",
];
impl Definition {
    pub fn build_folders(&self) -> Vec<String> {
        self.build_folders.clone().unwrap_or_else(|| DEFAULT_BUILD_FOLDERS.iter().map(|s| (*s).into()).collect())
    }
}
/// Only relative folders; a wildcard matches one directory, never an ancestor or git metadata.
pub fn valid_folder(path: &str) -> bool {
    !path.is_empty() && path.len() <= 256 && path.split('/').all(|c| !c.is_empty()
        && c != "." && c != ".." && !c.eq_ignore_ascii_case(".git") && !c.eq_ignore_ascii_case("reports") && !c.contains('\\')
        && c.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-*".contains(&b))
        && (!c.contains('*') || c == "*")) && !path.ends_with('*')
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupEffect {
    pub id: String,
    pub workspace: String,
    pub revision: u64,
    pub item: String,
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<String>,
    pub tree: Option<RunTree>,
    pub folders: Vec<String>,
    pub uncertain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<OwnedDirectory>,
}
/// Identity captured from the engine-created directory before a process can use it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedDirectory {
    pub path: String,
    pub device: u64,
    pub inode: u64,
    pub owner_effect: String,
    pub retain_until_ms: i64,
}

pub fn waiting_at_rest(phase: &Phase) -> bool {
    matches!(phase, Phase::WaitingApproval | Phase::WaitingReview | Phase::WaitingJoin | Phase::Joined)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    pub bytes_freed: u64,
    pub skipped: Vec<String>,
    /// A transient blocker prevented release; a later sweep may try again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retry: bool,
}
/// Select at the same accepted transition that releases build ownership, including crashes and ends.
/// The intent survives an item leaving live state; a later sweep never has to guess its old policy.
pub fn effects(before: &State, after: &State) -> Vec<CleanupEffect> {
    before.items.iter().filter_map(|(id, old)| {
        if old.definition.is_ticket() || old.follows.is_some() { return None; }
        let new = after.items.get(id);
        let ended = new.is_none_or(|i| matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled | Phase::Uncertain));
        // A check or reviewer may have rebuilt output after the agent left Build. Release again
        // on entry to review, join or landing so those waiting runs don't retain that output.
        let entered_wait = new.is_some_and(|i| i.definition.is_ticket()
            || (waiting_at_rest(&i.phase) && i.phase != old.phase)
            || (i.stage != old.stage && i.definition.stages.get(i.stage)
                .is_some_and(|s| matches!(s.kind, StageKind::Review(_) | StageKind::Join { .. }
                    | StageKind::Land | StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr))));
        if !(entered_wait || ended) || new.is_some_and(|i| i.phase == old.phase && i.stage == old.stage) { return None; }
        Some(CleanupEffect { id: digest(&("build-cleanup/v1", &after.workspace, after.revision, id)),
            workspace: after.workspace.clone(), revision: after.revision, item: id.clone(), run: old.run.clone(),
            restart: old.restart.clone(), tree: old.tree.clone(), folders: old.definition.build_folders(),
            uncertain: new.is_some_and(|i| i.phase == Phase::Uncertain), directory: None })
    }).collect()
}
