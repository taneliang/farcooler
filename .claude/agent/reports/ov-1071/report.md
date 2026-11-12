# ov-1071: Main's workflow runs our lane rules

`workflow-main.json` is ov-1087's bundle (`.claude/agent/reports/ov-1087/workflow-codex.json`), with the changes below.
The engine changes that make each one expressible are on this branch. I couldn't publish it myself: from an agent session,
every `farcooler` call into a daemon's workflow is refused as an agent (see ov-1087's report). So it's a file the owner
publishes, checked by a hand-run test.

## What changed in the engine

1. **Batch cap.** A group `limit` of `0` means no cap of its own: every ticket waiting joins, up to the engine's 32
   (`MAX_GROUP_MEMBERS`). The back-edge cap went from 3 to 5, so a full group of 32 can still split down to the one
   failing ticket. Templates take `--batch 0`, and split as deep as their batch needs (3 for 8 or fewer, so older pins
   don't move). The workflow summary says "Every finished ticket that's ready queues together".
2. **Prompts.** The build prompt was already the adapter's `prompt`. An agent's review gate can now pin options as its
   third argument (`ReviewOptions`, `crates/daemon/src/review_agent.rs`): a `prompt` said to the reviewer after the
   engine's brief.
3. **Review.**
   - **Model by path:** the same options carry `models`, `[glob, model]` rules. Before the reviewer starts, the
     engine reads the paths changed from the base to the candidate, and the earliest rule any file matches picks the
     model. It writes that choice down once (`<effect>.review-model.json`), so a settlement or recovery names the same
     model. No match keeps the run's pick. Every model a rule names must be one the step lists, so the publish-time
     "can this runtime launch it" check covers it.
   - **More rounds:** a back edge may be taken up to 5 times (was 3).
   - **Fresh fix rounds:** every build already launched as a new agent with its own session id, one per binding. What
     was missing was telling it so. A build that a later step turned back (iteration > 0, not a send-back) is now told
     it's fix round N, that it's a fresh agent, and to read the reasons in the ticket's newest notes. The engine's
     review brief now tells a reviewer that rejects to write what must change with
     `farcooler task note <key> --kind finding`.
4. **CI.** An `AwaitCi` step can pin a fifth argument, a JSON list of other workflows (`["Web","Doc comments"]`). Each
   look runs the CI command once per workflow, with `FARCOOLER_CI_WORKFLOW` naming it. The configured workflow must have
   a run. Another workflow counts only if the push started it, because `Web` has a path filter and often doesn't run.
   Any red result is red, any run still going is waited for, and all green is green. A look that finds no run now exits 0 and
   writes nothing; a look that fails still exits non-zero. The first-draft CI command (`CI_LOOK`) does the same, and
   falls back to `$CI_WORKFLOW`, so configurations made before this keep working.

## What the bundle changes, against ov-1087's

- `groups.landing.limit` 8 → **0**; `fold` and `gate-0` split limits 3 → **5**.
- `harness.Native.prompt`: points at `/Users/e-liang/Dev/overnight/.claude/agent/reports/lanes/rules.md` and names the rules
  that bite most: no cargo fmt or stash, `scripts/heavy`, CI-like git for tests, captures, net app lines, and the report in
  the last message.
- `gate-0` (the agent review): options with a review prompt pointing at the same rules, which judges UI polish from the
  captures, plus 22 path rules that send engine and security paths to `claude:opus`. Its `rejected → build` limit goes
  from 2 to **4**.
- `models.gate-0` now leads with `claude:sonnet`, so Sonnet reviews by default and the rules move engine and security
  work to Opus. Before, Opus was first, so it reviewed everything unless the orchestrator picked otherwise. That's my
  call, and it's reversible: put `claude:opus` first again to keep Opus as the default.
- The group's `ci` step waits on `["Web","Doc comments"]` besides `CI`. The CI command selects on
  `env.FARCOOLER_CI_WORKFLOW // env.CI_WORKFLOW` and exits 0 on no run.
- So the configuration's pin changed, and every operation's first argument is rewritten to it.

## Checks

```
FARCOOLER_OV1071_BUNDLE=$PWD/.claude/agent/reports/ov-1071/workflow-main.json \
  CARGO_BUILD_JOBS=4 cargo test -p farcooler-daemon --lib main_bundle -- --ignored
```

That test is `main_bundle_tests.rs`. It checks that the file round-trips as `set-workflow --file` reads it, that every
operation pins the configuration, each of the four acceptance items in the file, and that the longest fix-round brief fits one
launch command. It also runs the live configuration's own `validate_definition` on this runner, with its pinned hook files on disk.

## Publishing (the owner, from your own shell)

```
farcooler-canary workflow get <workspace-id> --json     # revision, epoch, sequence, and the current pin
farcooler-canary workspace set-workflow <workspace-id> \
  --file .claude/agent/reports/ov-1071/workflow-main.json \
  --expected-pin <the current pin> --command-id "$(uuidgen)" --revision <revision> --epoch <epoch> --sequence <sequence+1>
```

The daemon has to include this branch first: an older one refuses the review gate's third argument and the CI step's
fifth. If the live publication has moved on from ov-1087's bundle, apply the same edits to it instead. Then rerun the test with
`FARCOOLER_OV1071_REPIN=1` to rewrite the pins.
