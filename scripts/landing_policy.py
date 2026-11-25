"""Local landing policy (ov-1185); CI and plain lane-gate retain full proof.

Families have private GitHub environment files. Apple jobs share SwiftPM output
and run serially. Cargo arbitrates its shared target directory with its own
lock; do not give each worker a cold target. Admission still uses scripts/heavy.
"""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import threading
import time


# Reviewed gate job users (ov-1326). None explicitly means full/uncertain.
# Reasons describe execution/import edges, not the source language being linted.
# Meta-tests that copy a tree do not make every copied file an executed input.
# Missing paths stay full at runtime and fail the coverage test until reviewed.
SCRIPT_USERS = {
    '.github/workflows/canary-baseline.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/canary.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/ci-runner-retry-recover.yml': {'wire'},  # Workflow lint and retry replay cover the recovery controller.
    '.github/workflows/ci-runner-retry.yml': {'wire'},  # Workflow lint and retry replay cover the primary controller.
    '.github/workflows/ci.yml': None,  # Full: steps_for derives every gate command from this workflow.
    '.github/workflows/claude-prices.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/derp-hosts.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/doc-comments.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/linux-binaries.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/nightly.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/openai-prices.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/ov1186-cache.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/promote.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/relay.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/release.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/site-analytics.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/site.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/tunnel-e2e.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    '.github/workflows/web.yml': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/actionlint.sh': {'wire'},  # workflow_checks adds actionlint to wire for workflow edits; actionlint.sh builds with queue.patch.
    'scripts/actionlint/queue.patch': {'wire'},  # workflow_checks adds actionlint to wire for workflow edits; actionlint.sh builds with queue.patch.
    'scripts/android-ndk.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/appcast-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/appcast.py': {'wire'},  # wire appcast-test.sh executes feed generation; shipping feed generation is outside gate families.
    'scripts/build-android-libs.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/build-derper.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/build-ios-frameworks.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/build-linux.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/build-tailcat.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/build-tunnel-helper.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/canary-baseline-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/canary-baseline.sh': {'wire'},  # wire canary-baseline-test.sh invokes it, loading proto-lint; live stamping is release automation.
    'scripts/canary-plan.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/canary-shipped.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/capability-names.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/capture-contrast-check.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/capture-duplicates-allow.txt': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/capture-duplicates-test.sh': {'swift-tests'},  # ci.yml swift-tests invokes this test; wrapper subprocesses use stand-in tools.
    'scripts/capture-duplicates.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/capture-window-allow.txt': {'wire'},  # wire capture-window-lint reads its allowed capture counts.
    'scripts/capture-window-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/charter-intercept-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/ci-changes.py': None,  # Full: family classifier is imported by gate planning and canary-plan.
    'scripts/ci-concurrency-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/ci-flake-tally-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/ci-flake-tally.py': {'wire'},  # wire ci-flake-tally-test.sh calls shell wrapper then Python parser on saved log fixtures.
    'scripts/ci-flake-tally.sh': {'wire'},  # wire ci-flake-tally-test.sh calls shell wrapper then Python parser on saved log fixtures.
    'scripts/ci-install-tmux.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ci-job-time-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ci-macos-capacity.py': {'wire'},  # wire ci-concurrency-test imports capacity and runs both self-tests; live scheduling is CI-only.
    'scripts/ci-runner-retry-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/ci-runner-retry.py': {'wire'},  # ci.yml wire runs retry-test, which imports the helper; API-only workflow (ov-1324).
    'scripts/ci-yield-to-main.py': {'wire'},  # wire ci-concurrency-test imports capacity and runs both self-tests; live scheduling is CI-only.
    'scripts/claude-prices.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/claude-rewake-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/com.farcooler.site-proof.plist': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/copy-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/daemon-layer-check': {'wire'},  # Alias for daemon-layers.py, the wire dependency check.
    'scripts/daemon-layers-counts.txt': {'wire'},  # wire daemon-layers reads dependency counts and checks them against main.
    'scripts/daemon-layers.json': {'wire'},  # wire daemon-layers reads layer membership and allowed dependencies.
    'scripts/daemon-layers.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/demo-host.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/derp-hosts-check.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/device-keys-readiness.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/doc-comment-check.py': {'doc-comments', 'wire'},  # doc-comments executes checker; wire landing-gate-test exercises it against scratch commits.
    'scripts/engine-gate-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/engine-gate.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/engine-old-reader.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/file-size-budget.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/file-size-budget.txt': {'wire'},  # wire file-size-budget reads grandfathered size ceilings.
    'scripts/fixtures/ci-flake-tally/annotation-not-acquired.txt': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/dns-crates-io.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/leaked-farcoolerd.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/manifest.tsv': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/nextest-answer-wake.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/nextest-route-tests.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/swift-testing-flick.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/swift-testing-row-budget.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/synthetic-gradle.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/ci-flake-tally/synthetic-xctest.log': {'wire'},  # wire ci-flake-tally-test.sh passes saved fixtures through the tally parser.
    'scripts/fixtures/landing-timing/mtimes.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:12129828fe20f3749bab5dcf9fd21112d003b42aa0303768aea5e3ad7e1b1805.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:123d809b122ec0e5a2196e3beb38ac53153c060927df13196698aa3651151f73.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:2aec8132ec7c1b45dcb7024b985643a693030dd24522f0408e6fc711b1ecfcbe.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:2aec8132ec7c1b45dcb7024b985643a693030dd24522f0408e6fc711b1ecfcbe.output': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:2fdadf1c8c8c0409d14004157703c9f43f17aba660e1f9906663033f0daa93cc.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:388d8d36baaf256c0ad937653e9b7c7641cd622bf56431256da0e059bcc47aab.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:4c9483465574fd121a9cfdff79a1c5bdd6a42de7eafc8dc1d736a25ceb53b269.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:5007e45d13e14b650eec491ae84cbf1e0e29dbd543ef9b0246a380b118720c7e.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:50d0a0216383dc873f24690edcd520b479cae45f1439e90e0f43f384194ce4ff.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:5cccb8c77ef7ec018c17fedf81260b992c7c28632d0116eabeae0cd1e36ea6bb.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:61229bdbd4ca8d230d51fd3d140c3c81878b0891e3f143e484ebf2af0679978a.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:65d1a61d5b1cd58cff02c91524f7470de70aa6f041fed88a39aee73d603f8b98.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:6cbb04f073c51b274d21f701c927c0085a97d26d6bd4353f7add6a11ee5cd4b8.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:6d57e92b38c297a19980cc3cab666e107b0b7ae5d5a65e3915a13fea0b3acfc4.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:7138f663d249494925d8bf782d67eb3eb685996a5a2bfca91693c402d7591c1c.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:71b0e3c2bf149055e31f85735aa79f40a0bca133d1a7b738e6eae246545bad49.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:7806241308cf89860fec478ed36816bf9de457f47be1099d09a2e455442b7253.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:830fa9c63bdcc0d437c9664e2e4a72720917059fb984b17f20f5238203f475ad.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:a19ea1be532292f6f53a19633eb215e249ab0590d918e0bddd54a91ad43212a5.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:b48e62734df2549b4a5d3e3f2230d3334e515cc16cbe529880f5899670841960.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:c3f9304db0c32a4244f48d51772c486cbcaa68258272d5c6f2fc2fe502e7899a.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:da5ef33f0ea6f20f2983bfae97122951721e1ef23c029cad2e6d415543fc18e0.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fixtures/landing-timing/wf1:e285fca49b073c72050695692118c53bd67d5d5238ed67bbea17e61bcb7085ba.json': {'wire'},  # wire landing-timing --self-test reads this receipt/mtime/output fixture.
    'scripts/fold-baselines.py': None,  # Full: bundled into the daemon's landing fold and exercised by its Rust tests.
    'scripts/fold-check.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/fresh-script-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/gate-evidence-mutation-test.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/gate-evidence-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/gate-flaky.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/gate-summary-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/gate-test-fixture-test.py': None,  # Full: end-to-end self-test regression.
    'scripts/gate-test-fixture.py': None,  # Full: gate self-test repository construction; retain orchestration coverage.
    'scripts/gate_disk.py': None,  # Full: imported by lane-gate.py and landing-gate at admission.
    'scripts/gate_sccache.py': None,  # Full: optional compiler wrapper applied to every gate family.
    'scripts/gate_evidence.py': None,  # Full: canonical evidence planner and execution attest every family.
    'scripts/gen-wire-types.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/generator-test.sh': {'ios'},  # ci.yml ios invokes this regression test; no other gate family calls it.
    'scripts/gh-api-retry.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/gh-download-test.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/gh-download.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/git-hooks/commit-msg': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/heavy': None,  # Full: landing-gate and platform wrappers enter shared machine admission.
    'scripts/heavy_lock.py': None,  # Full: admission and process locks shared by every gate job.
    'scripts/icon-label.swift': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/icon-test.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/import-ios-certificate-test.sh': {'ios'},  # ci.yml ios invokes this regression test; no other gate family calls it.
    'scripts/import-ios-certificate.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/install-git-hooks.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ios-agentkit-subset-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/ios-ui-lease-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/ios-ui-pick-simulator.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ios-ui-plan.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ios-ui-retry-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/ios-ui-shard-budget.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ios-ui-shards.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ios-ui-tests.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/landing-benchmark-test.py': {'wire'},  # wire lane-gate --self-test runs this regression suite in its scratch repository.
    'scripts/landing-benchmark.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/landing-gate': None,  # Full: entry wrapper chooses and admits the entire gate.
    'scripts/lane-gate-lifetime-test.py': {'wire'},  # wire checks the self-test process lifetime.
    'scripts/landing-gate-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/landing-policy-test.py': {'wire'},  # wire lane-gate --self-test runs this regression suite in its scratch repository.
    'scripts/landing-timing.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/landing_policy.py': None,  # Full: selection, command rewrites and worker scheduling affect every family.
    'scripts/lane-gate.py': None,  # Full: orchestrates and selects every gate job, including preflight and evidence.
    'scripts/lint-gate.sh': {'wire'},  # Workflow lint check runs this wrapper; its lint scripts are wire users.
    'scripts/mac-capture-stop-test.sh': {'swift-tests'},  # ci.yml swift-tests invokes this test; wrapper subprocesses use stand-in tools.
    'scripts/mac-capture.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-bundle-native-test.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-entitlements-test.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-rust-tests.json': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-rust-tests.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-test-partitions.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/macos-test-wrapper-test.sh': {'swift-tests'},  # ci.yml swift-tests invokes this test; wrapper subprocesses use stand-in tools.
    'scripts/manager-skill-pressure/fake-farcooler.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/fake-gh.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/new-scratch-repo.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/render-skill.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/run-claude.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/scenarios.md': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/manager-skill-pressure/score.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/migration-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/nightly-dispatch-test.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/nightly-report.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/notarytool-retry.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/openai-prices.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/ov-1324-report.md': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ov-1326-report.md': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/ov-1328-report.md': None,  # Full: ticket evidence; retain conservative fallback.
    'scripts/pixel-scale-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/plan-layer-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/portal-app-groups.rb': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/process_guard.py': None,  # Full: lane-gate imports process control for every step.
    'scripts/process_guard_test.py': {'wire'},  # wire lane-gate --self-test runs this regression suite in its scratch repository.
    'scripts/proto-lint.py': {'wire'},  # wire runs lint; canary-shipped and canary-baseline-test also load it within wire.
    'scripts/quarantine-test.py': {'rust'},  # ci.yml rust runs regression suite; it imports quarantine and tests a scratch Cargo workspace.
    'scripts/quarantine.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/reap-orphan-daemons.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/reap-stale-tmux.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/release.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/runloop-spin-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/runner-upgrade-e2e.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/rust-archive.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/rust-select-landings.json': {'wire'},  # wire rust-select-test reads this historical selection fixture.
    'scripts/rust-select-test.py': {'wire'},  # ci.yml wire invokes this regression suite; its gate/build subprocesses use scratch fixtures.
    'scripts/rust-select.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/rust-shards.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/shared-core-baseline.json': {'wire'},  # wire shared-core-lint reads its baseline and checks it against main.
    'scripts/shared-core-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/site-proof-fixtures/commits.txt': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-fixtures/expected.html': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-fixtures/milestones.json': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-fixtures/page.html': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-fixtures/plan.json': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-fixtures/tasks.json': {'wire'},  # wire site-proof --self-test reads this proof-page fixture.
    'scripts/site-proof-milestones.json': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/site-proof-refresh.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/site-proof-tokens.json': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/site-proof.py': {'wire'},  # ci.yml wire runs self-test/check; script calls are checks or release automation, not platform builds.
    'scripts/sparkle-key-test.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/sparkle-public-key.swift': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/stage-demo-transcript.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/stage-demo.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/swallow-lint-allow.txt': {'wire'},  # wire swallow-lint reads its exception list.
    'scripts/swallow-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/swift-build-cache.py': {'swift', 'swift-tests'},  # ci.yml swift and swift-tests run self-test/restore/save; cache helper has no other gate caller.
    'scripts/swift-test-guard-test.sh': {'swift-tests'},  # ci.yml swift-tests invokes this test; wrapper subprocesses use stand-in tools.
    'scripts/swift-test-guard.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/test-floor-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/test-leak-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/test-window-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/tmux-leak-check.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/tmux-stale-sockets.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/tunnel-e2e.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/tunnel-smoke.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/version-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
    'scripts/version.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/visual-tokens-lint.py': {'wire'},  # ci.yml wire runs self-test and scan; leaf checker, no platform imports or subprocess callers.
    'scripts/wait-for-unlock': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/wait-for-unlock-test.sh': {'swift-tests'},  # ci.yml swift-tests invokes this test; wrapper subprocesses use stand-in tools.
    'scripts/web-client-tree-check.py': {'web'},  # ci.yml web runs self-test and dependency check; no other gate caller.
    'scripts/workflow-review-timing-test.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/workflow-review-timing.py': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/workos-client-id.sh': None,  # Full: transitive gate users not established; retain conservative fallback.
    'scripts/workos-test.sh': {'wire'},  # ci.yml wire invokes this shell test; subprocesses exercise temporary repositories or fake tools.
}

def script_users(path, commands):
    """Reviewed consumers plus any newly named gate users; None means unknown.

    Command text alone cannot prove absence of transitive consumers. Never
    narrow an unreviewed path just because a job does not name it directly.
    """
    if SCRIPT_USERS.get(path) is None:
        return None
    users = set(SCRIPT_USERS[path])
    for job, command in commands.items():
        if path in command:
            users.add(job)
    return users


def workflow_checks(plans, files, step_type):
    """Keep workflow validation without selecting a platform build for it."""
    if any(path.startswith(".github/workflows/") for path in files):
        plans["wire"].extend([
            step_type("wire", "Workflow lint", "./scripts/actionlint.sh\n"),
            step_type("wire", "Workflow shard check", "./scripts/ios-ui-shards.py --check\n"),
        ])


# Wire is mostly source checks. These steps can compile tools or fixtures, or
# have taken more than a minute on the landing runner (see reports/ov-1443).
DEFERRED_WIRE = {
    "python3 scripts/rust-select-test.py",
    "python3 scripts/landing-gate-test.py",
    "HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test",
    "python3 scripts/lane-gate-lifetime-test.py",
    "python3 scripts/gate-evidence-test.py",
    "python3 scripts/engine-gate-test.py",
    "./scripts/ios-ui-retry-test.sh",
    "./scripts/ios-ui-lease-test.sh",
    "./scripts/actionlint.sh",
}
QUICK_COMMANDS = {
    "rust": {"./scripts/tmux-leak-check.py --self-test",
             "./scripts/tmux-stale-sockets.py --self-test", "./scripts/test-leak-check.py --self-test"},
    "swift-tests": {"./scripts/macos-test-wrapper-test.sh"},
    "swift": {"./scripts/gh-download-test.sh", "./scripts/macos-entitlements-test.sh",
              "./scripts/wait-for-unlock-test.sh", "./scripts/swift-test-guard-test.sh",
              "./scripts/test-leak-check.py --self-test",
              "./scripts/mac-capture-stop-test.sh", "./scripts/capture-duplicates-test.sh"},
    "ios": {"./scripts/ios-ui-shards.py --check", "./scripts/ios-ui-shards.py --self-test",
            "./scripts/ios-ui-pick-simulator.py --self-test", "./scripts/ios-ui-plan.py --self-test",
            "./scripts/import-ios-certificate-test.sh"},
    "web": {"./scripts/web-client-tree-check.py --self-test", "./scripts/web-client-tree-check.py",
            "node scripts/size-gate.mjs --self-test"},
}


def split_quick(plans, chosen, base, step_type, *, typeshare_ready, review):
    """Split build-free checks from the expanded local landing plan."""
    quick, remaining = {}, dict(plans)
    for job in chosen:
        early, later, origin_checks = [], [], []
        quick[job], remaining[job] = early, later
        for step in plans[job]:
            first = step.label().removesuffix(" ...")
            def with_run(run):
                return step_type(job, step.name, run, step.cwd, step.env)

            if step.skip:
                later.append(step)
            elif job == "wire" and step.name in {"Baseline did not grow", "Daemon layers"}:
                # Review can compare against its pinned base without fetching.
                # The landing still compares against the freshly fetched main.
                local, fetch, remote = step.run.partition("git fetch --no-tags origin main\n")
                if not fetch:
                    raise ValueError(f"cannot split the base comparison in {step.name}")
                if review:
                    early.append(with_run(local + remote.replace("FETCH_HEAD", shlex.quote(base))))
                else:
                    if local.strip() and not all(line.startswith("#") for line in local.strip().splitlines()):
                        early.append(with_run(local))
                    origin_checks.append(with_run(fetch + remote))
            elif job == "wire" and step.name == "The generated wire types are current" and typeshare_ready:
                early.append(with_run(step.run.replace("./scripts/gen-wire-types.sh\n",
                                                       "./scripts/gen-wire-types.sh --check\n", 1)))
            elif job == "wire" and step.name.startswith("Install typeshare") and typeshare_ready:
                # A pinned executable was already verified. Retain the install
                # and generation path below when the tool is cold.
                continue
            elif job == "wire" and (first in DEFERRED_WIRE or step.name.startswith("Install typeshare")
                                    or step.name == "The generated wire types are current"):
                later.append(step)
            elif job in {"wire", "doc-comments"} or first in QUICK_COMMANDS.get(job, ()) or (
                    job == "tunnel" and step.name == "Check gofmt"):
                early.append(step)
            elif first == "python3 scripts/swift-build-cache.py self-test":
                early.append(with_run(first + "\n"))
                later.append(with_run(step.run.replace(first + "\n", "", 1)))
            else:
                later.append(step)
        # Scan the tree first so a source violation returns in seconds.
        if job == "wire":
            early.sort(key=lambda step: step.run.strip() != "./scripts/shared-core-lint.py")
            early.extend(origin_checks)
    return quick, remaining


def fold_base(repo, given):
    """Read the same exact workspace/item/candidate index as daemon process_base.

    Only durable evidence can narrow an old runner base. Remote refs, commit
    subjects and HEAD's parent cannot prove where a multi-commit fold began.
    Keep the given base on missing, foreign, malformed or non-ancestral evidence.
    """
    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)

    try:
        workspace, item, candidate, receipt = (os.environ.get(key, "") for key in
            ("FARCOOLER_WORKSPACE", "FARCOOLER_WORKFLOW_ITEM", "FARCOOLER_CANDIDATE", "FARCOOLER_RECEIPT"))
        if not all((workspace, item, candidate, receipt)):
            raise ValueError("missing fold context")
        head = git("rev-parse", "HEAD")
        if head.returncode or candidate != head.stdout.strip():
            raise ValueError("candidate differs from HEAD")
        # farcooler_workflow::digest serializes this tuple as compact UTF-8 JSON.
        identity = json.dumps([workspace, item], ensure_ascii=False, separators=(",", ":")).encode()
        name = f"fold-wf1:{hashlib.sha256(identity).hexdigest()}-{candidate}.json"
        record = json.loads((Path(receipt).parent / name).read_text())
        if not isinstance(record, dict) or record.get("item") != item or record.get("folded") != candidate:
            raise ValueError("foreign fold record")
        base = record.get("base")
        if not isinstance(base, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", base):
            raise ValueError("invalid fold base")
        for older, newer in ((given, base), (base, candidate)):
            if git("merge-base", "--is-ancestor", older, newer).returncode:
                raise ValueError("fold base does not advance the given base within candidate history")
    except (OSError, ValueError) as error:
        print(f"lane-gate: fold base unavailable ({error}); falling back to given base {given}")
        return given
    if base.lower() == given.lower():
        return given
    print(f"lane-gate: using fold base {base} instead of stale given base {given}")
    return base


def readonly_git(env):
    """Suppress optional index writes while retaining dirty reads and real writes."""
    env.setdefault("GIT_OPTIONAL_LOCKS", "0")
    # Git diff's automatic refresh can still write with optional locks off.
    # Append command-scoped config without replacing caller-provided entries.
    count = int(env.get("GIT_CONFIG_COUNT", "0"))
    env[f"GIT_CONFIG_KEY_{count}"] = "diff.autoRefreshIndex"
    env[f"GIT_CONFIG_VALUE_{count}"] = "false"
    env["GIT_CONFIG_COUNT"] = str(count + 1)


def budget_worker(env, workers):
    """Share the host between families instead of each builder taking every CPU.

    Tests still use CI's profile and groups. Only local scheduling changes;
    Cargo and Go compilation concurrency do not alter artifact contents.
    """
    # gen-wire-types rewrites identical tracked outputs, then git status/diff
    # refresh their stat cache. Protocol watches the index for its build stamp;
    # that optional refresh otherwise invalidates its entire reverse closure.
    # Reads still inspect the worktree and real git writes still take locks.
    readonly_git(env)
    share = max(1, (os.cpu_count() or 1) // workers)
    # Cargo serializes writers to a shared profile. A per-family fraction
    # leaves that single active compiler underfed while other families test.
    # Cap it at four, retaining room for Swift/Gradle and explicit user limits.
    env.setdefault("CARGO_BUILD_JOBS", str(max(share, min(4, os.cpu_count() or 1))))
    env.setdefault("GOMAXPROCS", str(share))
    # Most integration tests wait on I/O. Dividing this pool between compiler
    # families serializes those waits; six is the measured stable local pool
    # documented in nextest.toml. Keep CI's groups and stricter user limits.
    env.setdefault("NEXTEST_TEST_THREADS", str(min(6, os.cpu_count() or 1)))
    env["GOFLAGS"] = (env.get("GOFLAGS", "") + f" -p={share}").strip()
    env["GRADLE_OPTS"] = (env.get("GRADLE_OPTS", "") + f" -Dorg.gradle.workers.max={share}").strip()


def record_timing(logdir, job, step, result, started, ended):
    """Flush each completed step so an interrupted gate retains real timings.

    A job has one worker and one file, so parallel families never interleave
    JSON writes. These timestamps include preflight and Cargo lock waits.
    """
    row = {"job": job, "command": step.label(), "run": step.run,
           "result": result, "seconds": ended - started,
           "start_monotonic": started, "end_monotonic": ended}
    with (logdir / f"{job}-timings.jsonl").open("a") as stream:
        stream.write(json.dumps(row) + "\n")


def rust_selection(repo, base, load_module):
    """Select against a verified green ancestor, including staged/dirty inputs.

    Missing network, metadata, green history, or unknown paths means full. Never
    treat the previous commit or an arbitrary --base as proof of a green base.
    """
    shadow = load_module("landing_rust_select", repo / "scripts/rust-select.py")
    try:
        candidates_file = os.environ.get("CANDIDATES")
        if candidates_file:
            with open(candidates_file) as stream:
                candidates = [json.loads(line)["head_sha"] for line in stream if line.strip()]
        else:
            raw = subprocess.run([
                "gh", "api", "repos/{owner}/{repo}/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100",
                "--jq", ".workflow_runs[].head_sha"], cwd=repo, capture_output=True, text=True,
                check=True, timeout=30).stdout
            candidates = raw.splitlines()
        head = shadow.capture(repo, "git", "rev-parse", "HEAD").strip()
        green = shadow.green_base(repo, head, candidates)
        if not green:
            raise ValueError("no verified green ancestor")
        # Include BOTH the requested range and changes since green, plus dirty
        # files. Unknown untracked files force full just like tracked unknowns.
        merge = shadow.capture(repo, "git", "merge-base", base, "HEAD").strip()
        changes = []
        for start in (green, merge):
            raw = shadow.capture(repo, "git", "diff", "--no-renames", "--name-status", "-z", start).split("\0")
            changes.extend(zip(raw[0:-1:2], raw[1:-1:2]))
        raw = shadow.capture(repo, "git", "ls-files", "--others", "--exclude-standard", "-z")
        changes.extend(("A", p) for p in raw.split("\0") if p)
        metadata = json.loads(shadow.capture(repo, "cargo", "metadata", "--locked", "--offline",
                                             "--format-version", "1", "--all-features", "--no-deps"))
        def workflow_texts(path):
            return shadow.capture(repo, "git", "show", f"{green}:{path}"), (repo / path).read_text()
        packages, reasons, full = shadow.select(repo, metadata, changes, workflow_texts)
        return with_binaries(shadow, metadata, {"full": full, "packages": packages, "reasons": reasons, "base": green})
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
        return {"full": True, "packages": [], "reasons": [f"full: {exc}"], "selected_binaries": [], "skipped_binaries": [],
                "binaries_note": "unavailable: selection failed before the workspace was read"}


def with_binaries(shadow, metadata, proposal):
    """Name the test binaries the proposal runs and skips, for the gate's evidence (ov-1278).

    A red main is traced to a skip by reading the last landing's rust-selection.json.
    """
    names = set(proposal["packages"])
    if proposal["full"]:
        names = {p["name"] for p in metadata["packages"] if p["id"] in set(metadata["workspace_members"])}
    run, skipped = shadow.test_binaries(metadata, names)
    return {**proposal, "selected_binaries": run, "skipped_binaries": skipped}


def forced_full(repo, load_module, reason):
    """The whole suite on request (--full-rust), still with every binary named in the evidence."""
    proposal = {"full": True, "packages": [], "reasons": [reason], "selected_binaries": [], "skipped_binaries": [],
                "binaries_note": "unavailable: workspace metadata could not be read"}
    try:
        shadow = load_module("landing_rust_select", repo / "scripts/rust-select.py")
        metadata = json.loads(shadow.capture(repo, "cargo", "metadata", "--locked", "--offline",
                                             "--format-version", "1", "--all-features", "--no-deps"))
        return with_binaries(shadow, metadata, {k: proposal[k] for k in ("full", "packages", "reasons")})
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        return proposal


def apply_local(plans):
    """Apply local platform setup and retain the release-only skips."""
    for job, steps in plans.items():
        for step in steps:
            if step.skip:
                continue
            if job == "web" and "GOOS=js GOARCH=wasm go test" in step.run:
                # Go/Wasm reserves only 8 KiB for argv + env. A developer's
                # App Store session is unrelated to this test and can exhaust
                # that space before any test starts; CI doesn't supply it.
                # Its runtime has no OS threads, unlike the host Go builders.
                step.env = {**step.env, "FASTLANE_SESSION": "", "GOMAXPROCS": "1"}
            if job == "swift" and (step.name == "Build Far Cooler.app"
                                   or "apps/macos/build/$APP_NAME.app" in step.run
                                   or step.name == "Resolve this channel's app name"):
                if "macos-entitlements-test.sh" in step.run:
                    step.run = "./scripts/macos-entitlements-test.sh\n"
                else:
                    step.skip = "local landing: release bundle and its consumers run in CI"


def apply(plans, proposal):
    """Keep features/lints/docs full and narrow only the default-host nextest."""
    apply_local(plans)
    for step in plans.get("rust", ()):
        if not step.skip:
            apply_rust("rust", step, proposal)


def apply_rust(job, step, proposal):
    """Narrow the default-host nextest step to the selected packages (ov-1268).

    Shared by the landing gate and the plain lane gate. Features, lints and
    doctests stay full, and CI on main always runs the whole suite.
    """
    if job == "rust" and step.name == "Full Rust workspace (local)":
        wrappers = "./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- "
        tail = "cargo nextest run --profile gate --no-fail-fast --archive-file "
        # CI wraps the run in the flaky-test quarantine runner (ov-1229);
        # the local run is the unpartitioned archive run (ov-1210), after
        # a line or two that link the channel aliases the tests spawn.
        lines = step.run.rstrip().split("\n")
        command = lines[-1]
        # The gate (not CI on main) also reruns what failed once, alone (ov-1285).
        rerun = "python3 scripts/gate-flaky.py run -- "
        if not (command.startswith(wrappers + tail)
                or command.startswith(wrappers + "python3 scripts/quarantine.py run -- " + tail)
                or command.startswith(wrappers + rerun + "python3 scripts/quarantine.py run -- " + tail)):
            raise ValueError("Rust CI command changed; refusing to narrow an unknown shape")
        if not proposal["full"]:
            expression = " or ".join(f"package(={p})" for p in proposal["packages"])
            if not expression:
                step.skip = "no Rust test input changed (docs and non-Rust files only); CI on main runs the full suite"
                return
            lines[-1] = command + " -E " + shlex.quote(expression)
            step.run = "\n".join(lines) + "\n"


def families(chosen):
    groups = {}
    for job in chosen:
        # swift build and tests mutate the same .build; iOS shares AgentKit.
        family = "apple" if job in {"swift-tests", "swift", "ios"} else job
        groups.setdefault(family, []).append(job)
    # Start the long critical paths first; light checks use the first freed
    # worker instead of delaying every heavy family by the wire suite.
    priority = {"rust": 0, "apple": 1, "android": 2, "rust-lints": 3}
    return [jobs for family, jobs in sorted(groups.items(), key=lambda item: priority.get(item[0], 4))]


def clippy_steps(steps):
    return [step for step in steps if step.run.lstrip().startswith("cargo clippy ")]


def run(chosen, plans, rows, repo, tmp, ctx, logdir, args, run_one_job):
    """Bounded parallel families, ordered steps, deterministic complete results."""
    import copy
    args = copy.copy(args)
    args.stop_event = threading.Event() if getattr(args, "fail_fast", False) else None
    def worker(jobs):
        own_rows = []
        failed = False
        for job in jobs:
            if args.stop_event and args.stop_event.is_set():
                break
            job_tmp = tmp / job
            job_tmp.mkdir()
            try:
                failed = run_one_job(job, plans, own_rows, repo, job_tmp, ctx, logdir, args) or failed
            except Exception as exc:
                own_rows.append((job, "family runner", f"FAIL: {type(exc).__name__}: {exc}", "", ""))
                failed = True
            if failed and args.stop_event:
                args.stop_event.set()
        return failed, own_rows

    # Clippy does not consume the binaries/PATH exported by Rust's test setup.
    # Run its feature variants serially beside the test family. Cargo's build
    # lock still protects artifacts; test execution can overlap lint compilation.
    chosen, plans = list(chosen), dict(plans)
    if "rust" in chosen:
        lints = clippy_steps(plans["rust"])
        if lints:
            plans["rust-lints"] = lints
            plans["rust"] = [s for s in plans["rust"] if s not in lints]
            chosen.append("rust-lints")
    started = time.monotonic()
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [pool.submit(worker, jobs) for jobs in families(chosen)]
        failed = False
        for future in futures:
            bad, job_rows = future.result()
            rows.extend(job_rows)
            failed = bad or failed
    if "ios" in chosen:
        rows.append(("ios", "iOS UI smoke", "not run here: CI; use scripts/ios-ui-tests.sh", "", ""))
    rows.append(("landing", "parallel families wall time", "FAIL" if failed else "pass",
                 f"{time.monotonic() - started:.2f}", ""))
    return failed
