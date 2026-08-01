//! Where an agent is allowed to read and write.
//!
//! Resolution before comparison, always. A prefix check on the path as written
//! passes `worktree/escape/passwd` when `escape` is a symlink to `/etc`, which
//! is the whole attack. The deepest existing ancestor is canonicalized and the
//! remaining components are appended, so a file that does not exist yet is
//! still judged by where it would actually land.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FsGuardError {
    #[error("path resolves outside the workspace worktree")]
    Escapes,
    #[error("worktree path could not be resolved")]
    BadWorktree,
}

/// Resolve `requested` and return it only if it lands inside `worktree`.
pub fn confine(worktree: &Path, requested: &Path) -> Result<PathBuf, FsGuardError> {
    let root = std::fs::canonicalize(worktree).map_err(|_| FsGuardError::BadWorktree)?;

    let absolute =
        if requested.is_absolute() { requested.to_path_buf() } else { root.join(requested) };

    // Canonicalize the deepest ancestor that exists, then re-append the rest.
    // `canonicalize` on a missing path fails outright, and every file creation
    // is a missing path.
    let mut existing = absolute.as_path();
    let mut tail: Vec<Component<'_>> = Vec::new();
    let resolved_head = loop {
        match std::fs::canonicalize(existing) {
            Ok(p) => break p,
            Err(_) => match existing.parent() {
                Some(parent) => {
                    if let Some(name) = existing.components().next_back() {
                        tail.push(name);
                    }
                    existing = parent;
                }
                None => return Err(FsGuardError::Escapes),
            },
        }
    };

    let mut resolved = resolved_head;
    for component in tail.into_iter().rev() {
        match component {
            // A `..` that survived to here would climb out of the resolved
            // head, so it is refused rather than normalized away.
            Component::ParentDir => return Err(FsGuardError::Escapes),
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }

    if resolved.starts_with(&root) { Ok(resolved) } else { Err(FsGuardError::Escapes) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn worktree() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("overnight-guard-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("src"));
        fs::canonicalize(&dir).expect("temp worktree")
    }

    #[test]
    fn a_path_inside_the_worktree_is_allowed() {
        let wt = worktree();
        let ok = confine(&wt, &wt.join("src/main.rs")).expect("inside is allowed");
        assert!(ok.starts_with(&wt));
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_allowed_inside() {
        // Every create goes through this. Requiring the file to exist would
        // make the capability useless for new files.
        let wt = worktree();
        assert!(confine(&wt, &wt.join("src/brand_new.rs")).is_ok());
    }

    #[test]
    fn dot_dot_cannot_climb_out() {
        let wt = worktree();
        let err = confine(&wt, &wt.join("../../etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn an_absolute_path_elsewhere_is_refused() {
        let wt = worktree();
        let err = confine(&wt, std::path::Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_symlink_pointing_out_is_refused() {
        // The one that a naive prefix check on the unresolved string misses,
        // and the reason resolution has to happen before comparison.
        let wt = worktree();
        let link = wt.join("escape");
        let _ = fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", &link).expect("symlink");
        let err = confine(&wt, &link.join("passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_symlink_that_is_itself_the_requested_path_is_refused() {
        // Distinct from the directory-symlink case above: here the symlink
        // *is* the whole requested path, so `canonicalize` resolves it on the
        // first attempt with an empty tail, and the escape check still has to
        // catch it — the comparison can't rely on the tail-reappend loop ever
        // running.
        let wt = worktree();
        let link = wt.join("direct");
        let _ = fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", &link).expect("symlink");
        let err = confine(&wt, &link).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_sibling_directory_with_a_prefix_matching_name_is_refused() {
        // `starts_with` must compare path components, not raw strings. A
        // string-prefix check would let this through, because the text
        // "overnight-guard-123-evil" starts with the text
        // "overnight-guard-123" even though the paths are unrelated siblings.
        let wt = worktree();
        let sibling = PathBuf::from(format!("{}-evil", wt.display()));
        let _ = fs::create_dir_all(&sibling);
        let sibling = fs::canonicalize(&sibling).expect("sibling worktree");
        let err = confine(&wt, &sibling.join("file.txt")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_dot_dot_cancelled_out_by_a_real_directory_is_still_allowed() {
        // `src/../src/x` is not an escape attempt — the `..` is consumed by
        // canonicalizing the existing ancestor, not by the tail-reappend
        // refusal. This guards against the escape fix becoming so strict it
        // rejects ordinary paths a caller never wrote by hand but that a
        // library might construct.
        let wt = worktree();
        let ok = confine(&wt, &wt.join("src/../src/new_file.rs")).expect("resolves back inside");
        assert!(ok.starts_with(&wt));
    }
}
