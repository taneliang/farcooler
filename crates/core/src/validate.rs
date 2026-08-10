//! Input validation at the boundary.

use crate::error::{DomainError, Result};
use farcooler_protocol::{MAX_COLUMNS, MAX_ROWS, MIN_COLUMNS, MIN_ROWS};

/// Display names are 1-80 UTF-8 scalar values.
pub fn display_name(s: &str) -> Result<()> {
    scalar_len(s, 1, 80, "display_name")
}

/// Worktree names are 1-60, and must survive slugging.
///
/// Sixty rather than a task name's old 120 because this is a path component
/// now: it is the directory the worktree is created in, and it is what every
/// client shows in a sidebar row. A name generated rather than typed —
/// `write a haiku about worktrees in HAIKU.md` — is a directory nobody wants
/// and a row nobody can read.
///
/// The slug check is the one that cannot be expressed as a length: a name of
/// pure punctuation is 1-60 scalars and still has no directory to be.
pub fn worktree_name(s: &str) -> Result<()> {
    scalar_len(s, 1, 60, "worktree_name")?;
    if crate::names::slug(s).is_empty() {
        return Err(DomainError::InvalidArgument { what: "worktree_name" });
    }
    Ok(())
}

/// Command-preset identifiers are 1-64 ASCII characters.
pub fn command_preset(s: &str) -> Result<()> {
    if s.is_empty() || s.chars().count() > 64 || !s.is_ascii() {
        return Err(DomainError::InvalidArgument { what: "command_preset" });
    }
    Ok(())
}

fn scalar_len(s: &str, min: usize, max: usize, what: &'static str) -> Result<()> {
    let n = s.chars().count();
    if n < min || n > max {
        return Err(DomainError::InvalidArgument { what });
    }
    Ok(())
}

/// Branch names must pass `git check-ref-format --branch`. This is the cheap
/// structural pre-filter; the daemon still defers to git itself.
pub fn branch_name(s: &str) -> Result<()> {
    let bad = s.is_empty()
        || s.starts_with('-')
        || s.starts_with('/')
        || s.ends_with('/')
        || s.ends_with('.')
        || s.ends_with(".lock")
        || s.contains("..")
        || s.contains("//")
        || s.contains("@{")
        || s.contains(['~', '^', ':', '?', '*', '[', '\\', ' '])
        || s.chars().any(|c| c.is_control());
    if bad {
        return Err(DomainError::InvalidArgument { what: "branch" });
    }
    Ok(())
}

/// Clamp to 20-500 columns and 5-200 rows.
pub fn clamp_size(columns: u32, rows: u32) -> (u32, u32) {
    (columns.clamp(MIN_COLUMNS, MAX_COLUMNS), rows.clamp(MIN_ROWS, MAX_ROWS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_bound_by_scalar_count_not_bytes() {
        assert!(display_name("ok").is_ok());
        assert!(display_name("").is_err());
        assert!(display_name(&"a".repeat(80)).is_ok());
        assert!(display_name(&"a".repeat(81)).is_err());
        // 80 emoji is 80 scalars even though it is far more bytes.
        assert!(display_name(&"🌙".repeat(80)).is_ok());
        assert!(display_name(&"🌙".repeat(81)).is_err());
    }

    #[test]
    fn worktree_names_are_bounded_and_must_slug_to_something() {
        assert!(worktree_name("rate limiting").is_ok());
        assert!(worktree_name("").is_err());
        assert!(worktree_name(&"a".repeat(60)).is_ok());
        assert!(worktree_name(&"a".repeat(61)).is_err());

        // Within the length and still not a directory. The old task name had no
        // reason to care; a name that has to become a path component does.
        assert!(worktree_name("!!!").is_err(), "punctuation slugs to nothing");
        assert!(worktree_name("---").is_err(), "so do dashes, once they are trimmed");
        assert!(worktree_name("🌙").is_err(), "and so does an emoji on its own");
        assert!(worktree_name("🌙 auth").is_ok(), "but it is fine beside something that survives");
    }

    #[test]
    fn command_preset_is_ascii_bounded() {
        assert!(command_preset("claude").is_ok());
        assert!(command_preset("").is_err());
        assert!(command_preset("🌙").is_err(), "non-ascii preset id rejected");
        assert!(command_preset(&"a".repeat(65)).is_err());
    }

    #[test]
    fn branch_names_reject_git_hostile_forms() {
        assert!(branch_name("feature/login").is_ok());
        assert!(branch_name("fix-123").is_ok());

        for bad in [
            "", "-lead", "/lead", "trail/", "trail.", "a..b", "a//b", "a@{b}", "a~b", "a^b",
            "a:b", "a?b", "a*b", "a[b", "a\\b", "a b", "x.lock",
        ] {
            assert!(branch_name(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn sizes_clamp_into_range() {
        assert_eq!(clamp_size(1, 1), (MIN_COLUMNS, MIN_ROWS));
        assert_eq!(clamp_size(9999, 9999), (MAX_COLUMNS, MAX_ROWS));
        assert_eq!(clamp_size(120, 40), (120, 40));
    }
}
