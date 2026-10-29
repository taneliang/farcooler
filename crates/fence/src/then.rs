//! [`update`](crate::update) with a step that runs inside the lock after the
//! write (ov-856), in its own file: `lib.rs` is at its size ceiling.

use std::path::Path;

use super::{
    Change, Entry, FenceError, Markers, Placement, hold_lock, open_fence_dir, parse_within, read_at, write_locked,
};

/// [`update`](crate::update), then `after` with the answer, still inside the
/// lock and only once the write (if `decide` asked for one) has landed.
///
/// For a change that has to follow the file and not precede it: the store's
/// half of a demotion, which commits only once the lowered lines are on disk
/// (a failed write changes nothing), and the rows a revoke clears for a
/// device whose line has just gone (a failed write must not leave a line
/// with no rows, which would read as the owner's). `after` runs on the same
/// thread, before the lock is released, so no other writer sees the file
/// changed and the store not.
pub fn update_then<T, E>(
    path: &Path,
    markers: Markers<'_>,
    placement: Placement,
    decide: impl FnOnce(&[Entry]) -> Result<(Change, T), E>,
    after: impl FnOnce(&T) -> Result<(), E>,
) -> Result<T, E>
where
    E: From<FenceError>,
{
    let name = path.file_name().ok_or(FenceError::Missing)?;
    let directory = open_fence_dir(path.parent().ok_or(FenceError::Missing)?)?;
    let _lock = hold_lock(&directory, name)?;

    // Inside the lock, and that is the entire point of this function. Moving
    // these three lines above `hold_lock` restores the old shape, and
    // `two_concurrent_enrollments_both_survive` fails when they are.
    let current = read_at(&directory, name)?;
    let text = current.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
    let entries = parse_within(text, markers)?;

    let (change, answer) = decide(&entries)?;
    if let Change::Write { entries, foreign } = change {
        write_locked(&directory, name, current.as_ref(), markers, &entries, &foreign, placement)?;
    }
    after(&answer)?;
    Ok(answer)
}
