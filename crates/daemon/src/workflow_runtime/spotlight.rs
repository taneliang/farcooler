//! Keep known, ignored build outputs in engine worktrees out of Spotlight.
use std::path::Path;

/// Only create directories Git already ignores, and never follow a symlink out of the tree.
/// Failure is advisory: indexing must not prevent a run from starting.
pub(super) async fn exclude_builds(tree: &Path) {
    for relative in ["target", "build", ".build", "crates/web/target", "apps/macos/.build", "apps/macos/build",
        "apps/ios/build", "apps/shared/AgentKit/.build", "apps/android/build", "apps/android/app/build"] {
        let probe = format!("{relative}/");
        if !crate::git::git(tree, &["check-ignore", "--quiet", "--no-index", &probe]).await.is_ok_and(|r| r.ok) { continue; }
        let mut path = tree.to_path_buf();
        let mut safe = true;
        let parts: Vec<_> = Path::new(relative).components().collect();
        for (i, part) in parts.iter().enumerate() {
            path.push(part.as_os_str());
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.file_type().is_dir() => {},
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && i + 1 == parts.len() => {
                    if let Err(error) = std::fs::create_dir(&path) {
                        tracing::warn!(path = %path.display(), %error, "could not exclude build output from Spotlight");
                        safe = false; break;
                    }
                },
                _ => { safe = false; break; }
            }
        }
        if safe {
            let marker = path.join(".metadata_never_index");
            if let Err(error) = std::fs::OpenOptions::new().write(true).create_new(true).open(&marker) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    tracing::warn!(path = %marker.display(), %error, "could not exclude build output from Spotlight");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_ignored_build_dirs_inside_the_tree_are_marked() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(tree.join("apps/macos")).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&tree).status().unwrap();
        std::fs::write(tree.join(".gitignore"), "/target\n/build\napps/macos/build\napps/macos/.build\n").unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("apps/macos/build")).unwrap();
        exclude_builds(&tree).await;
        exclude_builds(&tree).await;
        assert!(tree.join("target/.metadata_never_index").is_file());
        assert!(tree.join("build/.metadata_never_index").is_file());
        assert!(tree.join("apps/macos/.build/.metadata_never_index").is_file());
        assert!(!outside.join(".metadata_never_index").exists());
        assert!(!tree.join("crates/web/target").exists());
    }
}
