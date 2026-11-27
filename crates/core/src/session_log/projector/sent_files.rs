//! The files an agent sends the person (`SendUserFile`, ov-1509).
//!
//! A view is told each file's name, size and kind (`Sent` carries no path),
//! and it asks for one by the tool row's id and the file's place on it
//! (`agent.file`): a request never names a path, and the runner reads only
//! what is recorded here. A path gets here one way, from
//! the `toolUseResult` claude writes once `SendUserFile` has delivered. A
//! call that failed, one not yet answered and any other tool's result put
//! nothing here, and the paths the call asked for are never read.
//!
//! Markdown that names a sent image (`![shot](/path/shot.png)`) carries the
//! same entry (`Prose::images`), so a view can draw it. Any other image in
//! Markdown stays its alt text.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::fold::Projection;
use super::record::ToolUseResult;
use super::rows::{RowKind, ToolStatus};

/// The tool that sends files.
pub const TOOL: &str = "SendUserFile";
/// An image up to this size is drawn as it is scrolled to; a larger one waits
/// for a click.
pub const AUTO_SHOW_BYTES: u64 = 4 * 1000 * 1000;
/// The largest file a view may fetch.
pub const MAX_SENT_FILE_BYTES: u64 = 32 * 1000 * 1000;
/// The most files read off one send.
const MAX_FILES: usize = 50;

/// What one `SendUserFile` call delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    pub files: Vec<SentFile>,
}

/// One delivered file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SentFile {
    pub name: String,
    /// By its extension.
    pub mime: &'static str,
    /// Its bytes when it was sent.
    pub size: u64,
    /// `size` as a person reads it, "48 KB".
    pub size_words: String,
    /// A picture a view may draw. SVG is not one: it can carry script.
    pub image: bool,
    /// Drawn without being asked for (`AUTO_SHOW_BYTES`).
    pub auto: bool,
    /// Small enough to fetch at all (`MAX_SENT_FILE_BYTES`).
    pub fetchable: bool,
    #[serde(skip)]
    path: PathBuf,
    /// What `path` resolved to when this was folded: a read must still
    /// resolve there. `None` when there was nothing to resolve.
    #[serde(skip)]
    pinned: Option<PathBuf>,
    /// The path as the agent asked for it, for Markdown that names it so.
    #[serde(skip)]
    asked: Option<String>,
}

impl SentFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// An image in a reply's Markdown that is one of the files sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProseImage {
    /// The destination as the Markdown writes it.
    pub src: String,
    /// The tool row that sent it, and its place there: what `agent.file` takes.
    pub row: String,
    pub index: u32,
    #[serde(flatten)]
    pub file: SentFile,
}

/// What the fold keeps to put sends on rows and match Markdown to them.
#[derive(Debug, Default)]
pub(super) struct SentState {
    /// A call's paths as asked, by `tool_use_id`, until it is answered.
    asked: std::collections::HashMap<String, Vec<String>>,
    /// Every way Markdown may name a sent image, to its row and place. The
    /// latest send of a name wins.
    names: std::collections::HashMap<String, (String, u32)>,
    /// Prose rows holding Markdown images, to look at again after a send.
    prose: std::collections::BTreeSet<usize>,
}

fn kind_of(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map_or_else(String::new, |(_, e)| e.to_ascii_lowercase());
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn drawable(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
}

/// "48 KB", in the decimal units a Mac counts files in.
pub fn size_words(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..999_500 => format!("{} KB", (bytes + 500) / 1_000),
        999_500..999_950_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{:.1} GB", bytes as f64 / 1e9),
    }
}

/// The destinations of the Markdown images in `text`: what stands between
/// `![…](` and the `)` or the title after it.
fn image_destinations(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("![").filter_map(move |(at, _)| {
        let rest = &text[at + 2..];
        let open = rest.find("](")?;
        let rest = rest[open + 2..].trim_start();
        let end = match rest.strip_prefix('<') {
            Some(angled) => return angled.split_once('>').map(|(inside, _)| inside),
            None => rest.find(|c: char| c == ')' || c.is_whitespace())?,
        };
        (end > 0).then(|| &rest[..end])
    })
}

/// `%20` and the like as the bytes they stand for; anything else as written.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

impl Projection {
    /// File `index` of the send on tool row `row`: the only way to a path.
    pub fn sent_file(&self, row: &str, index: usize) -> Option<&SentFile> {
        let row = self.row(row).filter(|r| !r.retracted)?;
        match &row.kind {
            RowKind::Tool(tool) => tool.sent.as_ref()?.files.get(index),
            _ => None,
        }
    }

    /// A `SendUserFile` call, its input as written: the paths it asks for,
    /// kept until the answer says what was delivered.
    pub(super) fn send_asked(&mut self, id: &str, input: Option<&str>) {
        let Some(input) = input.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()) else { return };
        let files = match &input["files"] {
            serde_json::Value::String(one) => vec![one.clone()],
            serde_json::Value::Array(many) => many.iter().filter_map(|f| f.as_str().map(str::to_string)).collect(),
            _ => return,
        };
        self.sent.asked.insert(id.to_string(), files);
    }

    /// Tool row `i` came back: if it is a send claude says it delivered, its
    /// files go on it, and the Markdown that names one is looked at again.
    pub(super) fn send_answered(&mut self, i: usize, id: &str, result: &ToolUseResult<'_>) {
        let asked = self.sent.asked.remove(id).unwrap_or_default();
        let row_id = self.rows[i].id.clone();
        let RowKind::Tool(tool) = &mut self.rows[i].kind else { return };
        if tool.name != TOOL || tool.status != ToolStatus::Done {
            return;
        }
        let paired = asked.len() == result.attachments.0.len();
        let files: Vec<SentFile> = result
            .attachments
            .0
            .iter()
            .enumerate()
            .filter_map(|(at, entry)| {
                let entry = entry.0.as_ref()?;
                let path = PathBuf::from(entry.path.get()?);
                if !path.is_absolute() {
                    return None;
                }
                let name = path.file_name()?.to_string_lossy().into_owned();
                let size = entry.size.int()?.max(0) as u64;
                let mime = kind_of(&name);
                let image = drawable(mime);
                let fetchable = size <= MAX_SENT_FILE_BYTES;
                Some(SentFile {
                    mime,
                    size,
                    size_words: size_words(size),
                    image: image && fetchable,
                    auto: image && size <= AUTO_SHOW_BYTES,
                    fetchable,
                    pinned: std::fs::canonicalize(&path).ok(),
                    asked: paired.then(|| asked[at].clone()),
                    name,
                    path,
                })
            })
            .take(MAX_FILES)
            .collect();
        if files.is_empty() {
            return;
        }
        let caption = result.caption.get().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
        for (index, file) in files.iter().enumerate().filter(|(_, f)| f.image) {
            let path = file.path.to_string_lossy().into_owned();
            for name in [Some(format!("file://{path}")), Some(path), file.asked.clone()].into_iter().flatten() {
                self.sent.names.insert(name, (row_id.clone(), index as u32));
            }
        }
        tool.sent = Some(Sent { caption, files });
        for at in self.sent.prose.clone() {
            if self.images_changed(at) {
                self.touch(at);
            }
        }
    }

    /// Put on prose row `i` the sent images its Markdown names. True when
    /// that changed the row.
    pub(super) fn images_changed(&mut self, i: usize) -> bool {
        let RowKind::Prose(prose) = &self.rows[i].kind else { return false };
        if !prose.text.contains("![") {
            return false;
        }
        self.sent.prose.insert(i);
        let mut found: Vec<ProseImage> = Vec::new();
        for src in image_destinations(&prose.text) {
            let Some((row, index)) = self.sent.names.get(src).or_else(|| self.sent.names.get(&decoded(src))) else { continue };
            let Some(file) = self.sent_file(row, *index as usize) else { continue };
            if !found.iter().any(|f| f.src == src) {
                found.push(ProseImage { src: src.to_string(), row: row.clone(), index: *index, file: file.clone() });
            }
        }
        let RowKind::Prose(prose) = &mut self.rows[i].kind else { return false };
        let changed = prose.images != found;
        prose.images = found;
        changed
    }
}

/// Why a sent file couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRefused {
    /// Not there any more as a plain file where it was sent from.
    Gone,
    /// There, and another size than what was sent.
    Changed,
    TooLarge,
}

/// Up to `max` bytes of `file` from `offset`, only while it is the file that
/// was sent: its path still resolves where it did, to a plain file with one
/// name (a hard link could be to anything), of the size it was sent at.
#[cfg(unix)]
pub fn read(file: &SentFile, offset: u64, max: usize) -> Result<Vec<u8>, ReadRefused> {
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if file.size > MAX_SENT_FILE_BYTES {
        return Err(ReadRefused::TooLarge);
    }
    let pinned = file.pinned.as_ref().ok_or(ReadRefused::Gone)?;
    if std::fs::canonicalize(&file.path).ok().as_ref() != Some(pinned) {
        return Err(ReadRefused::Gone);
    }
    let mut open = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(pinned)
        .map_err(|_| ReadRefused::Gone)?;
    let meta = open.metadata().map_err(|_| ReadRefused::Gone)?;
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(ReadRefused::Gone);
    }
    if meta.len() != file.size {
        return Err(ReadRefused::Changed);
    }
    let start = offset.min(file.size);
    let mut out = vec![0; (file.size - start).min(max as u64) as usize];
    open.seek(SeekFrom::Start(start)).and_then(|_| open.read_exact(&mut out)).map_err(|_| ReadRefused::Changed)?;
    Ok(out)
}

/// A browser build holds no runner's files.
#[cfg(not(unix))]
pub fn read(_: &SentFile, _: u64, _: usize) -> Result<Vec<u8>, ReadRefused> {
    Err(ReadRefused::Gone)
}
