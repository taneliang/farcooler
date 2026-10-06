//! Whether two build strings name the same build (ov-352).
//!
//! `BUILD` is stamped `0.2.0+a1b2c3d`, with `-dirty` when the tree had
//! uncommitted changes. That is the daemon's and the CLI's own spelling, but
//! it is not the only way a build gets written down: the Mac app says
//! `0.2.0 (canary a1b2c3d)`, a bug report or an appcast may carry only the
//! commit, and `git rev-parse --short` picks a longer abbreviation in a
//! bigger repository. Comparing the strings made the same commit look like a
//! different build, so a runner on the app's own Canary commit kept offering
//! "Update Runner". A build's identity is its commit and whether the tree was
//! clean, so that is what is compared.

/// A build, reduced to what makes it the same source.
#[derive(Debug, PartialEq, Eq)]
struct Identity<'a> {
    /// The commit, lowercase hex, however many digits the spelling gave.
    commit: String,
    /// Built from a tree with uncommitted changes: the commit alone doesn't
    /// say what source this was.
    dirty: bool,
    /// The whole string, for a spelling with no commit in it.
    raw: &'a str,
}

/// The fewest hex digits taken as naming a commit. Git's own shortest
/// abbreviation is 4, but a prefix this short would match by luck.
const MIN_COMMIT_DIGITS: usize = 7;

/// The commit in any spelling of a build: after the `+` of a stamp, inside the
/// parentheses of a display string (its last word), or the whole of a bare
/// commit. `None` for anything else (`unknown`, a test's `dtest`).
fn identity(build: &str) -> Identity<'_> {
    let build = build.trim();
    let tail = if let Some((_, after)) = build.rsplit_once('+') {
        after
    } else if let Some(inner) = build.strip_suffix(')').and_then(|s| s.rsplit_once('(')).map(|(_, i)| i) {
        inner.rsplit(' ').next().unwrap_or(inner)
    } else {
        build
    };
    let (word, dirty) = match tail.strip_suffix("-dirty") {
        Some(word) => (word, true),
        None => (tail, false),
    };
    let named = word.len() >= MIN_COMMIT_DIGITS && word.bytes().all(|b| b.is_ascii_hexdigit());
    Identity { commit: if named { word.to_ascii_lowercase() } else { String::new() }, dirty, raw: build }
}

/// Whether `a` and `b` are the same build: the same commit (one may be a
/// longer abbreviation of the other) and the same cleanliness. A string with
/// no commit in it is the same only as itself.
pub fn same_build(a: &str, b: &str) -> bool {
    let (a, b) = (identity(a), identity(b));
    if a.commit.is_empty() || b.commit.is_empty() {
        return a.raw == b.raw;
    }
    let (short, long) = if a.commit.len() <= b.commit.len() { (&a, &b) } else { (&b, &a) };
    long.commit.starts_with(&short.commit) && a.dirty == b.dirty
}

#[cfg(test)]
mod tests {
    use super::same_build;

    /// The stamp the daemon and CLI report, the app's display string, and a
    /// bare commit, all for one commit: the same build.
    #[test]
    fn one_commit_in_every_spelling_is_one_build() {
        let stamp = "0.2.0+a1b2c3d";
        assert!(same_build(stamp, "0.2.0 (canary a1b2c3d)"));
        assert!(same_build("0.2.0 (canary a1b2c3d)", stamp));
        assert!(same_build(stamp, "a1b2c3d"));
        assert!(same_build("0.2.0 (canary a1b2c3d)", "a1b2c3d"));
        assert!(same_build(stamp, stamp));
        // Case, and a longer abbreviation of the same commit.
        assert!(same_build("A1B2C3D", stamp));
        assert!(same_build("0.2.0+a1b2c3d4e", "0.2.0 (canary a1b2c3d)"));
    }

    /// Two commits are two builds in every pairing of the spellings.
    #[test]
    fn different_commits_are_different_builds() {
        assert!(!same_build("0.2.0+a1b2c3d", "0.2.0 (canary 9f8e7d6)"));
        assert!(!same_build("0.2.0 (canary a1b2c3d)", "9f8e7d6"));
        assert!(!same_build("0.2.0+a1b2c3d", "0.2.0+9f8e7d6"));
        assert!(!same_build("a1b2c3d", "9f8e7d6"));
    }

    /// An uncommitted tree isn't its commit.
    #[test]
    fn a_dirty_tree_is_not_the_clean_commit() {
        assert!(!same_build("0.2.0+a1b2c3d-dirty", "0.2.0+a1b2c3d"));
        assert!(!same_build("0.2.0+a1b2c3d-dirty", "0.2.0 (canary a1b2c3d)"));
        assert!(same_build("0.2.0+a1b2c3d-dirty", "a1b2c3d-dirty"));
    }

    /// With no commit to read, only an identical string matches.
    #[test]
    fn a_string_without_a_commit_matches_only_itself() {
        assert!(same_build("0.2.0+unknown", "0.2.0+unknown"));
        assert!(!same_build("0.2.0+unknown", "0.2.0+a1b2c3d"));
        assert!(same_build("dtest", "dtest"));
        assert!(!same_build("dtest", "a-newer-build"));
        assert!(!same_build("", "a1b2c3d"));
    }
}
