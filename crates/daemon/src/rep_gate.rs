//! The gate a rep's Claude Code passes before every tool call (ov-527, R-66).
//!
//! A rep is a messenger (ov-479 D3), so its pane gets a daemon-owned
//! `PreToolUse` hook (`hook_install::rep_settings`) that runs `farcooler
//! rep-gate`, which asks `decide` here. **Deny by default**: a call is
//! allowed only when it's one of these, and refused with a plain reason
//! otherwise.
//!
//! - **Bash**: one plain command, the farcooler CLI's `plan` (with or without
//!   `--json`), `task show <key>`, `task list`, `message orchestrator <text>`
//!   (with or without `--task <key>`), `task note <key> --kind comment
//!   --body <text>`, or `search <pattern> [--path <dir>]`. No shell
//!   metacharacter outside quotes, no expansion inside them, no other
//!   program, and no flag the grammar doesn't name, so no routing to another
//!   runner or board. The CLI by its absolute path only.
//! - **Read, Grep and Glob**: a path inside the workspace's own checkout,
//!   lexically and then through symlinks, never `.git` or a `.env` file (in
//!   any case, for a disk that ignores it), and a pattern that can't climb
//!   out of it. Grep names files, it doesn't print lines, and it searches one
//!   file: a directory search would cover `.git` and `.env*` too, and each
//!   answer is an oracle over a file Read refuses (ov-532). **Read and Grep
//!   reach only files git tracks** (`repo_grep`): a file git ignores (a key,
//!   a token file) is refused though its name is plain, and a Grep over it
//!   would read it out a prefix at a time (ov-555).
//! - **`search`**: the CLI's `search '<pattern>' [--path <dir>]`, which the
//!   runner answers over tracked files only (`repo_grep`).
//! - **Skill**: the rep skill itself.
//!
//! The hook's deny is a blocking exit, which Claude Code applies before any
//! allow rule, whichever settings file holds it. The daemon's allow-list
//! (`people::rep_may`) is the second wall, and neither depends on Claude
//! Code's own rule matching. A guardrail all the same, as `people` says: the
//! runner's Unix user is trusted, and this stops a steered model, not a
//! person with a shell.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// What the gate says about one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Refused, with the reason the model reads.
    Deny(&'static str),
}

/// The skill a rep runs on, the one skill it may load.
pub const REP_SKILL: &str = "farcooler:rep";

const NOT_A_MESSENGERS: &str =
    "A rep only carries messages. It can read the board, message the orchestrator and comment on a card.";
const ONE_PLAIN_COMMAND: &str = "A rep runs one plain Far Cooler command: plan, task show, task list, \
     message orchestrator, task note --kind comment, or search. No other program, flag or shell syntax.";
const OUTSIDE: &str = "A rep reads only inside its workspace's checkout, and never `.git` or a `.env` file.";
const CANT_CHECK: &str = "The runner couldn't check whether git tracks that file. Try again.";
const UNTRACKED: &str =
    "A rep reads only files git tracks. Find one with `search '<pattern>'`, then read it.";
const PATTERN: &str =
    "A rep's search pattern stays inside its workspace's checkout: no absolute path, `~`, `..` or braces.";
const ONE_FILE: &str =
    "A rep's search names one file inside its workspace's checkout. Glob finds the files; Grep checks one.";
const CONTENT: &str = "A rep's search names the files that match. Read one to see its lines.";
const SKILL: &str = "A rep loads only its own skill.";

/// The gate for one rep: its workspace's checkout, and the CLI it may run,
/// by its absolute path only, never a name found through `PATH`.
pub struct Gate<'a> {
    pub checkout: &'a Path,
    pub cli: &'a str,
    /// Whether a resolved path is a regular file (`Path::is_file` outside
    /// tests).
    pub is_file: &'a dyn Fn(&Path) -> bool,
    /// Whether a file, as a resolved path under the checkout, is one git
    /// tracks (`repo_grep::tracked_blocking` outside tests).
    /// `None` when git couldn't be asked, which is refused too, in its own
    /// words.
    pub tracked: &'a dyn Fn(&Path) -> Option<bool>,
}

/// Allow or refuse `tool` called with `input`. `resolve` is the path as the
/// file system sees it, symlinks followed, or `None` when it isn't there
/// (`std::fs::canonicalize` outside tests).
pub fn decide(gate: &Gate, tool: &str, input: &Value, resolve: &dyn Fn(&Path) -> Option<PathBuf>) -> Verdict {
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let verdict = |ok: bool, why| if ok { Verdict::Allow } else { Verdict::Deny(why) };
    match tool {
        "Bash" => verdict(text("command").is_some_and(|c| plain_cli_command(c, gate.cli)), ONE_PLAIN_COMMAND),
        "Read" => {
            let Some(within) = text("file_path").and_then(|p| within(gate.checkout, p, resolve)) else {
                return Verdict::Deny(OUTSIDE);
            };
            tracked_verdict(gate, &within)
        }
        "Grep" | "Glob" => {
            let path = match input.get("path") {
                // No path searches the checkout, a directory: Glob lists
                // names, Grep is refused below.
                None | Some(Value::Null) => true,
                Some(Value::String(p)) => inside(gate.checkout, p, resolve),
                Some(_) => false,
            };
            if !path {
                return Verdict::Deny(OUTSIDE);
            }
            // One file only. A search over a directory would reach `.git`,
            // `.env*` and the ignored files, and its counts would tell the
            // model what Read won't show. The path is resolved, so a link to
            // a directory is a directory here, and a link to a secret was
            // refused by `inside`.
            if tool == "Grep" {
                let file = text("path").and_then(|p| resolve(&checkout_path(gate.checkout, p)));
                if !file.is_some_and(|f| (gate.is_file)(&f)) {
                    return Verdict::Deny(ONE_FILE);
                }
                // The index too, as Read: a count over an ignored file reads
                // it out a prefix at a time.
                // A path that stopped resolving since `inside` is refused, not
                // let past this check.
                let Some(w) = text("path").and_then(|p| within(gate.checkout, p, resolve)) else {
                    return Verdict::Deny(OUTSIDE);
                };
                if let refused @ Verdict::Deny(_) = tracked_verdict(gate, &w) {
                    return refused;
                }
            }
            // Names, not lines: a line would show a `.env` file's contents,
            // which Read refuses.
            if tool == "Grep" && !matches!(text("output_mode"), None | Some("files_with_matches") | Some("count")) {
                return Verdict::Deny(CONTENT);
            }
            let pattern_key = if tool == "Glob" { "pattern" } else { "glob" };
            let pattern = match input.get(pattern_key) {
                None | Some(Value::Null) => tool == "Grep",
                Some(Value::String(p)) => stays_inside(p),
                Some(_) => false,
            };
            verdict(pattern, PATTERN)
        }
        "Skill" => verdict(text("skill") == Some(REP_SKILL), SKILL),
        _ => Verdict::Deny(NOT_A_MESSENGERS),
    }
}

/// Allow a file git tracks; refuse one it doesn't, or say git couldn't tell.
fn tracked_verdict(gate: &Gate, within: &Path) -> Verdict {
    match (gate.tracked)(within) {
        Some(true) => Verdict::Allow,
        Some(false) => Verdict::Deny(UNTRACKED),
        None => Verdict::Deny(CANT_CHECK),
    }
}

/// Whether `raw`, taken from the checkout when relative, is inside it, and
/// isn't `.git` or a `.env` file there.
///
/// Lexically first: a path that isn't under the checkout as written is
/// refused before the file system is asked about it, so the gate never waits
/// on a path the model chose. Then resolved, symlinks and `..` followed, and
/// refused unless it's really there and really inside.
fn inside(checkout: &Path, raw: &str, resolve: &dyn Fn(&Path) -> Option<PathBuf>) -> bool {
    within(checkout, raw, resolve).is_some()
}

/// [`inside`], answering where: the resolved path under the checkout.
fn within(checkout: &Path, raw: &str, resolve: &dyn Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let path = checkout_path(checkout, raw);
    let (written, lexical_root) = (lexical(&path)?, lexical(checkout)?);
    let root = resolve(checkout)?;
    if !written.starts_with(&lexical_root) && !written.starts_with(&root) {
        return None;
    }
    let real = resolve(&path)?;
    let within = real.strip_prefix(&root).ok()?;
    (!within.components().any(|c| private(&c.as_os_str().to_string_lossy()))).then(|| within.to_path_buf())
}

/// `raw` as a path, from the checkout when it's relative.
fn checkout_path(checkout: &Path, raw: &str) -> PathBuf {
    let given = Path::new(raw);
    if given.is_absolute() { given.to_path_buf() } else { checkout.join(given) }
}

/// `.git` and `.env` files: a repository's credentials and a project's
/// secrets, which a rep could otherwise post on a card.
///
/// Compared without case. A resolved path is safe either way, since
/// `canonicalize` returns the on-disk case, but a pattern is never resolved:
/// on a disk that ignores case (macOS's default) Glob's `pattern` and Grep's
/// `glob` can name `.ENV` and match `.env`.
pub(crate) fn private(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".git" || name.starts_with(".env")
}

/// `path` with `.` dropped and `..` taken off the part before it, without
/// asking the file system; `None` for one that climbs above its root.
fn lexical(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// A glob that can't name anything outside the directory it's run in, or
/// `.git` or a `.env` file.
fn stays_inside(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern.starts_with('/')
        && !pattern.starts_with('~')
        && !pattern.contains(['\0', '{', '}'])
        && !pattern.split(['/', '\\']).any(|part| part == ".." || private(part))
}

/// Whether `command` is one plain command of the CLI's grammar for a rep.
pub fn plain_cli_command(command: &str, cli: &str) -> bool {
    let Some(words) = words(command) else { return false };
    let Some((program, rest)) = words.split_first() else { return false };
    if !Path::new(cli).is_absolute() || program != cli {
        return false;
    }
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    match rest.as_slice() {
        ["plan"] | ["plan", "--json"] | ["task", "list"] | ["task", "list", "--json"] => true,
        ["task", "show", key] | ["task", "show", key, "--json"] => is_key(key),
        ["search", pattern] => is_text(pattern),
        ["search", pattern, "--path", dir] => is_text(pattern) && is_search_path(dir),
        ["message", "orchestrator", said] => is_text(said),
        ["message", "orchestrator", said, "--task", key] => is_text(said) && is_key(key),
        ["task", "note", key, a, b, c, d] => {
            let pairs = [(*a, *b), (*c, *d)];
            is_key(key)
                && pairs.contains(&("--kind", "comment"))
                && pairs.iter().any(|(flag, said)| *flag == "--body" && is_text(said))
        }
        _ => false,
    }
}

/// A card's key: letters, digits, `-` and `_`, never read as a flag.
fn is_key(word: &str) -> bool {
    (1..=40).contains(&word.len())
        && !word.starts_with('-')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The directory or file a search stays under: relative, plain parts, never
/// read as a flag, and never `.git` or a `.env` file. The runner checks it
/// again against the disk (`repo_grep`).
fn is_search_path(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with(['-', '/', '~', ':'])
        && !word.contains('\0')
        && !word.split(['/', '\\']).any(|part| part == ".." || private(part))
}

/// A message or a comment: something, never read as a flag.
fn is_text(word: &str) -> bool {
    !word.trim().is_empty() && !word.starts_with('-')
}

/// `command` as the words a shell would hand the program, or `None` when it
/// holds anything a shell would read as more than words: a metacharacter,
/// redirection, glob, `~`, brace, tab or comment outside quotes, an
/// expansion inside double quotes, an escape, or a line break anywhere.
pub fn words(command: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        '\n' | '\r' | '\0' => return None,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '$' | '`' | '\\' | '\n' | '\r' | '\0' => return None,
                        c => word.push(c),
                    }
                }
            }
            c if c.is_ascii_alphanumeric() || "_-./:=,@+%".contains(c) => {
                started = true;
                word.push(c);
            }
            _ => return None,
        }
    }
    if started {
        out.push(word);
    }
    Some(out)
}

#[cfg(test)]
#[path = "rep_gate_tests.rs"]
mod tests;
