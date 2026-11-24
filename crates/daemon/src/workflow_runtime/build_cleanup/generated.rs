//! Legacy and adopted trees can contain a person's files even inside ignored
//! build directories. Inventory recognized build artifacts individually.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

mod cargo;

const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const CACHE_TAG: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub(super) struct ArtifactPlan {
    files: Vec<(PathBuf, Metadata)>,
    empty_directories: BTreeSet<PathBuf>,
}

#[derive(Clone, Copy)]
enum Layout { Cargo, Swift, Android }

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
        inventory.scan(tree, folder, folder, device)?;
        let cargo = cargo::artifacts(tree, folder, &inventory.files)?;
        for session in &inventory.incremental_sessions {
            incremental_outputs(session, &cargo, &mut inventory.incremental);
        }
        let mut plan = Self { files: Vec::new(), empty_directories: cargo.directories.into_iter().collect() };
        let gate_archive = tree.join("target/rust-tests.tar.zst");
        let mut keep_tag = inventory.protected_entries;
        let mut tag = None;
        for (path, meta) in inventory.files {
            if cargo.files.get(&path) == Some(&cargo::TAG) {
                tag = Some((path, meta));
                continue;
            }
            let layout = if inventory.swift.get(&path).is_some_and(|window| window.contains(&meta)) {
                Some(Layout::Swift)
            } else if inventory.android.get(&path).is_some_and(|window| window.contains(&meta)) {
                Some(Layout::Android)
            } else {
                None
            };
            let recognized = cargo.files.contains_key(&path) || inventory.incremental.get(&path).is_some_and(|original| same_file(&meta, original)) || match layout {
                Some(layout) => layout.recognizes(&path)?,
                None => path == gate_archive && nextest_archive(&path, tree, &tree.join("target")).unwrap_or(false),
            };
            if recognized { plan.files.push((path, meta)); } else { keep_tag = true; }
        }
        if !keep_tag && let Some(tag) = tag { plan.files.push(tag); }
        // Keep the verification records available if deletion stops partway.
        plan.files.sort_by_key(|(path, _)| cargo.files.get(path).copied().unwrap_or_else(||
            u8::from(path.file_name().is_some_and(|name| name == "work-products.bin"))));
        Ok(plan)
    }

    pub(super) fn is_empty(&self) -> bool { self.files.is_empty() }

    pub(super) fn release(self, folder: &Path) -> io::Result<u64> {
        let mut bytes = 0u64;
        let mut directories = self.empty_directories;
        let mut unlinked = BTreeMap::new();
        for (path, original) in self.files {
            let parent = path.parent().expect("artifact has a parent");
            let key = (original.dev(), original.ino());
            let current = path.symlink_metadata()?;
            let expected = unlinked.get(&key).unwrap_or(&original);
            if !current.is_file() || parent.canonicalize()? != parent
                || !same_file(&current, expected) {
                return Err(io::Error::other("generated artifact changed during cleanup"));
            }
            // Removing one link changes the shared inode's ctime. Keep the
            // inode open so we can revalidate any other names in this plan.
            let shared = if current.nlink() > 1 {
                let file = open_regular(&path)?;
                if !same_file(&file.metadata()?, &current) {
                    return Err(io::Error::other("generated artifact changed during cleanup"));
                }
                Some(file)
            } else {
                None
            };
            fs::remove_file(&path)?;
            if let Some(file) = shared {
                let after = file.metadata()?;
                if after.nlink() != current.nlink() - 1 || after.len() != current.len()
                    || after.modified()? != current.modified()? {
                    return Err(io::Error::other("generated artifact changed during cleanup"));
                }
                unlinked.insert(key, after);
            }
            bytes = bytes.saturating_add(original.len());
            for dir in parent.ancestors().take_while(|p| p.starts_with(folder)) {
                directories.insert(dir.to_owned());
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

fn changed(meta: &Metadata) -> (i64, i64) { (meta.ctime(), meta.ctime_nsec()) }

fn same_file(current: &Metadata, original: &Metadata) -> bool {
    (current.dev(), current.ino(), current.len(), changed(current))
        == (original.dev(), original.ino(), original.len(), changed(original))
        && (current.mtime(), current.mtime_nsec()) == (original.mtime(), original.mtime_nsec())
}

type ChangeTime = (i64, i64);

#[derive(Clone, Copy)]
struct BuildWindow {
    start: Option<ChangeTime>,
    end: ChangeTime,
}

impl BuildWindow {
    fn contains(self, meta: &Metadata) -> bool {
        let time = changed(meta);
        self.start.is_none_or(|start| start <= time) && time <= self.end
    }
}

#[derive(Default)]
struct Inventory {
    swift: BTreeMap<PathBuf, BuildWindow>,
    android: BTreeMap<PathBuf, BuildWindow>,
    incremental: BTreeMap<PathBuf, Metadata>,
    incremental_sessions: Vec<PathBuf>,
    files: BTreeMap<PathBuf, Metadata>,
    protected_entries: bool,
}

impl Inventory {
    fn scan(&mut self, tree: &Path, folder: &Path, path: &Path, device: u64) -> io::Result<()> {
        let meta = path.symlink_metadata()?;
        if meta.dev() != device || path.file_name().is_some_and(|name|
            name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case("reports")) {
            self.protected_entries = true;
            return Ok(());
        }
        if meta.is_dir() && !is_repository(path) {
            if incremental_session(path) {
                self.incremental_sessions.push(path.to_owned());
            }
            for entry in fs::read_dir(path)? {
                self.scan(tree, folder, &entry?.path(), device)?;
            }
        } else if meta.is_file() {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            if name == "output-file-map.json" || name.ends_with("-OutputFileMap.json") {
                self.swift_outputs(tree, path, &meta);
            } else if name == "output-metadata.json" {
                self.android_outputs(folder, path, &meta);
            }
            self.files.insert(path.to_owned(), meta);
        } else {
            self.protected_entries = true;
        }
        Ok(())
    }

    fn swift_outputs(&mut self, tree: &Path, path: &Path, meta: &Metadata) {
        let Some(map) = read_metadata(path).and_then(|data| serde_json::from_str::<Value>(&data).ok()) else { return };
        let Some(sources) = map.as_object() else { return };
        let Some(window) = swift_build_window(tree, path, meta, &map) else { return };
        for source in sources.values().filter_map(Value::as_object) {
            for (kind, output) in source {
                if matches!(kind.as_str(), "object" | "swiftmodule" | "swift-dependencies" | "diagnostics")
                    && let Some(output) = output.as_str() {
                    self.swift.insert(tree.join(output), window);
                }
            }
        }
    }

    fn android_outputs(&mut self, folder: &Path, path: &Path, meta: &Metadata) {
        let Some(parent) = path.parent() else { return };
        let Ok(relative) = parent.strip_prefix(folder) else { return };
        let mut dirs = relative.iter().map(|dir| dir.to_str().unwrap_or_default());
        // AGP writes a BuiltArtifacts record alongside the packages it produces.
        // Match the output layout and the recorded type, then use only explicit
        // sibling filenames so a metadata entry cannot redirect the sweep.
        let (kind, extension) = match (dirs.next(), dirs.next()) {
            (Some("outputs" | "intermediates"), Some("apk")) => ("APK", "apk"),
            (Some("intermediates"), Some("processed_res")) => ("PROCESSED_RES", "ap_"),
            (Some("intermediates"), Some("linked_resources_binary_format")) => ("LINKED_RESOURCES_BINARY_FORMAT", "ap_"),
            (Some("intermediates"), Some("linked_resources_proto_format")) => ("LINKED_RESOURCES_PROTO_FORMAT", "ap_"),
            _ => return,
        };
        let Some(record) = read_metadata(path).and_then(|data| serde_json::from_str::<Value>(&data).ok()) else { return };
        if !matches!(record["version"].as_u64(), Some(1..=3))
            || record["artifactType"]["type"].as_str() != Some(kind)
            || record["artifactType"]["kind"] != "Directory"
            || !record["applicationId"].is_string()
            || record["variantName"].as_str().is_none_or(str::is_empty) {
            return;
        }
        let Some(elements) = record["elements"].as_array() else { return };
        // Keep later edits and replacements even if they retain the old mtime.
        let end = changed(meta).min((meta.mtime(), meta.mtime_nsec()));
        let window = BuildWindow { start: None, end };
        for name in elements.iter().filter_map(|element| element["outputFile"].as_str()) {
            let file = Path::new(name);
            if file.file_name().is_some_and(|basename| basename == name)
                && file.extension().is_some_and(|ext| ext == extension) {
                self.android.insert(parent.join(file), window);
            }
        }
    }
}

fn swift_build_window(tree: &Path, map_path: &Path, map_meta: &Metadata, map: &Value) -> Option<BuildWindow> {
    // SwiftPM writes the map before compiling. The Swift driver writes the
    // target's dependency graph (.priors) when the compilation completes.
    // The empty map key describes the target-wide dependency path on both
    // the native and XCBuild backends.
    let dependencies = tree.join(map[""]["swift-dependencies"].as_str()?);
    if dependencies.parent() != map_path.parent() || dependencies.extension()? != "swiftdeps" {
        return None;
    }
    let mut priors = open_regular(&dependencies.with_extension("priors")).ok()?;
    let mut magic = [0; 4];
    priors.read_exact(&mut magic).ok()?;
    let completion = priors.metadata().ok()?;
    if &magic != b"DDEP" || completion.dev() != map_meta.dev() { return None; }
    let start = changed(map_meta);
    let end = changed(&completion).min((completion.mtime(), completion.mtime_nsec()));
    (start <= end).then_some(BuildWindow { start: Some(start), end })
}

fn base36(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn incremental_session(path: &Path) -> bool {
    let Some((unit, incremental)) = path.parent().and_then(|unit| Some((unit, unit.parent()?))) else { return false };
    if incremental.file_name().is_none_or(|name| name != "incremental") { return false; }
    let Some((_, hash)) = unit.file_name().and_then(|name| name.to_str()).and_then(|name| name.rsplit_once('-'))
    else { return false };
    let Some(session) = path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("s-"))
    else { return false };
    // Finalized rustc session directories carry a timestamp, a disambiguator
    // and a hash. A session ending in "-working" was never finalized.
    let Some((timestamp, rest)) = session.split_once('-') else { return false };
    let Some((disambiguator, session_hash)) = rest.split_once('-') else { return false };
    base36(hash) && base36(timestamp) && base36(disambiguator) && base36(session_hash)
        && session_hash != "working"
        && incremental.parent().is_some_and(|profile|
            profile.join(".fingerprint").symlink_metadata().is_ok_and(|meta| meta.is_dir()))
}

const RUSTC_END: &[u8] = b"rust-end-file";

fn rustc_cache_header(file: &mut File) -> Option<Vec<u8>> {
    let mut prefix = [0; 7];
    file.read_exact(&mut prefix).ok()?;
    if &prefix[..4] != b"RSIC" || prefix[6] == 0 { return None; }
    let mut version = vec![0; usize::from(prefix[6])];
    file.read_exact(&mut version).ok()?;
    if !version.first()?.is_ascii_digit() || !version.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        return None;
    }
    let mut end = [0; RUSTC_END.len()];
    file.seek(SeekFrom::End(-(end.len() as i64))).ok()?;
    file.read_exact(&mut end).ok()?;
    (end == RUSTC_END).then(|| [prefix.as_slice(), &version].concat())
}

fn incremental_outputs(session: &Path, cargo: &cargo::Artifacts, outputs: &mut BTreeMap<PathBuf, Metadata>) {
    let index = session.join("work-products.bin");
    let Some((mut file, meta, header)) = open_regular(&index).ok().and_then(|mut file| {
        let meta = file.metadata().ok()?;
        let header = rustc_cache_header(&mut file)?;
        Some((file, meta, header))
    }) else { return };
    if meta.len() > METADATA_LIMIT { return; }
    let completion = cargo.incremental_completion(session, &meta).unwrap_or_else(|| changed(&meta));
    let mut products = Vec::new();
    if file.seek(SeekFrom::Start(header.len() as u64)).is_err()
        || file.take(METADATA_LIMIT).read_to_end(&mut products).is_err() { return; }
    let Ok(entries) = fs::read_dir(session) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(candidate) = open_regular(&path).ok().and_then(|mut file| {
            let candidate = file.metadata().ok()?;
            if candidate.dev() != meta.dev() || changed(&candidate) > completion
                || (candidate.mtime(), candidate.mtime_nsec()) > changed(&meta)
                || candidate.created().ok().zip(meta.created().ok()).is_some_and(|(file, index)| file > index) {
                return None;
            }
            let filename = entry.file_name();
            let name = filename.to_str()?;
            let verified = match name {
                "dep-graph.bin" | "query-cache.bin" => rustc_cache_header(&mut file)? == header,
                // Rustc's opaque encoder writes the saved filename as its
                // length, UTF-8 bytes and a 0xc1 string terminator. Only the
                // formats rustc saves here are eligible, even if named by
                // the index. Unknown formats and user files are retained.
                _ => {
                    let (stem, ext) = name.split_once('.')?;
                    if !base36(stem) || !matches!(ext, "o" | "rmeta") { return None; }
                    let length = u8::try_from(name.len()).ok().filter(|n| *n < 128)?;
                    let encoded = [&[length], name.as_bytes(), &[0xc1]].concat();
                    products.windows(encoded.len()).any(|bytes| bytes == encoded)
                        && Layout::Cargo.recognizes(&path).ok()?
                }
            };
            verified.then_some(candidate)
        }) else { continue };
        outputs.insert(path, candidate);
    }
    // Leave the index until all other planned files in the session have been
    // processed. A later sweep can still verify the remainder after an error.
    if let Some((prefix, _)) = session.file_name().and_then(|name| name.to_str()).and_then(|name| name.rsplit_once('-')) {
        let lock = session.with_file_name(format!("{prefix}.lock"));
        if let Ok(lock_meta) = lock.symlink_metadata()
            && lock_meta.is_file() && lock_meta.len() == 0 && lock_meta.dev() == meta.dev()
            && changed(&lock_meta) <= changed(&meta) {
            outputs.insert(lock, lock_meta);
        }
    }
    outputs.insert(index, meta);
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
            Self::Android => matches!(extension, "apk" | "ap_"),
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
            (Self::Android, _) => prefix == b"PK\x03\x04",
            _ => compiled || universal_macho,
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn swiftpm_outputs_are_reclaimed_and_later_replacements_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().canonicalize().unwrap();
        let sources = tree.join("Sources/CleanupFixture");
        fs::create_dir_all(&sources).unwrap();
        fs::write(tree.join("Package.swift"), concat!(
            "// swift-tools-version: 5.9\nimport PackageDescription\n",
            "let package = Package(name: \"CleanupFixture\", targets: [.target(name: \"CleanupFixture\")])\n",
        )).unwrap();
        for source in ["Keep", "Release"] {
            fs::write(sources.join(format!("{source}.swift")),
                format!("public func fixture{source}() -> Int {{ 1428 }}\n")).unwrap();
        }
        let folder = tree.join(".build");
        let result = std::process::Command::new("/usr/bin/swift").current_dir(&tree)
            .args(["build", "--disable-sandbox", "--manifest-cache", "local", "--scratch-path"])
            .arg(&folder).arg("--cache-path").arg(tree.join("swift-cache"))
            .env("CLANG_MODULE_CACHE_PATH", tree.join("module-cache")).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));

        let device = tree.metadata().unwrap().dev();
        let mut inventory = Inventory::default();
        inventory.scan(&tree, &folder, &folder, device).unwrap();
        let object = |stem: &str| inventory.files.keys()
            .find(|path| path.file_stem().and_then(|name| name.to_str())
                .is_some_and(|name| name == stem || name == format!("{stem}.swift"))
                && path.extension().is_some_and(|ext| ext == "o")).unwrap();
        let keep = object("Keep");
        let release = object("Release");
        let original = ArtifactPlan::inspect(&tree, &folder, device).unwrap();
        assert!([keep, release].iter().all(|output| original.files.iter().any(|(path, _)| path == *output)),
            "completed SwiftPM objects should be recognized");

        let replacement = fs::read(keep).unwrap();
        let temporary = tree.join("personal.o");
        fs::write(&temporary, &replacement).unwrap();
        fs::rename(&temporary, keep).unwrap();
        ArtifactPlan::inspect(&tree, &folder, device).unwrap().release(&folder).unwrap();
        assert_eq!(fs::read(keep).unwrap(), replacement);
        assert!(!release.exists());
    }
}
