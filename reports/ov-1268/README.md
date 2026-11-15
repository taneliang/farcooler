# ov-1268: the gate skips Rust tests a change can't affect

## Shadow record (acceptance 1)

- Command: `gh api "repos/{owner}/{repo}/actions/artifacts?name=rust-shadow-linux" -q .total_count` gives **0**; `gh run view <id> --json jobs` on the newest main Nightly (43619850635, 2026-11-14, head 73974f898) lists no `rust-shadow` job.
- Why: the `rust-shadow` job was added to `nightly.yml` in 1fe14084d on 2026-11-15, after the last Nightly that ran. No Nightly has run with it yet, so the record has **zero samples**. Zero samples is not zero misses.
- Decision: the plain lane gate keeps the **full** Rust suite by default, and narrows only with `--select-rust`. The default flips once the nightly record has samples and zero misses. `--landing` already selected before this ticket (ov-1185) and still does. `--full-rust` forces everything in both modes (regression test `test_full_rust_forces_the_whole_suite_in_both_modes`).
- Next: the next scheduled Nightly produces the first `rust-shadow-linux` artifact; read its miss count with the command above.

## Selection (acceptance 2, 4)

- `scripts/lane-gate.py` now narrows the default-host nextest step in the plain gate as well as `--landing`. `--full-rust` turns it off.
- `landing-policy-test.py::test_real_workspace_leaf_narrows_and_shared_inputs_select_everything` uses this repo's real `cargo metadata`: a `crates/vt` change selects a strict subset including `farcooler-vt`; `crates/protocol`, `Cargo.lock`, `Cargo.toml`, `crates/*/build.rs`, `rust-toolchain*` and `.cargo/config.toml` each select the whole workspace; CI's workflow carries no `package(=` filter.
- Doctests, feature checks and clippy stay full. `ci.yml` is unchanged, so CI on main runs the full suite.

## Gate time (acceptance 3)

Each measured once, foreground, warm `target/`, host load average 19 to 45, so read the ratio. The nextest step is the whole cost the selection changes; features, clippy and doctests are identical before and after.

| Named commit | Selection | Before (full) | After (selected) |
| --- | --- | --- | --- |
| `d612b7e42` (leaf: cli, daemon) | cli, client, daemon, jni: 4 of 18 crates, 4962 of 7273 tests | 11m09s (7273 tests) | 11m26s (4 unrelated-to-change timing flakes) |
| `8b4ed22bd` (shared: `crates/protocol`) | everything (full) | 11m09s | same command, 11m09s |

- "Before" is one full-workspace run, which does not depend on the change.
- Result: for a real change in the daemon and CLI the selection saves nothing measurable, because those crates hold most of the tests. The saving is for small leaf crates: a `crates/vt` change selected 139 tests and ran in 1m48s including a rebuild.
- Both runs had load-related failures (client real-daemon, CLI hook, daemon script-hook tests). They fail the same way with and without selection.

## Scheduled full run

- `nightly.yml`'s `rust-macos` and `rust-shadow` jobs run the whole workspace with no selection, every night. No caching of test results is introduced, so there are no external-input cache keys (tmux, OS, toolchain) to add.
- A red nightly now names the failing Rust test: `scripts/nightly-report.py` reads nextest `FAIL` lines as `rust <crate>` and tells the reader that a failure the gate skipped is a selection miss. Self-test covers the parse.
- Not verified on a live Nightly yet; the next scheduled run exercises it.
