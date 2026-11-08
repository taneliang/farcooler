# ov-966: the orchestrator picks the model per step

From the owner's answer to ov-964 item 1 (option A): the workflow lists the models a step may run
with, and a default; the orchestrator names one at Start; the run records it.

## Where the model lives today

- `CodingHarness::Native { preset, .. }` in the runtime's `coding.json` (`crates/protocol/src/workflow_config.rs:21`)
  holds one preset, such as `claude:opus`. Every build on every ticket launches it
  (`coding_execute.rs:92`, `service.rs:372` turns `harness:model` into `--model`).
- The build step's operation is pinned to `coding-harness/v1` with an empty argument (`coding.rs:174`), so a
  definition can't say anything about the model.
- `Command::Start` (`state.rs:146`) and `Action::Start` (`workflow_commands.rs:38`) take no model.
- `agent dispatch --harness` on a tracked ticket is refused as a flag the engine would drop
  (`plan_agent_dispatch.rs:85`).

## Design

1. **Definition.** `Stage` gets `models: Vec<String>`: the presets the step may run with, the first being the
   default. Serde default and skipped when empty, so every published definition and its pin are unchanged.
   `Definition::validate` allows it only on a `Build` step, at most 8, distinct, each `harness` or
   `harness:model` in plain identifier characters, at most 64 bytes.
2. **Start.** `Command::Start` and `Action::Start` get `models: BTreeMap<step id, preset>`, skipped when empty
   (old receipts and payloads hash the same). The reducer refuses (`Invalid`) a step that lists no models or a
   pick the step doesn't list. The item records `models`: every listed step, with the pick or its default.
   `Enqueue` records the defaults the same way.
3. **Effect.** `Effect` gets `model: Option<String>`, the item's record for the step it executes (skipped when
   `None`). The executor reads the model from the effect alone, as it reads everything else, so a recovery
   launches the same thing.
4. **Daemon.** A native build launches `effect.model` in place of the configured preset. The hooks it waits for
   are that harness's: `settings` for Claude, `project_hooks` for Codex or Cursor (its path says which).
   Publishing refuses a definition listing a model whose harness the configuration has no pinned hooks for, so
   a typo or a harness that isn't set up never reaches a run. A command harness gets the model as
   `FARCOOLER_WORKFLOW_MODEL`.
5. **CLI.** `workflow start --model <step>=<preset>` (repeatable). `agent dispatch --harness X` on tracked
   tickets sends `{the build step that lists models: X}`; a workflow with no such step, or more than one, is
   refused with what to do. `--harness` leaves the dropped-flag refusal; `--worktree`, `--new`, `--branch`,
   `--base`, `--title` and `--again` stay in it until ov-967.
6. **Start refused for open work (ov-965 minor).** The store refuses Start with `Resource` while the card has an
   open subagent record or a block. Dispatch's words for that refusal now say both causes and the commands that
   settle them.

## Where the brief is off

- "Allowed models per harness step": the only harness step is `Build`; review steps run a configured command,
  not an agent (that is ov-964 item 4). So the list is on `Build` only for now.
- The ov-965 refusal is the store's `Resource`, shared with "another run holds the resource". A new `Refusal`
  variant would break an older reader of the receipts, so the words carry both causes instead.

## Test

Acceptance: two tickets started on one workflow with different models each record their model, and each
build's launch carries it (`coding_tests.rs`, through the wire). Reducer: refusals for an unlisted step and an
unlisted model, defaults recorded. Definition: lists off `Build`, duplicates, bad names. Pinned: a model whose
harness has no hooks is refused. CLI: `--harness` maps to the step; refusals for none or two.
