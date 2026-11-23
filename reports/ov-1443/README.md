# ov-1443 · Quick checks before review

Before this change, Main's checked-in workflow ran the `daemon-layers.py` and
`file-size-budget.py` tree scans after Build. It had no pre-review step for
`shared-core-lint.py`. The ov-1341 draft adds a lint glob, but it is a separate
publication draft and does not cover the rest of the quick landing steps.
Landing scheduled Rust and Apple families ahead of Wire, where the shared-core
scan lived.

Main now runs `scripts/lane-gate.py --quick-only` on the committed build
candidate. A nonzero exit follows the existing Check rejection route to Build
with the failed command and its diagnostic. The next build's commit is checked
again before Review can start. Landing runs the same selection before screen and
load waits, the machine lock, or compile. The shared-core tree scan runs first.
Both modes derive commands and path selection from the landing plan.

## Quick landing steps

The table lists checks that can run against a clean checkout with the runner's
usual tools, require no compiled application or test artifacts, and normally
finish in under a minute per step. Timings come from the [cold landing
capture](../ov-1333/ov-1333-evidence/cold/summary.json) where available. The
current step inventory can be reproduced with
`scripts/lane-gate.py --quick-only --dry-run` against a base that selects every
family. A ticket runs the checks for its selected families. Wire and Doc comments
are always selected. Landing base `402409c0b` pauses the iOS and Android jobs
(ov-1440). The independent Wire checks still run, including the iOS shard check
on workflow edits.

| Landing job | Steps | Before review on the previous Main bundle |
| --- | --- | --- |
| Wire | `proto-lint.py --self-test`, preview and stable checks, and canary with `--since-baseline` | No |
| Wire | `canary-baseline-test.sh`, `canary-shipped.py --self-test`, `canary-plan.py --self-test`, `device-keys-readiness.py --self-test`, `site-proof.py --self-test`, `landing-timing.py --self-test` | No |
| Wire | `ci-changes.py --self-test`, `ci-concurrency-test.py`, `ci-runner-retry-test.py`, `quarantine.py --lint`, `rust-shards.py --self-test`, `macos-rust-tests.py --self-test --check`, `landing-gate-test.py`, `heavy_lock.py --self-test` | No |
| Wire | `notarytool-retry.sh --self-test`, `version-test.sh`, `workos-test.sh`, `appcast-test.sh`, `ci-flake-tally-test.sh`, `test-leak-check.py --lint` | No |
| Wire | `copy-lint.py` and `shared-core-lint.py` self-tests and tree scans, plus the shared-core baseline ratchet | No |
| Wire | `daemon-layers.py` self-test, tree scan, and counts ratchet | Tree scan only |
| Wire | `file-size-budget.py` self-test and tree scan | Tree scan only |
| Wire | `capability-names.py --self-test` and `--check`, and the Claude prices, OpenAI prices, and DERP hosts self-tests | No |
| Wire | Self-tests and tree scans for `plan-layer-lint.py`, `page-layer-lint.py`, `swallow-lint.py`, `visual-tokens-lint.py`, `pixel-scale-lint.py`, `test-window-lint.py`, `runloop-spin-lint.py`, `capture-window-lint.py`, `fresh-script-lint.py`, `charter-intercept-lint.py`, `ios-agentkit-subset-lint.py`, and `migration-lint.py` | No |
| Wire | Generated wire types and clean Git state for all three projections. Requires the installed pinned `typeshare 1.13.4` for the early check | No |
| Wire | `ios-ui-shards.py --check` on workflow edits | No |
| Doc comments | `doc-comment-check.py --self-test` and the base-to-candidate check | No |
| Tunnel | `gofmt -l` on the tunnel sources | No |
| Rust | `tmux-leak-check.py`, `tmux-stale-sockets.py`, and `test-leak-check.py` self-tests | No |
| Swift tests | `swift-build-cache.py self-test` and `macos-test-wrapper-test.sh` | No |
| Swift | `swift-build-cache.py self-test`, `gh-download-test.sh`, `macos-entitlements-test.sh`, `wait-for-unlock-test.sh`, `swift-test-guard-test.sh`, `test-leak-check.py --self-test`, `mac-capture-stop-test.sh`, and `capture-duplicates-test.sh` | No |
| Web | `web-client-tree-check.py --self-test` and tree scan, `size-gate.mjs --self-test` | No |

The paused iOS recipe keeps its quick-check classification for when the job is
restored. It covers `ios-ui-shards.py --check` and `--self-test`, simulator-picker
and UI-plan self-tests, and `import-ios-certificate-test.sh`. None ran before
review in the previous Main bundle.

The pre-review ratchets use the engine's pinned base. Landing retains the
comparison against freshly fetched main and does both fetches before compiling.
Landing retains the existing install and generation steps on a runner without
the pinned typeshare. A provisioned runner uses the read-only generator check
and the same Git diff and status checks.

The Wire lane-gate, lifetime, gate-evidence, engine-gate, iOS retry, and iOS lease
suites stay in the normal gate. Their process and timing fixtures can exceed a
minute. The Mac test partitions self-test also stays there after taking 61.4
seconds on the initial verification run. The Rust selection suite stays there
because it compiles a fixture workspace. Actionlint can build its patched Go executable
on cache miss. The macOS bundle, icon and Sparkle tests, iOS project generator and generator test, and
Rust quarantine integration suite compile app or fixture code. Android report
checks consume Gradle results, the JS typechecks and tests need an install, and
the browser rendering check bundles a harness. Swift toolchain selection and
cache restore/save are build setup and bookkeeping and stay with their builds.
Existing full-gate steps and CI coverage are retained.

## Regression and rollout

The local regression commits a real shared-core copy ratchet violation in a
candidate that selects Apple builds. It reads the pre-review Check command and
rejection route from the Main bundle. Before the change it fails because there
is no quick-check stage. With the change, the exact published command and the
landing entry point both exit 1 with the violating file and neither reaches
build planning. A second builder commit fixes the file and clears the check.

The rebase verification passed 78 steps in 371 seconds, including the Rust
selection test subsequently moved to the normal gate. The shared-core scan
took 5.8 seconds. The Mac wrapper fixture took 72 seconds under load, compared
with 18 seconds in the cold landing capture. The pre-review stage has a
ten-minute budget for the sequence.

The lane-gate self-tests, landing evidence tests, engine gate tests, and both
gate lifetime tests passed.
The flake-tally fixture also passed after replacing a short-reading pipe that
could fail with a broken pipe. The optional daemon Rust test waited at the
host's heavy-load limit and was stopped before starting.

The checked-in Main bundle has a new adapter pin. Publish it on the runner once
the scripts are available on the base and the pinned typeshare is installed.
The check commands put Homebrew Python first so the Rust manifest check gets
Python 3.11+ and its standard-library TOML parser. The live publication was
outside this ticket's branch work.
