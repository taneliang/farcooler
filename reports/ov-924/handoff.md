# ov-924 source/API handoff

Base: `4163081e91353481ae00e80ff7bd3cbab0924915`.
Final source pin: `18c724ab0` (normal PGP-signed commit, verified exit 0 in
`source-signature.log`), preceded by signed API commit `4d760f68f`.
Branch: `ov-924-workflow-presentation`. Stop at this source for independent
scoped review. This evidence commit adds reports only.

## Concrete contract and acceptance

- Shared Rust card/gate/reason/action contract: `crates/client/src/workflow_presentation.rs:6`
  (v1), `:42` (RunCard), `:80` (project), `:161` (action), `:201` (JSON).
  Current stage and gate principal/type are already projected. All stage policies,
  run/policy/definition/candidate/evidence/fence identities, uncertainty and rework
  bounds are retained. No platform derives domain rules.
- C bridge: `crates/client/src/workflow_presentation/ffi.rs:9`; header declaration
  in `crates/client/include/farcooler_client.h`. JNI:
  `crates/android/src/workflow.rs:5`; WASM: `crates/web/src/workflow.rs:3`.
  All delegate the same JSON implementation. C uses existing panic guard, null,
  required-size and untouched-short-buffer rules; JNI uses existing conversion
  and two-call buffer helper. No new FFI framework, registry or transport.
- Feature A: `EvidenceInvalidated` derives from retained evidence candidate/policy
  mismatch in Rust. `InspectLatestEvidence` returns the exact retained identity,
  including invalidated evidence; review/rework cannot use it as current proof.
  Shared fixture: `crates/client/src/testdata/workflow-presentation.json` contains
  automatic, every-stage approval, rework, uncertain and invalidated inputs/outputs.
- Action click validation reprojects against the latest State and current scoped
  capabilities. Guards bind snapshot/context/item. Commands remain intents;
  authenticated transport must reauthenticate and apply the reducer/store checks.
  The JSON DTO is explicitly not an authentication boundary or credential.
- Source/API and verification are ready. Independent scoped review has been
  requested from the orchestrator against **18c724ab0**; its result is pending.
  Acceptance 4's independent review part must not be marked complete yet.

## Exact focused checks and raw exits

Every Cargo row uses native Python `reports/ov-924/run-checks.py MODE`, normal
`scripts/heavy --load-limit 20 -- cargo ...`, jobs 6, offline, debug 0,
incremental off, owned process group/deadline and a 10 GiB reserve. Full commands
and raw exits are in each log. Own `target` was APFS-cloned from the inspected
runtime donor; Web uses this same owned target, not another cold native cache.
Provenance is in `cache-provenance.json`.

| Mode / log | Cargo arguments after cargo | Latest raw exit / result |
| --- | --- | --- |
| focused.log | test --offline -p farcooler-client --lib workflow_presentation -- --nocapture | 0, 5 passed |
| web.log | test --offline --manifest-path crates/web/Cargo.toml --test workflow_presentation | 0, 1 passed |
| jni.log | check --offline -p farcooler-jni | 0 |
| wasm.log | check --offline --manifest-path crates/web/Cargo.toml --target wasm32-unknown-unknown | 0 |
| clippy.log | clippy --offline -p farcooler-client --lib -- -D warnings | 0 |
| mutations.log | normal heavy route invokes mutations.py | 0, source restored |
| mutation-stale.log | test --offline -p farcooler-client --lib workflow_presentation_stale_and_auth_boundaries -- --nocapture | 101 expected |
| restored-stale.log | same test after exact-byte restoration | 0 |
| mutation-evidence.log | test --offline -p farcooler-client --lib workflow_presentation_invalidated_inspection_and_rework -- --nocapture | 101 expected |
| restored-evidence.log | same test after exact-byte restoration | 0 |
| final-focused.log | test --offline -p farcooler-client --lib workflow_presentation -- --nocapture after restorations | 0, 5 passed |

Tests cover deterministic shared golden output, C bytes for all five fixtures,
short buffers/null, Web bytes for the same fixtures, changed revision/epoch/run/
candidate/policy/evidence, revoked principal/read capability, unavailable actions,
invalid stage bounds, actual reducer acceptance of a reasoned rework intent, and
all six stages under selected/every-stage approvals. Fault insertions remove the
stale guard and candidate-evidence match, respectively; both tests fail, then pass
with the exact source restored. Source hashes in `source-sha256.json` identify the
restored final source; `evidence-sha256.json` identifies the saved evidence.

`size-budget.log`, `doc-comments.log` (base4163081e9..source pin), and
`diff-check.log` all exit 0. Lowest observed free space across the final bounded
runs was 18,800,640,000 bytes, above the 10,737,418,240-byte reserve. No reserve
cancellation. Web's dependency emits two existing dead-code warnings in
`session/step_up.rs` (`proof_bytes`, `key_state_json`); native strict Clippy passes.

`golden.log` retains expected exit 101 during fixture generation: UPDATE mode
writes the fixture, while include_str still compares the prior compiled bytes.
The subsequent non-update runs (including final-focused) pass. This is setup
history, not a claimed passing check. First commit hook attempt selected an
unsupported-architecture Python on inherited PATH; native /usr/bin-first PATH
resolved it, with hooks enabled. Both source commits are signed.

## Pending integration and boundaries

No native screen, Swift/Kotlin declaration/consumer, browser JS renderer, live
workflow subscription, authenticated transport endpoint, or actual JVM/device/
browser interaction was added or exercised. JNI was compiled natively; WASM was
checked for wasm32; the Web adapter's shared bytes were tested natively. This is
source/API evidence, not native/Web UI acceptance, full release parity, or the
parent's two completed feature exercises on devices. Artifact inspection returns
identity, not fetched artifact contents. No owner data, app/update/cutover,
provider/network probes, daemon/store/runtime/proto/CLI changes or obsolete
execution-authority removals occurred in this slice. Runtime publication review
and parent release/cutover remain separate.

The task brief's broad goal is larger than this slice: no claim that bridge
exports by themselves deliver shared behavior in actual native/Web screens.
