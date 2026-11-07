# ov-941 scope correction handoff

Base is exact signed54ad7d530cfb6a9f2abfcc2212bca481a61fec3d. This correction
supersedes the extra unshipped saved-file Retry interface, not an owner requirement.
Read scope-contract.md and correction-source.json for allocation and final hashes.
Original identity-seam.md/source-hashes.txt remain historical source research.
Absolute scope disposition verified SHA256
43f2bdbd409de361988b3bf0a70b5c3f039dc62cce3bffd9fed81501f586d23d.

## Result and invariants

Rework uses existing action -> connect_to(selected runner) -> send. Removed Retry,
mandatory prepared-file, CLI-only prepared save/load/request/send and lazy connector.
Reason validation still calls the common Rust validator; direct request validation
now calls the existing common action parser without changing serialized bytes.
Evidence reads remain bounded regular-file/no-follow/nonblocking. Identity, reason,
evidence values, run/pin, revision/epoch/sequence and selected runner are explicit.
No snapshot refresh, new mutation ID, resampling, automatic retransmission or
retargetable saved-command recovery. A caller repeating an invocation must retain
unchanged evidence contents and the same authenticated destination/principal.
A different destination is a new invocation, not recovery of the original recipient.
Existing engine run retry/uncertainty controls are unchanged.

Only workflow.rs production code changes. The original entire daemon wire file
(production and existing tests) is an exact unchanged prefix, with one private test
module appended. No public API/main/registration/Store/shared/Session/native/Web,
journal, auth/remote/transport/protocol/schema/capability/security-default changes.
No other source changes; unrelated pre-existing .codex/.cursor remain untouched.

## Proof and meaningful controls

* cli-focused: initial4 passes (private emitter ignored until seeded sequence).
* rust-receipt: initial seed/production CLI emit/real receipt consume pass.
* old-source-parser: expected101 at removed-Retry parser assertion against exact
  old production prefix plus new tests. Exact restoration recorded in
  old-source-control.json before subsequent deliberate common request-validation
  and final golden/workspace assertions; cli-restored binds those final bytes.
* rust-identity-control: expected failure at successful same-receipt equality.
  One temporary wire-ID change derives a new identity from current revision:
  original wire:owner:cli-rework:4 Ok5 becomes wire:owner:cli-rework:5 Stale.
  This is an actual real Store stale receipt, not a mock or stale-as-success proof.
  Merely bypassing Runtime lookup would still hit the Store transactional lookup;
  the fresh-identity control is necessary to expose blind reprepare semantics.
  Exact wire file restoration recorded in identity-control.json.
* cli-restored: final5 passes, seeded emitter intentionally ignored here and run
  explicitly by receipt-check.py. Includes legacy Review golden bytes, removed
  syntax, exact reason, max-u64 mutation fields/repeated payload and input bounds.

The private ignored daemon fixture creates one closed Runtime and exports the
actual WaitingReview observation. The compiled CLI parser/action/request emits
schema1 JSON payload twice from the whole unchanged invocation, checks selected
runner, target workspace and capability. The private daemon consumes those exact
bytes with the existing parser and real Runtime::wire_command/Store receipt path.
The test requires successful typed Receipt, one event/history entry/rework increment,
immediate duplicate receipt equality, discard-answer/drop/open/retransmit equality,
and unchanged complete state/events. Changed reason/evidence/run/pin/revision/epoch/
sequence under the same ID each require typed IdentityConflict and conservation.
Transport request IDs are intentionally per-request; mutation identity is unchanged.

These are private real receipt/engine fixtures with explicitly supplied existing
owner principal. Reopen/new handle models lost-answer/reconnect at that boundary;
this does not prove socket/OpenSSH authentication, durable original-recipient
identity, real runner reconnects or UI/app/platform behavior. Existing caller
authority implementation and accepted admission/security proofs remain unchanged.
No mock server or new public test API was introduced.

## Resource and review boundary

Each heavy check uses native Python, canonical normal FIFO/load5<20, original923
warm target, post-lock live-holder/source/profile observations, jobs6/dev-test-debug0/
incremental0/offline/locked, >=10GiB reserve, owned1800s foreground deadline and reap.
Preflight/cache-holder logs bind roots/profiles; final runs also record actual
Cargo/rustc/PATH identity. Check JSON records source before/after, raw argv/log,
exit, duration/free reserve/reap (final wrapper also records output log bytes).
No cold target, cleanup, forged lock flag, auto-retry or accepted46/platform suite.

Static checks preserve an initial sandbox-denied native py_compile cache write;
AST parsing without cache writes succeeds instead. Whitespace, allocated source
line ceilings and unchanged daemon original prefix are verified in static-checks.log.
No product execution used owner databases/configs/keys/providers/app/device/network.

Stop at signed frozen correction for SAME scoped source/evidence review. Actual
root/app/platform A/B and whole release proof remain parent932. No automatic fullB,
platform closure, owner requirement waiver, merge or release claim follows here.

Final rust-receipt-restored: seed/CLI emit/consume all pass against final source hashes.
Exact final payload/retransmission/observation/evidence/typed receipt copies are
in receipt-proof/; payload hashes match the retained raw log.

Final rust-lint: CLI/daemon all-target Clippy --offline --locked -- -D warnings:0.
All seven check records have expected exits, unchanged sources during execution,
reaped children and >=10GiB reserve; final CLI/receipt/lint share final source hashes.
SAME scoped independent review remains pending at the frozen signed forward pin.

Staged whitespace: authored rs/py/md/json/jsonl check passes. Full check reports
two verbatim failed-libtest progress-line trailing spaces in preserved raw control
logs (old-source-parser:28 and rust-identity-control:113); no log normalization.
