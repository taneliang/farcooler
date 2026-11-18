# ov-1349: run action weight landing

Started at `origin/main` (`7baa3ed87310d3fdbafee23b46807151dc7152d7`) and cherry-picked `67711dc0f`, `43df4df3f`, `7fa56f170`, `2548f7cbe`, and `a6243ec6c`, in order. The ov-1072 implementation and its acceptance evidence are in [the original report](../ov-1072/README.md).

## Validation

- `PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH sh scripts/lint-gate.sh` — passed all lint checks.
- `PATH="$HOME/.local/bin:$PATH" ./apps/macos/test.sh` — passed: 1,979 tests in 352 suites; the test floor (1,950) and leak checks passed. Run outside the sandbox because the process guard needs `ps`.

The default `/usr/local/bin/python3` is an incompatible Intel executable on this ARM machine, and `/opt/homebrew/bin/python3` is absent. `~/.local/bin/python3` points to Homebrew Python 3.14.10, which provides `tomllib` for the iOS planning self-test. An initial lint attempt using the default Python failed to launch it. An initial sandboxed Mac build could not create Clang temporary files; a retry with `TMPDIR=/private/tmp` built but the process guard was denied access to `ps`. The successful Mac run above used the ARM Python and the permissions required by the guard.
