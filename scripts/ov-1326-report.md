# ov-1326 script-user map report

Base: `f1d8cd63eb7a57501a9f4a1f604f0a88808c7654`. `SCRIPT_USERS` covers **218 files**: **109 map to fewer than all families**, and **109 explicitly stay full**. The entries use gate job names; swift, swift-tests and ios share the Apple scheduling family. Wire and doc-comments remain unconditional. Every narrowed entry has a one-line execution/import reason beside it. Full entries are listed below.

Consumers were traced from `lane-gate.steps_for` (CI run steps plus the iOS composite action), the gate self-test chain, Python imports/dynamic loads, and shell/subprocess calls. A lint reading Apple or Rust source does not make that platform a consumer of the lint. Regression suites use scratch tools/repos; mere copying into the self-test repository is not execution. Unresolved build/release helpers and workflows remain full.

Coverage is checked by the landing policy suite, already invoked by the gate self-test in wire. It enumerates cached and untracked, nonignored files with `git ls-files --cached --others --exclude-standard -z -- scripts/ .github/workflows/`; a missing entry fails. Deleted entries may remain to cover deletion diffs. Runtime unknowns still select full, and newly named direct consumers augment reviewed sets.

## Historical selection replay

The ticket says 43 commits since Nov 15. This checkout actually contains **458 commits** since `2026-11-15T00:00:00+08:00` through the base, using Git committer dates. With no supplied SHA cohort, the table uses the **most recent 43 commits** at that base; all are since Nov 15. Across those 43, **4 would run a narrower gate**. Across the complete since-Nov-15 range, **16 of 458** would narrow. No replay expands selection.

Each commit is replayed independently with `git diff --no-renames --name-only <sha>^ <sha>`. Before is the policy at the base (ov-1324); after is this map. Both use the same current CI commands and family rules. This is selection evidence, not measured runtime or Rust package selection. No full gate was run.

| Commit | Subject | Before | After |
| --- | --- | --- | --- |
| `b4cf8a0c0` | ov-1301: a builder a landing gate holds stopped keeps its time, and running out of it says so | **all 11** | **all 11** |
| `da29626b1` | ov-1310: retry macOS runner acquisition failures | **all 11** | **all 11** |
| `6e1ea85f0` | ov-1310: retry all cancelled Mac shards once | wire, doc-comments | wire, doc-comments |
| `eefe9eb5b` | ov-1310: track automatic retries across CI attempts | wire, doc-comments | wire, doc-comments |
| `91c9c5bb5` | ov-1311: record runner retry carry verification | wire, doc-comments | wire, doc-comments |
| `6bdf5161c` | ov-1305: gate folded candidates against their recorded fold base | **all 11** | **all 11** |
| `8112dfb33` | ov-1305: document stale base receipts and replay ten landed plans | wire, doc-comments | wire, doc-comments |
| `04f7ef6f7` | ov-1305: assert light-job lock behavior without a timing threshold | **all 11** | **all 11** |
| `19188513b` | ov-1305: include landed candidates without retained push facts | wire, doc-comments | wire, doc-comments |
| `ea58046de` | ov-1305: record passing validation and isolate evidence fixture context | **all 11** | wire, doc-comments |
| `b156baa55` | ov-1307: record gate verification | wire, doc-comments | wire, doc-comments |
| `2c88c0dd4` | ov-1307: record full Rust suite result | wire, doc-comments | wire, doc-comments |
| `432913ced` | ov-1307: capture native test diagnostics and verification | **all 11** | **all 11** |
| `695b3ff7c` | ov-1307: record review run flaky result | wire, doc-comments | wire, doc-comments |
| `da398eaaf` | ov-1313: record combined branch verification | wire, doc-comments | wire, doc-comments |
| `9a5c7e9b2` | ov-1308: bound Codex launches and retain window refusal reasons | **all 11** | **all 11** |
| `6858b8caa` | ov-1308: exercise the generated Codex launch through tmux | **all 11** | **all 11** |
| `18bb8e01b` | ov-1308: verify generated shell and pinned hooks in startup test | **all 11** | **all 11** |
| `ac2bfd884` | ov-1308: record full daemon and gate verification | wire, doc-comments | wire, doc-comments |
| `9460c3e89` | ov-1308: isolate Swift scratch daemon shells | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `762f65b17` | ov-1308: record repaired Swift gate validation | wire, doc-comments | wire, doc-comments |
| `3033e382e` | ov-1312: wait for resize restoration before the next test step | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `63374831f` | ov-1312: record CI evidence and resize test verification | wire, doc-comments | wire, doc-comments |
| `5f328d4fa` | ov-1318: reproduce stale landing base selection | **all 11** | wire, doc-comments |
| `54f2d170f` | ov-1318: recover landing base from exact durable fold record | **all 11** | **all 11** |
| `869f56103` | ov-1317: check Canary wire history in CI and the landing gate | **all 11** | **all 11** |
| `ef9ba0fdf` | ov-1317: distinguish pagination from board pages and record passing tests | **all 11** | **all 11** |
| `86a87f170` | ov-1317: preserve the recovery anchor through cherry-pick landing | **all 11** | **all 11** |
| `ff2fb0c55` | ov-1317: record passing fix-round validation | wire, doc-comments | wire, doc-comments |
| `068444023` | ov-1317: keep next_after and recover the unshipped wire baseline | **all 11** | **all 11** |
| `b9ac58fca` | ov-1317: record passing layer lints and TaskList tests | wire, doc-comments | wire, doc-comments |
| `cda08e591` | ov-1317: restore the original TaskList cursor name and recovery snapshot | **all 11** | **all 11** |
| `0d8d720fa` | ov-1317: distinguish task pagination in page lint and record passing checks | **all 11** | wire, doc-comments |
| `6f5e85d1f` | ov-1315: wait for uncertain CI recovery effect | **all 11** | **all 11** |
| `1e39652af` | ov-1315: record daemon integration verification | wire, doc-comments | wire, doc-comments |
| `83c9d8e59` | ov-1315: record complete daemon verification | wire, doc-comments | wire, doc-comments |
| `06fa623ab` | ov-1320: retry failed main CI jobs once | wire, doc-comments | wire, doc-comments |
| `e43502da3` | test(macos): wait for the shell prompt before typing claude (ov-1323) | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `12e282c75` | docs: record ov-1323 gate analysis and capped-load validation | wire, doc-comments | wire, doc-comments |
| `c242dcde3` | test(macos): wait through live lookup and watcher delays (ov-1323) | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `d79d5af53` | docs: prove ov-1323 lookup and watcher races under capped load | wire, doc-comments | wire, doc-comments |
| `54c9e4021` | ov-1324: reproduce CI-only script over-selection | **all 11** | wire, doc-comments |
| `f1d8cd63e` | ov-1324: select reviewed script consumers and retain workflow checks | **all 11** | **all 11** |

## Explicit full entries

- `.github/workflows/canary-baseline.yml`
- `.github/workflows/canary.yml`
- `.github/workflows/ci.yml`
- `.github/workflows/claude-prices.yml`
- `.github/workflows/derp-hosts.yml`
- `.github/workflows/doc-comments.yml`
- `.github/workflows/linux-binaries.yml`
- `.github/workflows/nightly.yml`
- `.github/workflows/openai-prices.yml`
- `.github/workflows/ov1186-cache.yml`
- `.github/workflows/promote.yml`
- `.github/workflows/relay.yml`
- `.github/workflows/release.yml`
- `.github/workflows/site-analytics.yml`
- `.github/workflows/site.yml`
- `.github/workflows/tunnel-e2e.yml`
- `.github/workflows/web.yml`
- `scripts/android-ndk.sh`
- `scripts/build-android-libs.sh`
- `scripts/build-derper.sh`
- `scripts/build-ios-frameworks.sh`
- `scripts/build-linux.sh`
- `scripts/build-tailcat.sh`
- `scripts/build-tunnel-helper.sh`
- `scripts/capability-names.py`
- `scripts/capture-contrast-check.sh`
- `scripts/capture-duplicates-allow.txt`
- `scripts/capture-duplicates.sh`
- `scripts/ci-changes.py`
- `scripts/ci-install-tmux.sh`
- `scripts/ci-job-time-check.py`
- `scripts/claude-rewake-check.py`
- `scripts/com.farcooler.site-proof.plist`
- `scripts/demo-host.sh`
- `scripts/engine-gate.py`
- `scripts/engine-old-reader.py`
- `scripts/fold-check.sh`
- `scripts/gate-evidence-mutation-test.py`
- `scripts/gate-flaky.py`
- `scripts/gate-summary-check.py`
- `scripts/gate-test-fixture.py`
- `scripts/gate_evidence.py`
- `scripts/gen-wire-types.sh`
- `scripts/gh-api-retry.sh`
- `scripts/gh-download-test.sh`
- `scripts/gh-download.sh`
- `scripts/git-hooks/commit-msg`
- `scripts/heavy`
- `scripts/heavy_lock.py`
- `scripts/icon-label.swift`
- `scripts/icon-test.sh`
- `scripts/import-ios-certificate.sh`
- `scripts/install-git-hooks.sh`
- `scripts/ios-ui-pick-simulator.py`
- `scripts/ios-ui-plan.py`
- `scripts/ios-ui-shard-budget.py`
- `scripts/ios-ui-shards.py`
- `scripts/ios-ui-tests.sh`
- `scripts/landing-benchmark.py`
- `scripts/landing-gate`
- `scripts/landing_policy.py`
- `scripts/lane-gate.py`
- `scripts/mac-capture.sh`
- `scripts/macos-bundle-native-test.py`
- `scripts/macos-entitlements-test.sh`
- `scripts/macos-rust-tests.json`
- `scripts/macos-rust-tests.py`
- `scripts/macos-test-partitions.py`
- `scripts/manager-skill-pressure/fake-farcooler.sh`
- `scripts/manager-skill-pressure/fake-gh.sh`
- `scripts/manager-skill-pressure/new-scratch-repo.sh`
- `scripts/manager-skill-pressure/render-skill.sh`
- `scripts/manager-skill-pressure/run-claude.py`
- `scripts/manager-skill-pressure/scenarios.md`
- `scripts/manager-skill-pressure/score.py`
- `scripts/nightly-dispatch-test.py`
- `scripts/nightly-report.py`
- `scripts/notarytool-retry.sh`
- `scripts/ov-1324-report.md`
- `scripts/ov-1326-report.md`
- `scripts/portal-app-groups.rb`
- `scripts/process_guard.py`
- `scripts/quarantine.py`
- `scripts/reap-orphan-daemons.py`
- `scripts/reap-stale-tmux.sh`
- `scripts/release.sh`
- `scripts/runner-upgrade-e2e.py`
- `scripts/rust-archive.sh`
- `scripts/rust-select.py`
- `scripts/rust-shards.py`
- `scripts/site-proof-milestones.json`
- `scripts/site-proof-refresh.sh`
- `scripts/site-proof-tokens.json`
- `scripts/sparkle-key-test.sh`
- `scripts/sparkle-public-key.swift`
- `scripts/stage-demo-transcript.py`
- `scripts/stage-demo.sh`
- `scripts/swift-test-guard.sh`
- `scripts/test-floor-check.py`
- `scripts/test-leak-check.py`
- `scripts/tmux-leak-check.py`
- `scripts/tmux-stale-sockets.py`
- `scripts/tunnel-e2e.sh`
- `scripts/tunnel-smoke.sh`
- `scripts/version.sh`
- `scripts/wait-for-unlock`
- `scripts/workflow-review-timing-test.py`
- `scripts/workflow-review-timing.py`
- `scripts/workos-client-id.sh`

## Validation

Pending final checks.
