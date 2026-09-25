//! Installing Far Cooler's hooks into files it does not own.
//!
//! This runner already has `herdr-agent-state.sh` and `.superset/hooks/*`
//! registered against all three agents — claude, codex and cursor.
//! **Far Cooler is not the only hook consumer and must never behave as
//! though it is**
//! (docs/superpowers/specs/2026-09-07-live-agent-sessions-design.md,
//! "Installing hooks, without owning them"):
//!
//! - **claude** never has a file rewritten here at all. `claude_settings`
//!   returns a settings object that Task 10 writes to the runtime directory
//!   and passes with `--settings <file>` on panes Far Cooler launches,
//!   which touches no user configuration.
//! - **codex** gets a merge into project-local `.codex/hooks.json` in the
//!   worktree.
//! - **cursor** gets a merge into project-local `.cursor/hooks.json`, flat
//!   and camelCase — no nested `hooks` array, and its own event names.
//!
//! An entry is ours only when it is exactly what we write: our shape, no key
//! we don't write, and one command in our grammar naming this CLI or this
//! runner's socket (`entry_is_ours`, `Us`). We merge, and we replace or
//! remove only entries that are ours by that test: a command of somebody's
//! inside one of our entries, a key of theirs on it, or a command line that
//! goes on after ours makes the entry theirs, and it is left exactly as it is.

use std::path::Path;

use serde_json::{Value, json};

/// The events claude and codex are both registered against, and whether
/// each is a gate a person's decision has to come back through.
///
/// claude and codex spell every one of these identically, so one table
/// drives both `claude_settings` and `merge_codex`.
///
/// **Cut down to exactly what something consumes today, fix round 1.** The
/// original table also carried `PreToolUse`, `PostToolUse` and
/// `PermissionRequest` (gating); nothing in this crate reads any of the
/// three yet — `assemble.rs`'s assembler only matches `UserPromptSubmit` and
/// `Stop`, and `hook_ingress::announced_terminal` binds a pane from ANY
/// event's `Facts`, not from `SessionStart` by name, but is still worth an
/// early hook so a pane binds before its first prompt rather than waiting
/// for one. Registering a gating event that nothing answers is not free: a
/// gating hook blocks on the socket for `hook::HOOK_DEADLINE` (400ms) before
/// deferring, so `PermissionRequest` today would add up to 400ms to every
/// permission prompt in every pane for no return. `PreToolUse`/`PostToolUse`
/// are cheaper — measured at ~15-18ms per invocation in the design doc — but
/// still buy nothing yet. Task 11 is what gives the daemon an opinion on a
/// permission request and re-adds `PermissionRequest`; whichever task wires
/// up a `PreToolUse`/`PostToolUse` consumer re-adds those two. This is
/// sequencing, not an oversight: see fix round 1 in
/// `.superpowers/sdd/2026-09-07-live-agent-sessions/task-9-report.md`.
const CLAUDE_CODEX_EVENTS: &[(&str, bool)] = &[
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("Stop", false),
];

/// Claude's alone, fix round 2: `MessageDisplay` is claude's streaming-prose
/// event, the entire subject of `MessageAssembler` (Task 4) and the thing
/// that makes claude the richest of the three agents per the spec's
/// capability table — and round 1's cut, in registering only what
/// `CLAUDE_CODEX_EVENTS` shared, dropped it without anything noticing,
/// because nothing here checked the direction "is every event the
/// assembler CONSUMES actually registered." `every_event_the_assembler_
/// consumes_is_registered_for_that_agent` in this module's tests is that
/// check now, against `farcooler_agent_hooks::assemble::CONSUMED` — a
/// constant that crate itself proves matches `accept`'s match arms, rather
/// than a hand-mirror kept here that could drift the same way again.
///
/// Not folded into `CLAUDE_CODEX_EVENTS`: codex has no streaming-prose hook
/// at all (capability table: "streaming prose: none" for codex), so
/// registering `MessageDisplay` there would install a hook for an event
/// codex never fires — harmless, since an unfired hook simply never runs,
/// but untrue, and it would make `CLAUDE_CODEX_EVENTS`'s claim that claude
/// and codex are "both registered against" every entry in it a small lie.
/// `claude_settings` unions this with `CLAUDE_CODEX_EVENTS`; `merge_codex`
/// deliberately does not.
const CLAUDE_ONLY_EVENTS: &[(&str, bool)] = &[("MessageDisplay", false)];

/// Cursor's own vocabulary for the same three moments, camelCase. Its tool
/// gates — `beforeShellExecution` and `beforeMCPExecution`, where claude and
/// codex both say `PreToolUse` — are cut for the same reason and by the same
/// fix round as `PermissionRequest` above; Task 11 re-adds both alongside it.
const CURSOR_EVENTS: &[(&str, bool)] = &[
    ("sessionStart", false),
    ("beforeSubmitPrompt", false),
    ("stop", false),
];

/// The project-local files Far Cooler writes into a worktree, relative to its
/// root — by `service::install_project_hooks` when it makes one, and by
/// `service::Service::prepare_launch_hooks` when a codex or cursor pane is
/// opened in one it did not make.
///
/// One list, read by three things that must agree: the installer that writes
/// them, `git::is_dirty`, and `change_set::working_tree`. Far Cooler wrote
/// these files, so Far Cooler must not then report them to the user as work —
/// a fresh worktree that opens with two files in its diff view, and a
/// just-created workspace that demands a typed confirmation to remove, are the
/// same bug read through two different signals. A second hand-kept copy of
/// these two strings is what would let the installer and the filters drift.
pub const CODEX_HOOKS: &str = ".codex/hooks.json";
pub const CURSOR_HOOKS: &str = ".cursor/hooks.json";
pub const PROJECT_HOOK_FILES: &[&str] = &[CODEX_HOOKS, CURSOR_HOOKS];

/// Take Far Cooler's own untracked hooks files out of a `git status` answer.
///
/// Only the untracked ones, the `?` records. A hooks file the repository
/// tracks is never one Far Cooler wrote (the installer refuses to; see
/// `service::install_project_hook_file`), so any change to it is somebody's
/// work: an edit, a staged deletion, a `git rm --cached`. It has to show in the
/// diff view, and it has to make `removal_needs_confirmation` ask before
/// `git worktree remove --force` throws it away. Subtracting these paths by
/// pathspec hid all of that.
///
/// Read from the same `git status` as the rest of the answer, so there is no
/// second git process and no second snapshot to disagree with the first. That
/// status must be `--untracked-files=all`: in git's default mode a wholly
/// untracked directory is one record (`? .codex/`), never
/// `? .codex/hooks.json`, and a filter on the file's path would match nothing
/// at all. With a file of the user's own beside ours, their file is its own
/// record and is not hidden.
///
/// Per worktree by construction, since tracking is: one branch can commit
/// `.codex/hooks.json` while its sibling doesn't. That is also why this is not
/// a line in `info/exclude`, which every checkout of the repository shares,
/// and which once hid an owner's own untracked hooks file in their main
/// checkout from their commits (31947322, reverted by 605f1815).
///
/// Hidden from the diff view is not the same as safe to delete: an untracked
/// hooks file can hold the owner's own entries, merged with ours or not.
/// `service::holds_an_unseen_hook_file` asks about those before removal.
///
/// Not the manager skill's files (`skill_install::PROJECT_SKILL_FILES`). Those
/// are hidden by git itself, through a line in the repository's
/// `info/exclude` (`service::exclude_locally`), which also keeps `git add -A`
/// from committing them.
pub fn hide_our_untracked(tree: &mut crate::change_set::WorkingTree) {
    tree.untracked.retain(|f| !PROJECT_HOOK_FILES.contains(&f.path.as_str()));
}

/// Whether a hooks file holds nothing but our registrations: a JSON object
/// whose keys are `hooks` (and cursor's numeric `version`), every value under
/// `hooks` a list of entries that are exactly what we write (`entry_is_ours`).
/// That is the only kind of hooks file removing a worktree can lose without
/// asking, because nobody else put anything in it. Anything else is
/// somebody's: their own entry, a key of their own on the file or on one of
/// our entries, a command of theirs inside our entry or appended to ours,
/// text that isn't a JSON object, or another daemon's registration.
///
/// `socket` is this runner's hook socket, which is how an entry of ours
/// written under an older CLI path is still recognized as ours (`Us`).
pub fn holds_only_ours(text: &str, socket: &Path) -> bool {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else { return false };
    let Some(Value::Object(hooks)) = root.get("hooks") else { return false };
    let us = Us::here(socket);
    root.iter().all(|(k, v)| k == "hooks" || (k == "version" && v.is_number()))
        && hooks.values().all(|list| list.as_array().is_some_and(|l| l.iter().all(|e| entry_is_ours(e, &us))))
}

/// The path this binary was launched as, resolved the same way the shim's
/// own command line is (`service::shim_binary`), so a hooks file and a pane
/// command never disagree about which CLI they mean.
fn binary_path() -> String {
    crate::service::shim_binary(std::env::current_exe().ok().as_deref())
}

/// Who "we" are, for telling an entry of ours from anybody else's.
///
/// A command of ours names two things: the CLI it runs and the socket it
/// reports to. The socket is `<runtime dir>/h.sock`
/// (`hook_ingress::HookIngress::socket_path`), fixed for as long as this
/// runner's home is, and no other daemon reports to it. The CLI path is not
/// stable: the app moves from `~/Downloads` to `/Applications`, a dev build
/// comes before the release. So a command is ours when it names this CLI at
/// its current path, or names this runner's socket through a CLI of ours at
/// any path. Another channel's daemon (its own home, its own socket, its own
/// CLI name) is somebody else, and the two installs sit side by side.
struct Us {
    binary: String,
    socket: String,
}

impl Us {
    fn here(socket: &Path) -> Self {
        Us { binary: binary_path(), socket: socket.display().to_string() }
    }

    fn wrote(&self, command: &OurCommand) -> bool {
        command.binary == self.binary || (command.socket == self.socket && is_a_far_cooler_cli(&command.binary))
    }
}

/// Every CLI name a channel installs or cargo builds begins with this
/// (`farcooler_protocol::Channel::cli_binary_candidates`).
fn is_a_far_cooler_cli(binary: &str) -> bool {
    Path::new(binary).file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("farcooler"))
}

/// A command line in the one grammar `hook_command` writes, taken apart.
struct OurCommand {
    binary: String,
    socket: String,
    /// The command re-rendered from its parts. Parsing is lenient and this
    /// is what makes it exact: a command is ours only if it is byte for byte
    /// what we would write for those parts.
    rendered: String,
}

/// Read a command of our grammar from the start of `command`, whatever
/// follows it. `None` for anything that doesn't begin with one.
fn parse_our_command(command: &str) -> Option<OurCommand> {
    let (binary, rest) = unquote_word(command)?;
    let rest = rest.strip_prefix(" hook --agent ")?;
    let (agent, rest) = rest.split_once(" --event ")?;
    let (event, rest) = rest.split_once(" --socket ")?;
    if !["claude", "codex", "cursor"].contains(&agent)
        || event.is_empty()
        || !event.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let (socket, rest) = unquote_word(rest)?;
    let gating = rest.starts_with(" --gating");
    let rendered = render_command(&binary, agent, event, &socket, gating);
    Some(OurCommand { binary, socket, rendered })
}

/// One word as `service::shell_quote` writes it, from the start of `s`:
/// its value, and what follows it.
fn unquote_word(s: &str) -> Option<(String, &str)> {
    let mut rest = s.strip_prefix('\'')?;
    let mut value = String::new();
    loop {
        let end = rest.find('\'')?;
        value.push_str(&rest[..end]);
        rest = &rest[end + 1..];
        if let Some(r) = rest.strip_prefix(r"\''") {
            value.push('\'');
            rest = r;
        } else if let Some(r) = rest.strip_prefix(r"\\'") {
            value.push('\\');
            rest = r;
        } else {
            return Some((value, rest));
        }
    }
}

/// Whether `command` is, exactly and in full, one we wrote.
fn is_our_command(command: &str, us: &Us) -> bool {
    parse_our_command(command).is_some_and(|c| c.rendered == command && us.wrote(&c))
}

/// Whether an entry is exactly what we write, in either shape, and nothing
/// more: codex's `{"hooks": [{"type": "command", "command": <ours>}]}` with
/// one inner hook, or cursor's flat `{"command": <ours>}`. A key we never
/// write, a second inner hook, or a command that only begins with ours makes
/// the entry somebody's, and a merge and `holds_only_ours` both leave it be.
fn entry_is_ours(entry: &Value, us: &Us) -> bool {
    let Some(entry) = entry.as_object() else { return false };
    if entry.len() != 1 {
        return false;
    }
    if let Some(Value::String(command)) = entry.get("command") {
        return is_our_command(command, us);
    }
    let Some(Value::Array(inner)) = entry.get("hooks") else { return false };
    let [Value::Object(hook)] = inner.as_slice() else { return false };
    hook.len() == 2
        && hook.get("type").and_then(Value::as_str) == Some("command")
        && hook.get("command").and_then(Value::as_str).is_some_and(|c| is_our_command(c, us))
}

/// Whether somebody's entry already runs `ours`, exactly this command, as
/// one of its commands or at the start of one (`<ours> && say done`). Then
/// the event already reports to us, and adding our own entry beside it would
/// run the hook twice: every prompt and every answer drawn twice in chat.
fn entry_runs(entry: &Value, ours: &str) -> bool {
    let runs = |c: &Value| c.as_str().and_then(parse_our_command).is_some_and(|c| c.rendered == ours);
    entry.get("command").is_some_and(runs)
        || entry.get("hooks").and_then(Value::as_array).is_some_and(|inner| {
            inner.iter().any(|h| h.get("command").is_some_and(runs))
        })
}

/// The full shell command line for one hook registration.
///
/// The binary path is quoted (`shell_quote`) because it is a path Far Cooler
/// did not choose and may contain spaces; `hook`, the flag names and the
/// event name are not, matching the shape already on this machine — e.g.
/// `bash '/Users/e-liang/.codex/herdr-agent-state.sh' session`.
fn hook_command(agent: &str, event: &str, socket: &Path, gating: bool) -> String {
    render_command(&binary_path(), agent, event, &socket.display().to_string(), gating)
}

/// `hook_command`'s grammar, from its parts. The one place it is written, so
/// `parse_our_command` can't drift from it.
fn render_command(binary: &str, agent: &str, event: &str, socket: &str, gating: bool) -> String {
    let mut command = format!(
        "{} hook --agent {agent} --event {event} --socket {}",
        crate::service::shell_quote(binary),
        crate::service::shell_quote(socket),
    );
    if gating {
        command.push_str(" --gating");
    }
    command
}

/// claude's settings object, for `--settings <file>` in Task 10.
///
/// Never merged with anything: claude never has a user file touched at all,
/// so there is nothing to preserve and every call with the same socket
/// produces the same value. Registers `CLAUDE_CODEX_EVENTS` (shared with
/// codex) union `CLAUDE_ONLY_EVENTS` (`MessageDisplay`, claude's alone) —
/// see that constant's doc for why the two are not one table.
pub fn claude_settings(socket: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, gating) in CLAUDE_CODEX_EVENTS.iter().chain(CLAUDE_ONLY_EVENTS) {
        hooks.insert(
            (*event).to_string(),
            json!([{
                "hooks": [
                    { "type": "command", "command": hook_command("claude", event, socket, *gating) }
                ]
            }]),
        );
    }
    json!({ "hooks": Value::Object(hooks) })
}

/// Merge our registrations into `existing`, a `.codex/hooks.json` this
/// machine's other tooling may already have entries in. Every event we
/// don't touch, and every entry under an event we do touch that is not
/// exactly ours (`entry_is_ours`), survives unchanged.
pub fn merge_codex(existing: &str, socket: &Path) -> String {
    merge(existing, socket, "codex", CLAUDE_CODEX_EVENTS, |command| {
        json!({ "hooks": [ { "type": "command", "command": command } ] })
    })
}

/// Merge our registrations into `existing`, a `.cursor/hooks.json`. Cursor's
/// shape is flatter than the other two — no inner `hooks` array — and its
/// event names are its own; both are captured in `CURSOR_EVENTS` and the
/// entry shape below, and nothing else about the file (its `version` key
/// included) is touched.
pub fn merge_cursor(existing: &str, socket: &Path) -> String {
    merge(existing, socket, "cursor", CURSOR_EVENTS, |command| json!({ "command": command }))
}

/// `merge_codex` and `merge_cursor`, which differ only in their events and
/// in the shape of one entry.
///
/// A `hooks` that isn't an object, or an event whose value isn't a list, is
/// somebody's arrangement we can't add to without replacing it, so it is
/// left as it is and we go without that registration.
fn merge(
    existing: &str,
    socket: &Path,
    agent: &str,
    events: &[(&str, bool)],
    entry: impl Fn(String) -> Value,
) -> String {
    let mut root = parse_or_empty_object(existing);
    let Some(hooks_obj) = hooks_object_mut(&mut root) else {
        tracing::info!(agent, "a hooks file's `hooks` is not an object; leaving it alone");
        return existing.to_string();
    };
    let us = Us::here(socket);
    for (event, gating) in events {
        let command = hook_command(agent, event, socket, *gating);
        merge_event(hooks_obj, event, &command, entry(command.clone()), &us);
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| existing.to_string())
}

/// Strip only the entries that are exactly ours (`entry_is_ours`), leaving
/// every other event and every other entry exactly as it was. Works on
/// either file's shape, since `entry_is_ours` reads whichever of the two it
/// finds.
pub fn remove_ours(existing: &str, socket: &Path) -> String {
    let Ok(mut root) = serde_json::from_str::<Value>(existing) else {
        return existing.to_string();
    };
    let us = Us::here(socket);
    if let Some(hooks_obj) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for arr in hooks_obj.values_mut() {
            let Some(list) = arr.as_array() else { continue };
            let kept: Vec<Value> = list.iter().filter(|entry| !entry_is_ours(entry, &us)).cloned().collect();
            *arr = Value::Array(kept);
        }
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| existing.to_string())
}

/// `existing`, parsed as a JSON object — or an empty one, for a file that is
/// missing, empty, or (should it somehow happen) not an object at all. A
/// file we cannot make sense of is treated as one with nothing in it yet,
/// never as a reason to give up and leave our hooks uninstalled.
/// (`service::install_project_hook_file` refuses to write over a present
/// file that isn't an object, so that case never reaches the disk.)
fn parse_or_empty_object(existing: &str) -> Value {
    match serde_json::from_str::<Value>(existing) {
        Ok(v) if v.is_object() => v,
        _ => json!({}),
    }
}

/// `root["hooks"]` as an object, created if it is missing. `None` when it is
/// there and is something else, which is somebody's and not ours to replace.
fn hooks_object_mut(root: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    let root_obj = root.as_object_mut().expect("parse_or_empty_object guarantees an object");
    root_obj.entry("hooks").or_insert_with(|| json!({})).as_object_mut()
}

/// Bring `event` up to date: drop the entries that are exactly ours, keep
/// every other entry as it is and where it is, and add our fresh one at the
/// end. Dropping our old entry first, rather than only appending, is what
/// makes a second merge a no-op instead of a second copy, and what replaces
/// an entry of ours that names an older CLI path.
///
/// Unless somebody's entry already runs `command`, exactly this one: an
/// owner who put a command of theirs inside our entry, or after ours on its
/// command line. That entry isn't ours to take apart, and it already reports
/// to us, so a second entry of ours would fire the hook twice (every prompt
/// and answer drawn twice in chat). Then we add nothing.
///
/// An event whose value isn't a list is left alone, for `merge`'s reason.
fn merge_event(
    hooks_obj: &mut serde_json::Map<String, Value>,
    event: &str,
    command: &str,
    new_entry: Value,
    us: &Us,
) {
    let existing_arr = match hooks_obj.get(event) {
        None => Vec::new(),
        Some(Value::Array(list)) => list.clone(),
        Some(_) => {
            tracing::info!(event, "a hooks file's event is not a list; leaving it alone");
            return;
        }
    };
    let mut kept: Vec<Value> = existing_arr.into_iter().filter(|e| !entry_is_ours(e, us)).collect();
    if kept.iter().any(|e| entry_runs(e, command)) {
        tracing::info!(event, "somebody's hook entry already runs ours; leaving it as they arranged it");
    } else {
        kept.push(new_entry);
    }
    hooks_obj.insert(event.to_string(), Value::Array(kept));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How every command of ours at this CLI path begins.
    fn ours_prefix() -> String {
        format!("{} hook --agent ", crate::service::shell_quote(&binary_path()))
    }

    const SOMEONE_ELSES: &str = r#"{
      "hooks": {
        "SessionStart": [
          { "hooks": [ { "type": "command", "command": "bash /Users/x/.codex/herdr-agent-state.sh session" } ] }
        ]
      }
    }"#;

    /// The bug this pins is a support incident, not a test failure: replacing
    /// this file silently disables somebody else's tooling, and they find out
    /// days later.
    #[test]
    fn merging_keeps_every_hook_that_was_already_there() {
        let merged = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        assert!(
            merged.contains("herdr-agent-state.sh"),
            "another tool's hook survives ours being added"
        );
        assert!(merged.contains("farcooler"), "and ours is there too");
    }

    #[test]
    fn merging_twice_installs_one_copy() {
        let once = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_codex(&once, Path::new("/tmp/h.sock"));
        assert_eq!(once, twice, "installing is idempotent");
    }

    /// The real case a daemon restart produces: the socket path rotates, so
    /// the second merge is never byte-identical to the first. An
    /// implementation that dedups by whole-entry equality (rather than by
    /// the `entry_is_ours` marker) would pass `merging_twice_installs_one_copy`
    /// above — same socket both times, so the two entries ARE equal — and
    /// still duplicate here, where they are not.
    #[test]
    fn merging_again_with_a_new_socket_replaces_the_stale_entry() {
        let once = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_codex(&once, Path::new("/tmp/other.sock"));
        let v: Value = serde_json::from_str(&twice).expect("json");
        let arr = v["hooks"]["SessionStart"].as_array().expect("SessionStart is an array");
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e, &Us::here(Path::new("/tmp/other.sock")))).collect();
        assert_eq!(ours.len(), 1, "a re-merge with a new socket replaces, not accompanies: {arr:?}");
        let command = ours[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(command.contains("other.sock"), "the surviving entry names the new socket: {command}");
        assert!(!command.contains("/tmp/h.sock"), "the stale socket is gone: {command}");
    }

    #[test]
    fn removing_ours_leaves_theirs_alone() {
        let merged = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let cleaned = remove_ours(&merged, Path::new("/tmp/h.sock"));
        assert!(cleaned.contains("herdr-agent-state.sh"));
        assert!(!cleaned.contains("farcooler"));
    }

    /// A command that merely MENTIONS our prefix somewhere other than the
    /// front — the way a wrapper script's log line or comment might quote
    /// it — must not be mistaken for ours. `parse_our_command` reads from the
    /// front of the command, never searches inside it.
    #[test]
    fn a_command_that_only_mentions_our_prefix_midstream_is_not_ours() {
        let foreign_command =
            format!("echo not-us && true # mentions {} elsewhere", ours_prefix());
        let existing = json!({
            "hooks": {
                "SessionStart": [
                    { "hooks": [ { "type": "command", "command": foreign_command } ] }
                ]
            }
        })
        .to_string();

        let merged = merge_codex(&existing, Path::new("/tmp/h.sock"));
        assert!(
            merged.contains("echo not-us"),
            "a command that only mentions our prefix midstream must survive merging: {merged}"
        );

        let cleaned = remove_ours(&merged, Path::new("/tmp/h.sock"));
        assert!(
            cleaned.contains("echo not-us"),
            "and remove_ours must not treat it as ours either: {cleaned}"
        );
    }

    /// Cursor's shape is flatter than the other two: no inner `hooks` array,
    /// and camelCase event names.
    #[test]
    fn cursor_gets_cursors_shape_not_claudes() {
        let merged = merge_cursor("{\"version\":1,\"hooks\":{}}", Path::new("/tmp/h.sock"));
        let v: serde_json::Value = serde_json::from_str(&merged).expect("json");
        assert!(v["hooks"]["beforeSubmitPrompt"][0]["command"].is_string());
        assert!(
            v["hooks"]["beforeSubmitPrompt"][0]["hooks"].is_null(),
            "cursor has no nested hooks array"
        );
    }

    /// `CURSORS_ELSES`, cursor's own flat shape holding somebody else's real
    /// hooks — six entries across six event keys, two of them under
    /// `sessionStart` alone, matching how `~/.cursor/hooks.json` on this
    /// machine actually reads (fix round 2: the single-entry, single-event
    /// version this fixture used to be could not tell "merge" from
    /// "wholesale replace" or "keep only the first entry" on a real,
    /// multi-entry array — `merge_event` is genuinely element-wise, but
    /// nothing here had proven it).
    const CURSORS_ELSES: &str = r#"{
      "version": 1,
      "hooks": {
        "sessionStart": [
          { "command": "bash /Users/x/.cursor/herdr-agent-state.sh session" },
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh SessionStart" }
        ],
        "sessionEnd": [
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh SessionEnd" }
        ],
        "beforeSubmitPrompt": [
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh Start" }
        ],
        "stop": [
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh Stop" }
        ],
        "beforeShellExecution": [
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh PermissionRequest" }
        ],
        "beforeMCPExecution": [
          { "command": "/Users/x/.superset/hooks/cursor-hook.sh PermissionRequest" }
        ]
      }
    }"#;

    /// The CRITICAL gap fix round 1 found: every one of the three
    /// properties this task exists to guarantee (merge preserves others,
    /// merge is idempotent, remove strips only ours) was exercised only
    /// through codex's nested shape. `entry_is_ours`'s flat-shape branch
    /// (the `entry.get("command")` path, reached only by cursor) was only
    /// ever proven to return `false` — never proven to return `true` on a
    /// real cursor entry. These three give cursor the same coverage codex
    /// already had, against a fixture with two entries under one event and
    /// entries under events we never touch at all (fix round 2).
    #[test]
    fn cursor_merging_keeps_every_hook_that_was_already_there() {
        let merged = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).expect("json");

        // Events CURSOR_EVENTS does not touch: exactly the one foreign
        // entry each already had, nothing of ours.
        assert_eq!(v["hooks"]["sessionEnd"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["beforeShellExecution"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["beforeMCPExecution"].as_array().unwrap().len(), 1);

        // sessionStart: both foreign entries survive, plus ours -- the
        // element-wise-ness a single-entry fixture could never have shown.
        let session_start = v["hooks"]["sessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 3, "two foreign entries plus ours: {session_start:?}");

        for needle in [
            "herdr-agent-state.sh",
            "cursor-hook.sh SessionStart",
            "cursor-hook.sh SessionEnd",
            "cursor-hook.sh Start",
            "cursor-hook.sh Stop",
            "cursor-hook.sh PermissionRequest",
        ] {
            assert!(merged.contains(needle), "another tool's hook ({needle}) survives ours being added");
        }
        assert!(merged.contains("farcooler"), "and ours is there too");
    }

    #[test]
    fn cursor_merging_twice_installs_one_copy() {
        let once = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_cursor(&once, Path::new("/tmp/h.sock"));
        assert_eq!(once, twice, "installing is idempotent");
    }

    #[test]
    fn cursor_merging_again_with_a_new_socket_replaces_the_stale_entry() {
        let once = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_cursor(&once, Path::new("/tmp/other.sock"));
        let v: Value = serde_json::from_str(&twice).expect("json");
        let arr = v["hooks"]["sessionStart"].as_array().expect("sessionStart is an array");
        assert_eq!(arr.len(), 3, "both foreign entries plus exactly one of ours: {arr:?}");
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e, &Us::here(Path::new("/tmp/other.sock")))).collect();
        assert_eq!(ours.len(), 1, "a re-merge with a new socket replaces, not accompanies: {arr:?}");
        let command = ours[0]["command"].as_str().unwrap();
        assert!(command.contains("other.sock"), "the surviving entry names the new socket: {command}");
        assert!(!command.contains("/tmp/h.sock"), "the stale socket is gone: {command}");
    }

    #[test]
    fn cursor_removing_ours_leaves_theirs_alone() {
        let merged = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let cleaned = remove_ours(&merged, Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&cleaned).expect("json");
        let session_start = v["hooks"]["sessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 2, "both foreign sessionStart entries survive, ours is gone");

        for needle in [
            "herdr-agent-state.sh",
            "cursor-hook.sh SessionStart",
            "cursor-hook.sh SessionEnd",
            "cursor-hook.sh Start",
            "cursor-hook.sh Stop",
            "cursor-hook.sh PermissionRequest",
        ] {
            assert!(cleaned.contains(needle), "another tool's hook ({needle}) must not be touched by remove_ours");
        }
        assert!(!cleaned.contains("farcooler"));
    }

    /// Rule 1 from this plan's cost ledger: a test must assert what reached
    /// the shape that will be read back, not just that some field is
    /// present. Every event in the table is registered, with the right
    /// gate, using the right key — not merely "SOMETHING is under `hooks`".
    #[test]
    fn every_codex_event_lands_with_the_right_gate() {
        let merged = merge_codex("{}", Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).expect("json");
        for (event, gating) in CLAUDE_CODEX_EVENTS {
            let command =
                v["hooks"][event][0]["hooks"][0]["command"].as_str().unwrap_or_else(|| {
                    panic!("codex hooks.json carries a command for {event}")
                });
            assert!(command.starts_with(&ours_prefix()), "{event}: {command}");
            assert!(command.contains(&format!("--event {event}")), "{event}: {command}");
            assert_eq!(command.contains("--gating"), *gating, "{event}: {command}");
        }
    }

    #[test]
    fn every_cursor_event_lands_with_the_right_gate() {
        let merged = merge_cursor("{}", Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).expect("json");
        for (event, gating) in CURSOR_EVENTS {
            let command = v["hooks"][event][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("cursor hooks.json carries a command for {event}"));
            assert!(command.contains(&format!("--event {event}")), "{event}: {command}");
            assert_eq!(command.contains("--gating"), *gating, "{event}: {command}");
        }
    }

    /// No installed hook widens its own deadline.
    ///
    /// `farcooler hook --deadline-ms` exists for the CLI's own tests, and a
    /// hook that carried it would keep an agent waiting up to a minute on a
    /// daemon that has wedged. Every hook an agent runs must get the CLI's
    /// 400 ms `HOOK_DEADLINE`, so nothing written here may name the flag:
    /// `hook_command` in both gate states (no table sets `gating` today, and
    /// Task 11 will), and every command in all three agents' files.
    #[test]
    fn no_installed_hook_carries_a_deadline_of_its_own() {
        let socket = Path::new("/tmp/h.sock");
        let mut commands = vec![
            hook_command("claude", "PermissionRequest", socket, true),
            hook_command("claude", "Stop", socket, false),
        ];
        let settings = claude_settings(socket);
        let codex: Value = serde_json::from_str(&merge_codex("{}", socket)).expect("json");
        let cursor: Value = serde_json::from_str(&merge_cursor("{}", socket)).expect("json");
        for (event, _) in CLAUDE_CODEX_EVENTS.iter().chain(CLAUDE_ONLY_EVENTS) {
            commands.push(settings["hooks"][event][0]["hooks"][0]["command"].to_string());
        }
        for (event, _) in CLAUDE_CODEX_EVENTS {
            commands.push(codex["hooks"][event][0]["hooks"][0]["command"].to_string());
        }
        for (event, _) in CURSOR_EVENTS {
            commands.push(cursor["hooks"][event][0]["command"].to_string());
        }
        for command in &commands {
            assert!(command.contains(" hook --agent "), "not a hook command: {command}");
            assert!(!command.contains("--deadline"), "an installed hook set its own deadline: {command}");
        }
    }

    /// The shape `claude_settings` hands Task 10 for `--settings <file>`.
    /// Covers `CLAUDE_CODEX_EVENTS` union `CLAUDE_ONLY_EVENTS` — including
    /// `MessageDisplay`, fix round 2.
    ///
    /// This test checks that shape only. The other half of the "claude
    /// never has a file touched" promise — that no `merge_*`/`remove_ours`
    /// call in this crate is ever made on claude's behalf — is not
    /// encoded as an assertion here: it was checked by hand,
    /// `grep -rn "hook_install::" crates/daemon/src crates/cli/src
    /// crates/daemon/tests`, which today returns nothing at all because
    /// nothing calls this module yet (see the task report). That grep needs
    /// re-running once a caller exists.
    #[test]
    fn claude_settings_carries_every_event() {
        let settings = claude_settings(Path::new("/tmp/h.sock"));
        for (event, gating) in CLAUDE_CODEX_EVENTS.iter().chain(CLAUDE_ONLY_EVENTS) {
            let command = settings["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("claude settings carries a command for {event}"));
            assert!(command.contains(&format!("--agent claude --event {event}")), "{event}: {command}");
            assert_eq!(command.contains("--gating"), *gating, "{event}: {command}");
        }
    }

    /// codex has no streaming-prose hook at all (spec capability table:
    /// "streaming prose: none"), so `MessageDisplay` must never be
    /// registered for it — the deliberate reason `CLAUDE_ONLY_EVENTS` is a
    /// separate table from `CLAUDE_CODEX_EVENTS` rather than folded in.
    #[test]
    fn codex_never_gets_a_messagedisplay_hook() {
        let merged = merge_codex("{}", Path::new("/tmp/h.sock"));
        assert!(
            !merged.contains("MessageDisplay"),
            "codex has no event to receive this hook: {merged}"
        );
    }

    /// What `MessageAssembler::accept` (`crates/agent-hooks/src/assemble.rs`)
    /// matches on for each agent — the direction fix round 1's cut never
    /// checked: round 1 verified "every REGISTERED event is consumed by
    /// something" (its own whole point) and never checked "every event the
    /// assembler CONSUMES is registered", which is exactly how
    /// `MessageDisplay` went missing from claude's table without any test
    /// noticing, since an unregistered event fails silently: a well-formed
    /// hooks file with a missing key just never fires that hook.
    ///
    /// Fix round 2 closed that with a hand-copied local list and said so in
    /// its own doc comment: "keep it in sync with `assemble.rs::accept` by
    /// eye; a stale list here fails exactly as silently as an unregistered
    /// event does." Fix round 3 replaces the copy with the thing itself:
    /// `farcooler_agent_hooks::assemble::CONSUMED` lives beside `accept` in
    /// the crate that owns the match, and that crate's own
    /// `consumed_matches_accepts_own_match_arms` test reads `accept`'s
    /// source text and asserts `CONSUMED` matches it exactly, in both
    /// directions. This test below no longer has to trust a mirror; it
    /// imports the checked original.
    #[test]
    fn every_event_the_assembler_consumes_is_registered_for_that_agent() {
        let claude = claude_settings(Path::new("/tmp/h.sock"));
        let claude_keys: Vec<String> =
            claude["hooks"].as_object().expect("claude settings has a hooks object").keys().cloned().collect();

        let codex = merge_codex("{}", Path::new("/tmp/h.sock"));
        let codex_v: Value = serde_json::from_str(&codex).expect("json");
        let codex_keys: Vec<String> =
            codex_v["hooks"].as_object().expect("codex hooks.json has a hooks object").keys().cloned().collect();

        let cursor = merge_cursor("{}", Path::new("/tmp/h.sock"));
        let cursor_v: Value = serde_json::from_str(&cursor).expect("json");
        let cursor_keys: Vec<String> =
            cursor_v["hooks"].as_object().expect("cursor hooks.json has a hooks object").keys().cloned().collect();

        for (agent, event) in farcooler_agent_hooks::assemble::CONSUMED {
            let registered = match *agent {
                farcooler_agent_hooks::Agent::Claude => claude_keys.iter().any(|k| k == event),
                farcooler_agent_hooks::Agent::Codex => codex_keys.iter().any(|k| k == event),
                farcooler_agent_hooks::Agent::Cursor => cursor_keys.iter().any(|k| k == event),
            };
            assert!(
                registered,
                "assemble.rs consumes {}'s {event}, so this installer must register it — it does not",
                agent.as_str(),
            );
        }
    }

    /// A socket path with a space in it — the exact shape of bug
    /// `shell_quote` exists to prevent (a worktree under `~/My Projects`
    /// splits into two arguments unquoted otherwise) — must still produce a
    /// command whose `--socket` value round-trips as one token, not two.
    #[test]
    fn a_socket_path_with_a_space_stays_one_argument() {
        let merged = merge_codex("{}", Path::new("/tmp/My Projects/h.sock"));
        let v: Value = serde_json::from_str(&merged).expect("json");
        let command = v["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(
            command.contains("--socket '/tmp/My Projects/h.sock'"),
            "the space-bearing path is quoted as one word: {command}"
        );
    }

    /// What removal may lose without asking: a file holding our entries and
    /// nothing else, in either agent's shape.
    #[test]
    fn only_a_file_of_nothing_but_ours_is_ours() {
        let socket = Path::new("/tmp/h.sock");
        assert!(holds_only_ours(&merge_codex("{}", socket), socket));
        assert!(holds_only_ours(&merge_cursor("", socket), socket));
        assert!(holds_only_ours(&merge_cursor(r#"{"version":1}"#, socket), socket), "cursor's own version key");
        assert!(!holds_only_ours(&merge_codex(SOMEONE_ELSES, socket), socket), "somebody's entry beside ours");
        let mut extra: Value = serde_json::from_str(&merge_codex("{}", socket)).unwrap();
        extra["theirs"] = json!(true);
        assert!(!holds_only_ours(&extra.to_string(), socket), "a key of their own");
        assert!(!holds_only_ours(SOMEONE_ELSES, socket), "nothing of ours at all");
        assert!(!holds_only_ours("{ not json", socket), "text we can't read");
        assert!(!holds_only_ours("[]", socket), "not an object");
    }

    /// m1's shapes, which removal used to lose without asking: each is our
    /// file with one thing of the owner's inside one of our entries.
    #[test]
    fn a_file_with_somebodys_content_inside_our_entry_is_not_only_ours() {
        let socket = Path::new("/tmp/h.sock");
        let sibling = codex_with_our_stop_entry_edited(|entry| {
            entry["hooks"].as_array_mut().unwrap().push(json!({ "type": "command", "command": "say done" }));
        });
        assert!(!holds_only_ours(&sibling, socket), "a command of theirs in our inner array");
        let key = codex_with_our_stop_entry_edited(|entry| entry["matcher"] = json!("*"));
        assert!(!holds_only_ours(&key, socket), "a key of theirs on our entry");
        let inner_key = codex_with_our_stop_entry_edited(|entry| entry["hooks"][0]["timeout"] = json!(5));
        assert!(!holds_only_ours(&inner_key, socket), "a key of theirs on our inner hook");
        let appended = codex_with_our_stop_entry_edited(|entry| {
            let ours = entry["hooks"][0]["command"].as_str().unwrap().to_string();
            entry["hooks"][0]["command"] = json!(format!("{ours} && say done"));
        });
        assert!(!holds_only_ours(&appended, socket), "a command line that goes on after ours");
        let mut cursor: Value = serde_json::from_str(&merge_cursor("{}", socket)).unwrap();
        cursor["hooks"]["stop"][0]["timeout"] = json!(5);
        assert!(!holds_only_ours(&cursor.to_string(), socket), "a key of theirs on cursor's flat entry");
        let mut version: Value = serde_json::from_str(&merge_cursor("{}", socket)).unwrap();
        version["version"] = json!({ "theirs": true });
        assert!(!holds_only_ours(&version.to_string(), socket), "a `version` that is not a number");
    }

    /// m3 for removal: our entry under an older CLI path, naming this
    /// runner's socket, is still ours, so removal doesn't ask about it. The
    /// same entry naming another socket is another daemon's.
    #[test]
    fn our_entry_under_an_old_cli_path_is_still_only_ours() {
        let at = |bin: &str, socket: &str| {
            let command = render_command(bin, "codex", "Stop", socket, false);
            json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": command } ] } ] } }).to_string()
        };
        let socket = Path::new("/tmp/h.sock");
        assert!(holds_only_ours(&at("/Applications/Far Cooler.app/Contents/MacOS/farcooler", "/tmp/h.sock"), socket));
        assert!(holds_only_ours(&at("/Users/x/Dev/overnight/target/debug/farcooler", "/tmp/h.sock"), socket));
        assert!(!holds_only_ours(&at("/usr/local/bin/other-tool", "/tmp/h.sock"), socket), "not a CLI of ours");
        assert!(!holds_only_ours(&at("/Users/x/.local/bin/farcooler-preview", "/tmp/p.sock"), socket), "another daemon's");
    }

    /// What we write, we read back as ours, whatever the paths hold: a
    /// space, a quote, a backslash. And nothing but what we write.
    #[test]
    fn a_command_we_render_is_the_only_command_we_parse_as_ours() {
        let us = Us { binary: "/x/it's a \\ farcooler".into(), socket: "/r/h.sock".into() };
        for (binary, socket) in [("/x/it's a \\ farcooler", "/tmp/h.sock"), ("/y/farcooler", "/r/h.sock")] {
            let command = render_command(binary, "cursor", "stop", socket, true);
            assert!(is_our_command(&command, &us), "{command}");
            assert!(!is_our_command(&format!("{command} "), &us), "a trailing space: {command}");
            assert!(!is_our_command(&command.replace(" hook ", "  hook "), &us), "a double space: {command}");
            assert!(!is_our_command(&command.replace("cursor", "opencode"), &us), "an agent we never register");
        }
    }

    /// Somebody's `hooks` that isn't an object, or an event of theirs that
    /// isn't a list, is theirs; a merge adds nothing rather than replace it.
    #[test]
    fn a_hooks_value_of_somebodys_shape_is_left_as_it_is() {
        let socket = Path::new("/tmp/h.sock");
        let odd = r#"{"hooks":["theirs"]}"#;
        assert_eq!(merge_codex(odd, socket), odd, "a `hooks` that is a list");
        let merged = merge_codex(r#"{"hooks":{"Stop":{"theirs":true}}}"#, socket);
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["hooks"]["Stop"], json!({ "theirs": true }), "an event that is an object: {merged}");
        assert!(v["hooks"]["SessionStart"].is_array(), "and every other event still gets ours: {merged}");
    }

    /// Every command string under `event`, in either shape, in file order.
    fn commands_under(text: &str, event: &str) -> Vec<String> {
        let v: Value = serde_json::from_str(text).expect("json");
        let mut out = Vec::new();
        for entry in v["hooks"][event].as_array().cloned().unwrap_or_default() {
            if let Some(c) = entry["command"].as_str() {
                out.push(c.to_string());
            }
            for inner in entry["hooks"].as_array().cloned().unwrap_or_default() {
                if let Some(c) = inner["command"].as_str() {
                    out.push(c.to_string());
                }
            }
        }
        out
    }

    /// Our codex file, with `edit` applied to the one entry under `Stop`.
    fn codex_with_our_stop_entry_edited(edit: impl FnOnce(&mut Value)) -> String {
        let mut v: Value = serde_json::from_str(&merge_codex("{}", Path::new("/tmp/h.sock"))).unwrap();
        edit(&mut v["hooks"]["Stop"][0]);
        v.to_string()
    }

    /// m1, the shape that deleted an owner's command: they added one of
    /// theirs to the inner `hooks` array of our `Stop` entry, and the next
    /// codex launch's merge counted the whole entry as ours and replaced it.
    /// Their command has to survive, and ours must not then run twice for
    /// the event (every prompt and answer would be drawn twice in chat).
    #[test]
    fn an_owners_command_inside_our_entry_survives_a_merge() {
        let existing = codex_with_our_stop_entry_edited(|entry| {
            entry["hooks"].as_array_mut().unwrap().push(json!({ "type": "command", "command": "say done" }));
        });
        let merged = merge_codex(&existing, Path::new("/tmp/h.sock"));
        let stop = commands_under(&merged, "Stop");
        assert!(stop.iter().any(|c| c == "say done"), "the owner's command survives: {merged}");
        let ours = stop.iter().filter(|c| c.starts_with(&ours_prefix())).count();
        assert_eq!(ours, 1, "and ours runs once for the event, not twice: {stop:?}");
        let before: Value = serde_json::from_str(&existing).unwrap();
        let after: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(after["hooks"]["Stop"], before["hooks"]["Stop"], "their entry is not altered at all");
        assert_eq!(merged, merge_codex(&merged, Path::new("/tmp/h.sock")), "and a second merge changes nothing");
    }

    /// m1: a key we never write on an entry (`timeout` here) makes it
    /// somebody's arrangement, not ours, and a merge must keep it.
    #[test]
    fn an_entry_of_ours_with_a_key_of_theirs_survives_a_merge() {
        let existing = codex_with_our_stop_entry_edited(|entry| entry["timeout"] = json!(5));
        let merged = merge_codex(&existing, Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).unwrap();
        let kept = v["hooks"]["Stop"].as_array().unwrap().iter().any(|e| e["timeout"] == json!(5));
        assert!(kept, "the entry carrying their key is still there: {merged}");

        let existing = codex_with_our_stop_entry_edited(|entry| entry["hooks"][0]["timeout"] = json!(5));
        let merged = merge_codex(&existing, Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).unwrap();
        let kept = v["hooks"]["Stop"].as_array().unwrap().iter().any(|e| e["hooks"][0]["timeout"] == json!(5));
        assert!(kept, "a key of theirs on the inner hook too: {merged}");

        let mut cursor: Value = serde_json::from_str(&merge_cursor("{}", Path::new("/tmp/h.sock"))).unwrap();
        cursor["hooks"]["stop"][0]["timeout"] = json!(5);
        let merged = merge_cursor(&cursor.to_string(), Path::new("/tmp/h.sock"));
        let v: Value = serde_json::from_str(&merged).unwrap();
        let kept = v["hooks"]["stop"].as_array().unwrap().iter().any(|e| e["timeout"] == json!(5));
        assert!(kept, "and on cursor's flat entry: {merged}");
    }

    /// m1: a command that starts with ours and goes on (`<ours> && say
    /// done`) is the owner's command line, not ours.
    #[test]
    fn a_command_that_only_starts_with_ours_survives_a_merge() {
        let existing = codex_with_our_stop_entry_edited(|entry| {
            let ours = entry["hooks"][0]["command"].as_str().unwrap().to_string();
            entry["hooks"][0]["command"] = json!(format!("{ours} && say done"));
        });
        let merged = merge_codex(&existing, Path::new("/tmp/h.sock"));
        let stop = commands_under(&merged, "Stop");
        assert!(stop.iter().any(|c| c.ends_with(" && say done")), "their command line survives: {merged}");
        assert_eq!(stop.len(), 1, "and ours isn't added beside it to run twice: {stop:?}");

        let mut cursor: Value = serde_json::from_str(&merge_cursor("{}", Path::new("/tmp/h.sock"))).unwrap();
        let ours = cursor["hooks"]["stop"][0]["command"].as_str().unwrap().to_string();
        cursor["hooks"]["stop"][0]["command"] = json!(format!("{ours}; say done"));
        let merged = merge_cursor(&cursor.to_string(), Path::new("/tmp/h.sock"));
        assert!(
            commands_under(&merged, "stop").iter().any(|c| c.ends_with("; say done")),
            "and on cursor's flat entry: {merged}"
        );
    }

    /// m3: an entry we wrote under another path to the CLI (the app moved,
    /// or a dev build came before the release) still names this runner's
    /// socket, and it is still ours: a merge replaces it rather than leaving
    /// it to fire into a binary that is gone.
    #[test]
    fn an_entry_of_ours_under_an_old_cli_path_is_replaced() {
        let old = "'/Users/x/Downloads/Far Cooler.app/Contents/MacOS/farcooler' hook --agent codex --event Stop --socket '/tmp/h.sock'";
        let existing = json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": old } ] } ] } });
        let merged = merge_codex(&existing.to_string(), Path::new("/tmp/h.sock"));
        let stop = commands_under(&merged, "Stop");
        assert_eq!(stop.len(), 1, "the old entry is replaced, not accompanied: {stop:?}");
        assert!(stop[0].starts_with(&ours_prefix()), "by one at this path: {stop:?}");
    }

    /// The other side of m3: another daemon's entry (another channel, with a
    /// socket of its own, at a path of its own) is not ours, and the two
    /// installs live side by side.
    #[test]
    fn another_daemons_entry_is_left_beside_ours() {
        let theirs = "'/Users/x/.local/bin/farcooler-preview' hook --agent codex --event Stop --socket '/Users/x/.farcooler-preview/h.sock'";
        let existing = json!({ "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": theirs } ] } ] } });
        let merged = merge_codex(&existing.to_string(), Path::new("/tmp/h.sock"));
        let stop = commands_under(&merged, "Stop");
        assert_eq!(stop.len(), 2, "theirs and ours: {stop:?}");
        assert_eq!(stop[0], theirs, "theirs untouched");
    }
}
