//! Legacy and adopted trees can contain a person's files even inside ignored
//! build directories. Inventory recognized compiler artifacts individually.
use std::collections::BTreeSet;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) struct ArtifactPlan(Vec<(PathBuf, Metadata)>);

#[derive(Clone, Copy)]
enum Layout { Cargo, Swift, Gradle }

impl ArtifactPlan {
    pub(super) fn inspect(folder: &Path, device: u64) -> io::Result<Self> {
        let mut plan = Self(Vec::new());
        if folder.symlink_metadata()?.dev() != device || is_repository(folder) { return Ok(plan); }
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().unwrap_or_default();
            let layout = match folder.file_name().and_then(|n| n.to_str()) {
                Some("target") if matches!(name, "debug" | "release") => Layout::Cargo,
                Some("target") if name.split('-').count() >= 3 => {
                    for profile in ["debug", "release"] {
                        plan.inspect_path(&entry.path(), device, Layout::Cargo, Some(profile))?;
                    }
                    continue;
                }
                Some(".build") if matches!(name, "debug" | "release" | "out") => Layout::Swift,
                Some(".build") if name.contains("-apple-") || name.contains("-unknown-") => {
                    for profile in ["debug", "release"] {
                        plan.inspect_path(&entry.path(), device, Layout::Swift, Some(profile))?;
                    }
                    continue;
                }
                Some("build") if matches!(name, "intermediates" | "outputs" | "kotlin" | "tmp") => Layout::Gradle,
                _ => continue,
            };
            plan.inspect_path(&entry.path(), device, layout, None)?;
        }
        Ok(plan)
    }

    fn inspect_path(&mut self, path: &Path, device: u64, layout: Layout, profile: Option<&str>) -> io::Result<()> {
        let meta = path.symlink_metadata()?;
        if meta.dev() != device { return Ok(()); }
        if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(".git") || n.eq_ignore_ascii_case("reports")) {
            return Ok(());
        }
        if meta.is_dir() {
            if is_repository(path) { return Ok(()); }
            if let Some(profile) = profile {
                let profile = path.join(profile);
                if profile.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                    self.inspect_path(&profile, device, layout, None)?;
                }
            } else {
                for entry in fs::read_dir(path)? {
                    self.inspect_path(&entry?.path(), device, layout, None)?;
                }
            }
        } else if profile.is_none() && meta.is_file() && layout.recognizes(path)? {
            self.0.push((path.to_owned(), meta));
        }
        Ok(())
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
            Self::Gradle => matches!(extension, "class" | "dex" | "o" | "so"),
        };
        if !candidate { return Ok(false); }
        let mut magic = [0u8; 8];
        let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
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
            (Self::Gradle, "class") => prefix == b"\xca\xfe\xba\xbe",
            (Self::Gradle, "dex") => prefix == b"dex\n",
            _ => compiled || universal_macho,
        })
    }
}
