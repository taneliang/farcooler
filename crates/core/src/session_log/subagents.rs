//! Following the transcripts a claude session's subagents write beside it.
//!
//! Claude Code writes each subagent's calls to its own file,
//! `<project>/<session>/subagents/agent-<agent>.jsonl`, next to the session's
//! `<project>/<session>.jsonl`, and none of them into the session's file
//! (every record there is `isSidechain: false`; every one here is `true`). A
//! pane's spend that reads only the session file leaves its subagents out,
//! so the pane's follower tails these too, for spend only: what a subagent
//! did is already summarized in the parent's own tool result.

use std::collections::BTreeMap;
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
            for line in tail.read_new_lines() {
                usage.subagent_line(agent, &line);
            }
        }
    }
}
