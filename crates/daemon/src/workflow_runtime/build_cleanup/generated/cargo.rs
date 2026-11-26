//! Ownership evidence from Cargo fingerprints and rustc dep-info.
use super::{BuildWindow, ChangeTime, Layout, changed, open_regular, read_metadata};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::Metadata;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type Files = BTreeMap<PathBuf, Metadata>;
// The order keeps build subjects ahead of the records that identify them.
#[derive(Default)]
pub(super) struct Artifacts {
    pub(super) files: BTreeMap<PathBuf, u8>,
    invocations: BTreeMap<PathBuf, Vec<BuildWindow>>,
    pub(super) directories: Vec<PathBuf>,
}

impl Artifacts {
    pub(super) fn incremental_completion(&self, session: &Path, index: &Metadata) -> Option<ChangeTime> {
        let unit = session.parent()?;
        let (crate_name, _) = unit.file_name()?.to_str()?.rsplit_once('-')?;
        let key = unit.with_file_name(crate_name);
        self.invocations.get(&key)?.iter().find(|window| window.contains(index)).map(|window| window.end)
    }
}

const OUTPUT: u8 = 0;
const DEP_INFO: u8 = 1;
const FINGERPRINT: u8 = 2;
const CACHE: u8 = 3;
pub(super) const TAG: u8 = 4;
const INVOKED: &str = "invoked.timestamp";
const INVOCATION_TEXT: &str = "This file has an mtime of when this was started.";
const RUN_SCRIPT: &str = "run-build-script-build-script-build";
const COMPILE_SCRIPT: &str = "build-script-build-script-build";

pub(super) fn artifacts(tree: &Path, folder: &Path, files: &Files) -> io::Result<Artifacts> {
    let mut result = Artifacts::default();
    for (path, meta) in files {
        if let Some(unit) = Unit::read(path, meta, files) {
            if unit.marker == RUN_SCRIPT {
                unit.release_script_outputs(files, &mut result);
            } else {
                unit.release_compile_outputs(tree, files, &mut result)?;
            }
        }
    }
    let profiles: BTreeSet<_> = result.invocations.keys()
        .filter_map(|key| key.parent()?.parent()).collect();
    for profile in &profiles {
        result.directories.extend([".fingerprint", "build", "deps", "examples", "incremental"]
            .map(|name| profile.join(name)));
    }
    release_cache_metadata(folder, files, &profiles, &mut result.files);
    Ok(result)
}

struct Unit<'a> {
    fingerprint: &'a Path,
    profile: &'a Path,
    build_unit: Option<&'a Path>,
    hash: &'a str,
    marker: &'a str,
    json: &'a Path,
    stamp: PathBuf,
    local: Value,
    end: ChangeTime,
}

impl<'a> Unit<'a> {
    fn read(path: &'a Path, meta: &Metadata, files: &Files) -> Option<Self> {
        let marker = path.file_name()?.to_str()?.strip_suffix(".json")?;
        let fingerprint = path.parent()?;
        let parent = fingerprint.parent()?;
        // Cargo 1.100 groups fingerprints and outputs under build/<package>/<hash>.
        let (profile, hash, build_unit) = match fingerprint.file_name()?.to_str()? {
            "fingerprint" => {
                let build = parent.parent()?.parent()?;
                if build.file_name()? != "build" { return None; }
                (build.parent()?, parent.file_name()?.to_str()?, Some(parent))
            }
            name if parent.file_name()? == ".fingerprint" =>
                (parent.parent()?, name.rsplit_once('-')?.1, None),
            _ => return None,
        };
        if hash.len() != 16 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
        let stamp = fingerprint.join(marker);
        let stamp_meta = files.get(&stamp)?;
        let digest = read_metadata(&stamp)?;
        let end = changed(meta).min((meta.mtime(), meta.mtime_nsec()));
        if digest.len() != 16 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || changed(stamp_meta) > end { return None; }
        let json: Value = serde_json::from_str(&read_metadata(path)?).ok()?;
        if !["rustc", "target", "profile", "path", "config"].iter().all(|key| json[key].is_u64())
            || !json["deps"].is_array() || !json["local"].is_array() { return None; }
        Some(Self { fingerprint, profile, build_unit, hash, marker, json: path, stamp, local: json["local"].clone(), end })
    }

    fn window(&self, invocation: &Path, files: &Files) -> Option<BuildWindow> {
        let start = changed(files.get(invocation)?);
        if start > changed(files.get(&self.stamp)?) || read_metadata(invocation)? != INVOCATION_TEXT { return None; }
        Some(BuildWindow { start: Some(start), end: self.end })
    }

    fn record_fingerprint(&self, invocation: PathBuf, result: &mut BTreeMap<PathBuf, u8>) {
        result.insert(invocation, FINGERPRINT);
        result.insert(self.stamp.clone(), FINGERPRINT);
        result.insert(self.json.to_owned(), CACHE);
    }

    fn release_compile_outputs(&self, tree: &Path, files: &Files, artifacts: &mut Artifacts) -> io::Result<()> {
        let invocation = self.fingerprint.join(INVOKED);
        let dep_cache = self.fingerprint.join(format!("dep-{}", self.marker));
        let Some(window) = self.window(&invocation, files) else { return Ok(()); };
        let Some(dep) = files.get(&dep_cache).filter(|meta| window.contains(meta)) else { return Ok(()); };
        let cache_named = self.local.as_array().into_iter().flatten().any(|entry|
            entry["CheckDepInfo"]["dep_info"].as_str().is_some_and(|name| recorded_path(name, &dep_cache)));
        if !cache_named || dep.len() < 6 || !is_cargo_dep_cache(&dep_cache) { return Ok(()); }

        let (name, legacy_dir) = match self.marker {
            COMPILE_SCRIPT => ("build_script_build", self.profile.join("build").join(self.fingerprint.file_name().unwrap())),
            marker if marker.starts_with("test-integration-test-") =>
                (marker.trim_start_matches("test-integration-test-"), self.profile.join("deps")),
            marker => {
                let Some((kind, name)) = marker.strip_prefix("test-").unwrap_or(marker).split_once('-') else { return Ok(()); };
                let output = match kind {
                    "bin" | "lib" | "test" | "bench" => "deps",
                    "example" => "examples",
                    _ => return Ok(()),
                };
                (name, self.profile.join(output))
            }
        };
        let dir = self.build_unit.map_or(legacy_dir, |unit| unit.join("out"));
        let crate_name = name.replace('-', "_");
        let hashed_stem = format!("{crate_name}-{}", self.hash);
        let dep_info = dir.join(format!("{hashed_stem}.d"));
        // In the unit layout some rustc invocations omit the hash from the
        // filename. The unit directory still binds the dep-info to this hash.
        let (stem, dep_info) = if self.build_unit.is_some() && !files.contains_key(&dep_info) {
            let dep_info = dir.join(format!("{crate_name}.d"));
            (crate_name.as_str(), dep_info)
        } else {
            (hashed_stem.as_str(), dep_info)
        };
        artifacts.invocations.entry(self.profile.join("incremental").join(&crate_name)).or_default().push(window);
        let result = &mut artifacts.files;

        // The fingerprint and binary dep cache authenticate Cargo's bookkeeping
        // even when an earlier sweep already removed the compiled binaries.
        self.record_fingerprint(invocation, result);
        result.insert(dep_cache, FINGERPRINT);
        let diagnostics = self.fingerprint.join(format!("output-{}", self.marker));
        if files.get(&diagnostics).is_some_and(|meta| window.contains(meta))
            && read_metadata(&diagnostics).is_some_and(|data| !data.is_empty() && data.lines().all(|line|
                serde_json::from_str::<Value>(line).is_ok_and(|json| json["level"].is_string()
                    || (json["$message_type"] == "unused_extern" && json["lint_level"].is_string()
                        && json["unused_extern_names"].as_array().is_some_and(|names| names.iter().all(Value::is_string)))))) {
            result.insert(diagnostics, FINGERPRINT);
        }

        let Some(dep_meta) = files.get(&dep_info).filter(|meta| window.contains(meta)) else { return Ok(()); };
        let Some(data) = read_metadata(&dep_info) else { return Ok(()); };
        let mut rules = data.lines().filter_map(|line| line.split_once(": "));
        let resolve = |name: &str| tree.join(name.replace("\\ ", " "));
        let Some((first, dependencies)) = rules.next() else { return Ok(()); };
        if resolve(first) != dep_info || dependencies.is_empty() { return Ok(()); }
        result.insert(dep_info.clone(), DEP_INFO);

        // rustc writes dep-info before generating these outputs. A loose .d
        // file supplies no start time and confers no ownership.
        let produced = BuildWindow { start: Some(changed(dep_meta)), end: self.end };
        for (name, prerequisites) in rules {
            let path = resolve(name);
            if prerequisites != dependencies || path.parent() != Some(&dir) || path == dep_info
                || !path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s == stem || s.strip_prefix("lib") == Some(stem))
                || !files.get(&path).is_some_and(|meta| produced.contains(meta))
                || !Layout::Cargo.recognizes(&path)? { continue; }
            self.release_export(tree, &path, window, files, result)?;
            result.insert(path, OUTPUT);
        }
        // Split codegen objects have the unit's hashed stem and two
        // rustc-generated disambiguators. They aren't listed in dep-info.
        let prefix = format!("{stem}.");
        for (path, meta) in files.range(dir.join(&prefix)..) {
            if path.parent() != Some(&dir) { break; }
            let Some(suffix) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(&prefix))
            else { break; };
            let Some(suffix) = suffix.strip_suffix(".rcgu.o").filter(|_| produced.contains(meta)) else { continue; };
            if suffix.split_once('.').is_some_and(|(unit, instance)| super::base36(unit) && super::base36(instance))
                && Layout::Cargo.recognizes(path)? {
                result.insert(path.clone(), OUTPUT);
            }
        }
        Ok(())
    }

    fn release_export(&self, tree: &Path, path: &Path, window: BuildWindow, files: &Files, result: &mut BTreeMap<PathBuf, u8>)
        -> io::Result<()>
    {
        let source = &files[path];
        if self.marker == COMPILE_SCRIPT {
            if self.build_unit.is_some() { return Ok(()); }
            let export = path.with_file_name("build-script-build").with_extension(path.extension().unwrap_or_default());
            if let Some(meta) = files.get(&export).filter(|meta| window.contains(meta))
                && same_output(path, source, &export, meta)? {
                result.insert(export, OUTPUT);
            }
            return Ok(());
        }
        let prefix = self.marker.strip_prefix("test-").unwrap_or(self.marker);
        let Some((kind, name)) = prefix.split_once('-') else { return Ok(()); };
        if !matches!(kind, "bin" | "lib" | "example") { return Ok(()); }
        let dir = if kind == "example" { self.profile.join("examples") } else { self.profile.to_owned() };
        let basename = if kind == "lib" { format!("lib{}", name.replace('-', "_")) } else { name.to_owned() };
        let export = dir.join(basename).with_extension(path.extension().unwrap_or_default());
        let Some(meta) = files.get(&export).filter(|meta| window.contains(meta)) else { return Ok(()); };
        if !same_output(path, source, &export, meta)? { return Ok(()); }
        let dep = export.with_extension("d");
        if files.get(&dep).is_some_and(|dep_meta| changed(dep_meta) >= changed(meta)
            && dep_meta.created().ok().and_then(timestamp)
                .is_none_or(|created| window.start.is_some_and(|start| start <= created)))
            && read_metadata(&dep).and_then(|data| {
                let (first, prerequisites) = data.lines().next()?.split_once(": ")?;
                Some(!prerequisites.is_empty() && tree.join(first.replace("\\ ", " ")) == export)
            }) == Some(true) {
            result.insert(dep, DEP_INFO);
        }
        result.insert(export, OUTPUT);
        Ok(())
    }

    fn release_script_outputs(&self, files: &Files, artifacts: &mut Artifacts) {
        let (out, run, stdout_name) = if let Some(unit) = self.build_unit {
            (unit.join("out"), unit.join("run"), "stdout")
        } else {
            let unit = self.profile.join("build").join(self.fingerprint.file_name().unwrap());
            (unit.join("out"), unit, "output")
        };
        let invocation = run.join(INVOKED);
        let root_output = run.join("root-output");
        let stdout = run.join(stdout_name);
        let Some(window) = self.window(&invocation, files) else { return; };
        if !files.get(&stdout).is_some_and(|meta| window.contains(meta))
            || !files.get(&root_output).is_some_and(|meta| window.contains(meta))
            || read_metadata(&root_output).as_deref().map(Path::new) != Some(out.as_path()) { return; }
        artifacts.directories.push(out.clone());
        let result = &mut artifacts.files;
        for (path, meta) in descendants(files, &out) {
            // Build scripts can produce arbitrary formats. A pre-existing file
            // subsequently modified in OUT_DIR still belongs to its creator.
            if window.contains(meta) && meta.created().ok().and_then(timestamp)
                .is_some_and(|created| window.start.is_some_and(|start| start <= created)) {
                result.insert(path.clone(), OUTPUT);
            }
        }
        for path in [stdout, root_output, run.join("stderr")] {
            if files.get(&path).is_some_and(|meta| window.contains(meta)) { result.insert(path, DEP_INFO); }
        }
        self.record_fingerprint(invocation, result);
    }
}

fn is_cargo_dep_cache(path: &Path) -> bool {
    let mut prefix = [0; 6];
    open_regular(path).and_then(|mut file| file.read_exact(&mut prefix)).is_ok()
        && prefix == [1, 0, 0, 0, 0xff, 1]
}

fn timestamp(time: SystemTime) -> Option<ChangeTime> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    Some((duration.as_secs().try_into().ok()?, duration.subsec_nanos().into()))
}

fn recorded_path(value: &str, path: &Path) -> bool {
    let value = Path::new(value);
    (value.is_absolute() && value == path) || (!value.is_absolute() && value.components().count() >= 4 && path.ends_with(value))
}

fn descendants<'a>(files: &'a Files, dir: &Path) -> impl Iterator<Item = (&'a PathBuf, &'a Metadata)> {
    files.range(dir.to_owned()..).take_while(move |(path, _)| path.starts_with(dir))
}

fn same_output(source: &Path, original: &Metadata, export: &Path, copy: &Metadata) -> io::Result<bool> {
    if original.ino() == copy.ino() && original.dev() == copy.dev() { return Ok(true); }
    if original.len() != copy.len() || original.modified()? != copy.modified()? { return Ok(false); }
    // Cargo may clone or copy instead of hardlinking (for example on APFS).
    let mut source = open_regular(source)?;
    let mut export = open_regular(export)?;
    let mut a = [0; 8192];
    let mut b = [0; 8192];
    let mut remaining = original.len();
    while remaining > 0 {
        let count = remaining.min(a.len() as u64) as usize;
        source.read_exact(&mut a[..count])?;
        export.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] { return Ok(false); }
        remaining -= count as u64;
    }
    Ok(true)
}

fn release_cache_metadata(folder: &Path, files: &Files, profiles: &BTreeSet<&Path>, result: &mut BTreeMap<PathBuf, u8>) {
    let tag = folder.join("CACHEDIR.TAG");
    let tagged = files.contains_key(&tag) && super::cargo_cache(folder);
    if !tagged && profiles.is_empty() { return; }
    let info = folder.join(".rustc_info.json");
    if files.contains_key(&info) && read_metadata(&info).and_then(|data| serde_json::from_str::<Value>(&data).ok())
        .is_some_and(|json| json["rustc_fingerprint"].is_u64() && json["outputs"].is_object()) {
        result.insert(info, CACHE);
    }
    if tagged {
        // ArtifactPlan keeps this discovery marker when unknown files survive.
        result.insert(tag, TAG);
    }
    for profile in profiles {
        for lock in [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"] {
            let path = profile.join(lock);
            if files.get(&path).is_some_and(|meta| meta.len() == 0) {
                result.insert(path, CACHE);
            }
        }
    }
}
