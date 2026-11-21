//! Paths inside a workspace's runtime home.

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// The charter's file name inside a home.
pub const CHARTER_FILE: &str = "charter.md";

/// A workspace's home: `<root>/workspaces/<workspace id>`. `root` is the
/// runtime directory (`FARCOOLER_HOME` when set). Only computed; see
/// `make_home`.
pub fn home(root: &Path, workspace: Uuid) -> PathBuf {
    root.join("workspaces").join(workspace.to_string())
}

/// A workspace's charter: `home/charter.md`. It may not exist yet.
pub fn charter_path(root: &Path, workspace: Uuid) -> PathBuf {
    home(root, workspace).join(CHARTER_FILE)
}
