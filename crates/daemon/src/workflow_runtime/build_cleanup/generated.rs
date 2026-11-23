//! Legacy and adopted trees can contain a person's files even inside ignored
//! build directories. Inventory recognized compiler artifacts individually.
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const CACHE_TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub(super) struct ArtifactPlan(Vec<(PathBuf, Metadata)>);

#[derive(Clone, Copy)]
enum Layout { Cargo, Swift }

/// Cargo writes this standard cache tag even when CARGO_TARGET_DIR names a
/// directory the workflow definition does not list.
pub(super) fn cargo_cache(path: &Path) -> bool {
    let Some(tag) = read_metadata(&path.join("CACHEDIR.TAG")) else { return false };
    let mut lines = tag.lines();
    lines.next().is_some_and(|line| line.as_bytes() == CACHE_TAG)
        && lines.next() == Some("# This file is a cache directory tag created by cargo.")
}

impl ArtifactPlan {
    pub(super) fn inspect(tree: &Path, folder: &Path, device: u64) -> io::Result<Self> {
        let mut inventory = Inventory::default();
        inventory.scan(tree, folder, device)?;
        let mut plan = Self(Vec::new());
        let gate_archive = tree.join("target/rust-tests.tar.zst");
        for (path, meta) in inventory.files {
            let layout = if inventory.cargo.contains(&path) {
                Some(Layout::Cargo)
            } else if inventory.swift.contains(&path) {
                Some(Layout::Swift)
            } else {
                None
            };
            let recognized = match layout {
                Some(layout) => layout.recognizes(&path)?,
                None => path == gate_archive && nextest_archive(&path, tree, &tree.join("target")).unwrap_or(false),
            };
            if recognized { plan.0.push((path, meta)); }
        }
        Ok(plan)
    }

    pub(super) fn is_empty(&self) -> bool { self.0.is_empty() }

    pub(super) fn release(self, folder: &Path) -> io::Result<u64> {
        let mut bytes = 0u64;
        let mut directories = BTreeSet::new();
        for (path, original) in self.0 {
            let parent = path.parent().expect("artifact has a parent");
            let current = path.symlink_metadata()?;
            if !current.is_file() || parent.canonicalize()? != parent
                || (current.dev(), current.ino(), current.len(), current.mtime(), current.mtime_nsec())
                    != (original.dev(), original.ino(), original.len(), original.mtime(), original.mtime_nsec()) {
                return Err(io::Error::other("generated artifact changed during cleanup"));
            }
            fs::remove_file(&path)?;
            bytes = bytes.saturating_add(original.len());
            for dir in parent.ancestors().take_while(|p| p.starts_with(folder)) {
                if !directories.insert(dir.to_owned()) { break; }
            }
        }
        // Only remove empty ancestors of artifacts we actually released, deepest first.
        for dir in directories.into_iter().rev() {
            if dir.canonicalize().ok().as_deref() == Some(dir.as_path()) {
                let _ = fs::remove_dir(dir);
            }
        }
        Ok(bytes)
    }
}

#[derive(Default)]
struct Inventory {
    cargo: BTreeSet<PathBuf>,
    swift: BTreeSet<PathBuf>,
    files: Vec<(PathBuf, Metadata)>,
}

impl Inventory {
    fn scan(&mut self, tree: &Path, path: &Path, device: u64) -> io::Result<()> {
        let meta = path.symlink_metadata()?;
        if meta.dev() != device || path.file_name().is_some_and(|name|
            name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case("reports")) {
            return Ok(());
        }
        if meta.is_dir() && !is_repository(path) {
            for entry in fs::read_dir(path)? {
                self.scan(tree, &entry?.path(), device)?;
            }
        } else if meta.is_file() {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            if let Some(stem) = name.strip_suffix(".d") {
                self.cargo_outputs(tree, path, stem);
            } else if name == "output-file-map.json" || name.ends_with("-OutputFileMap.json") {
                self.swift_outputs(tree, path);
            }
            self.files.push((path.to_owned(), meta));
        }
        Ok(())
    }

    fn cargo_outputs(&mut self, tree: &Path, path: &Path, stem: &str) {
        let Some(data) = read_metadata(path) else { return };
        let Some(parent) = path.parent() else { return };
        let mut rules = data.lines().filter_map(|line| line.split_once(": "));
        // Rustc starts dep-info with a rule for the dep-info file itself.
        // Cargo's top-level dep-info names its own final executable or library.
        let Some((first, dependencies)) = rules.next() else { return };
        let resolve = |name: &str| tree.join(name.replace("\\ ", " "));
        let first = resolve(first);
        if dependencies.is_empty() || first.parent() != Some(parent) { return; }
        let same_target = |output: &Path| {
            output.parent() == Some(parent) && output.file_stem().and_then(|name| name.to_str())
                .is_some_and(|name| name == stem || name.strip_prefix("lib") == Some(stem))
        };
        if first != path && !same_target(&first) { return; }
        if first != path { self.cargo.insert(first); }
        for (name, deps) in rules {
            let output = resolve(name);
            if deps == dependencies && output != path && same_target(&output) {
                self.cargo.insert(output);
            }
        }
    }

    fn swift_outputs(&mut self, tree: &Path, path: &Path) {
        let Some(map) = read_metadata(path).and_then(|data| serde_json::from_str::<Value>(&data).ok()) else { return };
        let Some(sources) = map.as_object() else { return };
        for source in sources.values().filter_map(Value::as_object) {
            for (kind, output) in source {
                if matches!(kind.as_str(), "object" | "swiftmodule" | "swift-dependencies" | "diagnostics") {
                    if let Some(output) = output.as_str() { self.swift.insert(tree.join(output)); }
                }
            }
        }
    }
}

fn read_metadata(path: &Path) -> Option<String> {
    let mut data = String::new();
    open_regular(path).ok()?.take(METADATA_LIMIT + 1).read_to_string(&mut data).ok()?;
    (data.len() as u64 <= METADATA_LIMIT).then_some(data)
}

fn open_regular(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    if !file.metadata()?.is_file() { return Err(io::Error::other("not a regular file")); }
    Ok(file)
}

/// The landing gate's archive begins with nextest's two metadata members.
/// Decode just those members and require that they describe this checkout.
fn nextest_archive(path: &Path, tree: &Path, target: &Path) -> io::Result<bool> {
    let mut decoder = zstd::stream::read::Decoder::new(open_regular(path)?)?;
    decoder.window_log_max(27)?;
    let mut archive = tar::Archive::new(decoder.take(METADATA_LIMIT));
    let mut entries = archive.entries()?.raw(true);
    let mut metadata = |expected: &str| -> io::Result<Value> {
        let entry = entries.next().ok_or_else(|| io::Error::other("missing nextest metadata"))??;
        if !entry.header().entry_type().is_file() || entry.path()?.as_ref() != Path::new(expected) {
            return Err(io::Error::other("unexpected nextest archive member"));
        }
        Ok(serde_json::from_reader(entry)?)
    };
    let binaries = metadata("target/nextest/binaries-metadata.json")?;
    let cargo = metadata("target/nextest/cargo-metadata.json")?;
    let matches_path = |value: &Value, path: &Path| value.as_str().is_some_and(|name| Path::new(name) == path);
    Ok(binaries["rust-binaries"].is_object()
        && matches_path(&binaries["rust-build-meta"]["target-directory"], target)
        && cargo["packages"].is_array()
        && matches_path(&cargo["workspace_root"], tree)
        && matches_path(&cargo["target_directory"], target))
}

fn is_repository(path: &Path) -> bool {
    path.join(".git").symlink_metadata().is_ok()
        || (path.join("HEAD").symlink_metadata().is_ok() && path.join("objects").symlink_metadata().is_ok())
}

impl Layout {
    fn recognizes(self, path: &Path) -> io::Result<bool> {
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or_default();
        let candidate = match self {
            Self::Cargo => matches!(extension, "" | "a" | "bin" | "dylib" | "o" | "rlib" | "rmeta" | "so" | "wasm"),
            Self::Swift => matches!(extension, "" | "a" | "dia" | "dylib" | "o" | "pcm" | "pch" | "sdkstatcache"
                | "so" | "swiftdeps" | "swiftmodule" | "xctest"),
        };
        if !candidate { return Ok(false); }
        let mut magic = [0u8; 8];
        let mut file = open_regular(path)?;
        match file.read_exact(&mut magic) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
        let prefix = &magic[..4];
        let compiled = matches!(prefix, b"\x7fELF" | b"\xfe\xed\xfa\xce" | b"\xce\xfa\xed\xfe"
            | b"\xfe\xed\xfa\xcf" | b"\xcf\xfa\xed\xfe");
        let universal_macho = prefix == b"\xca\xfe\xba\xbe"
            && (1..=32).contains(&u32::from_be_bytes(magic[4..].try_into().unwrap()));
        Ok(match (self, extension) {
            (Self::Cargo, "bin") => prefix == b"RSIC",
            (Self::Cargo, "rmeta") => prefix == b"rust" && magic[4..7] == [0; 3],
            (Self::Cargo, "wasm") => magic == *b"\0asm\x01\0\0\0",
            (Self::Cargo, "a" | "rlib") | (Self::Swift, "a") => magic == *b"!<arch>\n",
            (Self::Swift, "swiftmodule") => prefix == b"\xe2\x9c\xa8\x0e",
            (Self::Swift, "swiftdeps") => prefix == b"DEPS",
            (Self::Swift, "sdkstatcache") => prefix == b"STAT",
            (Self::Swift, "dia") => prefix == b"DIAG",
            (Self::Swift, "pcm" | "pch") => prefix == b"CPCH" || compiled,
            _ => compiled || universal_macho,
        })
    }
}
