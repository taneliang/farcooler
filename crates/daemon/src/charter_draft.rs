//! The charter the survey drafts (ov-519), and writing it.
//!
//! **The same file the interview writes.** The manager skill reads
//! `$FARCOOLER_CHARTER`, the workspace home's `charter.md`
//! (`workspace_home::charter_path`), and interviews the owner only for a
//! heading it lacks. A charter rendered here carries every heading of
//! `skill_install::CHARTER_SECTIONS`, in order, so an orchestrator started on
//! it goes straight to work.
//!
//! **Or a draft beside it.** Talk It Through writes `charter-draft.md` in the
//! same home instead. The skill finds it there when the charter is missing,
//! and goes through it with the owner rather than interviewing from nothing;
//! it writes the charter once they say yes. A file, not an argument to the
//! orchestrator's first prompt, because that prompt is sent once, and a
//! restart mid-conversation would lose it.
//!
//! **A charter is never overwritten**, as everywhere in `workspace_home`: it
//! is created or left alone. A draft is ours until the charter exists, so it
//! is replaced whole; once the charter is written, the draft is removed.

use std::io;
use std::path::{Path, PathBuf};

use crate::skill_install::CHARTER_SECTIONS;
use crate::survey::{Answer, Source};

/// The draft's file name, beside the charter in the workspace's home.
pub const DRAFT_FILE: &str = "charter-draft.md";

/// The comment after an answer, naming where it came from.
fn provenance(source: &Source) -> String {
    match source {
        // One line, and no run of dashes that could end the comment early,
        // however the signal was made (`farcooler_core::phrase`).
        Source::Signal(why) => format!("<!-- from: {} -->", farcooler_core::phrase::phrase(why)),
        Source::Default => "<!-- default -->".to_string(),
        Source::Chosen => "<!-- chosen -->".to_string(),
    }
}

/// The charter `answers` make, dated `date` (`YYYY-MM-DD`): the first line
/// says it came from the survey, then every heading in
/// `CHARTER_SECTIONS`'s order, each answer under its heading as a paragraph
/// ending with the comment naming its source. An answer with no heading
/// (`harness`) is the app's, not the charter's.
pub fn render(answers: &[Answer], date: &str) -> String {
    let mut out = format!("<!-- charter, written {date} from Far Cooler's repository survey; edit freely -->\n");
    for heading in CHARTER_SECTIONS {
        out.push('\n');
        out.push_str(heading);
        out.push('\n');
        let under: Vec<&Answer> = answers.iter().filter(|a| a.heading == *heading).collect();
        if under.is_empty() {
            out.push_str("Nothing yet.\n<!-- default -->\n");
        }
        for a in under {
            out.push_str(&farcooler_core::phrase::phrase_within(&a.text, crate::survey::TEXT_MAX));
            out.push('\n');
            out.push_str(&provenance(&a.source));
            out.push('\n');
        }
    }
    out
}

/// What `write` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    pub wrote: bool,
}

/// Write `text` into the workspace home `home`: as `charter.md` when `draft`
/// is false, never over one that exists, removing the draft once it is
/// written; or as `charter-draft.md`, whole, unless the charter already
/// exists, which a draft would only confuse.
///
/// The charter appears whole or not at all (review 2a, L6): it is written
/// to a temporary file in the home, synced, then hard-linked into place.
/// The link fails if `charter.md` exists, so it is still never overwritten,
/// and a write that fails (a full disk) leaves no partial charter that would
/// then be "never overwritten" forever. A draft written while another call
/// writes the charter can be left beside it; the skill deletes it.
pub fn write(home: &Path, text: &str, draft: bool) -> io::Result<Written> {
    write_between(home, text, draft, || {})
}

/// `write`, calling `between` once the charter's text is whole on disk and
/// before it takes the charter's name: where a test looks for a charter
/// that is visible too early.
fn write_between(home: &Path, text: &str, draft: bool, between: impl FnOnce()) -> io::Result<Written> {
    use std::io::Write;
    std::fs::create_dir_all(home)?;
    let charter = home.join(crate::workspace_home::CHARTER_FILE);
    let draft_path = home.join(DRAFT_FILE);
    if draft {
        if charter.exists() {
            return Ok(Written { path: draft_path, wrote: false });
        }
        crate::skill_install::write_atomically(&draft_path, text)?;
        return Ok(Written { path: draft_path, wrote: true });
    }
    if charter.symlink_metadata().is_ok() {
        return Ok(Written { path: charter, wrote: false });
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temporary = home.join(format!(".charter-{}-{nanos}.tmp", std::process::id()));
    let linked = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        between();
        std::fs::hard_link(&temporary, &charter)
    })();
    // The temporary name goes either way; the charter keeps the inode.
    let _ = std::fs::remove_file(&temporary);
    match linked {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(Written { path: charter, wrote: false }),
        Err(e) => return Err(e),
    }
    match std::fs::remove_file(&draft_path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %draft_path.display(), error = %e, "could not remove the charter's draft"),
    }
    Ok(Written { path: charter, wrote: true })
}

/// Today, as the charter's first line dates it, in UTC.
pub fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    let (y, m, d) = crate::pastes::civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
#[path = "charter_draft_tests.rs"]
pub(crate) mod tests;
