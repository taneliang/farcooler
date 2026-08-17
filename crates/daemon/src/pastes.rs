//! Files pasted or dropped into a terminal.
//!
//! A terminal takes bytes, and an agent running in one reads a file by opening
//! a path. So anything handed to a pane from a phone — or from a Mac attached
//! to a runner that is not this one — has to become a file here before it can
//! become anything the agent can see.
//!
//! The bytes arrive in chunks, accumulate under `.incoming`, and are renamed
//! into place only when the last one lands. Nothing reading the directory ever
//! sees a half-written image under a name that looks finished.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::MAX_PASTE_FILE_BYTES;

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

/// An image this daemon can recognize from its first bytes.
///
/// Only used to give a name an extension when the sender supplied neither. It
/// is no longer a gate: any file may be pasted, because the scope that reaches
/// this method is the same one that can type into a shell.
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

/// What the bytes are, when they are an image this daemon knows.
///
/// The `mime` on the wire is a claim by whoever is connected, so a recognizable
/// image gets its real extension regardless of what it was called. Anything
/// unrecognized keeps the sender's own extension, sanitized.
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
    name: &str,
    total_size: u64,
    offset: u64,
    chunk: &[u8],
) -> Result<Stored> {
    put_chunk_in(
        &crate::paths::pastes_dir_in(root)?,
        &crate::paths::pastes_incoming_dir_in(root)?,
        transfer_id,
        name,
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
    name: &str,
    total_size: u64,
    offset: u64,
    chunk: &[u8],
    now: SystemTime,
) -> Result<Stored> {
    if transfer_id.is_empty() {
        return Err(DomainError::InvalidArgument { what: "transfer id" });
    }
    if total_size == 0 || total_size > MAX_PASTE_FILE_BYTES {
        return Err(DomainError::InvalidArgument { what: "file size" });
    }

    // Hex, so a transfer id can never contribute a separator, a dot-dot, or a
    // NUL to the path it names. Deliberately not called `name`: that is the
    // sender's filename, and one shadowing the other is how a partial's hex
    // would end up being what the finished file is called.
    let partial_name: String = transfer_id.iter().map(|b| format!("{b:02x}")).collect();
    let partial = incoming.join(&partial_name);

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
        return Err(DomainError::InvalidArgument { what: "file offset" });
    }
    if stored + chunk.len() as u64 > total_size {
        return Err(DomainError::InvalidArgument { what: "file size" });
    }

    append(&partial, chunk).await?;
    let stored = stored + chunk.len() as u64;
    if stored < total_size {
        return Ok(Stored::Partial { stored });
    }

    let path = finalize(dir, &partial, name, sniff(&head(&partial).await?), now).await?;
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
async fn finalize(
    dir: &Path,
    partial: &Path,
    given: &str,
    kind: Option<Kind>,
    now: SystemTime,
) -> Result<PathBuf> {
    let stamp = stamp(now);
    let (base, ext) = name_parts(given, kind);

    // A counter rather than a longer timestamp: two pastes in the same second
    // is a person pasting twice, and `-2` says that more clearly than
    // milliseconds do.
    for n in 1..1000 {
        let stem = if n == 1 { format!("{stamp}-{base}") } else { format!("{stamp}-{n}-{base}") };
        let name = if ext.is_empty() { stem } else { format!("{stem}.{ext}") };
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

/// A safe stem and extension for a file the far side named.
///
/// The sender's name is worth keeping — `quarterly-report.pdf` in a prompt says
/// what it is and `2026-08-09-114812Z.bin` says nothing — but it is a string
/// from another runner that becomes a filename on this one. So it is rebuilt
/// rather than filtered: every character not explicitly allowed is dropped, and
/// what comes out cannot contain a separator, a `..`, a NUL, a leading dot or a
/// shell metacharacter, whatever went in.
///
/// A recognizable image overrides the extension it was given. That is the one
/// case where the bytes are more trustworthy than the label, and it costs
/// nothing to be right about a `.png` that arrived called `.txt`.
fn name_parts(given: &str, kind: Option<Kind>) -> (String, String) {
    // Only the last component, and only after `..` can no longer mean anything:
    // both separators, because a Windows client naming `a\b` must not produce
    // a directory here either.
    let last = given.rsplit(['/', '\\']).next().unwrap_or("");
    let (stem, given_ext) = match last.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, e),
        _ => (last, ""),
    };

    let keep = |s: &str, cap: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(cap)
            .collect()
    };

    let base = keep(stem, 64);
    let ext = match kind {
        Some(k) => k.extension().to_string(),
        None => keep(given_ext, 16).to_lowercase(),
    };
    // `file` rather than nothing, so a name made entirely of characters this
    // drops still produces something a person can read and select.
    (if base.is_empty() { "file".to_string() } else { base }, ext)
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
        put_chunk_in(&d.dir, &d.incoming, id, "shot.png", total, offset, chunk, SystemTime::now())
            .await
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
    async fn a_file_that_is_not_an_image_is_accepted_and_keeps_its_name() {
        // The feature is "drop the thing you are talking about on the pane",
        // and most of those things are not images. A PDF that arrived here and
        // was refused would be the whole point missed.
        let d = dirs();
        let pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let out = put_chunk_in(
            &d.dir,
            &d.incoming,
            &[5],
            "Quarterly Report.pdf",
            pdf.len() as u64,
            0,
            &pdf,
            SystemTime::now(),
        )
        .await
        .expect("accepted");
        let Stored::Complete { path, .. } = out else { panic!("expected complete") };
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.ends_with("-QuarterlyReport.pdf"), "{name}");
        assert_eq!(std::fs::read(&path).expect("read"), pdf);
    }

    #[tokio::test]
    async fn a_heic_photo_is_accepted_now_that_this_is_not_image_only() {
        // It was refused when this was an image feature, because no agent can
        // open one. It is accepted now for the same reason a PDF is: what the
        // agent does with the file is the agent's business, and refusing a
        // format is not this daemon's job once any file may be sent.
        let d = dirs();
        let heic = b"\0\0\0\x18ftypheic____________".to_vec();
        let out = put_chunk_in(
            &d.dir,
            &d.incoming,
            &[6],
            "IMG_0042.heic",
            heic.len() as u64,
            0,
            &heic,
            SystemTime::now(),
        )
        .await
        .expect("accepted");
        assert!(matches!(out, Stored::Complete { .. }));
    }

    #[tokio::test]
    async fn an_oversize_file_is_refused_before_a_byte_is_written() {
        let d = dirs();
        let err = put(&d, &[8], MAX_PASTE_FILE_BYTES + 1, 0, &png(64))
            .await
            .expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { what: "file size" }), "{err:?}");
        assert!(std::fs::metadata(d.incoming.join("08")).is_err());
    }

    #[tokio::test]
    async fn a_chunk_past_the_declared_size_is_refused() {
        let d = dirs();
        let err = put(&d, &[7], 32, 0, &png(64)).await.expect_err("refused");
        assert!(matches!(err, DomainError::InvalidArgument { what: "file size" }), "{err:?}");
    }

    #[tokio::test]
    async fn two_pastes_in_the_same_second_get_a_counter() {
        let d = dirs();
        let now = UNIX_EPOCH + Duration::from_secs(1_770_474_753);
        let bytes = png(16);
        let first =
            put_chunk_in(&d.dir, &d.incoming, &[10], "a.png", 16, 0, &bytes, now).await.expect("first");
        let second =
            put_chunk_in(&d.dir, &d.incoming, &[11], "a.png", 16, 0, &bytes, now).await.expect("second");
        let (Stored::Complete { path: a, .. }, Stored::Complete { path: b, .. }) =
            (first, second)
        else {
            panic!("expected two complete pastes")
        };
        assert_eq!(a.file_name().unwrap(), "2026-02-07-143233Z-a.png");
        assert_eq!(b.file_name().unwrap(), "2026-02-07-143233Z-2-a.png");
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
    fn a_senders_name_cannot_escape_the_paste_directory() {
        // The name crosses a network and becomes a filename on someone else's
        // runner. Rebuilt from allowed characters rather than filtered, so
        // there is no clever encoding left to find.
        assert_eq!(name_parts("../../etc/passwd", None), ("passwd".into(), String::new()));
        assert_eq!(name_parts("/etc/shadow", None), ("shadow".into(), String::new()));
        assert_eq!(name_parts(r"..\..\windows\system32\a.dll", None), ("a".into(), "dll".into()));
        // Nothing survives that could be a separator, a traversal or a shell
        // metacharacter — so the result is safe even before it is quoted.
        let (base, ext) = name_parts("a b;rm -rf ~/$(x).sh", None);
        assert!(!base.contains(['/', '\\', '.', ';', '$', '(', ')', ' ']), "{base}");
        assert_eq!(ext, "sh");
    }

    #[test]
    fn a_name_made_only_of_dropped_characters_still_produces_one() {
        // Otherwise the file is called `2026-08-09-114812Z-.pdf`, or worse,
        // nothing at all.
        assert_eq!(name_parts("???", None), ("file".into(), String::new()));
        assert_eq!(name_parts("", None), ("file".into(), String::new()));
        assert_eq!(name_parts("...", None), ("file".into(), String::new()));
    }

    #[test]
    fn recognized_image_bytes_override_the_extension_they_were_given() {
        // The one case where the bytes are more trustworthy than the label.
        assert_eq!(name_parts("screenshot.txt", Some(Kind::Png)), ("screenshot".into(), "png".into()));
        // And an unrecognized file keeps its own, lowercased.
        assert_eq!(name_parts("Report.PDF", None), ("Report".into(), "pdf".into()));
    }

    #[test]
    fn a_very_long_name_is_bounded() {
        let (base, ext) = name_parts(&format!("{}.txt", "a".repeat(500)), None);
        assert_eq!(base.len(), 64);
        assert_eq!(ext, "txt");
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
