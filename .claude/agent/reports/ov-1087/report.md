# ov-1087: Codex models in the workflow definition

## Result: the bundle is ready, but publishing it alone will be refused

`workflow-codex.json` is the live definition and adapter, byte for byte, except:

- `models.build` = claude:sonnet, claude:opus, claude:haiku, codex:gpt-6.1-sol, codex:gpt-6.1-terra, codex:gpt-6.1-luna
- `models.gate-0` = claude:opus, claude:sonnet, codex:gpt-6.1-sol, codex:gpt-6.1-terra, codex:gpt-6.1-luna

**The ticket's premise was incomplete.** The definition's model list isn't the only thing that blocks Codex. At publish time the runtime
refuses any definition that lists a model whose harness it can't launch (`coding.rs`, `launchable`). A Codex model is launchable only if
the adapter pins Codex's project hooks (`harness.Native.project_hooks`). The live adapter has `project_hooks: null`, so this bundle is
refused with "a listed model's harness isn't set up on this runtime". Scratch check below.

What Codex needs, none of which I could do (it is the owner's repository and the live app):

1. A `.codex/hooks.json` in the repository, **committed on the landing base** (an engine worktree is cut from it, and the launch
   re-verifies the worktree's copy against the pinned digest), whose hooks report to the **engine's** socket,
   `~/Library/Application Support/com.farcooler.FarCoolerCanary/workflow/h.sock`.
2. The adapter's `project_hooks` = `{path: <repository>/.codex/hooks.json, digest}`. Digest: `wf1:` + SHA-256 of
   `json.dumps(list(file_bytes), separators=(',',':'))` (Claude's settings digest, recomputed this way, matches the live adapter).
3. Every operation's first argument is the configuration digest, so adding `project_hooks` changes it: rewrite all 11 occurrences of
   `wf1:8261b6fc…` (the current one) to the new `CodingConfig::pin()`. The test prints it.

The checkout's existing `/Users/e-liang/Dev/overnight/.codex/hooks.json` is untracked and targets the app's `h.sock`, not the engine's
`workflow/h.sock`, so it can't be pinned as it is. **Decision for the owner:** whether to commit an engine hooks file (it holds this
Mac's absolute app and socket paths), or have the engine write one into each worktree. Until then `--model build=codex:…` stays refused.

## Acceptance 2: checked in a scratch runtime

The CLI could not do it: from an agent session every `farcooler` call into a scratch daemon is `scope-denied: known caller required`
(an agent is never a person), and I did not work around that. So the check is the in-process runtime test the repo already uses for
workflow commands, run with `FARCOOLER_OV1087_BUNDLE` pointing at the bundle (`coding_tests.rs`, `workflow_codex_bundle_publishes_and_starts_a_codex_build`, `#[ignore]`):

```
FARCOOLER_OV1087_BUNDLE=$PWD/.claude/agent/reports/ov-1087/workflow-codex.json \
  CARGO_BUILD_JOBS=4 cargo test -p farcooler-daemon --lib workflow_codex_bundle -- --ignored
```

It passes. It shows:

- the file reads as `set-workflow --file` reads it (it round-trips), with the models above, every operation pinning the configuration, and Claude's pinned hooks matching the file on disk;
- in a scratch runtime with the live adapter's shape (no Codex hooks), publishing is **refused** with the message above;
- with Codex hooks pinned, publishing is **accepted**, `build=codex:gpt-6.1-sol` starts and is recorded, and `build=codex:gpt-4` is refused as Invalid.

Not covered: gate-0 in the scratch (its definition has no review step; gate-0's models are asserted on the file), and a real Codex launch.

## Acceptance 3: the publish command (owner, once the hooks above exist and the bundle is regenerated with them)

```
farcooler-canary workflow get <workspace-id> --json     # read revision, epoch, sequence, and the current pin
farcooler-canary workspace set-workflow <workspace-id> \
  --file .claude/agent/reports/ov-1087/workflow-codex.json \
  --expected-pin wf1:13df72328ff7de385b6ce73fe85fc2e793a9c9926238195f1fb4942c699d2800 \
  --command-id "$(uuidgen)" --revision <revision> --epoch <epoch> --sequence <sequence+1>
```

Read `revision`, `epoch` and `sequence` from the `workflow get` state (`sequence` is the last accepted one, so pass the next). The
command id is any fresh unique string. Run it only from your own shell, not an agent's.
