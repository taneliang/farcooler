//! What every agent's hook payload can be asked, whatever else it carries.
//!
//! Three questions, and all three agents answer all three — which is the only
//! reason one design covers claude, codex and cursor at once. They do not
//! answer them the same way: cursor has no `cwd` and reports
//! `workspace_roots` instead, which is the kind of difference that is found in
//! production rather than in review unless it is pinned here.

use std::path::{Path, PathBuf};

use crate::Agent;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The join key onto `Terminal.agent_session_id`.
    pub session_id: Option<String>,
    /// The worktree this session is in, for binding a session nobody declared.
    pub cwd: Option<PathBuf>,
    /// Where this conversation is written down.
    ///
    /// The agent tells us outright, which is what `session_discovery` exists
    /// to guess. For codex and cursor it is also the only source of prose.
    pub transcript_path: Option<PathBuf>,
}

fn string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload.get(key)?.as_str().map(str::to_string)
}

pub fn facts(agent: Agent, payload: &serde_json::Value) -> Facts {
    let cwd = match agent {
        // Cursor sends no `cwd`. The first workspace root is the worktree; the
        // rest are `--add-dir` extras and are not what a terminal is keyed by.
        Agent::Cursor => payload
            .get("workspace_roots")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(PathBuf::from),
        Agent::Claude | Agent::Codex => string(payload, "cwd").map(PathBuf::from),
    };

    Facts {
        session_id: string(payload, "session_id"),
        cwd,
        transcript_path: string(payload, "transcript_path").map(PathBuf::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Agent;

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(&path).expect(&path)).expect("valid json")
    }

    #[test]
    fn claude_names_its_session_its_worktree_and_its_transcript() {
        let f = facts(Agent::Claude, &fixture("claude-message-display"));
        assert_eq!(f.session_id.as_deref(), Some("7ecba53c-31c6-4c8e-9704-b2d582039d04"));
        assert_eq!(f.cwd.as_deref(), Some(Path::new("/tmp/probe")));
        assert!(
            f.transcript_path.is_some_and(|p| p.ends_with(
                "7ecba53c-31c6-4c8e-9704-b2d582039d04.jsonl"
            )),
            "the transcript path is what retires session_discovery's guessing"
        );
    }

    #[test]
    fn codex_answers_the_same_three_questions() {
        let f = facts(Agent::Codex, &fixture("codex-session-start"));
        assert_eq!(f.session_id.as_deref(), Some("01a07e3a-1ec5-70f1-86f1-f3677d7dfb06"));
        assert_eq!(f.cwd.as_deref(), Some(Path::new("/tmp/probe-codex")));
        assert!(f.transcript_path.is_some(), "codex carries its rollout path");
    }

    /// The asymmetry that would otherwise be found in production.
    ///
    /// Cursor sends `workspace_roots`, an ARRAY, and no `cwd` at all. A reader
    /// written against claude and codex reads `cwd`, finds nothing, and every
    /// cursor session fails to bind to a terminal — silently, because an
    /// unbound session is a legitimate state.
    #[test]
    fn cursor_has_no_cwd_and_its_worktree_is_the_first_workspace_root() {
        let f = facts(Agent::Cursor, &fixture("cursor-stop"));
        assert_eq!(f.session_id.as_deref(), Some("68d64051-6ab2-4668-97a1-a45df658bada"));
        assert_eq!(
            f.cwd.as_deref(),
            Some(Path::new("/tmp/probe-cursor")),
            "cursor's worktree comes from workspace_roots[0], not from cwd"
        );
    }

    #[test]
    fn a_payload_missing_everything_is_facts_with_nothing_in_it() {
        let f = facts(Agent::Claude, &serde_json::json!({}));
        assert_eq!(f.session_id, None);
        assert_eq!(f.cwd, None);
        assert_eq!(f.transcript_path, None);
    }
}
