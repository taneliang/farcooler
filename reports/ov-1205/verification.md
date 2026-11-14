# ov-1205 verification

Read the ticket with the installed Canary CLI and read the three inherited
ov-1188 commits before editing. Rebased those commits onto the run-start base,
`2cc183694e6924141097c2893c36cc07f3f76452`, preserving their steering work.

## Failing-first evidence

Codex CLI **0.167.2**, model **gpt-6.1-sol**, scratch Codex homes and scratch
repositories only. No owner Codex settings were changed; no `cargo fmt` ran.

- In scratch thread `01a1da17-c6b0-7fe0-8256-9108c0428361`, queued direction
  while the first turn was waiting on a shell command. `codex queue` exited 0
  at approximately 12:46:17 UTC. The first turn then wrote `ORIGINAL` and
  completed at 12:46:25.664. The queued instruction became a user message at
  12:46:25.693, in a new turn. Queue success does not prove mid-run delivery.
- The expanded regression
  `native_direction_reaches_both_harnesses_before_their_next_tool_runs`
  failed against the original handler: “direction must arrive before the tool
  call, without a Stop.” Claude passed; Codex left its direction queued.
- A scratch linked-worktree engine probe also wrote `ORIGINAL`. Codex’s
  `/hooks` panel reported zero discovered hooks, although the engine had
  copied the project hook file. Its native receipt had no hooks. Adding an
  empty project config and trusting both repository and worktree did not make
  that probe discover them. This is why the fix supplies pinned hooks through
  per-invocation configuration as well as retaining the checked project copy.

Read R-77 using `plan ruling list`: its concern is an exit-0 queue operation
without evidence that the TUI read it, and duplicate delivery after timeout.
This change does not use the queue side channel or change its opt-in policy.

## Implementation and live verification

Native Codex configuration now includes the all-tools, blocking `PreToolUse`
registration. The runtime requires that exact registration when opening the
configuration and before launching, just as it already did for Claude. The
engine passes the required, validated engine registrations as a TOML
`--config` argument too,
so a skipped project layer cannot silently remove its delivery boundary.
The launch argument is shell quoted; the pinned hook file and worktree copy
remain checked. Ordinary pane hook registration is unchanged. Expected argv
reconstruction uses those engine registrations without reading launch files,
so a changed or missing hook file cannot block cancellation or recovery. The
recovery regression now exercises Claude and Codex for each drift case.

Both harnesses receive direction as a denial of their next supported tool,
using `permissionDecision: deny` and `permissionDecisionReason`. No allow or
permission bypass is emitted. An already executing tool continues. Stop still
handles direction between tools and fences final settlement. Admission,
undelivered-message recording, and the exact-step recipient remain intact.
Duplicate Codex Stop notifications from multiple config sources replay the
same continuation rather than performing another final empty-queue check.

A standalone live hook probe, thread
`01a1da19-188d-7b11-9a9c-744f6f94d391`, blocked the pending ORIGINAL write at
12:47:48.263 UTC. Codex wrote `STEERED` at 12:47:50.496 and finished that same
turn at 12:47:53.728.

The opt-in `running_codex_receives_ticket_direction_before_its_first_stop`
test runs the real Codex TUI under the engine’s retained PTY, against a scratch
board and repository. It sends a ticket message through the real
`message.send` handler while the first shell tool is running, and uses the
built CLI for the engine’s hook registrations. With per-launch hooks, it
passed in 38.66 seconds: the result was `STEERED`, committed candidate
`3cc126f270b2631b9b43388f301cb1de359a4ff7`, with SessionStart,
UserPromptSubmit, PreToolUse events, and **one Stop**, so no Stop continuation
was needed. The message queue was empty afterward.

The test uses a disposable `CODEX_HOME`, a copied scratch login, trusted scratch
project entries, and the one-invocation hook-trust bypass for the pinned test
hooks. It uses automatic approval for scratch Git operations; the initial
never-approve probe could not commit to the linked worktree’s Git directory.
Production retains Codex’s normal hook-trust flow. Updated workflow pins and
hook trust are required; the older lifecycle-only configuration is refused.
The ticket’s unsupported-input fallback is unnecessary for the tested Codex:
its tool hook safely delivers mid-run input.

The official [hook reference](https://learn.chatgpt.com/docs/hooks) documents
the denial shape, hook trust, and tool coverage. Hosted tools and polls of an
existing unified-exec session do not fire another PreToolUse. The
[configuration reference](https://learn.chatgpt.com/docs/config-file/config-basic)
documents per-launch configuration precedence. These limitations are also
recorded in `docs/engine-steering.md`.

## Automated checks

Tests remove inherited native FARCOOLER launch variables, use TMPDIR=/tmp,
and build into a scratch target directory. Build the CLI first for PTY tests.
The ignored live test additionally needs FARCOOLER_LIVE_AGENTS=1 and
OV1205_CODEX_AUTH pointing to a scratch auth.json. It creates its own new
Codex home rather than running in the supplied login directory.

- Failing-first handler regression: reproduced, then passed.
- Initial native regression/control/recovery run: 38 passed, one live test ignored.
- CLI hook tests: 18 passed, including both harnesses’ denial output.
- Final native regression run, including duplicate Stop continuation: 38 passed, one live test ignored.
- Final live rerun after duplicate-Stop protection: passed in 38.10 seconds,
  committed `e0910d3acc7c453a8155fb5855b041bf82f0d31c`, with four PreToolUse
  events and one Stop.
- Pinned-hook TOML round trip and native launch wiring: passed.
- File-size budget, doc comments, swallowed-call lint, copy lint, and whitespace checks: passed.
- Codex and Claude recovery with changed/missing hook settings or missing executable:
  passed in the final 38-test native run.
- `cargo clippy -p farcooler-daemon -p farcooler-cli --all-targets -- -D warnings`: passed.
- `cargo nextest run -p farcooler-daemon -p farcooler-cli --no-fail-fast`:
  **4,321 passed, 18 skipped**, 513.904 seconds. The final argv-recovery
  adjustment was additionally verified by the 38-test native run and refreshed
  clippy; it reconstructs the same engine arguments without mutable-file reads.


## Fix round 1: explicit native direction with answer wake disabled

Read the newest ticket finding and the inherited branch commits before editing.
The run-start base remains `2cc183694e6924141097c2893c36cc07f3f76452` and is
already an ancestor of this branch.

The expanded `native_direction_reaches_both_harnesses_before_their_next_tool_runs`
regression now sends through the real `message.send` handler, then exercises the
native `PreToolUse` handler, for Claude and Codex with `wake_on_answer` both on
and off. Failing-first checks exposed two independent gates on this checkout:

- Before any production changes, the disabled-preference case failed admission
  with `Conflict { what: "typing_off" }` in the store.
- After introducing native-only admission, the same case queued successfully
  but failed the assertion that direction must arrive before the next tool.
  This confirmed the reviewer's early-return finding in `native_followup`.

Explicit native messages now queue through a separate store entry point while
holding the exact live agent's existing admission lock. It uses the same atomic
note/queue transaction and workflow-incarnation fence, and refuses runner
notices. Ordinary pane messages and automatic notices retain their wake
preference. Native tool and Stop hooks consume admitted direction independently
of that preference. The regression also checks single consumption; the final
Stop regression checks continuation and closure of late-message admission with
the preference both on and off for each harness.

The initial sandboxed test attempt could not initialize its scratch Git fixture;
rerunning with the required Git/socket access reproduced the actual failures
above. No owner Codex settings were changed and no `cargo fmt` ran. The original
scratch live-Codex evidence above remains applicable to the unchanged hook
transport; this round exercises the admission and delivery preference through
the real handlers without launching another paid Codex session.

Round 1 validation:

- Focused daemon message and native-handler regressions: **27 passed**.
- `cargo build -p farcooler-cli`: passed before the broader tests.
- `cargo nextest run -p farcooler-daemon -p farcooler-cli -p farcooler-store --no-fail-fast`:
  **4,976 passed, 21 skipped**, 539.479 seconds.
- File-size budget, doc comments, swallowed-call lint, copy lint, and whitespace:
  passed.
- `cargo clippy -p farcooler-daemon -p farcooler-cli -p farcooler-store --all-targets -- -D warnings`:
  passed.
