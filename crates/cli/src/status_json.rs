//! `status --json`'s object, out of `main.rs` for its size budget (ov-818).

use super::*;

/// `status --json`'s object.
///
/// A function rather than built inline in `status`, which only runs against a
/// live runner: a key dropped from an object nothing can test is dropped
/// without a word, and this is the object the Mac reads for its build dot and
/// its capabilities.
pub(crate) fn status_json(
    host: &farcooler_protocol::v1::Host,
    capabilities: &[String],
    counts: StatusCounts,
) -> serde_json::Value {
    let healthy = host.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;
    serde_json::json!({
        "daemonVersion": host.daemon_version,
        "cliVersion": farcooler_protocol::BUILD,
        // Two builds that cannot agree on what they are running is a
        // fact a client needs, not a detail. It is how a fix that was
        // compiled and tested goes on reproducing in the app.
        "buildsMatch": farcooler_protocol::build_identity::same_build(&host.daemon_version, farcooler_protocol::BUILD),
        // Distinct from `buildsMatch`, and they answer different
        // questions. That one is "are these the same build"; this is
        // "what can that runner do", which is the one a client acts on
        // when it is newer than the runner it reached.
        "capabilities": capabilities,
        "platform": host.platform,
        "branchPrefix": host.settings
            .as_ref()
            .map(|s| s.branch_prefix.clone())
            .unwrap_or_default(),
        "runtimeHealthy": healthy,
        // The fingerprint of the key this caller's connection authenticated
        // with (ov-818), or null from a runner too old to say or a caller with
        // no line of its own (this Mac's local runner). The Mac remembers it,
        // so a refusal it reads later counts only when it's this key's.
        "keyFingerprint": (!host.caller_key_fingerprint.is_empty()).then_some(&host.caller_key_fingerprint),
        "livePanes": host.live_terminal_count,
        // What every agent launch on this runner runs instead of the agent,
        // or null when agents launch as themselves. The same key, and the
        // same null from an older runner, as the phones' `host.health`.
        "standInAgent": stand_in_agent(host),
        // The owner's shell-exports-a-key warning (ov-651), or null.
        "claudeKeyWarning": farcooler_core::claude_keys::warning(&host.rc_claude_keys),
        // Whether this runner's task notices reach the owner's devices as
        // pushes (ov-94): the Mac then leaves them to the push.
        "pushPaired": host.push_paired,
        // Which agents the runner can start, by program name (ov-205). Null
        // from a runner too old to say, whose empty list means nothing.
        "agentsFound": capabilities
            .iter()
            .any(|c| c == farcooler_protocol::capability::AGENTS_FOUND)
            .then_some(&host.agents_found),
        // The runner's own id (`Host.runner_id`), which a task notice names
        // it by (ov-106): a click on a pushed notice finds its runner here.
        // Null from a runner too old to say.
        "runnerId": (!host.runner_id.is_empty()).then_some(&host.runner_id),
        // Whether this runner's database is newer than this build (ov-143),
        // so builds that differ differ in that direction: installing this
        // build there is a downgrade, and the Mac must not offer it as an
        // update. False from a runner too old to say.
        "runnerIsNewer": runner_is_newer(host),
        // The extra folders the Files viewer can read on this runner
        // (ov-232), by name; `path` is empty for a read-scoped client. Null
        // from a runner too old to say, which has none.
        "readOnlyFolders": capabilities
            .iter()
            .any(|c| c == farcooler_protocol::capability::READ_ONLY_FOLDERS)
            .then(|| {
                host.read_only_folders
                    .iter()
                    .map(|f| serde_json::json!({"name": f.name, "path": f.path}))
                    .collect::<Vec<_>>()
            }),
        // What the app sends the orchestrator for Suggest Themes (ov-519):
        // empty from a runner from before, where the app offers none.
        "suggestThemesPrompt": host.suggest_themes_prompt,
        "roots": counts.roots,
        "repositories": counts.repositories,
        "worktrees": counts.worktrees,
        "terminals": counts.terminals,
    })
}
