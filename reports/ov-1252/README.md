# ov-1252: gate size budget and the provision race

## Changes

- `crates/daemon/src/git.rs` is 1,494 lines (was 1,506). The guarded/unguarded
  environment setup in `spawn_bounded` moved into `git_guard::prepare`; behavior is
  unchanged (guarded: pins, work tree, git-lfs `PATH`; unguarded: strip inherited `GIT_`
  variables, ov-1250). `scripts/file-size-budget.txt` is untouched.
- `.config/quarantine.txt`: `farcooler-daemon` `workflow_runtime::coding_tests::workflow_coding_two_tickets_start_with_different_models_and_each_build_launches_its_own`, ov-1220, expires 2026-11-29.

## Commands and results

- `./scripts/file-size-budget.py`: ok.
- `cargo nextest run -p farcooler-daemon git_sandbox`: 15 passed, including
  `the_control_ignores_the_runners_git_environment`.
- `python3 scripts/quarantine.py --lint` and `--lint --source`: exit 0. The name was
  confirmed against `cargo nextest list -p farcooler-daemon`.
- `cargo clippy --workspace --all-targets -- -D warnings`: finished clean.
- `CI=true ./scripts/lane-gate.py --base origin/main --only wire,doc-comments < /dev/null`:
  exit 0, 65 steps pass. With the prescribed PATH alone, `python3` is the system 3.9.6 and
  `macos-rust-tests.py --self-test --check` fails ("requires Python 3.11"); put Python
  3.11+ first on PATH (I used 3.14).
