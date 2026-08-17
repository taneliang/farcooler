//! Which session-log file belongs to which pane.
//!
//! The three parsers in `farcooler_core::session_log` turn a file into
//! events; nothing about their output says which of possibly hundreds of
//! files on disk is the one a given pane is running. That is this module's
//! whole job, and it has a different answer for every agent
//! (`docs/agent-session-logs.md`, "Which file belongs to which pane"):
//!
//! - **codex** holds its rollout file open for as long as it runs, so
//!   `lsof -p <pid>` names it directly -- except before the first turn is
//!   submitted, when nothing is open yet, and a wrapper's own shadow log can
//!   be open alongside it and must not be mistaken for the rollout.
//! - **claude and cursor** append and close, so `lsof` finds nothing for
//!   them. Their project directory comes from the pane's `cwd` by the slug
//!   rules in `farcooler_core::session_log`, and within it the right file
//!   is picked by matching the pane's OSC title against the file's own
//!   title, when more than one file could be it.
//!
//! Every path here degrades to `None` rather than guessing: attaching a pane
//! to the wrong conversation is a worse failure than showing nothing, the
//! same principle `session_discovery` already applies to hand-started
//! sessions.

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_core::session_log::{claude_slug, cursor_slug};
use farcooler_core::title;

/// The session-log file a pane is running, if one can be found honestly.
///
/// `preset` selects the agent (`"claude"`, `"codex"`, `"cursor"`, matched by
/// prefix the same way `service::terminal_mode_command` reads a preset
/// string -- a preset can carry a suffix). `pid` is the pane's foreground
/// process, needed only for codex. `cwd` and `title` are read straight off
/// the pane. An unrecognized preset -- anything Far Cooler does not have a
/// join strategy for -- yields `None` rather than guessing at one.
pub fn find_session_log(preset: &str, pid: Option<i32>, cwd: &str, title: &str) -> Option<PathBuf> {
    let home = home_dir()?;
    find_session_log_under(&home, preset, pid, cwd, title)
}

/// `$HOME` is not always set (a stripped-down service environment, a test
/// harness), and a missing home directory means every root this module reads
/// is unreachable -- `None` up front rather than three separate failures
/// downstream that all say the same thing.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `find_session_log` with `home` injected, so tests can point every root at
/// a temp directory instead of a real, large, concurrently-changing one.
fn find_session_log_under(home: &Path, preset: &str, pid: Option<i32>, cwd: &str, pane_title: &str) -> Option<PathBuf> {
    if preset.starts_with("claude") {
        find_claude(home, &resolved(cwd), pane_title)
    } else if preset.starts_with("codex") {
        find_codex(pid)
    } else if preset.starts_with("cursor") {
        find_cursor(home, &resolved(cwd), pane_title)
    } else {
        None
    }
}

/// The spelling of `cwd` that the agent itself would have slugged.
///
/// Both slug-based joins turn a path into a directory name character by
/// character, so two spellings of one directory produce two different slugs
/// and the lookup finds nothing at all -- silently, since a missing directory
/// is indistinguishable from an agent that has not written yet. The agents
/// slug the path the kernel hands them, which is fully resolved; Far Cooler
/// knows a worktree by the path it was configured with, which need not be.
/// On macOS that difference is routine rather than exotic: `/tmp` is a
/// symlink to `/private/tmp`, so a worktree under `/tmp` never joins.
///
/// A path that cannot be resolved -- deleted out from under the pane, or
/// unreadable -- keeps its original spelling, which is still the best guess
/// available and no worse than not trying.
fn resolved(cwd: &str) -> String {
    std::fs::canonicalize(cwd).map_or_else(|_| cwd.to_string(), |path| path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------
// claude
// ---------------------------------------------------------------------

/// Claude's project directory is a straight function of `cwd` -- no
/// truncation was ever observed for it (`claude_slug`'s own doc), so this
/// needs no prefix fallback the way cursor's does.
///
/// What that directory holds, though, is not one conversation per pane. Every
/// pane in a workspace shares one `cwd` (`watch.rs` joins on
/// `workspace.worktree_path`), so they all slug to the SAME project
/// directory, and Far Cooler's own chat panes -- which drive claude through
/// the SDK rather than a terminal -- write their sessions there too. A real
/// directory on this machine holds three SDK sessions next to four terminal
/// ones. So being the only file in the directory says nothing about whose
/// conversation it is: only a session a TERMINAL pane could actually be
/// running is a candidate at all (`is_terminal_session`), and only then does
/// the counting rule below apply -- one candidate is unambiguous, several need
/// the title.
fn find_claude(home: &Path, cwd: &str, pane_title: &str) -> Option<PathBuf> {
    let dir = home.join(".claude/projects").join(claude_slug(cwd));
    let candidates = jsonl_files(&dir).into_iter().filter(|p| is_terminal_session(p)).collect();
    pick_candidate(candidates, pane_title, read_claude_title)
}

/// Whether this file is a conversation someone typed into a terminal.
///
/// Claude records how it was started in an `entrypoint` field near the head of
/// every session file: `cli` is a person at a terminal, `sdk-cli` and `sdk-ts`
/// are programmatic sessions driven through the SDK -- which is exactly what
/// Far Cooler's own chat panes are. A tmux pane, the only thing this module is
/// ever asked about, is never running an SDK session, so those files are not
/// its conversation no matter how few other files sit beside them.
///
/// This accepts `cli` rather than rejecting `sdk-*`, so an entrypoint claude
/// has not shipped yet is refused rather than joined: a new programmatic
/// entrypoint would otherwise be attached to a terminal pane the day it
/// appears, and the whole module's principle is that a wrong conversation is
/// worse than none. Same reasoning for a file whose head names no entrypoint
/// at all -- of 376 real files on this machine the 34 without one are all
/// title-only metadata stubs and synthetic fixtures, none of them a session a
/// pane could be running.
fn is_terminal_session(path: &Path) -> bool {
    let entrypoint = scan_head(path, |record| {
        record.get("entrypoint").and_then(serde_json::Value::as_str).map(str::to_string)
    });
    entrypoint.as_deref() == Some("cli")
}

/// `aiTitle` is re-emitted at every checkpoint and never changes within a
/// session (`docs/agent-session-logs.md`), so the first `ai-title` record
/// found is as good as the last -- there is nothing to disambiguate between
/// them within one file.
fn read_claude_title(path: &Path) -> Option<String> {
    scan_head(path, |record| {
        if record.get("type").and_then(serde_json::Value::as_str) != Some("ai-title") {
            return None;
        }
        record.get("aiTitle").and_then(serde_json::Value::as_str).map(str::to_string)
    })
}

/// The first record at the head of `path` that `pick` recognizes.
///
/// Bounded on purpose, and the bound is the point. Both fields this reads are
/// header fields, but the file they head can be enormous -- the largest
/// session file on this machine is 108 MB, and one project directory holds
/// 382 MB across 50 of them. `pick_candidate` has to exhaust its filter to
/// prove a match is unique, so reading whole files meant reading that entire
/// directory on every attempt; and because a failed join is retried every
/// `LOG_JOIN_INTERVAL_MS` forever, a directory where the title can never match
/// became a permanent read loop over hundreds of megabytes. Streaming is not
/// enough on its own either: `.lines()` over a whole file still walks to the
/// end when the field is absent, which is the common case (274 of 376 real
/// files have no `ai-title`). So the walk itself is what has to stop.
///
/// 64 lines and 1 MiB, whichever comes first, measured against every claude
/// session file on this machine: `entrypoint` appears by line 34 at the
/// latest, `ai-title` by line 38 and by byte 359 KB. Each bound covers its
/// worst observed case with room to spare, and they are needed together
/// because either alone can be defeated -- a single line can reach 1.35 MB, so
/// 64 lines is not a byte bound, and a header field 40 lines down is not
/// covered by a byte count alone.
///
/// What is given up: a file that buries these fields deeper than any real file
/// does reads as if it had none. That costs a join (`None`, refuse), never a
/// wrong one, which is the direction this module always errs in.
fn scan_head<T>(path: &Path, pick: impl Fn(&serde_json::Value) -> Option<T>) -> Option<T> {
    const HEAD_LINES: usize = 64;
    const HEAD_BYTES: u64 = 1 << 20;

    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file.take(HEAD_BYTES));
    // `map_while` rather than `flatten`: a read that fails -- the byte bound
    // cutting a multi-byte character in half, most likely -- ends the scan,
    // because everything after it is a line this never saw the start of.
    reader.lines().take(HEAD_LINES).map_while(Result::ok).find_map(|line| {
        let record: serde_json::Value = serde_json::from_str(&line).ok()?;
        pick(&record)
    })
}

// ---------------------------------------------------------------------
// cursor
// ---------------------------------------------------------------------

/// Cursor's transcripts live a level deeper than claude's -- under
/// `<project-dir>/agent-transcripts/<uuid>/<uuid>.jsonl` -- and cursor writes
/// nothing shaped like claude's `aiTitle` in any of the four real transcripts
/// this was checked against, so there is no title field of its own to match.
/// `pick_candidate`'s title reader is therefore a function that always
/// returns `None`: with exactly one candidate that is never consulted; with
/// more than one, there is nothing honest to compare and the ambiguity rule
/// refuses rather than guess which one the pane means.
fn find_cursor(home: &Path, cwd: &str, pane_title: &str) -> Option<PathBuf> {
    let root = home.join(".cursor/projects");
    let dir = resolve_cursor_dir(&root, cwd)?;
    let candidates = cursor_transcript_files(&dir.join("agent-transcripts"));
    pick_candidate(candidates, pane_title, |_| None)
}

/// Find the real `~/.cursor/projects/<slug>` directory for `cwd`.
///
/// `cursor_slug` truncates any path long enough to push
/// `.cursor/projects/<slug>` past cursor's own 92-character bound, and once
/// truncated its output ends in `-<7 hex>` computed by an FNV-1a hash that
/// does NOT match cursor's own sha256 (see `cursor_slug`'s doc comment). So a
/// truncated slug is not a real directory name: comparing it with `==`, or
/// opening it directly, looks in a directory that does not exist. What IS
/// real is the portion before that hash suffix, because cursor computes that
/// same prefix from the same path -- so once the exact match fails, this
/// lists the root and matches by that prefix instead.
///
/// The 84-character cut point `cursor_slug` uses is itself an unverified
/// guess -- no cursor directory on this machine is long enough to have
/// exercised it -- so this does not hardcode 84 (or the 67 characters that
/// number implies once the hash suffix is subtracted) anywhere. It asks
/// `truncated_prefix` whether a cut happened at all, which is answerable from
/// the path itself and tolerates the real cut point landing a few characters
/// off from today's guess in either direction. That question matters: a slug
/// short enough that cursor never touched it names a real directory or none,
/// and prefix-matching it would reach into a neighboring project.
fn resolve_cursor_dir(root: &Path, cwd: &str) -> Option<PathBuf> {
    let slug = cursor_slug(cwd);
    let direct = root.join(&slug);
    if direct.is_dir() {
        return Some(direct);
    }
    let prefix = truncated_prefix(&slug, cwd)?;
    let mut matches = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with(prefix));
    let first = matches.next()?;
    // A second directory sharing the same prefix means the prefix alone
    // cannot tell them apart -- there is nothing honest left to choose
    // between, so this refuses rather than guess which one `cwd` meant.
    if matches.next().is_some() {
        return None;
    }
    Some(root.join(first))
}

/// The prefix a TRUNCATED cursor slug shares with cursor's own directory name.
///
/// The shape of the suffix is not enough to know a slug was truncated, which
/// is what this exists to correct. A path like `/Users/e/build-a1b2c3d` slugs
/// to `Users-e-build-a1b2c3d`, nowhere near long enough to truncate, yet it
/// ends in seven lowercase hex characters all the same -- and stripping them
/// would leave `Users-e-build` to prefix-match `Users-e-build-system`, a
/// DIFFERENT project whose transcripts the pane would then be shown. That is
/// the one failure this module exists to prevent, arrived at from a path that
/// simply has no cursor directory of its own and should have yielded nothing.
///
/// What tells the two apart is that `cursor_slug` only ever DROPS characters
/// when it truncates: an untruncated slug still carries every alphanumeric
/// character of the path, in order, since the rest of the rule only rewrites
/// separators. So comparing those two sequences answers "was anything cut"
/// exactly, without this having to know cursor's cut point -- which stays
/// unverified, and which `cursor_slug` may yet have wrong by a few characters.
///
/// It errs in the safe direction, too: the only way this can be wrong is by
/// calling a truncated slug untruncated, which loses the prefix path and
/// yields `None`. It can never invent a truncation that did not happen.
fn truncated_prefix<'a>(slug: &'a str, cwd: &str) -> Option<&'a str> {
    let alphanumerics = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<Vec<_>>();
    if alphanumerics(slug) == alphanumerics(cwd) {
        return None;
    }
    strip_hash_suffix(slug)
}

/// The portion of a cursor slug before its truncation hash, if it has that
/// shape. Only ever asked about a slug `truncated_prefix` has already
/// established was truncated -- on its own the shape proves nothing.
fn strip_hash_suffix(slug: &str) -> Option<&str> {
    let (prefix, suffix) = slug.rsplit_once('-')?;
    let is_lowercase_hex = suffix.len() == 7 && suffix.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    is_lowercase_hex.then_some(prefix)
}

/// The `.jsonl` files one level under `agent_transcripts_dir` -- one per
/// `<uuid>` subdirectory, matching cursor's own
/// `agent-transcripts/<uuid>/<uuid>.jsonl` layout. A missing directory (no
/// cursor session has ever run for this project) yields no candidates rather
/// than an error.
fn cursor_transcript_files(agent_transcripts_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(agent_transcripts_dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .flat_map(|e| jsonl_files(&e.path()))
        .collect()
}

// ---------------------------------------------------------------------
// codex
// ---------------------------------------------------------------------

/// Codex holds its rollout file open for as long as it runs, so `lsof`
/// answers this directly -- no slug or title needed, and none would help: a
/// freshly started pane with no rollout open yet is not a failure to
/// disambiguate, it is a pane that genuinely has nothing on disk
/// (`docs/agent-session-logs.md`, "codex -- lsof").
fn find_codex(pid: Option<i32>) -> Option<PathBuf> {
    pick_codex_rollout(&open_files_for_pid(pid?))
}

/// The pane's process and everything descended from it.
///
/// The pane's pid is its top process, which is the user's SHELL: Far Cooler
/// deliberately opens a shell and lets a person type `codex` into it rather
/// than being told in advance what a pane runs. Codex is therefore a child of
/// that shell -- a grandchild, with fish, which re-execs itself -- and the
/// shell holds no rollout open at all. Asking `lsof` about the pane pid alone
/// finds nothing, forever, in the ordinary way the product is used.
///
/// One `ps` for the whole table rather than a walk that spawns per level:
/// the tree is rebuilt from `pid`/`ppid` pairs, so depth costs nothing.
fn process_subtree(root: i32) -> Vec<i32> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return vec![root] };
    let text = String::from_utf8_lossy(&out.stdout);

    let pairs: Vec<(i32, i32)> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();

    // Breadth-first from the pane. `ps` output is finite and every pid appears
    // once, so a pid already collected is never expanded twice -- which also
    // makes a cycle (a reparented orphan) impossible to loop on.
    let mut subtree = vec![root];
    let mut next = 0;
    while next < subtree.len() {
        let parent = subtree[next];
        next += 1;
        for &(pid, ppid) in &pairs {
            if ppid == parent && !subtree.contains(&pid) {
                subtree.push(pid);
            }
        }
    }
    subtree
}

/// Every path a pane's processes hold open, from `lsof`.
///
/// Same shape as `farcooler_core::ports::listening_ports`: one process
/// spawn, and failure degrades to an empty list rather than an error -- a
/// machine without `lsof`, or a `lsof` invocation the OS refuses, must lose
/// this join, not crash the daemon over it.
fn open_files_for_pid(pid: i32) -> Vec<PathBuf> {
    // `lsof -p` takes a comma-separated list, so the whole subtree costs the
    // same single spawn one pid did.
    let pids = process_subtree(pid).iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let out = Command::new("lsof")
        .args(["-p", &pids, "-Fn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return Vec::new() };

    // `-Fn` is a field-per-line format: the block opens with a `p<pid>` line
    // (not a path, and skipped by the tag check below), followed by one
    // `n<path>` line per open file.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (tag, value) = line.split_at(1.min(line.len()));
            (tag == "n").then(|| PathBuf::from(value))
        })
        .collect()
}

/// Codex's rollout file among everything a pane's process has open.
///
/// The process is not guaranteed to hold only its own log open: a codex
/// started through a wrapper also holds the wrapper's own shadow jsonl,
/// under a temp directory unrelated to `~/.codex/sessions`
/// (`docs/agent-session-logs.md`, "codex decoys"). So this matches the
/// rollout's own naming rule specifically -- `~/.codex/sessions/**/rollout-*.jsonl`
/// -- rather than "the open jsonl", which the wrapper's decoy would satisfy
/// just as well and pick wrong.
fn pick_codex_rollout(open_files: &[PathBuf]) -> Option<PathBuf> {
    open_files.iter().find(|p| is_codex_rollout(p)).cloned()
}

fn is_codex_rollout(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    (name.starts_with("rollout-") && name.ends_with(".jsonl")) && path.to_string_lossy().contains("/.codex/sessions/")
}

// ---------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------

/// The `.jsonl` files directly in `dir`, or none if it does not exist.
///
/// Not recursive: both claude's project directory and (once resolved)
/// cursor's per-`uuid` transcript directory hold their session files
/// directly. Claude's subagent transcripts live in a sibling `subagents/`
/// directory this join has no reason to descend into -- Task 7 is about the
/// pane's own conversation, not the agents it spawned.
fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect()
}

/// Choose which candidate file is the pane's own session.
///
/// A candidate is a file the caller has already established this pane could be
/// running -- `find_claude` vets that, and it is the vetting rather than the
/// counting that keeps a pane off another pane's conversation. What is left
/// here is only telling several plausible sessions apart.
///
/// Exactly one candidate does not need the title at all
/// -- requiring one would fail the overwhelmingly common case
/// (`docs/agent-session-logs.md`, "claude and cursor -- slug, then title").
/// With more than one, only a candidate whose own title (`read_title`)
/// equals the pane's title -- once its leading activity glyph is stripped --
/// is trustworthy; picking any other would attach the row to a conversation
/// that is not the one on screen. So no match, and more than one match, both
/// refuse rather than guess.
fn pick_candidate(mut candidates: Vec<PathBuf>, pane_title: &str, read_title: impl Fn(&Path) -> Option<String>) -> Option<PathBuf> {
    if candidates.len() == 1 {
        return candidates.pop();
    }
    if candidates.is_empty() {
        return None;
    }
    let stripped = title::strip_glyph(pane_title).trim();
    let mut matches = candidates.into_iter().filter(|p| read_title(p).as_deref() == Some(stripped));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farcooler-log-join-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A session shaped like the ones a real terminal pane writes: the
    /// `entrypoint` attachment claude emits within the first few records, then
    /// the conversation. `entrypoint` is what says a person typed this, and
    /// every real `cli` file on this machine carries it.
    fn write_claude_session(dir: &Path, session_id: &str, ai_title: Option<&str>) -> PathBuf {
        write_claude_session_with_entrypoint(dir, session_id, Some("cli"), ai_title)
    }

    fn write_claude_session_with_entrypoint(dir: &Path, session_id: &str, entrypoint: Option<&str>, ai_title: Option<&str>) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{session_id}.jsonl"));
        let mut lines = Vec::new();
        if let Some(e) = entrypoint {
            lines.push(format!(r#"{{"type":"attachment","entrypoint":"{e}"}}"#));
        }
        lines.push(r#"{"type":"user","promptSource":"typed"}"#.to_string());
        if let Some(t) = ai_title {
            lines.push(format!(r#"{{"type":"ai-title","aiTitle":"{t}"}}"#));
        }
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    // -----------------------------------------------------------------
    // claude
    // -----------------------------------------------------------------

    #[test]
    fn claude_with_one_terminal_session_is_chosen_without_a_title() {
        let home = scratch("claude-only");
        let cwd = "/Users/e-liang/Dev/only-one";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        let expected = write_claude_session(&dir, "sess-a", None);

        // The overwhelmingly common case, and the one the head bound must not
        // break: a pane that has typed once, whose session has no `ai-title`
        // yet, and nothing else in the directory to confuse it with. No title
        // match is possible and none is required.
        let found = find_session_log_under(&home, "claude", None, cwd, "some unrelated pane title").unwrap();
        assert_eq!(found, expected);
    }

    /// The failure this pins is the one that motivated the whole candidate
    /// rule: every pane in a workspace shares one `cwd`, so a chat pane and a
    /// terminal pane slug to the SAME project directory. Open a chat in a
    /// fresh worktree and the shim's SDK session is the only file there; open
    /// a terminal beside it and type `claude`, and "the only file in the
    /// directory" used to be answer enough -- the terminal pane reported the
    /// chat's turn boundaries and feed rows, live, as its own.
    #[test]
    fn claude_does_not_join_a_pane_to_a_chat_panes_sdk_session() {
        let home = scratch("claude-sdk-decoy");
        let cwd = "/Users/e-liang/Dev/shared-with-chat";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session_with_entrypoint(&dir, "sess-shim", Some("sdk-cli"), None);

        // Alone in the directory, and still not this pane's conversation.
        assert_eq!(find_session_log_under(&home, "claude", None, cwd, "✳ Write a haiku"), None);

        // Once the terminal writes its own session, that one is joined -- and
        // without a title, because the shim's file was never a candidate to be
        // ambiguous with.
        let expected = write_claude_session(&dir, "sess-terminal", None);
        let found = find_session_log_under(&home, "claude", None, cwd, "✳ Write a haiku").unwrap();
        assert_eq!(found, expected);
    }

    /// The 270-byte metadata stubs that sit in real project directories --
    /// an `ai-title` and an `agent-name`, no conversation and no entrypoint.
    /// They would match a pane's title perfectly and have nothing to report.
    #[test]
    fn claude_does_not_join_a_title_only_stub() {
        let home = scratch("claude-stub");
        let cwd = "/Users/e-liang/Dev/stub-only";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session_with_entrypoint(&dir, "sess-stub", None, Some("Write a haiku"));

        assert_eq!(find_session_log_under(&home, "claude", None, cwd, "✳ Write a haiku"), None);
    }

    /// An `ai-title` further into the file than the first line, which is where
    /// real ones land: the head bound has to reach it, or the disambiguating
    /// case it exists for stops working the moment a session gets going.
    #[test]
    fn claude_reads_a_title_that_is_not_on_the_first_line() {
        let home = scratch("claude-deep-title");
        let cwd = "/Users/e-liang/Dev/deep-title";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session(&dir, "sess-other", Some("Fix the flaky test"));

        let path = dir.join("sess-mine.jsonl");
        let mut lines = vec![r#"{"type":"attachment","entrypoint":"cli"}"#.to_string()];
        for _ in 0..20 {
            lines.push(format!(r#"{{"type":"assistant","text":"{}"}}"#, "padding ".repeat(500)));
        }
        lines.push(r#"{"type":"ai-title","aiTitle":"Write a haiku"}"#.to_string());
        std::fs::write(&path, lines.join("\n")).unwrap();

        let found = find_session_log_under(&home, "claude", None, cwd, "✳ Write a haiku").unwrap();
        assert_eq!(found, path);
    }

    /// The bug this pins was found live, not reasoned about: a scratch
    /// workspace whose worktree sat under `/tmp` never joined to its session
    /// log, and its feed stayed empty for a whole turn, because claude had
    /// slugged `/private/tmp/...` while Far Cooler slugged `/tmp/...`. The two
    /// spellings name one directory and produce two slugs, and the miss is
    /// silent. Everything else about that run was correct.
    #[test]
    fn claude_joins_through_a_symlinked_path() {
        let home = scratch("claude-symlink");
        let real = home.join("real-worktree");
        std::fs::create_dir_all(&real).unwrap();
        let link = home.join("linked-worktree");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // The agent slugs the path the kernel gave it, which is resolved.
        let canonical = real.canonicalize().unwrap();
        let dir = home.join(".claude/projects").join(claude_slug(&canonical.to_string_lossy()));
        let expected = write_claude_session(&dir, "sess-a", None);

        // The pane is known by the unresolved spelling, and must still join.
        let found = find_session_log_under(&home, "claude", None, &link.to_string_lossy(), "some pane title").unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn claude_with_two_files_is_told_apart_by_ai_title() {
        let home = scratch("claude-two");
        let cwd = "/Users/e-liang/Dev/shared-worktree";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session(&dir, "sess-other", Some("Fix the flaky test"));
        let expected = write_claude_session(&dir, "sess-mine", Some("Write a haiku"));

        let found = find_session_log_under(&home, "claude", None, cwd, "✳ Write a haiku").unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn claude_strips_the_leading_activity_glyph_before_comparing() {
        // The exact worked example from the reference doc, verified live
        // there: `✳ Write haiku about lighthouse` against
        // `"aiTitle":"Write haiku about lighthouse"`.
        let home = scratch("claude-glyph");
        let cwd = "/Users/e-liang/Dev/glyph-case";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session(&dir, "sess-decoy", Some("Some other task"));
        let expected = write_claude_session(&dir, "sess-target", Some("Write haiku about lighthouse"));

        let found = find_session_log_under(&home, "claude", None, cwd, "✳ Write haiku about lighthouse").unwrap();
        assert_eq!(found, expected);

        // A spinner glyph, not just the resting one, must strip the same way.
        write_claude_session(&dir, "sess-target", Some("Write haiku about lighthouse"));
        let found = find_session_log_under(&home, "claude", None, cwd, "◐ Write haiku about lighthouse").unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn claude_with_two_files_and_no_matching_title_refuses() {
        let home = scratch("claude-no-match");
        let cwd = "/Users/e-liang/Dev/no-match";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        write_claude_session(&dir, "sess-a", Some("Task A"));
        write_claude_session(&dir, "sess-b", Some("Task B"));

        assert_eq!(find_session_log_under(&home, "claude", None, cwd, "✳ Task C"), None);
    }

    // -----------------------------------------------------------------
    // cursor
    // -----------------------------------------------------------------

    #[test]
    fn cursor_short_path_finds_its_transcript_directly() {
        let home = scratch("cursor-short");
        let cwd = "/Users/e-liang/Dev/Verdela";
        let slug = cursor_slug(cwd);
        let transcript_dir = home.join(".cursor/projects").join(&slug).join("agent-transcripts").join("uuid-1");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let expected = transcript_dir.join("uuid-1.jsonl");
        std::fs::write(&expected, r#"{"role":"user"}"#).unwrap();

        let found = find_session_log_under(&home, "cursor", None, cwd, "anything").unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn cursor_long_path_is_found_by_prefix_after_truncation() {
        // Long enough that `cursor_slug` truncates and appends its hash --
        // this is the case the whole prefix-matching path exists for. The
        // real directory name is NOT `cursor_slug(cwd)` (that string names no
        // real directory at all, per `cursor_slug`'s own doc): it is
        // whatever cursor itself would have named it, sharing only the
        // prefix. That is simulated here by writing under a directory whose
        // name is the computed prefix plus a DIFFERENT suffix than the one
        // `cursor_slug` happened to compute -- proving the match is by
        // prefix, not by accidentally reproducing the same hash.
        let home = scratch("cursor-long");
        let cwd = format!("/Users/e-liang/{}", "x".repeat(200));
        let slug = cursor_slug(&cwd);
        let prefix = strip_hash_suffix(&slug).expect("a 200-character path must truncate");

        let real_dir_name = format!("{prefix}-cafefee"); // cursor's own (different) hash
        let transcript_dir = home.join(".cursor/projects").join(&real_dir_name).join("agent-transcripts").join("uuid-2");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let expected = transcript_dir.join("uuid-2.jsonl");
        std::fs::write(&expected, r#"{"role":"user"}"#).unwrap();

        let found = find_session_log_under(&home, "cursor", None, &cwd, "anything").unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn cursor_two_directories_sharing_a_truncated_prefix_refuse() {
        // Vanishingly unlikely on a real machine (two different paths would
        // have to share the same 67-character prefix) but not impossible, and
        // the brief is explicit: an ambiguous prefix match must fail soft,
        // not guess.
        let home = scratch("cursor-ambiguous");
        let cwd = format!("/Users/e-liang/{}", "y".repeat(200));
        let slug = cursor_slug(&cwd);
        let prefix = strip_hash_suffix(&slug).unwrap();

        for suffix in ["aaaaaaa", "bbbbbbb"] {
            let dir = home.join(".cursor/projects").join(format!("{prefix}-{suffix}"));
            std::fs::create_dir_all(dir.join("agent-transcripts").join("uuid")).unwrap();
            std::fs::write(dir.join("agent-transcripts").join("uuid").join("uuid.jsonl"), "{}").unwrap();
        }

        assert_eq!(find_session_log_under(&home, "cursor", None, &cwd, "anything"), None);
    }

    /// A short path whose last segment merely LOOKS like a truncation hash.
    /// Nothing about it was ever cut, so its slug names the only directory it
    /// could name -- and when that directory does not exist, the honest answer
    /// is nothing, not the transcripts of whichever neighboring project
    /// happens to share the prefix.
    #[test]
    fn cursor_does_not_prefix_match_a_path_that_was_never_truncated() {
        let home = scratch("cursor-false-hash");
        let cwd = "/Users/e-liang/Dev/build-a1b2c3d";

        let neighbor = home.join(".cursor/projects").join("Users-e-liang-Dev-build-system");
        let transcript_dir = neighbor.join("agent-transcripts").join("uuid-9");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(transcript_dir.join("uuid-9.jsonl"), r#"{"role":"user"}"#).unwrap();

        assert_eq!(find_session_log_under(&home, "cursor", None, cwd, "anything"), None);
    }

    #[test]
    fn cursor_with_no_directory_at_all_is_none() {
        let home = scratch("cursor-missing");
        assert_eq!(find_session_log_under(&home, "cursor", None, "/Users/e-liang/Dev/never-run", "anything"), None);
    }

    // -----------------------------------------------------------------
    // codex
    // -----------------------------------------------------------------

    /// Holds two files open at once via raw file descriptors -- one shaped
    /// like a real rollout, one an unrelated decoy -- for as long as the
    /// returned child lives. This is what a wrapped codex actually does
    /// (`docs/agent-session-logs.md`, "codex decoys"), reproduced with `sh`
    /// rather than a real codex binary so the test does not depend on one
    /// being installed.
    fn hold_files_open(paths: &[&Path]) -> std::process::Child {
        let mut script = String::new();
        for (i, p) in paths.iter().enumerate() {
            script.push_str(&format!("exec {}<{:?}\n", 3 + i, p));
        }
        script.push_str("sleep 30\n");
        std::process::Command::new("sh").arg("-c").arg(script).spawn().expect("spawn sh to hold files open")
    }

    /// Hold `path` open from a GRANDCHILD, leaving the returned process
    /// holding nothing itself -- the real shape of a pane, whose pid is a
    /// shell that spawns the agent.
    fn hold_file_open_from_a_grandchild(dir: &Path, path: &Path) -> std::process::Child {
        // Three script files rather than nested `sh -c` strings: one level of
        // shell quoting is readable, three is a bug waiting to happen.
        let holder = dir.join("holder.sh");
        std::fs::write(&holder, format!("exec 3<{}\nsleep 30\n", path.display())).unwrap();
        let middle = dir.join("middle.sh");
        std::fs::write(&middle, format!("sh {} &\nwait\n", holder.display())).unwrap();
        let outer = dir.join("outer.sh");
        std::fs::write(&outer, format!("sh {} &\nwait\n", middle.display())).unwrap();

        // `& wait` at each level keeps the parents alive, so the spawned pid
        // stays the root of a two-deep tree holding nothing itself.
        std::process::Command::new("sh").arg(&outer).spawn().expect("spawn nested sh")
    }

    /// The bug this pins was found by driving a real codex: the pane's pid is
    /// the SHELL, codex is its grandchild, and only codex holds the rollout.
    /// Asking `lsof` about the pane pid alone found nothing every time, so the
    /// codex feed was empty and its turn clock silently came from the screen.
    #[test]
    fn codex_finds_a_rollout_held_by_a_grandchild_of_the_pane() {
        let root = scratch("codex-grandchild");
        let rollout_dir = root.join(".codex/sessions/2026/08/16");
        std::fs::create_dir_all(&rollout_dir).unwrap();
        let rollout = rollout_dir.join("rollout-nested.jsonl");
        std::fs::write(&rollout, "{}").unwrap();

        let mut child = hold_file_open_from_a_grandchild(&root, &rollout);
        let pane_pid = child.id() as i32;

        let found = poll_until(Duration::from_secs(10), || find_codex(Some(pane_pid)));

        let _ = child.kill();
        let _ = child.wait();

        let found = found.expect("the rollout is open in the pane's subtree, not on the pane pid itself");
        assert_eq!(found.canonicalize().unwrap(), rollout.canonicalize().unwrap());
    }

    #[test]
    fn codex_picks_the_rollout_over_a_wrapper_decoy() {
        let root = scratch("codex-decoy");
        let rollout_dir = root.join(".codex/sessions/2026/08/16");
        std::fs::create_dir_all(&rollout_dir).unwrap();
        let rollout = rollout_dir.join("rollout-x.jsonl");
        std::fs::write(&rollout, "{}").unwrap();

        let decoy_dir = root.join("tmp-wrapper");
        std::fs::create_dir_all(&decoy_dir).unwrap();
        let decoy = decoy_dir.join("superset-codex-session-1.jsonl");
        std::fs::write(&decoy, "{}").unwrap();

        let mut child = hold_files_open(&[&decoy, &rollout]);
        let pid = child.id() as i32;

        // `lsof` needs a moment to see a just-spawned process's fd table on a
        // loaded machine; poll rather than assume the first look is enough.
        let found = poll_until(Duration::from_secs(5), || find_codex(Some(pid)));

        let _ = child.kill();
        let _ = child.wait();

        // `lsof` reports the RESOLVED path, and on macOS the system temp
        // directory is reached through a `/var` -> `/private/var` symlink --
        // so the string it names differs from the one this test built the
        // file with, though both point at the same file. Canonicalize both
        // sides rather than assert on a symlink this test does not control.
        let canonical_rollout = rollout.canonicalize().unwrap();
        assert_eq!(found.map(|p| p.canonicalize().unwrap()), Some(canonical_rollout));
    }

    #[test]
    fn codex_with_nothing_open_yet_is_none_not_a_failure() {
        // A freshly started pane before the first turn is submitted: the
        // process exists, but has opened no rollout at all. That is the
        // documented normal case, not something to report as broken.
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;

        // Give lsof the same settling room as the decoy test, then confirm
        // the answer stays None rather than flipping true on a slow first read.
        std::thread::sleep(Duration::from_millis(300));
        let found = find_codex(Some(pid));

        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(found, None);
    }

    #[test]
    fn codex_with_no_pid_is_none() {
        assert_eq!(find_codex(None), None);
    }

    fn poll_until<T>(deadline: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let start = std::time::Instant::now();
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if start.elapsed() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // -----------------------------------------------------------------
    // unknown preset
    // -----------------------------------------------------------------

    #[test]
    fn an_unknown_preset_is_none() {
        let home = scratch("unknown-preset");
        assert_eq!(find_session_log_under(&home, "opencode", None, "/Users/e/Dev/x", "anything"), None);
    }

    // -----------------------------------------------------------------
    // sanity check against the real machine
    // -----------------------------------------------------------------

    /// Read-only, run by hand alongside the slug check above. Measures the
    /// cost of the join's WORST case against this machine's largest real
    /// project directory: a title that matches nothing, which is what forces
    /// every candidate to be read and what a pane in a directory with no
    /// `ai-title` anywhere hits on every tick, forever.
    #[test]
    #[ignore]
    fn a_real_claude_lookup_costs_what_the_head_bound_says_it_should() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let cwd = "/Users/e-liang/Dev/overnight";
        let dir = home.join(".claude/projects").join(claude_slug(cwd));
        if !dir.is_dir() {
            eprintln!("no real project directory at {}, nothing to measure", dir.display());
            return;
        }
        let files = jsonl_files(&dir);
        let bytes: u64 = files.iter().filter_map(|p| p.metadata().ok()).map(|m| m.len()).sum();

        let start = std::time::Instant::now();
        let found = find_claude(&home, cwd, "a title no session on this machine has");
        let elapsed = start.elapsed();

        eprintln!(
            "{} files / {:.0} MB on disk, worst-case lookup took {:?}, answered {found:?}",
            files.len(),
            bytes as f64 / 1e6,
            elapsed
        );
    }

    /// Read-only, run by hand: `cargo test -p farcooler-daemon log_join -- --ignored --nocapture`.
    /// Confirms `claude_slug` (consumed here, defined in Task 1) still names
    /// a directory that exists on THIS machine's real `~/.claude/projects`,
    /// rather than trusting the unit tests' temp directories alone. Does not
    /// assert on which directory -- the real tree changes while this runs
    /// and is too large to pin -- only that at least one real project
    /// directory's name round-trips through `claude_slug` applied to itself,
    /// proving the naming rule this join depends on is still what claude
    /// actually does today.
    #[test]
    #[ignore]
    fn a_real_claude_project_directory_exists_where_the_slug_says_it_should() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let root = home.join(".claude/projects");
        let Ok(entries) = std::fs::read_dir(&root) else { return };

        let mut checked = 0;
        for entry in entries.flatten().take(20) {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // The directory name IS the slugged cwd, so slugging it again is
            // idempotent -- claude never emits a character `claude_slug`
            // would itself replace.
            assert_eq!(claude_slug(&name), name, "slugging a real directory name must be a no-op: {name}");
            checked += 1;
        }
        eprintln!("checked {checked} real ~/.claude/projects director{ies}", ies = if checked == 1 { "y" } else { "ies" });
        assert!(checked > 0, "no real claude project directories found to check against");
    }
}
