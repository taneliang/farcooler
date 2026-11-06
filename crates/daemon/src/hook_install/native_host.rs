use super::{Path, hook_command, parse_hooks_file, entry_runs};

/// Read-only native-host lifecycle check using the retained registration grammar.
/// Other pinned user hooks/settings need not be ours; no merge is performed.
pub(crate) fn native_completion_targets(text: &str, agent: &str, socket: &Path) -> bool {
    let Some(value) = parse_hooks_file(text) else { return false };
    let events: &[(&str, bool)] = match agent {
        "claude" => &[("UserPromptSubmit", false), ("Stop", false)],
        "codex" => &[("SessionStart", false), ("UserPromptSubmit", false), ("Stop", false)],
        "cursor" => &[("sessionStart", false), ("beforeSubmitPrompt", false), ("stop", true)],
        _ => return false,
    };
    events.iter().all(|(event, gating)| {
        let ours = hook_command(agent, event, socket, *gating);
        value["hooks"][event].as_array().is_some_and(|entries| entries.iter().any(|entry|
            entry_runs(entry, &ours, agent != "cursor")))
    })
}
