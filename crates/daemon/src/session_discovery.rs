//! Whether a conversation has anything on disk to resume.
//!
//! `claude --resume` and `codex resume` both fail on a session that never
//! completed a turn, so a pane restarted onto one starts fresh instead
//! (`service::respawn_command`).

use std::path::Path;

/// Claude Code's project-directory name for a working directory.
///
/// Every non-alphanumeric character becomes a dash, which is why an absolute
/// path gains a leading one and a dotfile directory gains a double.
pub fn project_dir_name(worktree: &Path) -> String {
    worktree
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether a session has anything on disk to resume.
///
/// Claude Code writes a transcript only once a turn has happened, so a session
/// that exists may not exist on disk at all. `claude --resume` answers "No
/// conversation found with session ID" for those.
pub fn transcript_exists(home: &Path, worktree: &Path, session_id: &str) -> bool {
    let resolved = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    home.join(".claude/projects")
        .join(project_dir_name(&resolved))
        .join(format!("{session_id}.jsonl"))
        .exists()
}

/// The exact identical guard as `transcript_exists`, for codex.
///
/// Verified end to end, not assumed: after a real turn in an ACP session with
/// id `019fc8af-daff-7692-b7be-4457fda0b01c`, `@agentclientprotocol/codex-acp`
/// had written
/// `~/.codex/sessions/2026/08/03/rollout-2026-08-03T10-33-15-019fc8af-daff-7692-b7be-4457fda0b01c.jsonl`,
/// and `codex resume 019fc8af-daff-7692-b7be-4457fda0b01c` in that session's
/// worktree restored the conversation. A session with no completed turn — the
/// same ordinary case `transcript_exists` exists for — writes no rollout at
/// all, and `codex resume` on an id with nothing behind it fails with an error
/// a user cannot act on, same as `claude --resume` does.
///
/// A recursive search under `<home>/.codex/sessions`, not a direct path build:
/// the filename embeds the timestamp of when the session STARTED, which
/// nothing on this side of the connection ever learns, so the date directory
/// cannot be computed — only searched for.
pub fn codex_rollout_exists(home: &Path, session_id: &str) -> bool {
    let root = home.join(".codex/sessions");
    let suffix = format!("-{session_id}.jsonl");
    find_rollout(&root, &suffix, 0)
}

/// Deep enough for codex's documented `YYYY/MM/DD` nesting with room to
/// spare, shallow enough that a stray symlink loop under `~/.codex` cannot
/// turn a resumability check into a hang.
const MAX_ROLLOUT_SEARCH_DEPTH: u32 = 6;

fn find_rollout(dir: &Path, suffix: &str, depth: u32) -> bool {
    if depth > MAX_ROLLOUT_SEARCH_DEPTH {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        // A file type that cannot be read is skipped, not assumed either way.
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if find_rollout(&path, suffix, depth + 1) {
                return true;
            }
        } else if file_type.is_file() {
            let matches = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(suffix));
            if matches {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;


    #[test]
    fn a_worktree_path_munges_the_way_claude_munges_it() {
        // Read off a real machine: every non-alphanumeric becomes a dash, and
        // the leading slash produces a leading dash.
        assert_eq!(project_dir_name(Path::new("/Users/e/Dev/farcooler")), "-Users-e-Dev-farcooler");
        assert_eq!(project_dir_name(Path::new("/Users/e/.claude/jobs")), "-Users-e--claude-jobs");
    }

    fn scratch(name: &str) -> PathBuf {
        farcooler_core::scratch::dir(&format!("disc-{name}"))
    }

    /// The refusal has to SAY what it found.
    ///
    /// This module's header promises that an ambiguous lookup "refuses and
    /// names what it found", and the names have always been on the error --
    /// but the caller logs the `Display` form and nothing else, so for as long
    /// as the message omitted them, the load-bearing refusal was the one event
    /// in this path that could not be audited after the fact. A candidate list
    /// that reaches a struct field nobody prints is not a diagnostic.
    #[test]
    fn a_session_with_no_turn_yet_has_nothing_to_resume() {
        // The exact failure a user hit: open a chat, switch straight back
        // without saying anything, and `claude --resume` answers "No
        // conversation found with session ID". The session is real; the
        // transcript is not written until a turn happens.
        let home = scratch("no-transcript");
        let worktree = Path::new("/Users/e/Dev/fresh");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();

        assert!(!transcript_exists(&home, worktree, "never-spoke"));

        std::fs::write(dir.join("has-spoken.jsonl"), "{}").unwrap();
        assert!(transcript_exists(&home, worktree, "has-spoken"));
    }

    #[test]
    fn a_codex_session_with_no_turn_yet_has_no_rollout_to_resume() {
        // Codex's version of the claude bug above: a session id is declared
        // and real, but `@agentclientprotocol/codex-acp` writes nothing under
        // `~/.codex/sessions` until a turn actually happens.
        let home = scratch("codex-no-rollout");
        std::fs::create_dir_all(home.join(".codex/sessions/2026/08/03")).unwrap();

        assert!(!codex_rollout_exists(&home, "019fc8af-daff-7692-b7be-4457fda0b01c"));
    }

    #[test]
    fn a_codex_rollout_is_found_nested_under_its_date_directory() {
        // The exact shape verified on a real machine: a session id with a
        // completed turn has a file named `rollout-<timestamp>-<uuid>.jsonl`
        // three directories deep, and nothing on this side of the connection
        // knows which date directory without looking.
        let home = scratch("codex-rollout");
        let id = "019fc8af-daff-7692-b7be-4457fda0b01c";
        let dir = home.join(".codex/sessions/2026/08/03");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("rollout-2026-08-03T10-33-15-{id}.jsonl")), "{}").unwrap();

        assert!(codex_rollout_exists(&home, id));
        // A different id must not match merely because it shares a directory
        // with one that does.
        assert!(!codex_rollout_exists(&home, "00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn a_missing_codex_sessions_directory_is_simply_not_resumable() {
        // No `.codex/sessions` at all — codex has never run on this runner,
        // or `$HOME` in the test is a bare scratch directory. Either way this
        // must report false rather than error: the caller's fallback (start
        // clean) is exactly the right behavior for "nothing to find".
        let home = scratch("codex-no-dir");
        assert!(!codex_rollout_exists(&home, "019fc8af-daff-7692-b7be-4457fda0b01c"));
    }
}
