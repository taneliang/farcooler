//! Following the transcripts a claude session's subagents write beside it.
//!
//! Claude Code writes each subagent's calls to its own file,
//! `<project>/<session>/subagents/agent-<agent>.jsonl`, next to the session's
//! `<project>/<session>.jsonl`, and none of them into the session's file
//! (every record there is `isSidechain: false`; every one here is `true`). A
//! pane's spend that reads only the session file leaves its subagents out,
//! so the pane's follower tails these too, for spend only: what a subagent
//! did is already summarized in the parent's own tool result.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::tail::Tail;
use super::usage::LogUsage;

/// The subagent transcripts of the session a pane follows.
#[derive(Default)]
pub struct SubagentLogs {
    /// The `subagents` directory of the session being followed.
    dir: Option<PathBuf>,
    /// By agent id.
    tails: BTreeMap<String, Tail>,
    /// The agents whose meta file has been read and said something.
    described: BTreeSet<String>,
}

impl SubagentLogs {
    /// Read whatever the subagents of `session_log` appended, into `usage`.
    ///
    /// `look` lists the directory for new subagents: worth it only when the
    /// session itself just wrote something (a subagent is started by a line
    /// in its parent), so a quiet pane costs one read per known file, not a
    /// directory listing a second. A session not followed before is always
    /// listed.
    pub fn follow(&mut self, session_log: &Path, look: bool, usage: &mut LogUsage) {
        let dir = session_log.with_extension("").join("subagents");
        let fresh = self.dir.as_deref() != Some(dir.as_path());
        if fresh {
            self.tails.clear();
            self.described.clear();
            self.dir = Some(dir.clone());
        }
        if look || fresh {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = entry.file_name();
                let Some(agent) = name.to_str().and_then(|n| n.strip_prefix("agent-")?.strip_suffix(".jsonl"))
                else {
                    continue;
                };
                self.tails.entry(agent.to_string()).or_insert_with(|| Tail::new(entry.path()));
            }
        }
        for (agent, tail) in &mut self.tails {
            let mut grew = false;
            for line in tail.read_new_lines() {
                usage.subagent_line(agent, &line);
                grew = true;
            }
            // The meta file is written beside the transcript, and may land a
            // moment after its first line: look again until it says something,
            // but only when the transcript moved or the directory was listed.
            if (grew || look || fresh) && !self.described.contains(agent) {
                if let Some(description) = description_of(&dir, agent) {
                    usage.subagent_description(agent, &description);
                    self.described.insert(agent.clone());
                }
            }
        }
    }
}

/// The description in `agent-<agent>.meta.json`: the same 3-5 word summary the
/// parent's `Agent` tool call carried as `input.description`, character for
/// character on every one of the 315 calls on the machine this was measured
/// on (`claude::spawned`). None when the file is absent, unreadable, or says
/// nothing.
fn description_of(dir: &Path, agent: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(format!("agent-{agent}.meta.json"))).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let description = value.get("description")?.as_str()?.trim();
    (!description.is_empty()).then(|| description.to_string())
}

#[cfg(test)]
#[path = "subagents_tests.rs"]
mod tests;
