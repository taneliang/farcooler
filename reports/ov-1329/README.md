# ov-1329: Where a Rust landing gate spends its time

I would first remove the local archive round trip while preserving the same compiled test binaries, wrappers and selection; measure it before broadening the change. The expected saving is modest (roughly 0.1–0.3 minutes), but its coverage risk is lower than narrowing tests or sharing a mutable build directory. Most of the apparent 2.6–3.0 minute archive step is compilation that still has to happen.

## Measurements

These are the three complete Rust-selecting gates named below, in seconds, rounded to 0.1. Sources are `/tmp/lane-gate-logs-<column>/rust-timings.jsonl` and `rust-lints-timings.jsonl`; submeasurements come from the corresponding numbered `rust-*.log` files. The timing files record monotonic step start/end and elapsed time, not an independently measured end-to-end landing duration. The Rust job spans 25.2–26.2 minutes; setup and reporting outside these records can add time. Skipped tool installers have zero recorded time. Values within a parent row are included in it, not additional time.

| Rust step or concurrent family | mwr8k8gf | cqalrde0 | abft9e9e |
|---|---:|---:|---:|
| linked host check | 16.9 | 15.4 | 16.4 |
| linked musl cross-check | 147.8 | 143.3 | 147.4 |
| helper feature nextest, including compile | 164.9 | 163.8 | 166.9 |
| five daemon helper tests, sequential commands, including compile | 291.6 | 297.7 | 283.7 |
| daemon tailcat all-target check | 45.6 | 49.3 | 65.7 |
| workspace runtime binary build | 54.2 | 53.5 | 56.6 |
| four leak/quarantine self-tests | 58.2 | 62.1 | 61.5 |
| archive command, including build | 158.3 | 165.3 | 177.1 |
| ↳ nextest test-profile build (log, rounded to whole seconds) | 143 | 150 | 159 |
| ↳ archive packing (log) | 2.4 | 2.5 | 2.3 |
| main nextest command, including extraction, wrappers and reruns | 563.3 | 594.2 | 590.5 |
| ↳ first extraction (log) | 5.6 | 5.8 | 7.1 |
| ↳ first nextest run (log summary) | 536.4 | 552.6 | 547.9 |
| ↳ rerun extraction, if present (log) | 0.0 | 9.3 | 9.3 |
| ↳ rerun nextest summaries (log) | 2.6 | 3.6 | 3.6 |
| workspace doctests | 9.2 | 9.4 | 8.9 |
| **Rust job span** | **1510.0** | **1554.2** | **1574.6** |
| Rust lints, concurrent job span | 620.2 | 619.1 | 613.3 |
| Swift tests, concurrent family span | 662.3 | 676.8 | 728.2 |
| Android, concurrent family span | 810.1 | 817.6 | 826.9 |
| wire, concurrent family span | 433.1 | 448.8 | 458.6 |
| latest non-Rust recorded step end, relative to Rust start (wire) | 1053.3 | 1068.0 | 1071.9 |

Archive logs are respectively `rust-9261582.log`, `rust-58042400.log`, `rust-56471409.log`; main run logs are `rust-99889518.log`, `rust-92315277.log`, `rust-33783785.log`. The first run starts 7,417 tests across 159 binaries in each. It reports one, two and two failures respectively; the local flaky wrapper reruns those successfully. Extraction and test summary timers do not account for all wrapper, discovery and cleanup time. In mwr8k8gf the rerun does not log a second extraction. The first main run's 536–553 seconds is a concurrent nextest pool, not a sum of test durations. The five helper commands above run sequentially, but their time includes compilation: for example mwr8k8gf's helper-feature test log `rust-59275552.log` reports 163 seconds compiling and 0.55 seconds running 21 tests. Calling all of that serial test execution would be misleading.

The critical recorded path is the sequential Rust job through feature checks and builds, archive build, main nextest and doctests. Rust begins alongside lints and the other families, and finishes 457–503 seconds after the last recorded non-Rust step. The Apple family runs Swift tests, Swift and iOS sequentially; Android, wire and other families have their own scheduling, so their spans must not be added to Rust. The last wire end includes its delayed start; see each `wire-timings.jsonl`. Other last ends relative to Rust start are web 1015/1030/1062 seconds and Apple iOS 886/912/956 seconds. The baseline floor after accelerating Rust is therefore roughly 17.6–17.9 minutes in these runs, assuming the other lanes do not change under different contention. This is not a universal 10.5-minute floor.

`rust-selection.json` says all three are full: mwr8k8gf and cqalrde0 include gate/policy script changes, while abft9e9e includes unknown report inputs. All list 159 selected and zero skipped binaries. They do not demonstrate the time of a narrowed run. `/tmp/lane-gate-logs-ov1322-measured` is not a fourth complete Rust measurement: its Rust timings stop after the daemon check, although Swift tests continue to 1673.5 seconds. It shows that contention or a different run can put another family on the critical path.

## Ranked opportunities

Estimates below are conditional, non-additive wall-time hypotheses, not benchmark results. They assume the other families' finish times above remain fixed; any Rust time removed beyond their finish saves no gate wall time. Ranking weighs likely saving against coverage and operational risk.

1. **Local no-archive execution — 0.1–0.3 minutes, low to medium risk.** Packing takes 2.3–2.5 seconds and initial extraction 5.6–7.1 seconds; avoiding repeated extraction on flaky reruns can save another 9 seconds in two examples. The 143–159 seconds compiling test targets is still required, as are runtime binaries. Use the same profile, filters, environment, leak/quarantine/flaky wrappers and runtime binaries directly from the gate tree; verify target discovery and compare executed test inventories before adoption. Risk: archive remapping, included files or runtime binary paths differ. Keep archive-based full CI on main to exercise the artifact and fail on missing inputs; compare local and archive inventories before relying on local proof.

2. **Extend safe changed-crate selection to more confidently classified inputs — 0–8 minutes, medium to high risk.** `scripts/landing_policy.py` already selects the reverse dependency closure against a verified green ancestor and filters only the local default-host nextest; features, lints and doctests stay full. `scripts/rust-select.py` includes dev/build/optional/target edges and a CLI-to-daemon edge. The observed full loop is 536–553 seconds; with the other-family floor, at most 457–503 seconds of Rust-path improvement can affect wall time here. Actual savings depend on which binaries remain and can be zero. These examples all deliberately fall back to full: unknown paths, deletes/renames, manifests, build scripts, fixtures, config, protocol/generated inputs and relevant workflow changes are conservative; missing green history, network or metadata also falls back to full. Only classify a path after proving all consumers; preserve full fallback for uncertainty. A missed dependency could allow a real regression through the landing gate, so retain full main CI and periodically compare full versus selected outcomes and recorded skipped binary inventories; a post-merge failure is later detection, not equivalent landing protection.

3. **Reuse compilation through a controlled shared cache — 0–4 minutes, medium to high risk.** The cross-check is 143–148 seconds, helper step 164–167 seconds and archive test compilation 143–159 seconds; some may hit a cache, but these logs do not establish hit rates or portability between worktrees. This estimate assumes roughly 1–4 minutes of compilations are reusable, capped by the other-family floor; zero is possible. Prefer a content-addressed compiler cache with toolchain, flags, target and inputs in the key over sharing a live `target` directory. Risk: stale or cross-worktree artifacts and lock contention; gate must still invoke Cargo with fingerprint validation, isolate mutable outputs, and fail closed on cache errors. Full clean CI on main and periodic clean-versus-cached comparisons catch cache mistakes, potentially after landing.

4. **Parallelize independent helper-feature commands only after isolating their resources — 0–2 minutes, high risk and uncertain.** The five sequential commands total 284–298 seconds, but individual logs show compilation within them; concurrent Cargo commands may serialize on locks or contend with other families. A two-minute saving assumes about 120 seconds of truly independent work overlaps; the logs do not prove that. Run separate target directories or prebuild once, retain every test and feature combination, and benchmark under gate load. Shared tunnel, tmux, ports or host state could cause interference or mask failures. Preserve isolation and strict failure propagation, compare repeated serial and parallel runs, and keep serial full CI until equivalence is established.

5. **Relax serial nextest groups selectively — 0–0.1 minutes, high risk for little demonstrated gain.** `.config/nextest.toml` limits slow tmux tests to one, daemon socket tests to four, file events to two, and a few PTY-sensitive tests to exclusive execution. In these logs the 19 `farcooler-tmux server::tests::` PASS durations sum to only 6.3–6.5 seconds, and they can overlap other tests; their sum is an upper bound on their removable contribution, not measured idle time. The configuration documents real shared tmux, watcher and PTY contention and previous flakes. Parallelize only independently isolated tests, keep all tests and failure handling, and validate repeatedly under load; retain the conservative CI groups to catch interference-sensitive failures. This evidence does not justify globally increasing concurrency.
