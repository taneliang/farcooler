//! Where a comment still points, computed rather than remembered.
//!
//! The rule this module exists to enforce: **an anchor never resolves to a
//! guessed line.** It resolves to exactly one place, or it degrades one rung and
//! says which rung it fell to. That is the same contract the terminal state
//! machine keeps when it reports `LOST` instead of a plausible `running` — the
//! product is allowed to not know, and is never allowed to invent.
//!
//! Nothing here stores a line number. An entry carries the text it was written
//! about and a fingerprint of what surrounded it; the position is a search
//! result, recomputed every time it is read, because an agent is editing the
//! file underneath it.
//!
//! ```text
//!   anchored text sought in the file as it is NOW
//!                 |
//!    +------------+-------------+---------------+
//!  1 match      1 match       >1 match        0 matches
//!  ctx same     ctx moved         |               |
//!    |             |         ctx picks one?   region changed
//!  EXACT         MOVED        yes -> EXACT     since capture?
//!                             no  -> AMBIGUOUS   |        |
//!                                    -> File    yes      no
//!                                          NEEDS_REREAD  OUTDATED
//!                                             -> File     -> File
//!                                      (file gone -> FILE_GONE -> Workspace)
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diff::Hunk;

/// How many lines either side of an anchor are fingerprinted.
///
/// Three is enough to tell two copies of the same edit apart in real code and
/// short enough that reformatting further away does not move the anchor.
pub const CONTEXT_RADIUS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// The default, and the most common. Over half of real review comments are
    /// not about a particular line, so this is the normal case and the capture
    /// UI must reach it without asking anything.
    None,
    Workspace,
    Branch { branch: String },
    Commit { sha: String },
    File { path: String },
    Hunk { path: String, fingerprint: String, context_fingerprint: String },
    Lines { path: String, side: Side, text: String, context_fingerprint: String },
}

impl Anchor {
    pub fn path(&self) -> Option<&str> {
        match self {
            Anchor::File { path }
            | Anchor::Hunk { path, .. }
            | Anchor::Lines { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The rung below this one. Degradation is always downward and always by
    /// one step, so a reader can see how much precision was lost.
    pub fn degraded(&self) -> Anchor {
        match self {
            Anchor::Hunk { path, .. } | Anchor::Lines { path, .. } => {
                Anchor::File { path: path.clone() }
            }
            Anchor::File { .. } => Anchor::Workspace,
            other => other.clone(),
        }
    }
}

/// What was true when the comment was written.
///
/// Immutable, persisted with the entry, and the reason re-read detection
/// survives a daemon restart. A version counter would not: it is meaningless
/// after a process exits and says nothing at all across a rebase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureManifest {
    pub base_commit: String,
    pub head_commit: String,
    /// Hash over HEAD plus, for every path git reports as dirty, that path, its
    /// status, AND its content. Contents matter: porcelain output alone does not
    /// change when a file that was already modified is modified again, which is
    /// precisely the case an unanchored comment needs to notice.
    pub worktree_digest: String,
    /// The anchored file's content hash, for File/Hunk/Lines anchors.
    pub file_content_hash: Option<String>,
    /// The anchored file's bytes, when it was dirty at capture and small enough
    /// to keep. Git holds the prior content for a committed capture; for a dirty
    /// one it exists nowhere else, and without it this entry can only say THAT
    /// the file changed, not which ranges.
    pub file_snapshot: Option<String>,
}

/// The world as it is now, against which an anchor resolves.
#[derive(Debug, Clone, Default)]
pub struct Current {
    pub head_commit: String,
    pub worktree_digest: String,
    /// `None` means the file is gone.
    pub file_content: Option<String>,
    pub file_content_hash: Option<String>,
    /// Present only when the caller already computed the file's hunks, which is
    /// the case when it is rendering them anyway.
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    /// One match, and its surroundings are what they were.
    Exact,
    /// One match, but the code around it changed. Still the right place.
    Moved,
    /// More than one place it could be, and context did not break the tie.
    /// Degraded rather than guessed.
    Ambiguous,
    /// Gone from where it was, and the region moved since capture — an agent
    /// probably addressed it. Worth re-reading.
    NeedsReread,
    /// Gone, and nothing moved. The comment outlived its subject.
    Outdated,
    FileGone,
}

impl AnchorState {
    /// Whether this state may carry a position.
    ///
    /// "May", not "does": an unanchored comment on an unmodified worktree is
    /// `Exact` and has no line, because there was never a line to be exact
    /// about. What this rules out is the other direction — a position attached
    /// to `Ambiguous`, `Outdated` or `NeedsReread`, which is exactly the guess
    /// this module refuses to make.
    pub fn may_carry_position(self) -> bool {
        matches!(self, AnchorState::Exact | AnchorState::Moved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub state: AnchorState,
    /// The anchor as it should now be treated: the original when located, the
    /// degraded one otherwise. Clients render this, never the stored anchor.
    pub effective: Anchor,
    /// 1-based line where the anchored text begins, when located.
    pub line: Option<u32>,
}

/// Fingerprint the lines surrounding `start..end` in `lines`.
pub fn context_fingerprint(lines: &[&str], start: usize, end: usize) -> String {
    let lo = start.saturating_sub(CONTEXT_RADIUS);
    let hi = (end + CONTEXT_RADIUS).min(lines.len());
    let mut h = Sha256::new();
    for l in &lines[lo..start] {
        h.update(l.as_bytes());
        h.update([0]);
    }
    h.update(b"|");
    for l in &lines[end..hi] {
        h.update(l.as_bytes());
        h.update([0]);
    }
    format!("{:x}", h.finalize())
}

/// Hash a file's contents. Used for `file_content_hash` on both sides.
pub fn content_hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// Resolve one anchor against the current state.
pub fn resolve(anchor: &Anchor, manifest: &CaptureManifest, current: &Current) -> Resolution {
    match anchor {
        // Altitudes with no position to lose. They are never Exact — there is no
        // line to be exact about — but they DO need re-read, and getting that
        // wrong for the unanchored case would miss the majority of all comments.
        Anchor::None | Anchor::Workspace | Anchor::Branch { .. } | Anchor::Commit { .. } => {
            let moved = current.head_commit != manifest.head_commit
                || current.worktree_digest != manifest.worktree_digest;
            Resolution {
                state: if moved { AnchorState::NeedsReread } else { AnchorState::Exact },
                effective: anchor.clone(),
                line: None,
            }
        }

        Anchor::File { .. } => {
            if current.file_content.is_none() {
                return Resolution {
                    state: AnchorState::FileGone,
                    effective: anchor.degraded(),
                    line: None,
                };
            }
            let changed = current.file_content_hash != manifest.file_content_hash;
            Resolution {
                state: if changed { AnchorState::NeedsReread } else { AnchorState::Exact },
                effective: anchor.clone(),
                line: None,
            }
        }

        Anchor::Hunk { fingerprint, context_fingerprint: ctx, .. } => {
            if current.file_content.is_none() {
                return Resolution {
                    state: AnchorState::FileGone,
                    effective: anchor.degraded(),
                    line: None,
                };
            }
            let matches: Vec<&Hunk> =
                current.hunks.iter().filter(|h| &h.fingerprint == fingerprint).collect();
            match matches.len() {
                0 => Resolution {
                    state: gone_state(manifest, current),
                    effective: anchor.degraded(),
                    line: None,
                },
                1 => Resolution {
                    // A hunk carries its own context in `header`; when the
                    // stored context fingerprint still matches the hunk header
                    // the surroundings are unchanged.
                    state: if hunk_context(matches[0]) == *ctx {
                        AnchorState::Exact
                    } else {
                        AnchorState::Moved
                    },
                    effective: anchor.clone(),
                    line: Some(matches[0].new_start),
                },
                _ => {
                    let narrowed: Vec<&&Hunk> =
                        matches.iter().filter(|h| hunk_context(h) == *ctx).collect();
                    if narrowed.len() == 1 {
                        Resolution {
                            state: AnchorState::Exact,
                            effective: anchor.clone(),
                            line: Some(narrowed[0].new_start),
                        }
                    } else {
                        Resolution {
                            state: AnchorState::Ambiguous,
                            effective: anchor.degraded(),
                            line: None,
                        }
                    }
                }
            }
        }

        Anchor::Lines { text, context_fingerprint: ctx, .. } => {
            let Some(content) = current.file_content.as_deref() else {
                return Resolution {
                    state: AnchorState::FileGone,
                    effective: anchor.degraded(),
                    line: None,
                };
            };
            let haystack: Vec<&str> = content.lines().collect();
            let needle: Vec<&str> = text.lines().collect();
            if needle.is_empty() {
                return Resolution {
                    state: AnchorState::Outdated,
                    effective: anchor.degraded(),
                    line: None,
                };
            }

            let hits = find_runs(&haystack, &needle);
            match hits.len() {
                0 => Resolution {
                    state: gone_state(manifest, current),
                    effective: anchor.degraded(),
                    line: None,
                },
                1 => {
                    let at = hits[0];
                    let here = context_fingerprint(&haystack, at, at + needle.len());
                    Resolution {
                        state: if here == *ctx { AnchorState::Exact } else { AnchorState::Moved },
                        effective: anchor.clone(),
                        line: Some(at as u32 + 1),
                    }
                }
                _ => {
                    // Several copies of the same code. Context is exactly what
                    // it is for: if it singles one out, that is the place.
                    let narrowed: Vec<usize> = hits
                        .iter()
                        .copied()
                        .filter(|&at| {
                            context_fingerprint(&haystack, at, at + needle.len()) == *ctx
                        })
                        .collect();
                    if narrowed.len() == 1 {
                        Resolution {
                            state: AnchorState::Exact,
                            effective: anchor.clone(),
                            line: Some(narrowed[0] as u32 + 1),
                        }
                    } else {
                        Resolution {
                            state: AnchorState::Ambiguous,
                            effective: anchor.degraded(),
                            line: None,
                        }
                    }
                }
            }
        }
    }
}

/// The anchored text is not in the file. Did the file move under it, or did the
/// comment simply outlive its subject?
fn gone_state(manifest: &CaptureManifest, current: &Current) -> AnchorState {
    let file_changed = match (&manifest.file_content_hash, &current.file_content_hash) {
        (Some(a), Some(b)) => a != b,
        // Nothing to compare: fall back to the whole worktree, which is coarser
        // and never wrong in the unsafe direction — it over-reports a re-read
        // rather than silently declaring a live comment stale.
        _ => current.worktree_digest != manifest.worktree_digest,
    };
    if file_changed { AnchorState::NeedsReread } else { AnchorState::Outdated }
}

fn hunk_context(h: &Hunk) -> String {
    // A hunk's own header carries git's chosen context line, which is the
    // cheapest stable description of where it sits.
    format!("{:x}", Sha256::digest(h.header.as_bytes()))
}

/// Every index at which `needle` appears as a contiguous run in `haystack`.
fn find_runs(haystack: &[&str], needle: &[&str]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    (0..=haystack.len() - needle.len())
        .filter(|&i| &haystack[i..i + needle.len()] == needle)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> CaptureManifest {
        CaptureManifest {
            base_commit: "base".into(),
            head_commit: "head".into(),
            worktree_digest: "digest".into(),
            file_content_hash: None,
            file_snapshot: None,
        }
    }

    fn lines_anchor(text: &str, ctx: &str) -> Anchor {
        Anchor::Lines {
            path: "src/x.rs".into(),
            side: Side::New,
            text: text.into(),
            context_fingerprint: ctx.into(),
        }
    }

    fn current_with(content: &str) -> Current {
        Current {
            head_commit: "head".into(),
            worktree_digest: "digest".into(),
            file_content: Some(content.into()),
            file_content_hash: Some(content_hash(content)),
            hunks: Vec::new(),
        }
    }

    fn ctx_at(content: &str, needle: &str) -> String {
        let hay: Vec<&str> = content.lines().collect();
        let nee: Vec<&str> = needle.lines().collect();
        let at = find_runs(&hay, &nee)[0];
        context_fingerprint(&hay, at, at + nee.len())
    }

    #[test]
    fn an_unchanged_anchor_resolves_exactly_where_it_was() {
        let content = "one\ntwo\nTARGET\nthree\nfour\n";
        let a = lines_anchor("TARGET", &ctx_at(content, "TARGET"));
        let r = resolve(&a, &manifest(), &current_with(content));
        assert_eq!(r.state, AnchorState::Exact);
        assert_eq!(r.line, Some(3));
    }

    #[test]
    fn code_moving_around_an_anchor_makes_it_moved_not_lost() {
        let before = "one\ntwo\nTARGET\nthree\n";
        let after = "PREAMBLE\nentirely\ndifferent\nTARGET\nthings\nafter\n";
        let a = lines_anchor("TARGET", &ctx_at(before, "TARGET"));
        let r = resolve(&a, &manifest(), &current_with(after));
        assert_eq!(r.state, AnchorState::Moved);
        assert_eq!(r.line, Some(4), "still points at the code it was about");
    }

    #[test]
    fn an_anchored_line_that_appears_twice_is_ambiguous_not_the_first_match() {
        // Both copies have identical surroundings, so context cannot break the
        // tie. Guessing here is how a comment lands on the wrong code.
        let content = "pad\nx\nDUP\ny\npad\nx\nDUP\ny\npad\n";
        let a = lines_anchor("DUP", "a fingerprint matching neither");
        let r = resolve(&a, &manifest(), &current_with(content));
        assert_eq!(r.state, AnchorState::Ambiguous);
        assert_eq!(r.line, None, "ambiguity must not carry a position");
        assert_eq!(r.effective, Anchor::File { path: "src/x.rs".into() });
    }

    #[test]
    fn context_breaks_a_tie_when_it_can() {
        let content = "alpha\nDUP\nbeta\n\n\n\n\ngamma\nDUP\ndelta\n";
        // Fingerprint the SECOND occurrence.
        let hay: Vec<&str> = content.lines().collect();
        let second = find_runs(&hay, &["DUP"])[1];
        let ctx = context_fingerprint(&hay, second, second + 1);
        let a = lines_anchor("DUP", &ctx);
        let r = resolve(&a, &manifest(), &current_with(content));
        assert_eq!(r.state, AnchorState::Exact);
        assert_eq!(r.line, Some(second as u32 + 1));
    }

    #[test]
    fn a_multi_line_anchor_matches_only_as_a_contiguous_run() {
        let content = "a\nONE\nTWO\nb\nONE\nc\nTWO\n";
        let a = lines_anchor("ONE\nTWO", &ctx_at(content, "ONE\nTWO"));
        let r = resolve(&a, &manifest(), &current_with(content));
        assert_eq!(r.state, AnchorState::Exact);
        assert_eq!(r.line, Some(2), "the split copies later in the file do not count");
    }

    #[test]
    fn an_anchor_whose_code_was_rewritten_needs_a_reread() {
        let before = "one\nTARGET\ntwo\n";
        let after = "one\nREWRITTEN\ntwo\n";
        let mut m = manifest();
        m.file_content_hash = Some(content_hash(before));
        let a = lines_anchor("TARGET", &ctx_at(before, "TARGET"));
        let r = resolve(&a, &m, &current_with(after));
        assert_eq!(r.state, AnchorState::NeedsReread);
        assert_eq!(r.effective, Anchor::File { path: "src/x.rs".into() });
    }

    #[test]
    fn an_anchor_that_never_matched_an_unchanged_file_is_outdated() {
        let content = "one\ntwo\n";
        let mut m = manifest();
        m.file_content_hash = Some(content_hash(content));
        let a = lines_anchor("NEVER PRESENT", "ctx");
        let r = resolve(&a, &m, &current_with(content));
        assert_eq!(r.state, AnchorState::Outdated, "nothing moved, so nothing to re-read");
    }

    #[test]
    fn a_deleted_file_degrades_all_the_way_to_the_workspace() {
        let a = lines_anchor("TARGET", "ctx");
        let gone = Current {
            head_commit: "head".into(),
            worktree_digest: "digest".into(),
            file_content: None,
            file_content_hash: None,
            hunks: Vec::new(),
        };
        let r = resolve(&a, &manifest(), &gone);
        assert_eq!(r.state, AnchorState::FileGone);
        assert_eq!(r.effective, Anchor::File { path: "src/x.rs".into() });
        // One rung at a time: File is the next rung down from Lines, and File's
        // own degradation is Workspace.
        assert_eq!(r.effective.degraded(), Anchor::Workspace);
    }

    #[test]
    fn an_unanchored_entry_rereads_when_the_worktree_digest_moves() {
        // The majority case. An earlier design derived this from vanished anchor
        // text, which an unanchored entry does not have, so it never fired.
        let mut cur = Current { head_commit: "head".into(), ..Default::default() };
        cur.worktree_digest = "digest".into();
        let r = resolve(&Anchor::None, &manifest(), &cur);
        assert_eq!(r.state, AnchorState::Exact, "nothing moved yet");

        cur.worktree_digest = "a file was edited again".into();
        let r = resolve(&Anchor::None, &manifest(), &cur);
        assert_eq!(r.state, AnchorState::NeedsReread);
        assert_eq!(r.effective, Anchor::None, "an unanchored entry has no rung to fall to");
    }

    #[test]
    fn an_unanchored_entry_rereads_when_head_moves_even_if_the_tree_is_clean() {
        let cur = Current {
            head_commit: "a new commit".into(),
            worktree_digest: "digest".into(),
            ..Default::default()
        };
        assert_eq!(resolve(&Anchor::None, &manifest(), &cur).state, AnchorState::NeedsReread);
    }

    #[test]
    fn a_file_anchor_rereads_on_its_own_file_and_ignores_the_rest() {
        let a = Anchor::File { path: "src/x.rs".into() };
        let mut m = manifest();
        m.file_content_hash = Some(content_hash("original"));

        let same = current_with("original");
        assert_eq!(resolve(&a, &m, &same).state, AnchorState::Exact);

        let changed = current_with("edited");
        assert_eq!(resolve(&a, &m, &changed).state, AnchorState::NeedsReread);
    }

    #[test]
    fn degrading_is_one_rung_at_a_time_and_never_skips() {
        let lines = lines_anchor("x", "c");
        let file = lines.degraded();
        assert!(matches!(file, Anchor::File { .. }));
        assert_eq!(file.degraded(), Anchor::Workspace);
        // Workspace is the floor: it degrades to itself rather than vanishing.
        assert_eq!(Anchor::Workspace.degraded(), Anchor::Workspace);
        assert_eq!(Anchor::None.degraded(), Anchor::None);
    }

    #[test]
    fn a_position_is_never_attached_to_a_state_that_did_not_earn_one() {
        // The invariant the whole module exists for. A line may only ride along
        // with Exact or Moved; Ambiguous, NeedsReread, Outdated and FileGone
        // must arrive bare, because any number they carried would be a guess.
        let content = "a\nTARGET\nb\n";
        let dup = "pad\nx\nDUP\ny\npad\nx\nDUP\ny\npad\n";
        let cases = vec![
            resolve(&lines_anchor("TARGET", &ctx_at(content, "TARGET")), &manifest(), &current_with(content)),
            resolve(&lines_anchor("TARGET", "stale ctx"), &manifest(), &current_with(content)),
            resolve(&lines_anchor("MISSING", "ctx"), &manifest(), &current_with(content)),
            resolve(&lines_anchor("DUP", "matches neither"), &manifest(), &current_with(dup)),
            resolve(&Anchor::None, &manifest(), &current_with(content)),
            resolve(&Anchor::File { path: "src/x.rs".into() }, &manifest(), &current_with(content)),
        ];
        for r in cases {
            if r.line.is_some() {
                assert!(
                    r.state.may_carry_position(),
                    "state {:?} carried line {:?}, which it has no right to",
                    r.state,
                    r.line
                );
            }
        }
    }

    #[test]
    fn a_positional_anchor_that_resolves_always_says_where() {
        let content = "a\nTARGET\nb\n";
        for ctx in ["stale ctx", &ctx_at(content, "TARGET")] {
            let r = resolve(&lines_anchor("TARGET", ctx), &manifest(), &current_with(content));
            assert!(r.state.may_carry_position());
            assert!(r.line.is_some(), "a resolved Lines anchor without a line is useless");
        }
    }
}
