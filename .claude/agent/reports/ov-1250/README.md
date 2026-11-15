# ov-1250: the sandbox test that failed only under the landing gate

## The single cause

`GIT_OPTIONAL_LOCKS=0`, exported by the landing gate, reached the test's
unguarded git.

- `scripts/lane-gate.py --landing` calls `landing_policy.readonly_git(os.environ)`.
  `budget_worker` calls it again for each family's env (ov-1185). That sets
  `GIT_OPTIONAL_LOCKS=0` plus one `GIT_CONFIG_*` entry
  (`diff.autoRefreshIndex=false`) for every step, including the Rust tests.
- With optional locks off, `git status` does not write back the refreshed
  index. With no index write, `post-index-change` never fires.
- `git_sandbox::alone` turns the guard off (`git::UNGUARDED`), and that test-only
  path skipped `git_guard::apply` entirely, including its removal of inherited
  `GIT_` variables. So the daemon's status in the control arm inherited the
  runner's `GIT_OPTIONAL_LOCKS=0`, no hook ran, and the control failed:
  "with neither guard, nothing ran". Production can never see this: every
  guarded git strips inherited `GIT_` variables first.
- The failure time gives it away: 1.2 s is one `restat` sleep (1.1 s) plus one
  status. A passing run takes 2.4 s, two of each.

The TMPDIR, the wrappers, the quarantine runner, `env -i` and parallel jobs
make no difference: the engine's env through every wrapper passes. Adding
the one variable fails it every time.

## Reproduce (before the fix, at bb13cdc26)

```
env -i HOME=$HOME USER=$USER LOGNAME=$USER \
  PATH=/Users/e-liang/.local/bin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin \
  CARGO_BUILD_JOBS=4 GIT_OPTIONAL_LOCKS=0 \
  ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- python3 scripts/quarantine.py run -- \
  cargo nextest run --profile ci -p farcooler-daemon git_sandbox::alone < /dev/null
```

- Without `GIT_OPTIONAL_LOCKS=0`: 6/6 pass, and the hook test takes 2.365 s.
- With it: `TRY 1/2/3 FAIL [~1.22s]` at `git_sandbox.rs:838`, the gate's exact
  signature.
- With only the `GIT_CONFIG_*` entry (`diff.autoRefreshIndex=false`): passes.

## The fix

The fix is in the daemon's test seam, not the gate. The gate's readonly git is
deliberate: it keeps the protocol build stamp warm. Also, any shell or editor
that exports `GIT_OPTIONAL_LOCKS=0` (many do, for background status) broke
this control the same way.

- `git_guard::strip_inherited` is split out of `git_guard::apply`.
- `git::spawn_bounded` calls it on the unguarded path too. That path still
  gets no pins, no flags and no work tree, but it no longer gets the runner's
  `GIT_` variables.
- The sandbox and every pin are unchanged. The security assertion is unchanged
  too, and it is now never vacuous: under the gate's env, the sandbox arm
  could only pass because no hook would have run anyway.

## After the fix

The same command plus the gate's full readonly-git env, run in the
foreground:

```
env -i HOME=$HOME USER=$USER LOGNAME=$USER PATH=... CARGO_BUILD_JOBS=4 \
  GIT_OPTIONAL_LOCKS=0 GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.autoRefreshIndex GIT_CONFIG_VALUE_0=false \
  ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- python3 scripts/quarantine.py run -- \
  cargo nextest run --profile ci -p farcooler-daemon git_sandbox git_guard git:: < /dev/null
```

Result: `44 tests run: 44 passed`, including
`git_sandbox::alone::a_planted_hook_is_refused_with_the_pins_off`.

## Regression check

`git_sandbox::alone::the_control_ignores_the_runners_git_environment` reruns
the hook test in a child of its own test binary, with the gate's
`GIT_OPTIONAL_LOCKS=0` and `GIT_CONFIG_*`.

- With the `strip_inherited` call on the unguarded path commented out, it
  fails (1.24 s, the child panics at `git_sandbox.rs:838`).
- With the fix, it passes.

## Quarantine

The ov-1248 entry is removed from `.config/quarantine.txt`, and
`python3 scripts/quarantine.py --lint` passes. The test was never flaky.

## Also run

`cargo clippy -p farcooler-daemon --all-targets -- -D warnings`: clean.
