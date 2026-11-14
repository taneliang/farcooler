# ov-1181 validation

Starting point: `ov-1160-mac-rust-only` at `d3183b206`. The hosted before/after measurements and red-first selection tests are retained in `reports/ov-1160/`; `python3 reports/ov-1160/verify-hosted.py` passed here (full: 2560 s; selected: 724 s).

- `actionlint .github/workflows/*.yml`: passed with ShellCheck enabled.
- `python3 scripts/ci-changes.py --self-test`: passed.
- `/opt/homebrew/bin/python3.14 scripts/macos-rust-tests.py --self-test --check`: passed, including unselected and partially selected Mac-only dependency fixtures.
- Reviewed the workflow diff from the starting point: only narrowly scoped ShellCheck directives and explanatory comments were added; no job commands or triggers changed. Reviewed the inherited CI and nightly workflow diff against the branch base: Linux retains the full suite, macOS builds the workspace and runs the native selection, and nightly on main runs the full macOS suite.
