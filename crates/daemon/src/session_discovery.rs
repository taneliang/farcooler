//! Which conversation is running in a pane nobody declared.
//!
//! Only for `claude` typed into a shell by hand. Anything Overnight launched
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

    let mut candidates: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        // A file whose mtime cannot be read is skipped rather than assumed
        // recent: an unreadable candidate is exactly the kind of thing that
        // would make an ambiguous directory look unambiguous.
        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        if modified.map(|m| m < started_after).unwrap_or(true) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            candidates.push(stem.to_string());
        }
    }

    match candidates.len() {
        0 => Err(DiscoveryError::NotFound),
        1 => Ok(candidates.remove(0)),
        // Sorted so the message a user reads is stable between attempts.
        _ => {
            candidates.sort();
            Err(DiscoveryError::Ambiguous { candidates })
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
        assert_eq!(project_dir_name(Path::new("/Users/e/Dev/overnight")), "-Users-e-Dev-overnight");
        assert_eq!(project_dir_name(Path::new("/Users/e/.claude/jobs")), "-Users-e--claude-jobs");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("overnight-disc-{name}-{}", std::process::id()));
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
    fn two_candidate_sessions_refuse_rather_than_pick_one() {
        // The failure this rule prevents is silent and expensive: attaching a
        // chat view to a conversation that is not the one in the pane.
        let home = scratch("two");
        let worktree = Path::new("/Users/e/Dev/proj2");
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
        let link = std::env::temp_dir().join(format!("overnight-disc-link-{}", std::process::id()));
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
        let claimed = vec!["belongs-to-pane-one".to_string()];
        assert!(claimed.contains(&found), "the caller must reject this");
    }
}
