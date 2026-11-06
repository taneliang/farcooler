# ov-927 shared guarded Session handoff

Source pin: **4f242bd557277f82e6a88938575b96de86b9671c**, signed; `source-signature.log` verifies with exit 0. Stop source edits here for independent scoped security review. This is a review handoff, not a production/native release approval.

## Exact source composition

Base: manager-authorized corrected ov-925 `7038e170476ee842ebb9e741005516a9b0bb82bb`.
Approved schema2: ov-924 `72ccadf5c9987b851972780409fcccc9cf6edc37`.
Five unique commits versus `416308` were signed forward cherry-picks, no merge/rewrite. `union.json` records approved patch IDs and union hashes. `provenance.json` records the integration mapping, final source SHA256 values, and unchanged pure schema2 sources. Both module/dependency sides survive; Cargo revalidation adds the ov-925 protocol workflow/serde dependencies to the separate web lockfile.

## Contract and implementation to read

- `session-contract.md`: exact C/JNI ticket/poll and WASM signatures/JSON. No callback ABI or new FFI framework.
- `crates/client/src/session/workflow.rs:40`: wrapper projection binds each opaque Action.guard to authenticated admission context, preserving approved pure projection source/exports.
- `crates/client/src/session/workflow.rs:48`: prepare checks original identity revision/epoch and entire supplied descriptor against a fresh authenticated projection. Only then calls the approved pure action validator. No newer unseen run, pin, candidate or evidence substitution; local stale is ResourceConflict / what=workflow_stale / retryable=false.
- `crates/client/src/session/workflow.rs:63`: single finite Intent-to-wire mapping (Approve, Review, Cancel); inspections return the exact retained evidence or uncertainty identity. Cancel obtains run/pin only from that validated observation.
- `crates/client/src/session/workflow.rs:87`: actual Session getter uses the existing transport and WORKFLOW_COMMANDS capability requirement. It never takes caller capabilities/principal/Grant/snapshot arguments.
- `crates/client/src/session/workflow.rs:103`: submit validates closed prepared vocabulary and sends the retained JSON unchanged. Stable mutation id/source sequence/revision/epoch remain in the payload; transport correlation IDs may differ on reconnect. Retry is explicit, bounded by existing Session deadlines; no automatic reprepare/reconnect loop. Host retains prepared envelope across connection/process replacement.
- `crates/daemon/src/workflow_runtime/admission.rs:69`: informational server context derives only from captured/rechecked admission. Binding includes host/workspace, authority generation, positive enrollment, peer device/scope, process incarnation/seat/orchestrator proof and captured revocation generation. Ordinary equivalent reconnect can preserve binding; scope or revoke/readmit changes invalidate selection.
- `crates/daemon/src/workflow_runtime.rs:317`: original ov-925 consumption recheck remains before reads/receipt lookup/writes. Getter context is attached synchronously after that check. Mutation reauthorization/durable receipt path is unchanged.
- `crates/client/src/ffi.rs:1588`: existing generic C route; JNI already forwards to this same C call (`crates/android/src/lib.rs:139`). `crates/web/src/wire.rs:177` and `crates/web/src/js.rs:320` delegate to the same Rust Session route.

The context binding and opaque guards are observation identities, not credentials. An arbitrary prepared envelope still faces daemon authentication and exact revision/epoch/run/pin/evidence policy. Pure caller-fed projection fixtures alone are not authenticated connectivity.

## Validation and raw evidence

All heavy checks ran sequentially through normal `scripts/heavy --load-limit 20`, native `/usr/bin/python3`, jobs6, offline, dev/test debug0 + incremental0. Only the authorized original ov-923 target cache was reused. It had no holder before entry; normal FIFO serialized access. Each subprocess group has a 30-minute owned bound and 10GiB free-space reserve monitoring; exact commands, hashes, exits and minimum reserves are in each JSON. No production selector, installed data, owner DB, provider or live app was used.

| Check | Exit | Evidence |
|---|---:|---|
| Final shared projection/Session tests, 11 passed | 0 | final-client.log/json |
| Extracted unchanged FFI identity tests, 9 passed | 0 | final-identity.log/json |
| Daemon workflow/admission tests, 31 passed | 0 | green.log/json |
| Existing daemon wire socket tests, 2 passed | 0 | socket.log/json |
| JNI check | 0 | jni.log/json |
| Web shared golden test | 0 | web.log/json |
| wasm32 check, including actual WebRunner export | 0 | wasm.log/json |
| Client clippy -D warnings, final source | 0 | final-client-lint.log/json |
| Daemon all-target clippy -D warnings | 0 | lint.log/json |
| Diff/doc-comment checks | 0 | diff-check.log, doc-comments.log |
| Source signature verification | 0 | source-signature.log |

`final-client.log` contains two actual isolated Unix-socket client-to-daemon traces. Cancel drops its pending answer after another Session observes acceptance, restores identical serialized prepared bytes, reconnects and gets the same durable receipt twice. The effectful Approve trace lets automatic fixture stages reach Done at revision 7, discards the original answer, then replays the exact revision-1 approval and receives its original revision-2 receipt with effects; final state stays unchanged. This uses real shared Session/transport/RPC/runtime/store, with the C/JNI dispatch arm also exercised; it is not an actual native app/JVM/browser runtime navigation proof.

Additional composition assertions cover changed caller scope with identical state, revoke/readmit generation, withdrawn access even on accepted-receipt retry, changed canonical content under the same id, post-prepare revision race with durable Stale refusal, and caller authority/context field injection. Unit assertions cover exact review evidence, candidate/run/policy/epoch drift, and both inspection intents.

`mutation-guard.log` disables stale descriptor comparison: three tests fail (including real socket auth-context refusal and unseen evidence). `mutation-bytes.log` changes retained mutation revision on submit: socket retry fails. Both exits are 101, source restored in finally; `mutations.json` records them. Final restored source passes. Earlier test-fixture failures are retained in `expanded-first.*` and `fixture-resource.*`: reviewer Dispatch versus Person and exclusive resource collision; fixture setup was corrected without production policy changes.

The final FFI-only change after the eight-check matrix moves the existing inline identity tests verbatim to `ffi/identity_tests.rs`; `static-checks.json` verifies exact deindented equality. Final client tests and lint rerun after extraction and the added effectful test. JNI/WASM/daemon production implementation is otherwise the same as their green matrix source hashes. `provenance.json` records exact final hashes.

## Remaining review/integration work

**Inherited gate failure:** file-size-budget exit 1 solely for `crates/daemon/src/rpc.rs` at 2482 lines versus ceiling 2450. It is byte-identical to approved ov-925 base 7038 (also 2482). Own FFI ceiling excess is fixed by the test extraction. The inherited RPC ownership decision has been sent to the orchestrator; no ceiling was raised. Do not describe the whole repository as gate-green.

Independent scoped security review is requested from the orchestrator against this pin. Focus: server-derived context, unchanged consumption authorization, exact stale observation validation, prepared byte preservation and receipt retry. Review is pending; acceptance involving review is not self-approved.

ov-926 owns native/DOM declarations/navigation and must consume authenticated `workflow.snapshot.presentation`, forward entire selected Action to prepare, and preserve prepared for submit/retry. Early signatures and source pin were sent through the orchestrator (direct sibling messages are disallowed). Actual native navigation and parent release gates remain outside this Rust proof; no native connectivity or production-auth proof is claimed. This uses the existing isolated trusted fixture boundary.

Source commit hook initially selected an incompatible inherited python3; the same signed commit succeeded with native `/usr/bin` first, with hooks enabled. No fmt/stash/reset/clean/amend/add-A, cold duplicate target, installed data mutation or production selector.
