# ov-1333: opt-in Rust compiler cache trial

The implementation is committed, but **the cold/warm landing measurements are
blocked by disk admission**. Leave `FARCOOLER_GATE_SCCACHE` unset. The default
remains off regardless of any eventual result. This ticket is not fully accepted
until the two measurement rows below are populated from actual landing gates.

| Run | Rust job span | sccache hit rate | Cache disk used |
| --- | --- | --- | --- |
| Baseline (ov-1332 / ov-1329) | 1510–1575 s | not measured | no compiler cache |
| Cold landing gate | not run: less than 70 GiB free | unavailable | unavailable |
| Warm landing gate | not run: less than 70 GiB free | unavailable | unavailable |

**A warm improvement of at least 180 s has not been demonstrated. Recommend
leaving the switch off.** Against the entire baseline range, that requires a
warm Rust span of at most 1330 s (1510 − 180), not merely beating 1575 s.
Do not substitute the server smoke test's zero requests for a gate hit rate.

## Disk and run budget

No full landing gate was started: **0 of at most 2 runs used**, including review.
The reviewer must use builder measurements and run no gate. Repeated foreground
`df -g .` / `df -k .` checks between 16:40 and 16:54 UTC on 2026-11-17
(00:40–00:54 Singapore, November 18) returned only 55–56 GiB free. The final
`df -k .` check at 16:53:58 UTC returned
58,800,000 KiB free (56.08 GiB). The heavy queue was empty at that check;
insufficient disk independently prevents starting. No unrelated files were
deleted to manufacture headroom. The gate's existing 61 GiB admission threshold
does not override this ticket's 70 GiB minimum.

## Implementation and ownership

`FARCOOLER_GATE_SCCACHE=1` is the only enabled value. Other values and an unset
switch leave step commands and environments unchanged. The regression test
compares the complete environment and shell command of every runnable workflow
step with the original merge of inherited and CI step settings; it also covers
the extracted `rust-lints` family. With the switch on, policy is applied after
step-level overrides and previous `GITHUB_ENV` updates, including nested Cargo
commands in platform scripts. Target directories stay per landing.

The fixed root is `~/.cache/farcooler-gate-sccache`, outside builder worktrees
and temporary directories. Its `tools`, `cache` and `tmp` directories must be
owned by the gate user with mode 0700 and no symlink path components. The tool
path cannot resolve through a symlink. Provision only from trusted gate context:

```sh
python3 scripts/gate_sccache.py --install
```

The installer ran successfully on this host: sccache 0.19.0, pinned and locked,
via `cargo install --root <gate-root>/tools --no-default-features`. Its Cargo
home/downloads, compilation target and temporary files are all under the same
gate root; no system installation or `~/.cargo` config was touched. Disk used
after installation: tools 5856 KiB, Cargo/downloads 140912 KiB, installation build
375216 KiB. These are separate from the compiler cache's 20 GiB ceiling.

The opt-in wrapper is the absolute gate-owned binary. `CARGO_INCREMENTAL=0`
and dev/test/release incremental profile overrides disable incremental builds.
`SCCACHE_DIR` points to `<gate-root>/cache`; `SCCACHE_CACHE_SIZE=20G` caps it.
Inherited sccache settings are removed; an empty gate config and a binary built
without remote features prevent routing to another storage backend. A private
Unix socket avoids another user's/default sccache server, and an exclusive
cache-session lock serializes access. The server runs without daemonization as
a child of the foreground gate and is stopped/reaped on normal completion and
exceptions. A startup failure fails the gate; it never substitutes a passing
result. Each run saves `sccache-server.log`, `sccache-stats.txt` from
`--show-stats`, and `sccache-disk.txt` from `du -sk` in its gate log directory.
The machine heavy lock is acquired before the cache-session lock, matching the
landing wrapper's outer lock order. The opt-in rejects releasing heavy between
jobs, which would invert that order against an ordinary landing gate.

**Builder isolation is a sandbox boundary, not Unix ownership alone.** This
host's builder sandbox grants writes only to its worktree and temporary paths.
An actual install attempt under that sandbox failed with `Operation not
permitted` creating the gate root; installation succeeded in trusted context.
Never add this root (or an ancestor) to builder write grants, and do not expose
the socket to builders. Mode 0700 does not isolate an unsandboxed builder that
runs as the gate user; such a deployment needs a separate gate account or an
equivalent enforced boundary before opting in. Scripts cannot revoke access
from an unsandboxed process with the same identity.

The configuration follows upstream's [local cache settings](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Local.md)
and [server implementation](https://github.com/mozilla/sccache/blob/v0.19.0/src/commands.rs).

## Measurement procedure once admitted

1. Check `df` on the target/cache volume and the machine heavy lock immediately
   before **each** full run. Start only with at least 70 GiB free and no holder;
   otherwise wait and recheck every few minutes. Retain normal gate locking.
2. Use the same source revision, selection, workers and toolchain as the
   baseline; run the landing gate with full Rust selection and the switch on.
   Start the cold run with the still-empty compiler cache and an empty target.
   Preserve every test and lint. Keep all logs and the source revision.
3. Run the warm landing gate against a fresh per-landing target, keeping only
   the compiler cache warm. Reusing the cold run's target would measure Cargo
   fingerprints rather than the compiler cache. This is the second and last
   full gate run; a failed attempt still counts. Do not run a third for review.
4. Derive Rust job span from first start to last end in `rust-timings.jsonl`,
   including gaps, not the sum of Cargo `Finished` messages. Read hits, misses,
   errors and non-cacheable calls from each saved `sccache-stats.txt`; report
   hits / (hits + misses), with an undefined rate if there are no cacheable
   requests. Report `du -sk` disk use and sccache's own cache size. Compare
   selections, test inventories and outcomes, and state the 180 s decision.

## Validation

- Gate self-test: PASS, including process supervision, policy, landing gate and
  benchmark fixture suites (`/tmp/ov-1333-self-test.log`). No full gate runs.
  Repeated after the lock-order change with the switch enabled: PASS
  (`/tmp/ov-1333-enabled-self-test.log`); synthetic fixtures isolate their own
  switch and fold context.
- Landing gate tests: PASS, 18 tests, including the new cache environment and
  path/permission regressions (`/tmp/ov-1333-landing-tests.log`).
- Policy suite: PASS, 24 tests (`/tmp/ov-1333-policy.log`).
- All 37 script lint invocations: PASS; every `*lint.py` self-test and scan,
  stable/canary protocol checks including `--since-baseline`, actionlint
  self-test/scan, test-leak lint and quarantine lint. Logs and exit codes:
  `/tmp/ov-1333-lints.log`, `/tmp/ov-1333-lints.json`.
- Installed server startup, stats and shutdown smoke: PASS. It reported local
  disk storage, version 0.19.0, max size 20 GiB, zero requests and 0 KiB used.
- `git diff --check`: PASS. No `cargo fmt` was run.

Tests used native Python 3.14 on PATH; the default Python executable has the
wrong architecture. Process fixture suites ran with process access because
the builder sandbox denies `ps`. Policy fixtures removed inherited engine
`FARCOOLER_*` context so their synthetic fold bases are independent. Initial
restricted/default-Python runs failed for these host reasons; final runs above
passed. All commands ran in the foreground.
