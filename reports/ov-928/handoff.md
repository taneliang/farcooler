# ov-928 source handoff (incomplete task)

Baseline: `7038e170476ee842ebb9e741005516a9b0bb82bb`. The signed commit containing
this handoff is the source pin; `source-sha256.json` identifies exact source bytes.
No integration, push, app activation or owner-state change is authorized or claimed.

Implemented: real configured local coding argv, selected canonical Git repository,
initial SHA/local landing ref, immutable per-item base, multiple configured checks,
optional command reviewer, approval progression, bounded rework, local Git landing
CAS and exact-SHA CI provider receipts. Fixture construction/dispatch stays intact.
There is no fixed coding script or generated CI success in configured dispatch.

**Still incomplete:** existing interactive native terminal/hook ownership and
retained remote landing-job identity. Native configuration descriptors/rendering
are source-tested, but constructors refuse native execution. The existing preset
builder launches an interactive TUI (`service.rs:366`); the owned command runner
uses null stdin (`effect_runner.rs:54`) and cannot substitute for its terminal or
hook-completion owner. The orchestrator has the concrete coordination request with
ov-927. `wire_commands.rs` was restored exactly to baseline because manager decision
b1038644 reserves it for ov-927. Configured definitions have trusted-local API only.
The task stays **in_progress**, not accepted as complete. Independent security
review remains requested before any integration. Signed-in provider/native smoke,
remote push, whole-app cutover and main gates need explicit later allocations.

## API

Public exports live in `farcooler_daemon::workflow_runtime`:

- `CodingConfig`, `CodingCommand`, `CodingHarness`, `CodingReviewer`, `CodingCi`,
  `LaunchFile`, `CodingReviewReceipt`, `CodingCiReceipt`.
- `Runtime::create_configured(root, config).await` creates a new private runtime
  root and its own database/receipts. The selected scratch repository already
  exists, its landing ref must initially equal the full configured SHA, and its
  `.git` gets a lifetime exclusive workflow-writer flock. Different runtime roots
  cannot concurrently mutate the same repository. `Runtime::open(root)` resumes
  exact persisted config under that lock; no legacy Service selector changes.
- `runtime.configured_definition()` builds all required pinned operations. Set
  stage `progression`, `max_attempts`, and bounded review policy before enqueue.
  Removing/changing required operation/config pins is refused. No reviewer means
  no review stage or rework edge. Checks carry their own ordered index and config.
- Mint `controller()`, then `submit(&controller, Request { ... Command::Enqueue ... })`.
  Use the existing exact revision/sequence/id semantics and canonical `resource()`.
  Policy pins are included in every effect/receipt. `decision_session`/`decide`
  handle approvals as before; controller/result grants are not wire inputs.
- Mint `command_handle(&controller)` before `run(shutdown)`. Submit, decide, state
  and cancellation remain responsive while configured processes execute. A
  trusted delayed-receipt observer invokes `handle.reconcile()` with no supplied
  result. Idle execution waits on inbox/real claim deadlines, not ACK or history.

`CodingCommand` uses an absolute executable argv, an explicit cleared environment
and a bounded timeout. Build must create a clean actual descendant commit itself;
neither the adapter nor an exit message invents a commit. Checks/review/CI verify
the exact candidate before and after execution. Reviewer command exit zero is
insufficient: it must publish `CodingReviewReceipt` at `FARCOOLER_RECEIPT`, with
`FARCOOLER_OPERATION_BINDING`, configured identity, exact candidate and verdict.
CI publishes `CodingCiReceipt` with that binding, selected repository and a retained
`ci_watch::GhRun` JSON object. Nonzero/missing run ID, wrong workflow/event/SHA,
missing/foreign receipt, or pending CI cannot assert success. No provider SDK.

Context variables additionally pin run, fence, resource generation, definition,
policy, candidate, base and iteration. Logs and quiescent process facts persist
outside the coding worktree, in the private runtime receipt directory.

## Recovery and cancellation

`coding_execute.rs:108` writes process settlement only after the group is killed,
reaped and observed quiescent, including descendants. The shared supervisor retains
landing-gate environment behavior. Exact argv/environment/output digest and full
binding are checked during recovery. Git settlements are written only after the
retained guarded operation returns; recovery also checks actual common-dir, owned
branch, full HEAD and clean tree. Local landing additionally needs the exact CAS
reflog identity. A visible Git ref/worktree alone is insufficient.

`coding_execute.rs:213` reconciliation has no adapter-command launch. Missing,
malformed or foreign facts leave the effect uncertain with ownership held.
Per-item base publication precedes durable effect admission; later items select
the then-current local landing ref exactly once. Rework/stop/reopen reuse that pin.
Cancellation signals the owned group while the inbox remains available. Stop
reconciles the original operation even if Cancel fenced its Observe or its first
journal was uncertain; late exact provider facts may then settle it. Unknown
external acceptance stays uncertain. Cancellation does not claim to undo landing.

This is trusted-local runner configuration/receipt storage, not a sandbox against
hostile same-UID code. No public arbitrary result/Resolve override was added.

## Verification

`check.py` records exact commands, exits, before/after source SHA256, baseline,
elapsed time and monitored minimum free bytes. All checks use the original ov-923
warm target, normal exclusive FIFO, load5 below20, jobs6, offline dependencies,
debug0/test0/incremental0, 1800-second process-group deadline and 10GiB reserve.
Host preflight found no Cargo/rustc/heavy holder, load5=3.94 and about36GiB free.
Toolchain: native arm64 rustc1.98.1/cargo1.98.1 and `/usr/bin/python3`.

- `initial-red.*`: first compilation succeeded; tests caught semantic-vs-dispatch
  reviewer policy and a test that failed its command before testing dirty HEAD.
- `native-path-red.*`: 41/42 passed; a temporary path alias in the settings test
  was corrected to a canonical path. The API alias check was kept strict.
- `binding-red.*`, `binding-mutation.json`: deliberate removal of the CI binding
  gate makes the foreign-provider test fail with exit101. The script restores
  exact original bytes in `finally`; original/restored hashes match.
- `binding-green.*`: restored source rejects foreign CI/reviewer bindings.
- `runtime.*`: configured integration plus preserved runtime, process, admission
  and wire fixtures. Real local Git/configured commands cover bounded rejection
  rework, multiple checks, repeated bases, approvals, dirty/prose-only candidates,
  failed/missing/foreign/pending CI, every stage's settlement-before-journal crash,
  unknown admitted work, visible Git without quiescence, changed-base landing CAS,
  running-process cancellation and late-provider cancellation settlement.
- `gate.*`: retained landing-job regressions; `lint.*`: daemon all-target Clippy
  with warnings denied. Every final source hash must match `source-sha256.json`.

All repositories, databases, settings files and provider hooks used by tests are
private scratch data. Deterministic provider observations are not live-provider
acceptance. No formatter, stash/reset/clean/amend/add-A, target deletion, cold
duplicate target, owner database, signed-in provider, remote push or install used.
