# ov-931 source handoff

Implementation follows accepted preparation `04fbf0b` and authorization
`ab517f36`. Source checkpoint `a39b32d21` published the shared bundle API; final
freeze and exact source/evidence hashes are recorded in `freeze.json`.

## Source and behavior

* Ordinary exclusive daemon bootstrap calls `Service::start_workflows`; Service
  retains one runtime task, shutdown signal and authenticated command handle.
  Ordinary RpcFactory derives its handle from Service. Secondary Service opens
  never start an executor implicitly, and a second explicit owner refuses.
  Existing stdio forwarding reaches the ordinary daemon when it is listening;
  standalone fallback does not become another workflow writer.
* Service-owned runtime shares the Service Store. Versioned publication persists
  immutable definition/configuration bundles, selected bundle and authenticated
  receipt atomically. Items/effects already carry `policy_pin`, so dispatch,
  cancellation and reconciliation resolve their original bundle. The old
  definition selection table is inert migration input, not a second active
  selector. See `bundle-api.md` for exact public types and pin semantics.
* Actual registered repository identity, base/ref, finite command/native/CI/
  review/landing configuration and bounded file inputs are checked. Admission
  retains the consumption-time Session/caller proof, with a second recheck after
  bounded publication identity I/O and before synchronous receipt/transaction.
  Committed receipt replay bypasses mutable fresh-publication checks.
* Adapter validation accepts adapted IDs/source, extra build/check stages,
  configured rework destination, any-stage approvals, person review policy,
  omitted checks/review, and post-landing checks. Pure Definition validation
  remains authoritative. The optional archetype no longer defines mandatory
  engine shape. Landing/candidate/CI safety prerequisites remain explicit.
* Enqueue reserves durable execution ownership in the same transaction as state,
  event and outbox. Existing own-board key/UUID resolves to its board task;
  standalone items persist an explicit NULL task mapping. Existing legacy
  terminal/line/work/landing claims refuse engine admission, and DB triggers
  refuse subsequent legacy writes to engine-owned tasks. No owner tasks were
  migrated, and no per-workspace engine switch was introduced.
* Runtime wakes on its owned command/reconciliation inbox and lease deadlines.
  Shutdown signals active command cancellation, observes bounded settlement and
  joins before Service releases ownership; shutdown join failure propagates.
  Informational notification ACKs do not participate in routine execution.

## Evidence and limits

`affected.log/json` records **87 selected passing cases**: daemon library 60,
store 16, protocol 1, CLI producer/parser 2, ordinary Service socket/process 6,
and retained fixture socket 2. The ignored ordinary-service child helper is
explicitly selected in its separate-process test, not counted as a parent pass.
Zero-selected binaries in earlier broad-filter attempts are excluded. The mechanical
`adapter:None` addition to one client test fixture is source-hashed but outside
this bounded check allocation; no client/platform test pass is claimed.

Ordinary tests use Service::open_in with isolated state, the exact bootstrap owner
method used by main, normal RpcFactory (no workflow fixture injection), shared
preamble-to-peer mapping, real transport Sessions and kernel caller PID. Clients
run in an owned isolated real tmux terminal because the actual guard correctly
rejects Cargo's Codex ancestry as an unknown agent. No fake PID, caller-seat
injection, test_stub_agents flag or auth weakening is used.

The ordinary evidence includes A surviving B publication plus restart/rework;
extra build, renamed IDs, a different rework destination, multiple checks with a
post-landing check, automatic/every-stage approval policy; full coding completion
while disconnected with no ACKs; separate-process restart and daemon.shutdown;
active-command reaping/no relaunch; fast ship and no-Git; and an already-admitted
uncertain CI effect resolving A after B publication/restart with identical process
fact bytes and only one CI effect. The latter updates the isolated trusted receipt
and wakes reconciliation by a read command; it does not claim an FS watcher or
new external CI integration.

`negative-config.log/json` fails the ordinary A/B regression when item-bound lookup
is replaced with current workspace selection. `negative-owner.log/json` fails
with Ok(1) instead of Resource when legacy exclusion is bypassed. `mutations.json`
records both original/mutant hashes and exact restoration. No mutant was committed.
The restored affected run passes. A subsequent clippy manual_map finding was
fixed with the equivalent Option::map conversion in the resolver; the final six
ordinary cases passed again against that source. Final source is `4e01f8be46e28603a02e7464a984828ab07e54a4`. Both pre-lint source freezes
remain signed and unchanged. `lint.log/json`, `size-budget.log` and
`product-diff-check.log` record the remaining targeted gates. Raw terminal logs
are retained verbatim, including terminal trailing spaces.

All heavy phases use normal FIFO, the original ov-923 warm target, jobs 6,
dev/test debug 0, incremental 0, offline dependencies, load5 below 20, a 10 GiB
reserve monitor and owned 1800-second deadlines/kill/reap. Tests use one writer
thread; the retained FIFO child deadline remains 10 seconds. `entry.py` verifies
actual empty cargo holder and cache provenance after FIFO admission. It refreshes
local crate-root mtimes without changing bytes to prevent relative dep-info from
accepting a newer sibling-worktree library. No target deletion or cold duplicate.

Source framing corrections and retained reds are documented: the existing wake
loop already existed; secondary Service opens are not bootstrap owners;
`task_workers` is historical and removed by line migration (current authority
uses ticket_work/agents); pure Definition requires Build before Check, so an
incorrect preflight acceptance assertion was corrected without changing approved
core policy; actual guard/scope refusals were preserved while fixing the isolated
caller harness. `cache-investigation.md` records the stale-library diagnosis.

`composition.json` pins the approved 930/929/corrected-928 inputs and exact
ordered cherry-picks. Every changed input blob matches the composed union except
workflow_runtime.rs, whose retained reviewed-930 admission-context addition is
shown in `composition-overlap.patch`. New ov-931 deltas are independently reviewed
as new source. `input-signatures.*` verifies original reviewed signatures and
source checkpoint/union signatures. Original references/history are preserved.

Acceptance 1–3 have the bounded source evidence above. Acceptance 4 awaits
independent security review of the signed final source/composition. Full platform
main CI, owner-data rehearsal and whole-app cutover remain ov-921 obligations.
No installed app enable/update, owner-data rehearsal/migration, provider network
call, external push, TUI/ACP/new SDK or provider breadth was performed here.
Only authorized task metadata was updated through the installed task CLI.
