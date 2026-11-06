# ov-928 full source handoff for independent security review

The commit containing this handoff is the frozen source pin, stacked on signed
phase1 `b2329453fb64074b52164885067050f646af4e33` (baseline runtime7038).
Phase1 handoff/evidence are preserved unchanged. This continuation implements its
remaining native-host and private-remote landing gaps; it does not claim live
provider, external push, installed Service activation, or integration acceptance.
Independent full-source security review is required before integration.

## Native API and ownership

`Runtime::create_configured` / `Runtime::open` now accept the existing
`CodingHarness::Native` descriptor after validating pinned existing lifecycle
hooks. Use a short private runtime root and existing hook files already naming
`<root>/h.sock`. Claude uses existing settings; Codex/Cursor project hook bytes
must already exist in the provisioned worktree. No installer, config merger,
credential write, new provider SDK, ACP, or new TUI is invoked. Existing provider
TUI launch is built by `service::preset_command_with_hooks`, with explicit native
session identity and pinned process environment. TERM/COLORTERM have explicit
native defaults. This remains trusted-local construction; wire/auth/client and
installed Service authority are unchanged.

`coding_native.rs:176` launches through retained `TmuxServer` managed PTY/window
mechanics, holding a stdin gate until tags and the full owner binding are durable.
The real interactive standard input is retained. A private socket receives the
existing HookLine protocol and normalizes via retained facts/MessageAssembler;
selected agent, cwd, session and submitted prompt must correlate. Foreign or
subagent Stop, prose and exit status cannot create successful completion. No
permission approval is emitted. Raw correlated hooks and PTY output persist.

`coding_native.rs:87` verifies actual pane/daemon/worktree/terminal/tty tags,
freezes the tty roots plus descendants using retained process-table mechanics,
terminates them and observes quiescence/reaping before publishing settlement.
Then the existing configured engine verifies a clean descendant Git candidate.
Timeout/cancellation stops the same owner. After a crash, ordinary reconciliation
never restarts or kills unknown work; an admitted Stop can quiesce the original
bound live PTY (`coding_native.rs:217`) and settle the original operation first.
Missing/foreign/gone/dead ownership without quiescence proof remains uncertain.

## Retained landing identity and recovery

`CodingConfig::landing: Option<CodingLanding>` adds immutable job key, canonical
local bare repository, target ref, and pinned config digest. A lifetime remote
writer lock complements the selected repository's writer lock. `None` preserves
phase1 local CAS behavior. `coding_landing.rs:19` uses retained landing target
construction with real Git push under the existing owned argv supervisor.

The push is restricted to the selected canonical filesystem bare remote. Its
candidate must descend from the pinned per-item base, and the explicit expected
old-SHA lease supplies exact CAS; it cannot overwrite non-descendant history.
Transport receives a cleared environment and Git guard pins, system/global config
is disabled, and effective URL/include redirects are refused. No provider/network
remote is accepted by this adapter. Existing installed landing-job behavior is
unchanged apart from extracting its target formatter.

Actual bare-ref observation plus quiescent bound push settlement supplies landing
acceptance, followed by local exact Git CAS. Identity, argv/environment, stdout/
stderr, exit and digest facts are retained. Remote acceptance followed by lost
settlement or local-CAS failure stays uncertain; reconciliation does not push.
CI verifies the exact landed SHA/identity and retains phase1 provider receipt gates.
No deterministic fixture asserts actual signed-in CI/provider success.

## Verification and limits

`continuation-source-sha256.json` must match final continuation runtime, retained
landing, and Clippy check records. Checks use original923 warm target, exclusive
normal FIFO, profile debug0/test0/incremental0, jobs6, offline dependencies,
load5<20, 10GiB reserve, and the owned 1800-second cancel/reap supervisor. Checked
actual holders after lock: only this continuation's Cargo/rustc used the target.

- `continuation-red`: initial import compile error, exit101, unchanged source.
- `continuation-first-red`: runtime46/52 green; wrong retained hook classifier and
  repeated-item fixture content caught before correction; uncertainty-test helper
  also wrongly demanded terminal phase. The fixes retained production gates.
- `continuation-native-red`: deliberate removal of session correlation caused the
  foreign-only hook fixture to become Done, failing as intended (exit101).
  `continuation-native-mutation.json` records identical original/restored hashes;
  restoration preceded final checks/signing.
- `continuation`: final runtime54/54, real local Git and real private tmux PTYs for
  Claude/Codex/Cursor with deterministic interactive stand-ins; normalized-hook
  rejection, provider failure, live cancellation, original-owner Stop after
  reopen, foreign tty refusal, settled-crash recovery, exact private remote push,
  repeated bases, rejected ref-lock update, changed-base refusal, and missing
  post-push settlement without reexecution. Phase1 runtime fixtures remain green.
- `continuation-gate`: retained landing-job regressions, including real private
  Git push; `continuation-lint`: daemon all-target Clippy with warnings denied.
- `continuation-size-budget.json`: only preexisting unchanged `rpc.rs`2482 exceeds
  ceiling2450. ov-929 owns that extraction. All changed/new files meet the budget;
  no ceiling was raised. `git diff --check` passes.

The Native TEST_ARGV seam is cfg(test) only, so no real provider was launched.
Live signed-in smoke, network push/provider auth, whole-app cutover and main remain
explicit later allocations. This source stops for independent security review;
no push, integration, install/update, owner database, secret/config/security write,
proto/client/native/wire/Store-schema change, formatter, stash/reset/clean/amend,
or add-A was performed.
