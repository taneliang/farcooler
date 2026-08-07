//! Images pasted into a terminal.
//!
//! A terminal takes bytes, and an agent running in one reads an image by
//! opening a path. So an image pasted from a phone — or from a Mac attached to
//! a machine that is not this one — has to become a file here before it can
//! become anything the agent can see.
//!
//! The bytes arrive in chunks, accumulate under `.incoming`, and are renamed
//! into place only when the last one lands. Nothing reading the directory ever
//! sees a half-written image under a name that looks finished.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::MAX_IMAGE_PASTE_BYTES;

/// How long a finished paste survives.
///
/// Seven days rather than one because conversations get resumed: an agent that
/// read an image yesterday and is asked about it tomorrow morning should still
/// find the file its own transcript names.
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How long a partial transfer survives.
///
/// A transfer does not resume across a reconnect — the client starts over — so
/// anything still in `.incoming` an hour later is abandoned by definition.
const KEEP_INCOMING: Duration = Duration::from_secs(60 * 60);

/// An image format this daemon will write.
///
/// The list is what the agents can actually open. HEIC is deliberately absent:
/// Claude Code and Codex both refuse it, so accepting one here would produce a
/// file that exists, has a path, and cannot be read — the worst of the three
/// outcomes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Kind {
    fn extension(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
        }
    }
}

/// What the bytes are, whatever the sender called them.
///
/// The `mime` on the wire is a claim by whoever is connected. This is the
/// daemon deciding for itself, because the answer picks a filename extension on
/// the user's own machine and "trust the sender" is not a policy that survives
/// the question "and what if it lied".
pub fn sniff(b: &[u8]) -> Option<Kind> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some(Kind::Png);
    }
    // SOI, then any marker. The next byte varies by encoder, so matching more
    // than this rejects valid JPEGs.
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(Kind::Jpeg);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    // RIFF....WEBP: the four size bytes in between are not ours to check.
    if b.len() >= 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        return Some(Kind::Webp);
    }
    None
}

/// What a chunk did.
#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    /// More is expected. Carries the bytes on disk, which is what the sender
    /// must use as its next `offset`.
    Partial { stored: u64 },
    /// The image is complete and named. Nothing further is accepted for this
    /// transfer id.
    Complete { path: PathBuf, stored: u64 },
}

/// Accept one chunk of a transfer.
///
/// `offset` must equal what is already stored. A chunk entirely inside what has
/// been written is treated as a retransmit and changes nothing; anything else
/// fails the transfer rather than writing at a guessed position, because a file
/// assembled out of order is a file that sniffs correctly and decodes to
/// garbage.
pub async fn put_chunk(
    root: &Path,
    transfer_id: &[u8],
    total_size: u64,
    offset: u64,
    chunk: &[u8],
) -> Result<Stored> {
    put_chunk_in(
        &crate::paths::pastes_dir_in(root)?,
        &crate::paths::pastes_incoming_dir_in(root)?,
        transfer_id,
        total_size,
        offset,
        chunk,
        SystemTime::now(),
    )
    .await
}

/// `put_chunk` against explicit directories.
///
/// The directories and the clock are arguments so this is testable without a
/// process-wide `FARCOOLER_HOME`, which tests running in parallel cannot share.
#[allow(clippy::too_many_arguments)]
pub async fn put_chunk_in(
    dir: &Path,
    incoming: &Path,
    transfer_id: &[u8],
    total_size: u64,
    offset: u64,
    chunk: &[u8],
    now: SystemTime,
) -> Result<Stored> {
    if transfer_id.is_empty() {
        return Err(DomainError::InvalidArgument { what: "transfer id" });
    }
    if total_size == 0 || total_size > MAX_IMAGE_PASTE_BYTES {
        return Err(DomainError::InvalidArgument { what: "image size" });
    }

    // Hex, so a transfer id can never contribute a separator, a dot-dot, or a
    // NUL to the path it names.
    let name: String = transfer_id.iter().map(|b| format!("{b:02x}")).collect();
    let partial = incoming.join(&name);

    let stored = match tokio::fs::metadata(&partial).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    // A retransmit of bytes already accepted. Nothing to do, and saying so is
    // what lets a client retry a chunk whose response it never saw.
    if offset < stored && offset + chunk.len() as u64 <= stored {
        return Ok(Stored::Partial { stored });
    }
    if offset != stored {
        return Err(DomainError::InvalidArgument { what: "image offset" });
    }
    if stored + chunk.len() as u64 > total_size {
        return Err(DomainError::InvalidArgument { what: "image size" });
    }

    // Refuse on the first chunk, before any of it is on disk. Every format here
    // is identifiable from its first twelve bytes, so there is no case where
    // waiting for the whole upload would learn something this cannot.
    if offset == 0 && sniff(chunk).is_none() {
        return Err(DomainError::InvalidArgument { what: "image format" });
    }

    append(&partial, chunk).await?;
    let stored = stored + chunk.len() as u64;
    if stored < total_size {
        return Ok(Stored::Partial { stored });
    }

    let kind = sniff(&head(&partial).await?).ok_or(DomainError::InvalidArgument {
        what: "image format",
    })?;
    let path = finalize(dir, &partial, kind, now).await?;
    Ok(Stored::Complete { path, stored })
}

async fn append(path: &Path, chunk: &[u8]) -> std::result::Result<(), DomainError> {
    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "could not open a paste for append");
            DomainError::OperationFailed
        })?;
    f.write_all(chunk).await.map_err(|e| {
        tracing::warn!(error = %e, "could not write a paste chunk");
        DomainError::OperationFailed
    })?;
    f.flush().await.map_err(|_| DomainError::OperationFailed)
}

/// Enough of a file to sniff it.
async fn head(path: &Path) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await.map_err(|_| DomainError::NotFound)?;
    let mut buf = vec![0u8; 12];
    let n = f.read(&mut buf).await.map_err(|_| DomainError::OperationFailed)?;
    buf.truncate(n);
    Ok(buf)
}

/// Give a completed transfer its real name.
///
/// The rename is what publishes it. Until this returns, the bytes are under
/// `.incoming` with a name nothing looks for.
async fn finalize(dir: &Path, partial: &Path, kind: Kind, now: SystemTime) -> Result<PathBuf> {
    let stamp = stamp(now);
    let ext = kind.extension();

    // A counter rather than a longer timestamp: two pastes in the same second
    // is a person pasting twice, and `-2` says that more clearly than
    // milliseconds do.
    for n in 1..1000 {
        let name =
            if n == 1 { format!("{stamp}.{ext}") } else { format!("{stamp}-{n}.{ext}") };
        let candidate = dir.join(&name);
        if tokio::fs::metadata(&candidate).await.is_ok() {
            continue;
        }
        tokio::fs::rename(partial, &candidate).await.map_err(|e| {
            tracing::warn!(error = %e, "could not name a finished paste");
            DomainError::OperationFailed
        })?;
        return Ok(candidate);
    }
    Err(DomainError::OperationFailed)
}

/// `2026-08-07-141233Z`.
///
/// UTC, and said so with the `Z`. There is no date library in this workspace
/// and adding one to name a file is a poor trade; a local time would need a
/// timezone database, while this needs arithmetic. Sorts chronologically as
/// text, which is what makes the directory readable.
fn stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}-{hh:02}{mm:02}{ss:02}Z")
}

/// Days since the Unix epoch to a calendar date.
///
/// Hinnant's civil-from-days, which is exact for every date this will ever see
/// and needs no table. Shifting the era to March makes the leap day the last
/// day of the year, which is what removes the special cases.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The path as it should be typed into a terminal.
///
/// The default paste directory is under `~/Library/Application Support/Far
/// Cooler`, so the common case has two spaces in it before anything the user
/// did. Typed bare, a shell sees four arguments and an agent scanning a prompt
/// for a file reference finds one that stops at `Application`.
///
/// Double quotes rather than backslashes: an agent strips quotes readily and
/// handles `\ ` inconsistently, and a quoted path survives being echoed back
/// into a transcript legibly. `FARCOOLER_HOME` is whatever the user set, so the
/// escaping inside the quotes is real rather than decorative.
pub fn quote_for_paste(path: &str) -> String {
    let needs = path.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c));
    if !needs {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Wrap a paste so the program can tell it apart from typing.
///
/// The same rule as `farcooler_vt::encode_paste`, deliberately not the same
/// code. Reusing it would put `alacritty_terminal` in the daemon's dependency
/// graph — a terminal emulator linked into the host process — to wrap six
/// bytes around a path. If a third caller ever needs this, that is the moment
/// to lift it into `farcooler-core` rather than now.
///
/// The terminator is stripped from the payload for the reason vt strips it: a
/// paste that contains `ESC[201~` would otherwise close its own bracket, and
/// everything after it would arrive as typing — which for a shell means
/// running it.
pub fn encode_paste(bracketed: bool, text: &str) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }
    let mut out = Vec::with_capacity(text.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

/// Delete what has expired.
///
/// Runs on startup and daily. Failures are logged and skipped rather than
/// propagated: a paste that cannot be deleted is a directory that grows, not a
/// daemon that should stop.
pub async fn sweep(root: &Path) {
    let now = SystemTime::now();
    for (dir, keep) in [
        (crate::paths::pastes_dir_in(root), KEEP),
        (crate::paths::pastes_incoming_dir_in(root), KEEP_INCOMING),
    ] {
        let Ok(dir) = dir else { continue };
        sweep_dir(&dir, keep, now).await;
    }
}

/// Delete files in one directory older than `keep`.
///
/// Directories are skipped rather than recursed: `.incoming` lives inside the
/// paste directory and is swept on its own, shorter clock.
pub async fn sweep_dir(dir: &Path, keep: Duration, now: SystemTime) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Ok(meta) = entry.metadata().await else { continue };
        if meta.is_dir() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else { continue };
        if age > keep {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                tracing::warn!(error = %e, "could not sweep a paste");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accepted_format_is_recognized_by_its_magic() {
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]), Some(Kind::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some(Kind::Jpeg));
        assert_eq!(sniff(b"GIF89a...."), Some(Kind::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
    }

    #[test]
    fn anything_else_is_refused() {
        // HEIC in particular: an iPhone photo that reached the daemon
        // unconverted must fail here rather than become a path the agent
        // cannot open.
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), None);
        assert_eq!(sniff(b"#!/bin/sh\n"), None);
        assert_eq!(sniff(b""), None);
        // A truncated PNG signature is not a PNG.
        assert_eq!(sniff(&[0x89, b'P', b'N']), None);
    }

    #[test]
    fn a_path_with_spaces_is_quoted_and_a_plain_one_is_not() {
        assert_eq!(quote_for_paste("/tmp/a.png"), "/tmp/a.png");
        assert_eq!(
            quote_for_paste("/Users/e/Library/Application Support/Far Cooler/pastes/x.png"),
            "\"/Users/e/Library/Application Support/Far Cooler/pastes/x.png\""
        );
    }

    #[test]
    fn quoting_escapes_what_double_quotes_do_not_cover() {
        // FARCOOLER_HOME is whatever the user set it to.
        assert_eq!(quote_for_paste(r#"/tmp/a"b/x.png"#), r#""/tmp/a\"b/x.png""#);
        assert_eq!(quote_for_paste(r"/tmp/a\b/x.png"), r#""/tmp/a\\b/x.png""#);
        assert_eq!(quote_for_paste("/tmp/$HOME x/a.png"), r#""/tmp/\$HOME x/a.png""#);
    }

    #[test]
    fn a_paste_is_bracketed_only_when_the_program_asked() {
        assert_eq!(encode_paste(false, "/tmp/a.png "), b"/tmp/a.png ".to_vec());
        assert_eq!(
            encode_paste(true, "/tmp/a.png "),
            b"\x1b[200~/tmp/a.png \x1b[201~".to_vec()
        );
    }

    #[test]
    fn a_pasted_path_cannot_close_its_own_bracket() {
        // FARCOOLER_HOME is user-set, so a directory name really can contain
        // this. Left in, everything after it arrives as typing — and a shell
        // runs typing.
        let out = encode_paste(true, "/tmp/\x1b[201~; rm -rf ~/a.png ");
        let s = String::from_utf8(out).expect("utf-8");
        assert_eq!(s.matches("\x1b[201~").count(), 1, "one terminator, at the end: {s:?}");
        assert!(s.ends_with("\x1b[201~"));
    }

    #[test]
    fn a_stamp_is_utc_and_sorts_as_text() {
        let t = UNIX_EPOCH + Duration::from_secs(1_770_474_753);
        assert_eq!(stamp(t), "2026-02-07-143233Z");
        assert_eq!(stamp(UNIX_EPOCH), "1970-01-01-000000Z");
    }

    /// A minimal but real PNG header, followed by filler.
    fn png(len: usize) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        b.resize(len, 0x5a);
        b
    }

    struct Dirs {
        _tmp: tempfile::TempDir,
        dir: PathBuf,
        incoming: PathBuf,
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("pastes");
        let incoming = dir.join(".incoming");
        std::fs::create_dir_all(&incoming).expect("dirs");
        Dirs { _tmp: tmp, dir, incoming }
    }

    async fn put(d: &Dirs, id: &[u8], total: u64, offset: u64, chunk: &[u8]) -> Result<Stored> {
        put_chunk_in(&d.dir, &d.incoming, id, total, offset, chunk, SystemTime::now()).await
    }

    #[tokio::test]
    async fn a_whole_image_in_one_chunk_is_named_and_readable() {
        let d = dirs();
        let bytes = png(64);
        let out = put(&d, &[1, 2], 64, 0, &bytes).await.expect("stored");
        let Stored::Complete { path, stored } = out else { panic!("expected complete") };
        assert_eq!(stored, 64);
        assert_eq!(std::fs::read(&path).expect("read"), bytes);
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
    }

    #[tokio::test]
    async fn nothing_appears_under_a_final_name_until_the_last_chunk() {
        let d = dirs();
        let bytes = png(64);
        let out = put(&d, &[9], 64, 0, &bytes[..32]).await.expect("stored");
        assert_eq!(out, Stored::Partial { stored: 32 });

        // The whole point of `.incoming`: a directory listing of finished
        // pastes must not show one that is still arriving.
        let visible: Vec<_> = std::fs::read_dir(&d.dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        assert!(visible.is_empty(), "a partial paste was visible: {visible:?}");

        put(&d, &[9], 64, 32, &bytes[32..]).await.expect("stored");
        let files: Vec<_> = std::fs::read_dir(&d.dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn an_offset_that_does_not_match_is_refused_rather_than_written() {
        let d = dirs();
        let bytes = png(64);
        put(&d, &[3], 64, 0, &bytes[..32]).await.expect("stored");

        // A gap. Accepting this would produce a file that sniffs as a PNG and
        // decodes to garbage, which is worse than a failed paste.
        let err = put(&d, &[3], 64, 48, &bytes[48..]).await.expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { .. }), "{err:?}");
        assert_eq!(std::fs::metadata(d.incoming.join("03")).expect("partial").len(), 32);
    }

    #[tokio::test]
    async fn a_retransmitted_chunk_changes_nothing() {
        let d = dirs();
        let bytes = png(64);
        put(&d, &[4], 64, 0, &bytes[..32]).await.expect("stored");
        // The sender never saw the response and sent it again.
        let again = put(&d, &[4], 64, 0, &bytes[..32]).await.expect("stored");
        assert_eq!(again, Stored::Partial { stored: 32 });
    }

    #[tokio::test]
    async fn a_format_no_agent_can_open_is_refused_on_the_first_chunk() {
        let d = dirs();
        // HEIC: what an unconverted iPhone photo would arrive as.
        let heic = b"\0\0\0\x18ftypheic____________";
        let err = put(&d, &[5], 24, 0, heic).await.expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { what: "image format" }), "{err:?}");
        // Refused before anything was written, not after the whole upload.
        assert!(std::fs::metadata(d.incoming.join("05")).is_err());
    }

    #[tokio::test]
    async fn an_oversize_image_is_refused_before_a_byte_is_written() {
        let d = dirs();
        let err = put(&d, &[6], MAX_IMAGE_PASTE_BYTES + 1, 0, &png(64))
            .await
            .expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { what: "image size" }), "{err:?}");
        assert!(std::fs::metadata(d.incoming.join("06")).is_err());
    }

    #[tokio::test]
    async fn a_chunk_past_the_declared_size_is_refused() {
        let d = dirs();
        let err = put(&d, &[7], 32, 0, &png(64)).await.expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { what: "image size" }), "{err:?}");
    }

    #[tokio::test]
    async fn two_pastes_in_the_same_second_get_a_counter() {
        let d = dirs();
        let now = UNIX_EPOCH + Duration::from_secs(1_770_474_753);
        let bytes = png(16);
        let first =
            put_chunk_in(&d.dir, &d.incoming, &[10], 16, 0, &bytes, now).await.expect("first");
        let second =
            put_chunk_in(&d.dir, &d.incoming, &[11], 16, 0, &bytes, now).await.expect("second");
        let (Stored::Complete { path: a, .. }, Stored::Complete { path: b, .. }) =
            (first, second)
        else {
            panic!("expected two complete pastes")
        };
        assert_eq!(a.file_name().unwrap(), "2026-02-07-143233Z.png");
        assert_eq!(b.file_name().unwrap(), "2026-02-07-143233Z-2.png");
    }

    #[tokio::test]
    async fn the_sweep_deletes_at_seven_days_and_leaves_six() {
        let d = dirs();
        let old = d.dir.join("old.png");
        let new = d.dir.join("new.png");
        std::fs::write(&old, png(16)).expect("write");
        std::fs::write(&new, png(16)).expect("write");

        // Sweeping "now + 6 days" leaves both; "now + 8 days" takes the ones
        // written now. Moving the clock rather than the files' mtimes keeps
        // this independent of filesystem timestamp resolution.
        let now = SystemTime::now();
        sweep_dir(&d.dir, KEEP, now + Duration::from_secs(6 * 24 * 3600)).await;
        assert!(old.exists() && new.exists(), "nothing should expire at six days");

        sweep_dir(&d.dir, KEEP, now + Duration::from_secs(8 * 24 * 3600)).await;
        assert!(!old.exists() && !new.exists(), "both should expire at eight days");
    }

    #[tokio::test]
    async fn the_sweep_leaves_the_incoming_directory_alone() {
        let d = dirs();
        // It is a directory inside the swept one, and it has its own clock.
        sweep_dir(&d.dir, Duration::from_secs(0), SystemTime::now() + Duration::from_secs(1))
            .await;
        assert!(d.incoming.is_dir(), ".incoming must survive a sweep of its parent");
    }

    #[test]
    fn the_civil_calendar_handles_leap_days() {
        // 2024-02-29 is day 19782.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000 is a leap year and 1900 is not, which is the case a naive
        // every-fourth-year rule gets wrong.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
