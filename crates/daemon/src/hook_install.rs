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
const CLAUDE_CODEX_EVENTS: &[(&str, bool)] = &[
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("Stop", false),
    ("PreToolUse", false),
    ("PostToolUse", false),
    ("PermissionRequest", true),
];

/// Cursor's own vocabulary for the same five moments, camelCase and its own
/// words for the tool gates: `beforeShellExecution` and `beforeMCPExecution`
/// where claude and codex both say `PreToolUse`.
const CURSOR_EVENTS: &[(&str, bool)] = &[
    ("sessionStart", false),
    ("beforeSubmitPrompt", false),
    ("stop", false),
    ("beforeShellExecution", true),
    ("beforeMCPExecution", true),
];

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
/// produces the same value.
pub fn claude_settings(socket: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, gating) in CLAUDE_CODEX_EVENTS {
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

    #[test]
    fn removing_ours_leaves_theirs_alone() {
        let merged = merge_codex(SOMEONE_ELSES, Path::new("/tmp/h.sock"));
        let cleaned = remove_ours(&merged);
        assert!(cleaned.contains("herdr-agent-state.sh"));
        assert!(!cleaned.contains("farcooler"));
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

    /// claude's settings object touches no file at all, and both halves of
    /// that promise get checked: the shape Task 10 will write out, and that
    /// no `merge_*`/`remove_ours` function in this module is ever called on
    /// claude's behalf anywhere in this crate.
    #[test]
    fn claude_settings_carries_every_event_with_no_file_read() {
        let settings = claude_settings(Path::new("/tmp/h.sock"));
        for (event, gating) in CLAUDE_CODEX_EVENTS {
            let command = settings["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("claude settings carries a command for {event}"));
            assert!(command.contains(&format!("--agent claude --event {event}")), "{event}: {command}");
            assert_eq!(command.contains("--gating"), *gating, "{event}: {command}");
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
