# ov-1278: affected Rust tests on by default

## What changed

- **On by default.** The lane gate and the landing gate narrow the default-host nextest step to the crates the Cargo graph says a change reaches. `--full-rust` forces the whole suite in both modes. `--select-rust` is gone.
- **Selection (`scripts/rust-select.py`).** It reads `cargo metadata --all-features` and closes over reverse dependencies of every kind (normal, dev, build, optional, any target). Everything is selected when `Cargo.toml`, `Cargo.lock`, a build script, the toolchain file, `.cargo/config.toml`, a fixture, `crates/protocol`, a deleted or renamed file, or any path it cannot map changes. With no verified green ancestor commit it also selects everything.
- **Evidence.** Every gate run writes `rust-selection.json` into its log directory: `full`, `packages`, `reasons`, `selected_binaries` and `skipped_binaries`. To trace a red main to a skip, read the last landing's file.
- **Dead collector removed.** `scripts/rust-shadow.py`'s measuring runner, the `rust-shadow` job in `nightly.yml`, the "Green bases for Rust shadow" step in `ci.yml` and `docs/rust-shadow.md` are deleted. Nothing in code or docs claims a shadow miss record. The selector itself stays, renamed `rust-select.py`.
- **No separate nightly report.** The stale-skip report from ov-1268 is dropped; CI on main runs the full Rust suite on every push and is the safety net. `ci.yml` is otherwise untouched.

## Tests

- `scripts/rust-select-test.py`: graph closure, includes, every "select everything" input, test-binary split.
- `scripts/landing-policy-test.py`: real workspace, a `crates/vt` change selects a strict subset including `farcooler-vt`; `crates/protocol`, `Cargo.lock`, `Cargo.toml`, build scripts, toolchain files and `.cargo/config.toml` each select everything; `--full-rust` wins in both modes; the evidence file reads back selected and skipped binaries.
- `scripts/ci-concurrency-test.py` and `scripts/copy-lint.py` pass.

## What was measured, and what was not

- Measured earlier on ov-1268, each once, foreground, warm `target/`, host load average 19 to 45: a daemon and CLI change selected 4 of 18 crates (4962 of 7273 tests) and took 11m26s against 11m09s for the full run, so no saving. A `crates/vt` change selected 139 tests and ran in 1m48s including a rebuild.
- Not measured: paired before and after gate times on named commits, and a full gate rerun for this ticket. Those were deliberately skipped. The saving is for small leaf crates, not for the daemon or CLI.
- Not run: a live landing. The evidence file is covered by a test, not by a real gate run.
