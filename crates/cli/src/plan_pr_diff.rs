//! What a lane's branch changed, and where a reviewer should look first
//! (ov-576, ov-577): the pure half reads a `git diff` and sorts its files by
//! risk; the thin half runs git.
//!
//! **Risk is deterministic and from the diff alone.** No model is asked and no
//! note is read: the same diff always yields the same order, so a reviewer can
//! predict it. A file is risky when it is:
//!
//! - wire or protocol (`crates/protocol/`, `proto/`);
//! - a migration (a `migrations/` directory, or a `migrate*` file);
//! - auth or credentials (a path naming auth, token, keychain, credential or
//!   secret; "author" is not auth);
//! - deleted (a rename is not a deletion);
//! - a test file with more lines removed than added (a test deleted outright
//!   is named as such);
//! - CI (`.github/`);
//! - a library crate's source that adds a `pub fn`, `struct`, `enum` or `trait` (new public API).
//!
//! **Skim** is for what a person can't judge by reading: lock files, generated
//! files (`*.pbxproj`, snapshots, fixtures) and pure renames. A risky file
//! stays risky however generated it looks, because a fixture under the
//! protocol crate is still wire.
//!
//! The diff is the three-dot diff, the lane's branch against where it left
//! its base, so work that landed on the base since never counts against it.

use std::process::{Command, Stdio};

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One file of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// Where it is now (where it was, if deleted).
    pub path: String,
    pub kind: Kind,
    pub added: u64,
    pub deleted: u64,
    /// Some added line, outside a `#[cfg(test)]` module, declares public API
    /// (`pub fn`, `struct`, `enum` or `trait`).
    pub adds_pub_api: bool,
}

/// Why a file wants a judgment, in the words a reviewer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Wire,
    Migration,
    Auth,
    Deleted,
    TestDeleted,
    TestWeakened,
    Ci,
    PublicApi,
}

impl Risk {
    pub fn words(self) -> &'static str {
        match self {
            Risk::Wire => "wire format",
            Risk::Migration => "migration",
            Risk::Auth => "auth or credentials",
            Risk::Deleted => "deleted",
            Risk::TestDeleted => "test deleted",
            Risk::TestWeakened => "tests removed or weakened",
            Risk::Ci => "CI",
            Risk::PublicApi => "new public API",
        }
    }

    /// Bigger is more severe.
    fn severity(self) -> u8 {
        match self {
            Risk::Auth => 7,
            Risk::Wire => 6,
            Risk::Migration => 5,
            Risk::TestWeakened | Risk::TestDeleted => 4,
            Risk::Ci => 3,
            Risk::Deleted => 2,
            Risk::PublicApi => 1,
        }
    }
}

/// The net test lines a diff must remove, over all its test files, before
/// any of them is called weakened.
pub const WEAKENED_OVER: u64 = 10;

/// A test file by its path: Rust (`*_tests.rs`, `tests/`), Swift (`*Tests.swift`,
/// `Tests/`), TypeScript (`*.test.ts`, `test/`), Python (`test_*.py`) and
/// Kotlin (`*Test.kt`, `androidTest/`).
pub fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let dirs = |d: &str| path.starts_with(&format!("{d}/")) || path.contains(&format!("/{d}/"));
    name.ends_with("_tests.rs")
        || name.ends_with("_test.rs")
        || name.ends_with("Tests.swift")
        || name.ends_with("Test.swift")
        || name.ends_with("Test.kt")
        || name.ends_with("Tests.kt")
        || name.contains(".test.")
        || name.contains(".spec.")
        || (name.starts_with("test_") && name.ends_with(".py"))
        || dirs("tests")
        || dirs("Tests")
        || dirs("test")
        || dirs("androidTest")
}

/// The words of a path segment: split at anything that isn't a letter or digit
/// and at capitals (`KeychainStore` is keychain, store; `OAuthClient` is
/// oauth, client), lowercased.
fn words(segment: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let chars: Vec<char> = segment.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            continue;
        }
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let boundary = c.is_uppercase()
            && !word.is_empty()
            && (prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) || (prev.is_some_and(|p| p.is_uppercase()) && next.is_some_and(|n| n.is_lowercase())));
        if boundary {
            out.push(std::mem::take(&mut word));
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    // `OAuth` splits as o, auth by the rule above; read it as one word.
    let mut joined: Vec<String> = Vec::new();
    for w in out {
        if w == "auth" && joined.last().is_some_and(|l| l == "o") {
            joined.pop();
            joined.push("oauth".into());
        } else {
            joined.push(w);
        }
    }
    joined
}

fn is_auth(path: &str) -> bool {
    const WORDS: [&str; 15] = [
        "auth", "oauth", "authn", "authz", "authentication", "authorization", "keychain", "credential",
        "credentials", "secret", "secrets", "password", "passwords", "passwd", "token",
    ];
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    segments.iter().enumerate().any(|(i, seg)| {
        // A file is its name without the extension; a directory is its name.
        let stem = if i == last { seg.rsplit_once('.').map_or(*seg, |(stem, _)| stem) } else { seg };
        words(stem).iter().any(|w| WORDS.contains(&w.as_str())) || (i == last && stem.eq_ignore_ascii_case("tokens"))
    })
}

fn is_doc(path: &str) -> bool {
    path.starts_with("docs/") || path.ends_with(".md")
}

/// Whether public API added to `path` has callers outside the crate: the
/// wire, core and client crates, and any crate's `lib.rs`.
fn has_outside_consumers(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("crates/") else { return false };
    let Some((krate, tail)) = rest.split_once('/') else { return false };
    if krate == "cli" || !tail.starts_with("src/") || !path.ends_with(".rs") || is_test(path) {
        return false;
    }
    matches!(krate, "protocol" | "core" | "client") || tail == "src/lib.rs"
}

/// What a branch changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub files: Vec<Changed>,
}

impl Changes {
    pub fn added(&self) -> u64 {
        self.files.iter().map(|f| f.added).sum()
    }

    pub fn deleted(&self) -> u64 {
        self.files.iter().map(|f| f.deleted).sum()
    }

    pub fn test_files(&self) -> usize {
        self.files.iter().filter(|f| is_test(&f.path)).count()
    }

    /// Test lines removed minus test lines added, over every test file.
    fn net_test_removal(&self) -> u64 {
        let tests = || self.files.iter().filter(|f| is_test(&f.path));
        tests().map(|f| f.deleted).sum::<u64>().saturating_sub(tests().map(|f| f.added).sum())
    }

    /// "12 files changed, +340 \u{2212}58 lines, 4 test files touched".
    pub fn stat(&self) -> String {
        let n = self.files.len();
        let t = self.test_files();
        let files = if n == 1 { "1 file changed".to_string() } else { format!("{n} files changed") };
        let tests = match t {
            0 => "no test files touched".to_string(),
            1 => "1 test file touched".to_string(),
            t => format!("{t} test files touched"),
        };
        format!("{files}, +{} \u{2212}{} lines, {tests}", self.added(), self.deleted())
    }

    /// Every reason `file` needs a judgment, most severe first.
    pub fn risks(&self, file: &Changed) -> Vec<Risk> {
        let p = file.path.as_str();
        let name = p.rsplit('/').next().unwrap_or(p);
        let mut out = Vec::new();
        if p.starts_with("crates/protocol/") || p.starts_with("proto/") {
            out.push(Risk::Wire);
        }
        if p.contains("/migrations/") || p.starts_with("migrations/") || (name.starts_with("migrate") && !is_test(p)) {
            out.push(Risk::Migration);
        }
        if is_auth(p) {
            out.push(Risk::Auth);
        }
        if file.kind == Kind::Deleted {
            if is_test(p) {
                out.push(Risk::TestDeleted);
            } else if !is_doc(p) {
                out.push(Risk::Deleted);
            }
        } else if is_test(p) && file.kind != Kind::Renamed && file.deleted > file.added && self.net_test_removal() > WEAKENED_OVER {
            out.push(Risk::TestWeakened);
        }
        if p.starts_with(".github/") {
            out.push(Risk::Ci);
        }
        if file.adds_pub_api && has_outside_consumers(p) {
            out.push(Risk::PublicApi);
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.severity()));
        out
    }

    /// The risky files with their reasons: the most severe reason first, then
    /// the most reasons, then by path.
    pub fn risky(&self) -> Vec<(&Changed, Vec<Risk>)> {
        let mut out: Vec<(&Changed, Vec<Risk>)> =
            self.files.iter().map(|f| (f, self.risks(f))).filter(|(_, r)| !r.is_empty()).collect();
        let top = |r: &[Risk]| r.first().map_or(0, |r| r.severity());
        out.sort_by(|(a, ra), (b, rb)| {
            top(rb).cmp(&top(ra)).then_with(|| rb.len().cmp(&ra.len())).then_with(|| a.path.cmp(&b.path))
        });
        out
    }

    /// The files nothing flags, by path, then the skim ones, by path.
    pub fn rest(&self) -> Vec<(&Changed, bool)> {
        let mut plain: Vec<&Changed> = self.files.iter().filter(|f| self.risks(f).is_empty()).collect();
        plain.sort_by(|a, b| (skim(a), &a.path).cmp(&(skim(b), &b.path)));
        plain.into_iter().map(|f| (f, skim(f))).collect()
    }
}

/// A file a person can't judge by reading it.
pub fn skim(file: &Changed) -> bool {
    let p = file.path.as_str();
    let name = p.rsplit('/').next().unwrap_or(p);
    let lock = name.ends_with(".lock")
        || matches!(name, "package-lock.json" | "pnpm-lock.yaml" | "Package.resolved" | "Gemfile.lock" | "go.sum");
    let generated = name.ends_with(".pbxproj")
        || name.ends_with(".snap")
        || p.contains("__Snapshots__/")
        || p.contains("/fixtures/")
        || p.starts_with("fixtures/");
    let pure_rename = file.kind == Kind::Renamed && file.added == 0 && file.deleted == 0;
    lock || generated || pure_rename
}

/// Whether an added line declares public API: `pub fn` (and its `async`,
/// `const` and `unsafe` forms), `pub struct`, `pub enum` or `pub trait`, but
/// not `pub(crate) ...`.
fn declares_pub_api(added_line: &str) -> bool {
    let mut rest = added_line.trim_start();
    let Some(after) = rest.strip_prefix("pub ") else { return false };
    rest = after;
    loop {
        let t = rest.trim_start();
        if ["fn ", "struct ", "enum ", "trait "].iter().any(|k| t.starts_with(k)) {
            return true;
        }
        match ["async ", "const ", "unsafe ", "extern \"C\" "].iter().find_map(|w| t.strip_prefix(w)) {
            Some(next) => rest = next,
            None => return false,
        }
    }
}

/// A path as `git diff` prints it: unquoted when it was quoted, with C
/// escapes (`\t`, `\"`, `\\`) and octal bytes (`\303\251`) decoded.
fn unquote(path: &str) -> String {
    let path = path.trim_end_matches('\r');
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.trim_end_matches('\t').to_string();
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let c = bytes[i + 1];
        i += 2;
        match c {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b't' => out.push(b'\t'),
            b'n' => out.push(b'\n'),
            b'v' => out.push(11),
            b'f' => out.push(12),
            b'r' => out.push(b'\r'),
            b'0'..=b'3' => {
                let mut v = u32::from(c - b'0');
                for _ in 0..2 {
                    match bytes.get(i) {
                        Some(d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `token` (as in `a/x` or `"b/x"`) without its `a/` or `b/`.
fn without_side(token: &str, side: char) -> String {
    let path = unquote(token);
    path.strip_prefix(&format!("{side}/")).map_or(path.clone(), str::to_string)
}

/// The files of a `git diff -M -U0` patch.
pub fn parse(patch: &str) -> Changes {
    let mut files: Vec<Changed> = Vec::new();
    let mut in_body = false;
    // Inside a `#[cfg(test)]` module, as far as this hunk shows.
    let mut in_test_module = false;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            in_body = false;
            files.push(Changed { path: String::new(), kind: Kind::Modified, added: 0, deleted: 0, adds_pub_api: false });
            // The `b/` path, from the header, for a file with no ---/+++ lines (a mode change, a binary).
            let at = if line.ends_with('"') { line.rfind(" \"b/") } else { line.rfind(" b/") };
            if let (Some(f), Some(at)) = (files.last_mut(), at) {
                f.path = without_side(&line[at + 1..], 'b');
            }
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if in_body {
            if let Some(context) = line.strip_prefix("@@") {
                let context = context.rsplit("@@").next().unwrap_or("");
                in_test_module = context.contains("mod tests") || context.contains("cfg(test)");
            } else if let Some(added) = line.strip_prefix('+') {
                file.added += 1;
                let t = added.trim_start();
                if t.starts_with("#[cfg(test)]") || t.starts_with("mod tests") {
                    in_test_module = true;
                }
                file.adds_pub_api |= !in_test_module && declares_pub_api(added);
            } else if line.starts_with('-') {
                file.deleted += 1;
            }
            continue;
        }
        if let Some(context) = line.strip_prefix("@@") {
            in_body = true;
            let context = context.rsplit("@@").next().unwrap_or("");
            in_test_module = context.contains("mod tests") || context.contains("cfg(test)");
        } else if line.starts_with("new file mode") {
            file.kind = Kind::Added;
        } else if line.starts_with("deleted file mode") {
            file.kind = Kind::Deleted;
        } else if line.starts_with("rename from ") {
            file.kind = Kind::Renamed;
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.kind = Kind::Renamed;
            file.path = unquote(to);
        } else if let Some(p) = line.strip_prefix("--- ") {
            // A deleted file has no `+++` path: the old one names it.
            if file.kind == Kind::Deleted && p != "/dev/null" {
                file.path = without_side(p, 'a');
            }
        } else if let Some(p) = line.strip_prefix("+++ ") {
            if p != "/dev/null" {
                file.path = without_side(p, 'b');
            }
        }
    }
    files.retain(|f| !f.path.is_empty());
    Changes { files }
}

/// A ref that git could read as a flag, or that is empty, is not asked about.
fn plain_ref(r: &str) -> bool {
    !r.is_empty() && !r.starts_with('-') && !r.contains(char::is_whitespace)
}

/// What `branch` changed since it left `base`, from git in the current
/// directory. An empty diff is `Ok` with no files; `Err` is git (or a ref it
/// can't be asked about) failing, in git's first line, so the caller can say
/// the diff is unknown rather than pretend it is empty.
pub fn changes(base: &str, branch: &str) -> Result<Changes, String> {
    changes_in(None, base, branch)
}

/// `changes`, run in `dir` rather than the current directory.
pub fn changes_in(dir: Option<&std::path::Path>, base: &str, branch: &str) -> Result<Changes, String> {
    if branch.is_empty() {
        return Err("the lane has no branch recorded".into());
    }
    if !plain_ref(base) || !plain_ref(branch) {
        return Err("the base or branch isn't a name git can be asked about".into());
    }
    let mut git = Command::new("git");
    if let Some(dir) = dir {
        git.current_dir(dir);
    }
    let out = git
        .args(["-c", "core.quotepath=false", "diff", "-M", "-U0", "--no-color", "--no-ext-diff", "--no-textconv"])
        .arg(format!("{base}...{branch}"))
        .arg("--")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "git couldn't be started".to_string())?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let first = said.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("git failed");
        return Err(first.to_string());
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
#[path = "plan_pr_diff_tests.rs"]
mod tests;
