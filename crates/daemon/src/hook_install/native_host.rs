use super::{Path, hook_command, parse_hooks_file, entry_runs};

/// Read-only native-host lifecycle, active-steering and mid-turn context check.
/// Other pinned user hooks/settings need not be ours; no merge is performed.
pub(crate) fn native_completion_targets(text: &str, agent: &str, socket: &Path) -> bool {
    let Some(value) = parse_hooks_file(text) else { return false };
    if agent == "claude" && value.get("disableAllHooks").is_some_and(|v| v != false) { return false; }
    let events: &[(&str, bool)] = match agent {
        "claude" => &[("UserPromptSubmit", false), ("Stop", false), ("PreToolUse", true)],
        "codex" => &[("SessionStart", false), ("UserPromptSubmit", false), ("Stop", false)],
        "cursor" => &[("sessionStart", false), ("beforeSubmitPrompt", false), ("stop", true)],
        _ => return false,
    };
    if matches!(agent, "claude" | "codex") {
        // Lifecycle-only settings cannot steer active work. Require the exact all-tools, waiting
        // registration we install: a narrowed matcher, async hook, missing --gating or changed
        // endpoint can silently defer direction until after a tool has changed the candidate.
        let expected = if agent == "claude" { super::claude_settings(socket) } else { native_codex_hooks(socket) };
        if value["disableAllHooks"] == true || !value["hooks"]["PreToolUse"].as_array().is_some_and(|entries|
            entries.iter().any(|entry| entry == &expected["hooks"]["PreToolUse"][0])) {
            return false;
        }
    }
    events.iter().all(|(event, gating)| {
        let ours = hook_command(agent, event, socket, *gating);
        value["hooks"][event].as_array().is_some_and(|entries| entries.iter().any(|entry| {
            if agent == "claude" && *event == "PreToolUse" { context_entry_runs(entry, &ours) }
            else { entry_runs(entry, &ours, agent != "cursor") }
        }))
    })
}

/// Engine-only Codex hooks. Ordinary panes retain their existing lifecycle registrations.
/// Codex 0.167.2 reads PreToolUse denial in the same shape as Claude (live probe, ov-1205).
pub(crate) fn native_codex_hooks(socket: &Path) -> serde_json::Value {
    let mut value = parse_hooks_file(&super::merge_codex("", socket)).expect("generated Codex hooks");
    value["hooks"]["PreToolUse"] = serde_json::json!([{
        "matcher": "*",
        "hooks": [{ "type": "command", "command": hook_command("codex", "PreToolUse", socket, true),
            "timeout": super::CLAUDE_GATE_TIMEOUT_S }]
    }]);
    value
}

/// A context reply needs stdout from a waiting hook on every tool. The lifecycle grammar deliberately
/// rejects matchers and timeouts; here accept the engine's wildcard and hold timeout, but no async,
/// narrowed matcher, or shell suffix that could discard the context before Claude reads it.
fn context_entry_runs(entry: &serde_json::Value, ours: &str) -> bool {
    let Some(entry) = entry.as_object() else { return false };
    if entry.keys().any(|k| k != "matcher" && k != "hooks")
        || entry.get("matcher").is_some_and(|v| v != "*") { return false; }
    entry.get("hooks").and_then(serde_json::Value::as_array).is_some_and(|hooks| hooks.iter().any(|hook| {
        let Some(hook) = hook.as_object() else { return false };
        hook.keys().all(|k| matches!(k.as_str(), "type" | "command" | "timeout" | "async"))
            && hook.get("type").is_some_and(|v| v == "command")
            && hook.get("command").is_some_and(|v| v == ours)
            && hook.get("async").is_none_or(|v| v == false)
            && hook.get("timeout").is_none_or(|v| v.as_u64().is_some_and(|s| s >= super::CLAUDE_GATE_TIMEOUT_S))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_codex_requires_the_installed_all_tools_waiting_hook() {
        let socket = Path::new("/tmp/engine/h.sock");
        let original = native_codex_hooks(socket);
        let accepts = |v: &serde_json::Value| native_completion_targets(&v.to_string(), "codex", socket);
        assert!(accepts(&original));
        assert!(!native_completion_targets(&super::super::merge_codex("", socket), "codex", socket),
            "ordinary lifecycle-only panes are not an engine steering configuration");
        for fault in ["missing", "not-gating", "foreign-socket", "narrow-matcher", "async", "disabled", "short-timeout"] {
            let mut value = original.clone();
            match fault {
                "missing" => { value["hooks"].as_object_mut().unwrap().remove("PreToolUse"); }
                "not-gating" | "foreign-socket" => {
                    let command = value["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
                    value["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = if fault == "not-gating" {
                        command.replace(" --gating", "")
                    } else { command.replace("/tmp/engine/h.sock", "/tmp/other.sock") }.into();
                }
                "narrow-matcher" => value["hooks"]["PreToolUse"][0]["matcher"] = "Bash".into(),
                "async" => value["hooks"]["PreToolUse"][0]["hooks"][0]["async"] = true.into(),
                "disabled" => value["disableAllHooks"] = true.into(),
                _ => value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = 0.into(),
            }
            assert!(!accepts(&value), "accepted inactive Codex steering: {fault}");
        }
        let mut coexist = original;
        coexist["hooks"]["PreToolUse"].as_array_mut().unwrap().push(serde_json::json!({
            "matcher": "Bash", "hooks": [{"type": "command", "command": "owners-own-hook"}]
        }));
        assert!(accepts(&coexist), "other pinned hooks may coexist");
    }
}

#[cfg(test)]
mod claude_context_tests {
    use super::*;

    #[test]
    fn native_claude_requires_synchronous_context_hooks_for_every_tool() {
        let socket = Path::new("/private/engine/h.sock");
        let settings = super::super::claude_settings(socket);
        let accepts = |value: &serde_json::Value| native_completion_targets(&value.to_string(), "claude", socket);
        assert!(accepts(&settings), "the settings the engine supplies must work");
        for mode in ["missing", "non-gating", "async", "matcher", "timeout", "disabled", "redirected"] {
            let mut broken = settings.clone();
            match mode {
                "missing" => { broken["hooks"].as_object_mut().unwrap().remove("PreToolUse"); }
                "non-gating" => broken["hooks"]["PreToolUse"][0]["hooks"][0]["command"] =
                    hook_command("claude", "PreToolUse", socket, false).into(),
                "async" => broken["hooks"]["PreToolUse"][0]["hooks"][0]["async"] = true.into(),
                "matcher" => broken["hooks"]["PreToolUse"][0]["matcher"] = "Bash".into(),
                "timeout" => broken["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = 0.into(),
                "disabled" => broken["disableAllHooks"] = true.into(),
                "redirected" => {
                    let command = hook_command("claude", "PreToolUse", socket, true);
                    broken["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = format!("{command} | cat >/dev/null").into();
                }
                _ => unreachable!(),
            }
            assert!(!accepts(&broken), "accepted {mode} context hook");
        }
    }
}
