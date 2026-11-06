# ov-924 corrected shared display API handoff

This correction supersedes the earlier bare-enum review candidates `4d760f68f`
and `18c724ab0`. Prior signed evidence at `46a66972a` and its raw logs remain
preserved. Review corrected signed source **`791345ee80f7a28337ec49dfed514d176381760e`**, recorded in `source-sha256.json`.

## Revised concrete schema

Presentation **schema 2** supplies finite US English display copy from Rust:

- `RunCard.label` / `phase_label`: title and status text.
- `StageCard.label` / `progression_label`: stage and configured progression copy.
- `GateCard.label` / `text`: the already-derived gate and principal description.
- `ReasonCard { kind, label, text }`: semantic ID plus directly displayable copy.
- `Action { kind, label, guard }`: button copy plus the existing semantic kind and
  opaque guard fields. Shells forward the supplied descriptor unchanged.

`workflow_presentation/display.rs` owns this finite copy. Schema 1 presentation
requests are refused, while workflow State remains schema 1. Schema 2 guards
still bind complete immutable State, current capability context, and item; the
snapshot/context is hashed once per projection. Projection/transport authority
boundaries and finite command/inspection intents are unchanged.

The shared output includes **Evidence Invalidated**, **The latest evidence no
longer matches the current candidate or policy.**, and **Inspect Latest Evidence…**.
`workflow_presentation_generic_display_slots` renders these through generic text
and button slots without kind/phase/stage switches or platform copy maps, then
forwards the supplied inspection descriptor through JSON. This is source-level
proof that the contract can carry the feature; it is not an actual native/Web
consumer or UI acceptance claim. All five shared fixtures include the copy and
are compared byte-for-byte through C and the native Web adapter. JNI delegates C.

API details: `../contract.md`; source: `crates/client/src/workflow_presentation.rs`
and `crates/client/src/workflow_presentation/display.rs`. No native screen,
Session transport, daemon/store/runtime/proto/CLI or new framework edits. Task925
owns the transport work and must consume this corrected schema before freezing
its integration. No style/tone framework is needed for the existing text slots.

## Exact validation and provenance

`run-checks.py MODE` in this directory invokes the same owned target and normal
FIFO `scripts/heavy --load-limit 20 -- cargo ...` route as the original checks.
Native `/usr/bin/python3`, offline Cargo, jobs6, debug0, incremental off,
load5<20, 10GiB reserve, owned group/reap and 1800s deadline remain in force.
No new donor/cache was selected. Minimum observed free space was 45,056,000,000
bytes, above the 10,737,418,240-byte reserve; no cancellation occurred. Every command and raw exit is saved in the log.

| Log | Exact Cargo arguments after cargo | Expected raw exit |
| --- | --- | --- |
| focused.log | test --offline -p farcooler-client --lib workflow_presentation -- --nocapture | 0, 6 passed |
| web.log | test --offline --manifest-path crates/web/Cargo.toml --test workflow_presentation | 0, 1 passed |
| jni.log | check --offline -p farcooler-jni | 0 |
| wasm.log | check --offline --manifest-path crates/web/Cargo.toml --target wasm32-unknown-unknown | 0 |
| clippy.log | clippy --offline -p farcooler-client --lib -- -D warnings | 0 |
| mutations.log | normal FIFO invokes mutations.py | 0 after all restorations |
| mutation-display-copy.log | test --offline -p farcooler-client --lib workflow_presentation_generic_display_slots -- --nocapture | 101 expected with invalidation copy blanked |
| restored-display-copy.log | same test after exact-byte restoration | 0 |
| mutation-stale.log | test --offline -p farcooler-client --lib workflow_presentation_stale_and_auth_boundaries -- --nocapture | 101 expected with guard removed |
| restored-stale.log | same test after exact-byte restoration | 0 |
| mutation-evidence.log | test --offline -p farcooler-client --lib workflow_presentation_invalidated_inspection_and_rework -- --nocapture | 101 expected with candidate match removed |
| restored-evidence.log | same test after exact-byte restoration | 0 |
| final-focused.log | test --offline -p farcooler-client --lib workflow_presentation -- --nocapture after restorations | 0, 6 passed |

`golden.log` has the expected 101 from generation against the previously compiled
fixture; the following non-update focused and final-focused runs pass against
the regenerated fixture. `size-budget.log`, `doc-comments.log` and `diff-check.log`
record the focused source checks. The browser dependency's two pre-existing
step_up.rs dead-code warnings remain outside scope; native strict Clippy passes.
`source-sha256.json` pins all source/API paths; `evidence-sha256.json` pins this
correction's reports. Signature verification is in `source-signature.log`.

## Review boundary

Request **one independent scoped review of the corrected final source**; earlier
pins were missing manager6cb77ea3's display contract and are not review candidates.
Native/Swift/Kotlin/browser JS consumers and authenticated transport remain
pending in the parent work. No UI parity, actual JVM/device/browser interactions,
owner data or cutover acceptance is claimed. Reauthenticate current capabilities
and reapply latest State/reducer/store authority in the future transport.
