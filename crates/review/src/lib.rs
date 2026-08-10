//! What changed, and where a comment about it still points.
//!
//! Two jobs, and they are here together because the second is meaningless
//! without the first:
//!
//! 1. **Turn git's patch output into hunks**, or refuse it. A parser that
//!    guesses at a format it does not recognize produces a diff that looks
//!    right and is not, which is worse than no diff at all.
//!
//! 2. **Resolve an anchor against the diff as it is now.** A review comment is
//!    written against code that an agent is still editing. The line number it
//!    was written at is stale within seconds, so no line number is ever stored:
//!    an anchor carries the text it was about, and finding it again is a
//!    computation, not a lookup.
//!
//! Both live in one crate, in Rust, behind one C ABI, because three clients
//! render this and three implementations would disagree about what a hunk is —
//! the same reason colour resolution sits in `farcooler-vt` rather than in each
//! renderer.
//!
//! ```text
//!   git patch bytes ──> parse_unified ──> [Hunk] ──┐
//!                            │                     ├──> resolve() ──> AnchorState
//!                            └── refuses           │
//!                                combined diffs    │
//!   old text, new text ──> diff_texts ──> [Line] ──┘
//! ```

pub mod anchor;
pub mod ffi;
pub mod diff;

pub use anchor::{Anchor, AnchorState, CaptureManifest, Resolution};
pub use diff::{DiffError, FileDiff, Hunk, Line, LineKind, Truncation, diff_texts, parse_unified};

/// Caps, shared by everything that produces a [`FileDiff`].
///
/// A cap is not a performance tuning knob here, it is a promise: a client on a
/// phone asked for one file and must get an answer whose size it can predict,
/// whatever the file turned out to be. Exceeding one is reported as
/// [`Truncation`], never as a shorter diff that claims to be whole.
pub mod limits {
    /// Lines in one file's diff.
    pub const MAX_LINES: usize = 4_000;
    /// Hunks in one file's diff.
    pub const MAX_HUNKS: usize = 400;
    /// Bytes of patch text accepted for one file.
    pub const MAX_BYTES: usize = 2 * 1024 * 1024;
    /// Bytes of a file kept as a capture snapshot, so a comment written against
    /// a dirty working tree can still say what changed under it later.
    pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
}
