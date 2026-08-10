//! What a workspace is called.
//!
//! A workspace has exactly one name and does not store it: the worktree's
//! directory is the name, and this module reads it back as prose. The directory
//! is the only thing about a worktree stable enough to name it. A branch is
//! not — one worktree hosts a stack of commits over its life, and the branch
//! checked out inside it changes as that stack is built and rebased, so naming
//! a workspace after its branch means renaming it every time the work moves
//! forward. Two main checkouts would both be called `main`, as well.
//!
//! `slug` and `display` are inverses, which is the property worth protecting:
//! whatever a person types in the New workspace sheet has to survive the round
//! trip through the filesystem and come back looking like what they typed.

use std::path::Path;

/// Turn a typed name into one path component.
///
/// Every character outside `[A-Za-z0-9_-]` becomes a dash, runs of dashes
/// collapse, and dashes are trimmed from both ends. The collapsing and trimming
/// used to be absent, which was merely ugly while nothing read these back —
/// `Rate  Limiting!` became `Rate--Limiting-` and only ever appeared in a path.
/// Now `display` reads it back into the sidebar, so a doubled dash is a doubled
/// space in front of the user.
///
/// Case is preserved on purpose. It is the one thing a slug can carry that a
/// branch name cannot, and it costs nothing: `Rate Limiting` round-trips
/// exactly. Only branch suggestions lowercase, and they do it themselves.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.trim_matches('-').to_string()
}

/// What to call the workspace living at this worktree path.
///
/// The directory's own name, with the separators a filesystem forces on it read
/// back as spaces. `…/worktrees/overnight/rate-limiting` is "rate limiting".
///
/// One rule covers every workspace. A main checkout sits in the repository's own
/// directory, so `~/Dev/overnight` is "overnight" — which is what the reconciler
/// already called it, but now without an `is_main_checkout` branch to pick that
/// rule over another one. A detached worktree needs no case of its own either:
/// the name comes from the directory whatever the head is doing.
///
/// Worktrees created before the per-repository layout are prefixed with their
/// repository (`overnight-rate-limiting`) and read back with it. Nothing strips
/// it. Moving a worktree behind the user's back to tidy up a name is worse than
/// a wordy row, and worktrees are meant to be short-lived, so the old shape
/// empties itself out.
pub fn display(worktree_path: &str) -> String {
    // `file_name` is `None` only for a path with no final component, which no
    // worktree can have. A row showing its whole path is recoverable; a row
    // showing an empty string is a workspace the user cannot name or find.
    Path::new(worktree_path)
        .file_name()
        .map(|n| n.to_string_lossy().replace(['-', '_'], " "))
        .unwrap_or_else(|| worktree_path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_name_survives_the_round_trip() {
        for typed in ["rate limiting", "Rate Limiting", "fix the parser"] {
            assert_eq!(display(&format!("/w/overnight/{}", slug(typed))), typed);
        }
    }

    #[test]
    fn slug_collapses_and_trims() {
        assert_eq!(slug("Rate  Limiting!"), "Rate-Limiting");
        assert_eq!(slug("--leading and trailing--"), "leading-and-trailing");
        assert_eq!(slug("ok_name-1"), "ok_name-1");
        assert_eq!(slug("my repo/name"), "my-repo-name");
    }

    /// A name that is nothing but punctuation has no directory to be, which is
    /// why `validate::worktree_name` rejects it rather than letting the daemon
    /// try to create a worktree at a path ending in nothing.
    #[test]
    fn slug_can_be_empty() {
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug("---"), "");
    }

    #[test]
    fn display_reads_a_directory_as_prose() {
        assert_eq!(display("/w/overnight/rate-limiting"), "rate limiting");
        assert_eq!(display("/w/overnight/fix_the_parser"), "fix the parser");
        // The main checkout: the repository's own directory, no special case.
        assert_eq!(display("/Users/e/Dev/overnight"), "overnight");
        // A worktree from before the per-repository layout keeps its prefix.
        assert_eq!(display("/w/overnight-rate-limiting"), "overnight rate limiting");
    }

    #[test]
    fn display_falls_back_to_the_path_it_cannot_read() {
        assert_eq!(display("/"), "/");
    }
}
