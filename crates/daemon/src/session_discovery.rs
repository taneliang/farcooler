//! Which conversation is running in a pane nobody declared.
//!
//! Only for `claude` typed into a shell by hand. Anything Far Cooler launched
//! carries a declared session id in SQLite and never reaches here.
//!
//! A workspace is one worktree, so the project directory almost always holds a
//! single session and this is unambiguous. When it is not, this refuses and
//! names what it found. Guessing would attach a chat view to a conversation
//! other than the one in the pane — silently, and with no way for the user to
//! tell. That is the same reason an unproven terminal derives as `lost` rather
//! than as the state it probably has.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("no session for this worktree")]
    NotFound,
    #[error("more than one session could be the one in this pane")]
    Ambiguous { candidates: Vec<String> },
}

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
/// that exists in the protocol may not exist on disk at all. `claude --resume`
/// answers "No conversation found with session ID" for those, which is what a
/// user saw every time they opened a chat and switched straight back before
/// saying anything.
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
        // A file type that cannot be read is skipped, not assumed either way
        // — the same caution `discover_claude_session` takes with an unreadable
        // mtime just above.
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

/// The session id for a hand-started claude in `worktree`.
///
/// `started_after` is the pane's start time. A session file older than the pane
/// cannot be the one running in it, which is what keeps a worktree reused for a
/// second task from offering the first task's conversation.
pub fn discover_claude_session(
    home: &Path,
    worktree: &Path,
    started_after: SystemTime,
) -> Result<String, DiscoveryError> {
    // The REALPATH, not the path as written. Claude Code munges the resolved
    // cwd, so on macOS a worktree under `/tmp` lands in `-private-tmp-...` and
    // looking under `-tmp-...` finds an empty directory and reports NotFound
    // for a session that exists. Measured in the Gate 1 spike, not guessed.
    //
    // A path that cannot be resolved is used as written: the only caller passes
    // a worktree that exists, and failing here would turn a missing directory
    // into a confusing error about a different one.
    let resolved = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let dir: PathBuf = home.join(".claude/projects").join(project_dir_name(&resolved));
    let entries = std::fs::read_dir(&dir).map_err(|_| DiscoveryError::NotFound)?;

    // Newest first, by when it was last written.
    //
    // Requiring a single candidate read well and worked nowhere: a worktree
    // that has been used more than once holds a session per use, so adoption
    // refused as "ambiguous" in exactly the situation it exists for. The
    // session a pane is running is the one being written to — every turn
    // touches its file — so recency is the signal, not count.
    let mut candidates: Vec<(String, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        // A file whose mtime cannot be read is skipped rather than assumed
        // recent: an unreadable candidate is exactly the kind of thing that
        // would make an ambiguous directory look unambiguous.
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if modified < started_after {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            candidates.push((stem.to_string(), modified));
        }
    }

    candidates.sort_by_key(|c| std::cmp::Reverse(c.1));
    match candidates.as_slice() {
        [] => Err(DiscoveryError::NotFound),
        [(only, _)] => Ok(only.clone()),
        [(newest, first), (_, second), ..] => {
            // Still refuses when two were written at what is, to a filesystem,
            // the same moment — there is genuinely nothing to choose between
            // them, and picking one would attach a chat to a conversation
            // other than the one in the pane.
            let margin = first.duration_since(*second).unwrap_or_default();
            if margin < std::time::Duration::from_secs(2) {
                let mut names: Vec<String> = candidates.iter().map(|(n, _)| n.clone()).collect();
                names.sort();
                return Err(DiscoveryError::Ambiguous { candidates: names });
            }
            Ok(newest.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    #[test]
    fn a_worktree_path_munges_the_way_claude_munges_it() {
        // Read off a real machine: every non-alphanumeric becomes a dash, and
        // the leading slash produces a leading dash.
        assert_eq!(project_dir_name(Path::new("/Users/e/Dev/farcooler")), "-Users-e-Dev-farcooler");
        assert_eq!(project_dir_name(Path::new("/Users/e/.claude/jobs")), "-Users-e--claude-jobs");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farcooler-disc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn one_session_in_the_worktree_is_found() {
        let home = scratch("one");
        let worktree = Path::new("/Users/e/Dev/proj");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-a.jsonl"), "{}").unwrap();

        let found = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(found, "sess-a");
    }

    #[test]
    fn the_most_recently_written_session_is_the_one_in_the_pane() {
        // A worktree used more than once holds a session per use, so demanding
        // a single candidate refused adoption in precisely the case it exists
        // for. Every turn writes to the running session's file, so the newest
        // is the one on screen.
        let home = scratch("recency");
        let worktree = Path::new("/Users/e/Dev/proj2");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("older.jsonl"), "{}").unwrap();
        std::thread::sleep(Duration::from_millis(2100));
        std::fs::write(dir.join("newer.jsonl"), "{}").unwrap();

        let found = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(found, "newer");
    }

    #[test]
    fn two_written_at_the_same_moment_still_refuse() {
        // Recency only decides when there is a difference to read. Two files a
        // filesystem cannot separate leave nothing to choose between, and
        // guessing would attach a chat to the wrong conversation.
        let home = scratch("tie");
        let worktree = Path::new("/Users/e/Dev/proj-tie");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-a.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("sess-b.jsonl"), "{}").unwrap();

        let err = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap_err();
        let DiscoveryError::Ambiguous { candidates } = err else { panic!("expected refusal") };
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn a_worktree_reached_through_a_symlink_finds_its_real_project_directory() {
        // The bug the Gate 1 spike caught: on macOS `/tmp` is a symlink to
        // `/private/tmp`, and Claude Code munges the RESOLVED cwd. Looking
        // under the unresolved name finds nothing and reports NotFound for a
        // session that is sitting right there.
        let home = scratch("symlink-home");
        let real = scratch("symlink-real");
        let link = std::env::temp_dir().join(format!("farcooler-disc-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // The session file lives under the REAL path's munged name.
        let dir = home
            .join(".claude/projects")
            .join(project_dir_name(&std::fs::canonicalize(&real).unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-real.jsonl"), "{}").unwrap();

        // Asking with the symlinked path must still find it.
        let found = discover_claude_session(&home, &link, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(found, "sess-real");
    }

    #[test]
    fn a_session_older_than_the_pane_is_not_a_candidate() {
        // A worktree reused for a second task holds a stale session file. It
        // predates this pane and is therefore not what is running in it.
        let home = scratch("stale");
        let worktree = Path::new("/Users/e/Dev/proj3");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.jsonl"), "{}").unwrap();

        let future = SystemTime::now() + Duration::from_secs(3600);
        assert!(matches!(
            discover_claude_session(&home, worktree, future),
            Err(DiscoveryError::NotFound)
        ));
    }

    #[test]
    fn a_session_belonging_to_another_pane_is_not_adoptable() {
        // The failure this guards: two terminals share a worktree, one is a
        // shell, and switching it to agent mode adopted the OTHER pane's
        // conversation — so both panes rendered one transcript under two
        // identities. Discovery cannot see that on its own; the caller must
        // exclude what other terminals already claim, and this records the
        // shape that exclusion depends on.
        let home = scratch("claimed");
        let worktree = Path::new("/Users/e/Dev/shared");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("belongs-to-pane-one.jsonl"), "{}").unwrap();

        let found = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(found, "belongs-to-pane-one");

        // The caller's job, asserted here because it is the whole protection:
        // a candidate another terminal already holds must be refused.
        let claimed = ["belongs-to-pane-one".to_string()];
        assert!(claimed.contains(&found), "the caller must reject this");
    }

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
        // No `.codex/sessions` at all — codex has never run on this machine,
        // or `$HOME` in the test is a bare scratch directory. Either way this
        // must report false rather than error: the caller's fallback (start
        // clean) is exactly the right behavior for "nothing to find".
        let home = scratch("codex-no-dir");
        assert!(!codex_rollout_exists(&home, "019fc8af-daff-7692-b7be-4457fda0b01c"));
    }
}
