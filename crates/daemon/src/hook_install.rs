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
//! Entries we add are marked as ours by having their command begin with this
//! binary's own path. We merge, and we remove only entries carrying that
//! mark: `remove_ours` never touches a hook someone else registered, and
//! neither merge function ever rewrites a file it did not itself add to.

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

/// The project-local files `service::install_project_hooks` writes into a
/// worktree, relative to its root.
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

/// `PROJECT_HOOK_FILES` as git pathspecs that subtract them from an answer.
///
/// A pathspec rather than a filter over `git status` output, because git's own
/// matching is the only thing that gets this right. `git status --porcelain`
/// collapses a wholly-untracked directory to one entry — `?? .codex/`, never
/// `?? .codex/hooks.json` — so a filter comparing whole lines against these
/// paths would match nothing at all and silently do nothing. It is also
/// conservative in the direction that matters: with a file of the user's own
/// beside ours, git reports the directory again and their file is not hidden.
///
/// The caller puts `--` in front of these.
pub fn project_hook_exclusions() -> Vec<String> {
    PROJECT_HOOK_FILES.iter().map(|p| format!(":(exclude){p}")).collect()
}

/// The path this binary was launched as, resolved the same way the shim's
/// own command line is (`service::shim_binary`), so a hooks file and a pane
/// command never disagree about which CLI they mean.
fn binary_path() -> String {
    crate::service::shim_binary(std::env::current_exe().ok().as_deref())
}

/// Every command we register begins with this. `remove_ours` finds exactly
/// what these functions added, and nothing a different tool wrote, by
/// looking for it — and merging is idempotent because a merge starts by
/// dropping any entry that already carries it before adding a fresh one.
fn ours_prefix() -> String {
    format!("{} hook --agent ", crate::service::shell_quote(&binary_path()))
}

fn is_ours(command: &str) -> bool {
    command.starts_with(&ours_prefix())
}

/// One entry's command, whichever of the two shapes it is written in:
/// claude/codex's `{"hooks": [{"command": ...}]}` or cursor's flat
/// `{"command": ...}`.
fn entry_is_ours(entry: &Value) -> bool {
    if let Some(command) = entry.get("command").and_then(Value::as_str) {
        return is_ours(command);
    }
    if let Some(inner) = entry.get("hooks").and_then(Value::as_array) {
        return inner.iter().any(|h| h.get("command").and_then(Value::as_str).is_some_and(is_ours));
    }
    false
}

/// The full shell command line for one hook registration.
///
/// The binary path is quoted (`shell_quote`) because it is a path Far Cooler
/// did not choose and may contain spaces; `hook`, the flag names and the
/// event name are not, matching the shape already on this machine — e.g.
/// `bash '/Users/e-liang/.codex/herdr-agent-state.sh' session`.
fn hook_command(agent: &str, event: &str, socket: &Path, gating: bool) -> String {
    let mut command = format!(
        "{} hook --agent {agent} --event {event} --socket {}",
        crate::service::shell_quote(&binary_path()),
        crate::service::shell_quote(&socket.display().to_string()),
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
/// ours, survives unchanged.
pub fn merge_codex(existing: &str, socket: &Path) -> String {
    let mut root = parse_or_empty_object(existing);
    let hooks_obj = hooks_object_mut(&mut root);
    for (event, gating) in CLAUDE_CODEX_EVENTS {
        let new_entry = json!({
            "hooks": [
                { "type": "command", "command": hook_command("codex", event, socket, *gating) }
            ]
        });
        merge_event(hooks_obj, event, new_entry);
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| existing.to_string())
}

/// Merge our registrations into `existing`, a `.cursor/hooks.json`. Cursor's
/// shape is flatter than the other two — no inner `hooks` array — and its
/// event names are its own; both are captured in `CURSOR_EVENTS` and the
/// entry shape below, and nothing else about the file (its `version` key
/// included) is touched.
pub fn merge_cursor(existing: &str, socket: &Path) -> String {
    let mut root = parse_or_empty_object(existing);
    let hooks_obj = hooks_object_mut(&mut root);
    for (event, gating) in CURSOR_EVENTS {
        let new_entry = json!({ "command": hook_command("cursor", event, socket, *gating) });
        merge_event(hooks_obj, event, new_entry);
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| existing.to_string())
}

/// Strip only the entries `merge_codex`/`merge_cursor` added, leaving every
/// other event and every other tool's entry exactly as it was. Works on
/// either file's shape, since `entry_is_ours` reads whichever of the two it
/// finds.
pub fn remove_ours(existing: &str) -> String {
    let Ok(mut root) = serde_json::from_str::<Value>(existing) else {
        return existing.to_string();
    };
    if let Some(hooks_obj) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for arr in hooks_obj.values_mut() {
            let Some(list) = arr.as_array() else { continue };
            let kept: Vec<Value> = list.iter().filter(|entry| !entry_is_ours(entry)).cloned().collect();
            *arr = Value::Array(kept);
        }
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| existing.to_string())
}

/// `existing`, parsed as a JSON object — or an empty one, for a file that is
/// missing, empty, or (should it somehow happen) not an object at all. A
/// file we cannot make sense of is treated as one with nothing in it yet,
/// never as a reason to give up and leave our hooks uninstalled.
fn parse_or_empty_object(existing: &str) -> Value {
    match serde_json::from_str::<Value>(existing) {
        Ok(v) if v.is_object() => v,
        _ => json!({}),
    }
}

/// `root["hooks"]` as an object, creating or replacing it if it is missing
/// or was something else.
fn hooks_object_mut(root: &mut Value) -> &mut serde_json::Map<String, Value> {
    let root_obj = root.as_object_mut().expect("parse_or_empty_object guarantees an object");
    let hooks = root_obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    hooks.as_object_mut().expect("just ensured this is an object")
}

/// Replace whatever `event` held with everyone else's entries plus our own
/// fresh one. Dropping our own old entry first, rather than only appending,
/// is what makes a second merge with the same socket a no-op instead of a
/// second copy.
fn merge_event(hooks_obj: &mut serde_json::Map<String, Value>, event: &str, new_entry: Value) {
    let existing_arr = hooks_obj.get(event).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut kept: Vec<Value> = existing_arr.into_iter().filter(|e| !entry_is_ours(e)).collect();
    kept.push(new_entry);
    hooks_obj.insert(event.to_string(), Value::Array(kept));
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e)).collect();
        assert_eq!(ours.len(), 1, "a re-merge with a new socket replaces, not accompanies: {arr:?}");
        let command = ours[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(command.contains("other.sock"), "the surviving entry names the new socket: {command}");
        assert!(!command.contains("/tmp/h.sock"), "the stale socket is gone: {command}");
    }

    #[test]
    fn removing_ours_leaves_theirs_alone() {
        let merged = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let cleaned = remove_ours(&merged);
        assert!(cleaned.contains("herdr-agent-state.sh"));
        assert!(!cleaned.contains("farcooler"));
    }

    /// A command that merely MENTIONS our prefix somewhere other than the
    /// front — the way a wrapper script's log line or comment might quote
    /// it — must not be mistaken for ours. Pins `is_ours`'s use of
    /// `starts_with` rather than `contains`: flipping that one word makes
    /// this fail while every other test in this module stays green, because
    /// no other fixture's foreign command shares any substring with the
    /// prefix.
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

        let cleaned = remove_ours(&merged);
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
        let ours: Vec<&Value> = arr.iter().filter(|e| entry_is_ours(e)).collect();
        assert_eq!(ours.len(), 1, "a re-merge with a new socket replaces, not accompanies: {arr:?}");
        let command = ours[0]["command"].as_str().unwrap();
        assert!(command.contains("other.sock"), "the surviving entry names the new socket: {command}");
        assert!(!command.contains("/tmp/h.sock"), "the stale socket is gone: {command}");
    }

    #[test]
    fn cursor_removing_ours_leaves_theirs_alone() {
        let merged = merge_cursor(CURSORS_ELSES, Path::new("/tmp/h.sock"));
        let cleaned = remove_ours(&merged);
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
}
