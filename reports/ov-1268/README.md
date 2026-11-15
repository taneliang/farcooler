# ov-1268: the gate skips Rust tests a change can't affect

## Shadow record (acceptance 1)

- Command: `gh run list --workflow nightly.yml --limit 100 --json databaseId,headBranch` for `main`, then `gh run view <id> --json jobs` filtered on `shadow`; and `gh api "repos/{owner}/{repo}/actions/artifacts?name=rust-shadow-linux" -q .total_count`.
- Result: **0 `rust-shadow-linux` artifacts, and no `rust-shadow` job in any of the last 20 main Nightly runs.** The record is empty, so it shows zero misses only because it holds zero samples. That is not evidence the selection is safe.
- What stands in for it: `scripts/rust-shadow-test.py` plant proofs (an affected test fails under the selected gate; an unaffected one passes) and the new real-workspace test below. Treat the skips as unproven by shadow data; CI on main is the backstop.
- Follow-up: restore the nightly `rust-shadow` job so a record accumulates.

## Selection (acceptance 2, 4)

- `scripts/lane-gate.py` now narrows the default-host nextest step in the plain gate as well as `--landing`. `--full-rust` turns it off.
- `landing-policy-test.py::test_real_workspace_leaf_narrows_and_shared_inputs_select_everything` uses this repo's real `cargo metadata`: a `crates/vt` change selects a strict subset including `farcooler-vt`; `crates/protocol`, `Cargo.lock`, `Cargo.toml`, `crates/*/build.rs`, `rust-toolchain*` and `.cargo/config.toml` each select the whole workspace; CI's workflow carries no `package(=` filter.
- Doctests, feature checks and clippy stay full. `ci.yml` is unchanged, so CI on main runs the full suite.

## Gate time (acceptance 3)

Measured once each, foreground, on this worktree's warm `target/`. The host was busy (load average 25 to 45), so read the ratio, not the seconds. Not measured on the two named historical commits: they predate the current workflows, and `lane-gate.py` cannot plan against them.

| Change | nextest step | Wall |
| --- | --- | --- |
| leaf crate (`crates/vt`) | `-E 'package(=farcooler-vt)'`, 139 tests | 1m48s (includes a rebuild) |
| shared crate (`crates/protocol`) | selects everything, same as full | 11m09s, 7273 tests |

The full run had 4 failures under that load (a client real-daemon test and three daemon script-hook tests). They are timing-sensitive and unrelated to this change; unchanged by it.

## Scheduled full run

Nightly's `rust-macos` job already runs the whole leak-checked suite on main with no selection, and CI runs it on every push. No new workflow was added; naming the stale skipped test on a nightly failure is not done.
