# ov-1087: Codex models in the workflow definition

## Result: the bundle is ready, on ov-1088's adapter shape

`workflow-codex.json` is the live definition and adapter, byte for byte, except:

- `models.build` = claude:sonnet, claude:opus, claude:haiku, codex:gpt-6.1-sol, codex:gpt-6.1-terra, codex:gpt-6.1-luna
- `models.gate-0` = claude:opus, claude:sonnet, codex:gpt-6.1-sol, codex:gpt-6.1-terra, codex:gpt-6.1-luna
- `adapter.config.harness.Native.codex_hooks` = `{path: ~/Library/Application Support/com.farcooler.FarCoolerCanary/workflow-codex-hooks.json,
  digest: wf1:2b8957b0…}` (ov-1088), and so every operation's first argument, the configuration's pin, is now
  `wf1:0ee210a5…` instead of `wf1:8261b6fc…` (all 11).

**Why the models alone weren't enough.** At publish time the runtime refuses any definition that lists a model whose harness
it can't launch (`coding.rs`, `launchable`), and a Codex model needs Codex's hooks pinned. Before ov-1088 the only way was
`project_hooks`, a `.codex/hooks.json` committed in the repository, naming this Mac's paths.

**ov-1088 (the owner's ruling: the engine supplies Codex's hooks, as it does Claude's).** The adapter can pin
`codex_hooks`, a file the runner keeps outside the repository, beside Claude's `workflow-claude-hooks.json`. Before a Codex
build or agent review starts, the engine copies it into the worktree's `.codex/hooks.json` (untracked; never over a file the
branch commits) and checks the copy against the pin. Nothing is committed. The daemon writes the file itself, and every
`workflow get` reply, a published workspace's included, says it as `engine_codex_hooks` (path and digest).

The digest in the bundle is what a daemon with ov-1088 will write: Codex's hooks to the engine's socket, run by the same
`farcooler` binary Claude's engine settings name. The bundle test recomputes it from the live Claude settings file.

## Acceptance 2: checked in a scratch runtime

The CLI could not do it: from an agent session every `farcooler` call into a scratch daemon is `scope-denied: known caller required`
(an agent is never a person), and I did not work around that. So the check is the in-process runtime test the repo already uses for
workflow commands, run with `FARCOOLER_OV1087_BUNDLE` pointing at the bundle (`coding_tests.rs`, `workflow_codex_bundle_publishes_and_starts_a_codex_build`, `#[ignore]`):

```
FARCOOLER_OV1087_BUNDLE=$PWD/.claude/agent/reports/ov-1087/workflow-codex.json \
  CARGO_BUILD_JOBS=4 cargo test -p farcooler-daemon --lib workflow_codex_bundle -- --ignored
```

It passes (ov-1088), and it fails on the old bundle. It shows:

- the file reads as `set-workflow --file` reads it (it round-trips), with the models above, every operation pinning the configuration,
  Claude's pinned hooks matching the file on disk, no `project_hooks`, and `codex_hooks` beside Claude's file, outside the repository,
  with the digest of the bytes the daemon will write (checked against the file itself once it exists);
- in a scratch runtime with no Codex hooks, as the live adapter is today, publishing is **refused** with "a listed model's harness isn't set up on this runtime";
- with the runner's Codex hooks file pinned, publishing is **accepted** with no `.codex` in the repository, `build=codex:gpt-4` is refused as
  Invalid, `build=codex:gpt-6.1-sol` starts, and its build gets the hooks copied into its worktree and launches `codex --model gpt-6.1-sol`.

Not covered: gate-0 in the scratch (its definition has no review step; gate-0's models are asserted on the file), and a real Codex
launch. Whether Codex 0.167 runs hooks from an untracked project `.codex/hooks.json` without asking is the same question the app's own
Codex panes already depend on; this branch doesn't change it.

## Acceptance 3: the publish command (owner, once the app runs a build with ov-1088)

```
farcooler-canary workflow get <workspace-id> --json     # revision, epoch, sequence, the current pin, and engine_codex_hooks
farcooler-canary workspace set-workflow <workspace-id> \
  --file .claude/agent/reports/ov-1087/workflow-codex.json \
  --expected-pin wf1:13df72328ff7de385b6ce73fe85fc2e793a9c9926238195f1fb4942c699d2800 \
  --command-id "$(uuidgen)" --revision <revision> --epoch <epoch> --sequence <sequence+1>
```

1. The `workflow get` reply's `engine_codex_hooks` must equal the bundle's `codex_hooks` (path
   `…/com.farcooler.FarCoolerCanary/workflow-codex-hooks.json`, digest
   `wf1:2b8957b0ae8c657426f4951a73c5ce271b81d3670597906f82ac5c21041253e2`). That `get` is also what writes the file. If the digest
   differs (the app's `farcooler` moved), put the reply's value in the bundle and run the bundle test again; it gives the new pin to
   rewrite in the 11 operations.
2. Read `revision`, `epoch` and `sequence` from the same reply (`sequence` is the last accepted one, so pass the next). The command id
   is any fresh unique string. `--expected-pin` is the live publication's current pin, which this bundle replaces.
3. Run it only from your own shell, not an agent's.
