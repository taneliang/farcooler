# ov-893 focused validation allocation request

Implementation authorized by18946b25; plan-only approval ca6033b33. No tests have run yet. Saved actual Codex0.166 producer records are copied verbatim into core fixtures. Initial regression-only checkpoint will assert optional operation JSON and same-item output through the existing projector API, so intended failures can be runtime assertions rather than missing-type compile errors.

Request two sequential, nonoverlapping phases of ONE focused allocation, each through normal scripts/heavy/load20/jobs6/nativePython3.14/fullPATH, writable isolated targets/caches. No duplicate wrapper or queue reservation while not ready. No fullgate/display/simulator/provider/probe/liveDB/replay. Source/command/session/numeric exits and raw logs preserved. Signed checkpoints before red and after implementation; no amend/reset/stash or security/config changes.

Phase A, ready after signed test-only checkpoint:

    /opt/homebrew/bin/python3.14 scripts/heavy -- cargo test -p farcooler-core literal_operation -- --nocapture

Expected meaningful red: real standalone record lacks literal metadata and aggregated output; missing exit must be unconfirmed. Compound/forged parsed hint negatives already pass. Record actual result rather than treating compile/setup failure as red. Then release slot while implementing. No product revert mechanism or parallel red tree needed.

Phase B, only after implementation is ready and phase A has finished: one sequential wrapper:

    cargo test -p farcooler-core session_log::projector
    cargo clippy -p farcooler-core --all-targets -- -D warnings
    scripts/test-leak-check.py -- swift test --package-path apps/shared/AgentKit --jobs 6 --filter LiteralOperation
    cd apps/android && ./gradlew testInstrumentedUnitTest --tests 'com.farcooler.model.LiteralOperationTest' --max-workers=6

Use approved native JDK17/Android SDK and writable cache paths without inspecting credentials or global configuration. Swift filtered model tests must terminate with nonzero test counts; raw output must include Testing completion. No full-suite floor claim from a filtered run. Add focused wire/cache/presentation-helper regressions on both client paths; expand core adversarial/stable-row coverage before green. Any required command correction will be recorded before running; unsupported environment/setup stops that check with honest unrun status.

This focused allocation does not authorize Mac/iOS app builds, screenshots/captures, simulator/device activity, full lane gates or acceptance closure. Those require subsequent allocation after actual code review. Platform UI compilation remains an explicit outstanding gate until then; model checks do not establish it.
