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
//! we don't write, and one command in our grammar naming this runner's
//! socket through a CLI of ours (`entry_is_ours`, `Us`). We merge, and we replace or
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
/// So this returns what it hid, and `service::holds_an_unseen_hook_file`
/// looks inside those files before removal.
///
/// Not the manager skill's files (`skill_install::PROJECT_SKILL_FILES`). Those
/// are hidden by git itself, through a line in the repository's
/// `info/exclude` (`service::exclude_locally`), which also keeps `git add -A`
/// from committing them.
pub fn hide_our_untracked(tree: &mut crate::change_set::WorkingTree) -> Vec<crate::change_set::FileChange> {
    let (hidden, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut tree.untracked)
        .into_iter()
        .partition(|f| PROJECT_HOOK_FILES.contains(&f.path.as_str()));
    tree.untracked = kept;
    hidden
}

/// A hooks file's text as JSON, or `None` for text that isn't JSON or that
/// names one key twice in one object, at any depth.
///
/// serde_json keeps the last of two equal keys and says nothing. So a `Stop`
/// block the owner pasted above ours would vanish from a merge's rewrite, and
/// `{"hooks": <theirs>, "hooks": <ours>}` would read as nothing but ours to the
/// removal check. Every reader here goes through this instead: the installer
/// leaves such a file exactly as it is, the merge adds nothing to it, and
/// removal asks about it.
pub fn parse_hooks_file(text: &str) -> Option<Value> {
    serde_json::from_str::<NoKeyTwice>(text).ok().map(|NoKeyTwice(v)| v)
}

/// A `Value` that failed to deserialize if any object in it has a key twice.
struct NoKeyTwice(Value);

impl<'de> serde::Deserialize<'de> for NoKeyTwice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(NoKeyTwiceVisitor).map(NoKeyTwice)
    }
}

struct NoKeyTwiceVisitor;

impl<'de> serde::de::Visitor<'de> for NoKeyTwiceVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON with no key twice in one object")
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(NoKeyTwice(v)) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(serde::de::Error::custom("a key appears twice in one object"));
            }
            let NoKeyTwice(v) = map.next_value()?;
            out.insert(key, v);
        }
        Ok(Value::Object(out))
    }
}

/// Whether a hooks file holds nothing but our registrations: a JSON object
/// whose keys are `hooks` (and cursor's numeric `version`), every value under
/// `hooks` a list of entries that are exactly what we write (`entry_is_ours`).
/// That is the only kind of hooks file removing a worktree can lose without
/// asking, because nobody else put anything in it. Anything else is
/// somebody's: their own entry, a key of their own on the file or on one of
/// our entries, a command of theirs inside our entry or appended to ours,
/// text that isn't a JSON object, an object with a key twice
/// (`parse_hooks_file`), or another daemon's registration.
///
/// `socket` is this runner's hook socket, the mark every entry of ours
/// carries (`Us`).
pub fn holds_only_ours(text: &str, socket: &Path) -> bool {
    let Some(Value::Object(root)) = parse_hooks_file(text) else { return false };
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
/// comes before the release. So a command is ours when it names THIS runner's
/// socket, through a CLI at any path whose name is exactly one our channels
/// install (`is_a_far_cooler_cli`).
///
/// The socket is required every time. The CLI's path alone says nothing
/// about which daemon wrote the entry: two homes can run one cargo binary,
/// and a channel whose own CLI is missing falls back to stable's
/// (`service::shim_binary`). Either way, a path-only match would take another
/// daemon's entries for ours and replace them on every launch. Another
/// channel's daemon (its own home, its own socket) is somebody else, and the
/// two installs sit side by side.
struct Us {
    socket: String,
}

impl Us {
    fn here(socket: &Path) -> Self {
        Us { socket: socket.display().to_string() }
    }

    fn wrote(&self, command: &OurCommand) -> bool {
        command.socket == self.socket && is_a_far_cooler_cli(&command.binary)
    }
}

/// Whether a binary's file name is exactly one our channels install or cargo
/// builds (`farcooler_protocol::Channel::cli_binary_candidates`, all four
/// channels). Exactly, not by prefix: a `farcooler-trace` the owner wrapped
/// around our CLI is theirs, even registered in our grammar with our socket.
fn is_a_far_cooler_cli(binary: &str) -> bool {
    use farcooler_protocol::Channel;
    let Some(name) = Path::new(binary).file_name().and_then(|n| n.to_str()) else { return false };
    every_channel().into_iter().any(|c: Channel| c.cli_binary_candidates().contains(&name))
}

/// Every channel, spelled out through a `match` so a fifth one can't be added
/// without this list learning its CLI names.
fn every_channel() -> [farcooler_protocol::Channel; 4] {
    use farcooler_protocol::Channel;
    const EVERY: [Channel; 4] = [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable];
    // Not called; it exists to stop compiling when `Channel` grows.
    let _listed = |c: Channel| match c {
        Channel::Local | Channel::Canary | Channel::Preview | Channel::Stable => (),
    };
    EVERY
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
/// the file's own agent would read it. Then the event already reports to us,
/// and adding our own entry beside it would run the hook twice: every prompt
/// and every answer drawn twice in chat.
///
/// Only in the shape that agent reads (`nested`: codex's inner `hooks` list,
/// the hook `type: "command"`; otherwise cursor's flat `command`), and only
/// with no key on the entry, or on the hook that runs ours, that we don't
/// write ourselves. A key we don't know may switch the entry off or narrow
/// it (`enabled: false`, a `matcher`, a zero `timeout`), and an entry that
/// quietly doesn't run would cost this event its live view for good. A
/// second copy of ours firing is the visible failure of the two, so an
/// unknown key means we add ours.
fn entry_runs(entry: &Value, ours: &str, nested: bool) -> bool {
    let Some(entry) = entry.as_object() else { return false };
    if !nested {
        return entry.keys().all(|k| k == "command")
            && entry.get("command").and_then(Value::as_str).is_some_and(|c| command_runs(c, ours));
    }
    if entry.keys().any(|k| k != "hooks") {
        return false;
    }
    let Some(Value::Array(inner)) = entry.get("hooks") else { return false };
    inner.iter().any(|hook| {
        hook.as_object().is_some_and(|hook| {
            hook.keys().all(|k| k == "type" || k == "command")
                && hook.get("type").and_then(Value::as_str) == Some("command")
                && hook.get("command").and_then(Value::as_str).is_some_and(|c| command_runs(c, ours))
        })
    })
}

/// Whether a command line runs `ours` as its first command: `ours` itself,
/// or `ours` and then a shell separator (`<ours> && say done`, `<ours>; …`,
/// `<ours>` and a newline, `<ours> # note`). Not `<ours>.bak`, which the
/// shell reads as one longer word, and not `<ours> --gating`, which is
/// another command.
fn command_runs(command: &str, ours: &str) -> bool {
    let Some(rest) = command.strip_prefix(ours) else { return false };
    let after = rest.trim_start_matches([' ', '\t']);
    after.is_empty()
        || after.starts_with([';', '&', '|', '\n'])
        || (after.len() < rest.len() && after.starts_with('#'))
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
    merge(existing, socket, "codex", CLAUDE_CODEX_EVENTS, true, |command| {
        json!({ "hooks": [ { "type": "command", "command": command } ] })
    })
}

/// Merge our registrations into `existing`, a `.cursor/hooks.json`. Cursor's
/// shape is flatter than the other two — no inner `hooks` array — and its
/// event names are its own; both are captured in `CURSOR_EVENTS` and the
/// entry shape below, and nothing else about the file (its `version` key
/// included) is touched.
pub fn merge_cursor(existing: &str, socket: &Path) -> String {
    merge(existing, socket, "cursor", CURSOR_EVENTS, false, |command| json!({ "command": command }))
}

/// `merge_codex` and `merge_cursor`, which differ only in their events and
/// in the shape of one entry (`nested`: codex's).
///
/// Hands back `existing` itself, byte for byte, whenever the merge changes
/// nothing a JSON reader would see (`rewritten`), so a caller comparing
/// strings writes only when ours has to be added or replaced.
///
/// Text that isn't empty and isn't a JSON object we can read safely
/// (`parse_hooks_file`) is handed back unchanged too, and so is a `hooks`
/// that isn't an object. An event whose value isn't a list is skipped. Each
/// of those is somebody's arrangement we can't add to without replacing it,
/// so we go without that registration.
fn merge(
    existing: &str,
    socket: &Path,
    agent: &str,
    events: &[(&str, bool)],
    nested: bool,
    entry: impl Fn(String) -> Value,
) -> String {
    let before = if existing.trim().is_empty() {
        json!({})
    } else {
        match parse_hooks_file(existing) {
            Some(v) if v.is_object() => v,
            _ => {
                tracing::info!(agent, "a hooks file isn't a JSON object we can read safely; leaving it alone");
                return existing.to_string();
            }
        }
    };
    let mut after = before.clone();
    let Some(hooks_obj) = hooks_object_mut(&mut after) else {
        tracing::info!(agent, "a hooks file's `hooks` is not an object; leaving it alone");
        return existing.to_string();
    };
    let us = Us::here(socket);
    for (event, gating) in events {
        let command = hook_command(agent, event, socket, *gating);
        merge_event(hooks_obj, event, &command, entry(command.clone()), nested, &us);
    }
    rewritten(existing, &before, &after)
}

/// Strip only the entries that are exactly ours (`entry_is_ours`), leaving
/// every other event and every other entry exactly as it was. Works on
/// either file's shape, since `entry_is_ours` reads whichever of the two it
/// finds. Text we can't read safely is handed back unchanged.
pub fn remove_ours(existing: &str, socket: &Path) -> String {
    let Some(before) = parse_hooks_file(existing) else {
        return existing.to_string();
    };
    let mut after = before.clone();
    let us = Us::here(socket);
    if let Some(hooks_obj) = after.get_mut("hooks").and_then(Value::as_object_mut) {
        for arr in hooks_obj.values_mut() {
            let Some(list) = arr.as_array() else { continue };
            let kept: Vec<Value> = list.iter().filter(|entry| !entry_is_ours(entry, &us)).cloned().collect();
            *arr = Value::Array(kept);
        }
    }
    rewritten(existing, &before, &after)
}

/// The text of the file that said `before` and must now say `after`:
/// `existing` itself when the two are equal as JSON, so a file whose
/// spacing, key order or final newline isn't serde's is never rewritten for
/// nothing.
///
/// When it does change, it is written with the owner's indentation and
/// final newline. Not their key order: without serde_json's
/// `preserve_order` feature (off in this workspace, and turning it on
/// changes every map in every crate) an object's keys come out sorted.
fn rewritten(existing: &str, before: &Value, after: &Value) -> String {
    if before == after {
        return existing.to_string();
    }
    let indent = indent_of(existing);
    let mut out = Vec::new();
    {
        let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        if serde::Serialize::serialize(after, &mut serializer).is_err() {
            return existing.to_string();
        }
    }
    let Ok(mut text) = String::from_utf8(out) else { return existing.to_string() };
    if existing.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// One level of the owner's indentation: the leading spaces or tabs of the
/// first indented line, or serde's two spaces for a file with none.
fn indent_of(text: &str) -> String {
    text.lines()
        .map(|line| &line[..line.len() - line.trim_start_matches([' ', '\t']).len()])
        .find(|lead| !lead.is_empty())
        .unwrap_or("  ")
        .to_string()
}

/// `root["hooks"]` as an object, created if it is missing. `None` when it is
/// there and is something else, which is somebody's and not ours to replace.
fn hooks_object_mut(root: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    let root_obj = root.as_object_mut().expect("merge only starts from an object");
    root_obj.entry("hooks").or_insert_with(|| json!({})).as_object_mut()
}

/// Bring `event` up to date: drop the entries that are exactly ours, keep
/// every other entry as it is and where it is, and add our fresh one at the
/// end. Dropping our old entry first, rather than only appending, is what
/// makes a second merge a no-op instead of a second copy, and what replaces
/// an entry of ours that names an older CLI path.
///
/// Unless somebody's entry already runs `command`, exactly this one
/// (`entry_runs`): an owner who put a command of theirs inside our entry, or
/// after ours on its command line. That entry isn't ours to take apart, and
/// it already reports to us, so a second entry of ours would fire the hook
/// twice (every prompt and answer drawn twice in chat). Then we add nothing.
///
/// An event whose value isn't a list is left alone, for `merge`'s reason.
fn merge_event(
    hooks_obj: &mut serde_json::Map<String, Value>,
    event: &str,
    command: &str,
    new_entry: Value,
    nested: bool,
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
    if kept.iter().any(|e| entry_runs(e, command, nested)) {
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

    /// A socket is one daemon's for as long as its home is
    /// (`<runtime dir>/h.sock`), so an entry naming another socket, even
    /// through this very CLI, is another daemon's: two homes sharing one
    /// cargo binary. It stays, and ours is added beside it, once.
    #[test]
    fn merging_with_another_socket_keeps_that_daemons_entry() {
        let once = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_codex(&once, Path::new("/tmp/other.sock"));
        let v: Value = serde_json::from_str(&twice).expect("json");
        let arr = v["hooks"]["SessionStart"].as_array().expect("SessionStart is an array");
        assert_eq!(arr.len(), 3, "somebody's, the other daemon's, and ours: {arr:?}");
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e, &Us::here(Path::new("/tmp/other.sock")))).collect();
        assert_eq!(ours.len(), 1, "{arr:?}");
        let theirs: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e, &Us::here(Path::new("/tmp/h.sock")))).collect();
        assert_eq!(theirs.len(), 1, "the other daemon's entry is still there: {arr:?}");
        assert_eq!(twice, merge_codex(&twice, Path::new("/tmp/other.sock")), "and a third merge changes nothing");
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
    fn cursor_merging_with_another_socket_keeps_that_daemons_entry() {
        let once = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let twice = merge_cursor(&once, Path::new("/tmp/other.sock"));
        let v: Value = serde_json::from_str(&twice).expect("json");
        let arr = v["hooks"]["sessionStart"].as_array().expect("sessionStart is an array");
        assert_eq!(arr.len(), 4, "both foreign entries, the other daemon's, and ours: {arr:?}");
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e, &Us::here(Path::new("/tmp/other.sock")))).collect();
        assert_eq!(ours.len(), 1, "{arr:?}");
        assert_eq!(v["version"], json!(1), "cursor's `version` survives: {twice}");
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
        assert!(
            !holds_only_ours(&at("/Users/x/bin/farcooler-trace", "/tmp/h.sock"), socket),
            "the owner's wrapper around our CLI, in our grammar with our socket"
        );
        assert!(
            !holds_only_ours(&at(&binary_path(), "/tmp/p.sock"), socket),
            "this very CLI, reporting to another daemon"
        );
        assert!(!holds_only_ours(&at("/Users/x/.local/bin/farcooler-preview", "/tmp/p.sock"), socket), "another daemon's");
    }

    /// What we write, we read back as ours, whatever the paths hold: a
    /// space, a quote, a backslash. And nothing but what we write.
    #[test]
    fn a_command_we_render_is_the_only_command_we_parse_as_ours() {
        let us = Us { socket: "/r/it's a \\ dir/h.sock".into() };
        for binary in ["/x/it's a \\ dir/farcooler-canary", "/y/farcooler"] {
            let command = render_command(binary, "cursor", "stop", &us.socket, true);
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
        let stop = commands_under(&merged, "stop");
        assert!(stop.iter().any(|c| c.ends_with("; say done")), "and on cursor's flat entry: {merged}");
        assert_eq!(stop.len(), 1, "with ours not added beside it: {stop:?}");
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

    /// I1: a key twice in one object, at any depth, is not JSON we trust.
    /// serde_json would keep the last copy and drop the first without a word.
    #[test]
    fn a_key_twice_in_one_object_is_not_read_at_all() {
        let socket = Path::new("/tmp/h.sock");
        assert!(parse_hooks_file(r#"{"a": 1, "b": {"c": [1, {"d": 2}]}}"#).is_some(), "plain JSON reads");
        assert!(parse_hooks_file(r#"{"a": 1, "a": 2}"#).is_none(), "at the top");
        assert!(parse_hooks_file(r#"{"hooks": {"Stop": [], "Stop": []}}"#).is_none(), "one level down");
        assert!(parse_hooks_file(r#"{"hooks": {"Stop": [{"command": "x", "command": "y"}]}}"#).is_none(), "in an entry");

        let ours: Value = serde_json::from_str(&merge_codex("{}", socket)).unwrap();
        let twice = format!(r#"{{"hooks": {{"Stop": [{{"command": "say theirs"}}]}}, "hooks": {}}}"#, ours["hooks"]);
        assert!(!holds_only_ours(&twice, socket), "the last copy is ours, the first is theirs");
        assert_eq!(merge_codex(&twice, socket), twice, "a merge hands it back untouched");
        assert_eq!(remove_ours(&twice, socket), twice, "and so does removing ours");
    }

    /// m1 of the second review: a merge that changes nothing a JSON reader
    /// would see hands back the owner's text byte for byte, spacing, key
    /// order and final newline included. One that must change keeps their
    /// indentation and final newline.
    #[test]
    fn a_merge_that_changes_nothing_keeps_the_owners_text() {
        let socket = Path::new("/tmp/h.sock");
        let ours: Value = serde_json::from_str(&merge_codex("{}", socket)).unwrap();
        let h = &ours["hooks"];
        let owners = format!(
            "{{\n    \"hooks\": {{\n        \"UserPromptSubmit\": {},\n        \"Stop\": {},\n        \"SessionStart\": {}\n    }}\n}}\n",
            h["UserPromptSubmit"], h["Stop"], h["SessionStart"],
        );
        assert_eq!(merge_codex(&owners, socket), owners, "already current, so untouched");

        let theirs = "{\n    \"hooks\": {\n        \"sessionEnd\": [ { \"command\": \"say bye\" } ]\n    },\n    \"version\": 1\n}\n";
        let merged = merge_cursor(theirs, socket);
        assert!(merged.ends_with("}\n"), "their final newline: {merged}");
        assert!(merged.contains("\n    \"hooks\": {\n        \""), "their four-space indent: {merged}");
        assert!(merged.contains("say bye"), "{merged}");
        assert_eq!(serde_json::from_str::<Value>(&merged).unwrap()["version"], json!(1), "{merged}");
    }

    /// m3 of the second review: an entry of theirs counts as already running
    /// ours only when the agent would really run exactly ours from it.
    /// Anything less and we add ours, since an event that silently reports
    /// nothing is the worse failure.
    #[test]
    fn only_an_entry_that_really_runs_ours_stands_in_for_ours() {
        let socket = Path::new("/tmp/h.sock");
        let ours = hook_command("codex", "Stop", socket, false);
        let stop_after = |entry: Value| {
            let existing = json!({ "hooks": { "Stop": [entry] } }).to_string();
            commands_under(&merge_codex(&existing, socket), "Stop").len()
        };
        let hook = |command: String| json!({ "type": "command", "command": command });

        assert_eq!(stop_after(json!({ "hooks": [hook(ours.clone()), hook("say done".into())] })), 2, "runs ours");
        assert_eq!(stop_after(json!({ "hooks": [hook(format!("{ours}\nsay done"))] })), 1, "ours, then a newline");
        assert_eq!(stop_after(json!({ "hooks": [hook(format!("{ours} # note"))] })), 1, "ours, then a comment");

        assert_eq!(stop_after(json!({ "hooks": [hook(format!("{ours}.bak"))] })), 2, "a longer word");
        assert_eq!(
            stop_after(json!({ "hooks": [hook(format!("{ours} --gating"))] })),
            1,
            "a gating copy of ours is still ours, and is replaced by the current one"
        );
        assert_eq!(stop_after(json!({ "hooks": [hook(format!("{ours}#x"))] })), 2, "no space before `#`: one word");
        assert_eq!(
            stop_after(json!({ "hooks": [{ "type": "prompt", "command": ours.clone() }] })),
            2,
            "not a command hook"
        );
        assert_eq!(
            stop_after(json!({ "command": format!("{ours} && say done") })),
            2,
            "cursor's shape, which codex doesn't read"
        );
        assert_eq!(
            stop_after(json!({ "enabled": false, "hooks": [hook(ours.clone())] })),
            2,
            "a key we don't know on the entry may switch it off"
        );
        assert_eq!(
            stop_after(json!({ "hooks": [{ "type": "command", "command": ours.clone(), "timeout": 0 }] })),
            2,
            "or on the hook"
        );
        let unquoted = ours.replacen('\'', "", 2);
        assert_eq!(stop_after(json!({ "hooks": [hook(unquoted)] })), 2, "the same CLI, quoted differently, fires twice");

        let cursor = |entry: Value| {
            let existing = json!({ "hooks": { "stop": [entry] } }).to_string();
            commands_under(&merge_cursor(&existing, socket), "stop").len()
        };
        let ours = hook_command("cursor", "stop", socket, false);
        assert_eq!(cursor(json!({ "command": format!("{ours} && say done") })), 1, "cursor's own shape runs ours");
        assert_eq!(
            cursor(json!({ "hooks": [hook(format!("{ours} && say done"))] })),
            2,
            "codex's shape, which cursor doesn't read"
        );
    }
}
