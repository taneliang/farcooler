# ov-1333: opt-in Rust compiler cache trial

**Leave `FARCOOLER_GATE_SCCACHE` unset. The warm run did not demonstrate an
improvement of at least 180 s across the 1510–1575 s baseline range.** Its
1348.13 s Rust span saves 161.87 s against 1510 s and 226.87 s against 1575 s.
The conservative threshold is 1330 s (1510 − 180); this run misses it by 18.13 s.
Default remains off regardless of the result.

| Run | Rust job span | Overall sccache hit rate | Rust hit rate | Cache disk used (`du -sk`) | Gate result |
| --- | ---: | ---: | ---: | ---: | --- |
| Baseline (ov-1332 / ov-1329) | 1510–1575 s | not measured | not measured | no compiler cache | passed |
| Cold | 1394.72 s | 27.11% (1255 / 4630) | 3.13% (58 / 1851) | 1,586,620 KiB (1.51 GiB) | one host-tool failure; doctests not reached |
| Warm | 1348.13 s | 86.88% (4032 / 4641) | 67.45% (1256 / 1862) | 2,636,132 KiB (2.51 GiB) | all steps passed |

Hit rates are hits / (hits + misses), directly from each run's
`sccache --show-stats`. Stats report cache sizes rounded to 1 GiB cold / 3 GiB
warm, with a 20 GiB maximum in both. Both had zero cache read/write errors,
timeouts and unsupported compiler calls. Non-cacheable calls: 1297 cold,
1431 warm. Both report 15 compilation failures (individual invocations were not
attributed); these are distinct from cache errors and failed gate steps.

## Runs, admission and evidence

**Exactly 2 of 2 full gate attempts used, including review. Do not run a third.**
The earlier disk-blocked work used none. Review must use these measurements.
Both runs used source `23dfb8d9f`, base
`aa1be1f49fccce49cc5db03341807e32b9311504`, native Python 3.14,
sccache 0.19.0, four landing workers, and full Rust selection. Each used its own
fresh detached Git worktree with no target folder. The compiler cache was
verified empty before cold and retained for warm. No targets were copied.

- Cold admitted at 2026-11-17 16:58:55 UTC: `df -k` showed 89,150,216 KiB free
  (85.02 GiB), no live heavy tickets. Tree: `/tmp/ov-1333-cold`.
- Warm admitted at 2026-11-17 17:23:52 UTC: 84,786,632 KiB free (80.86 GiB),
  no live heavy tickets. Tree: `/tmp/ov-1333-warm`. Only the completed cold
  trial's generated target/build directories were released to restore headroom;
  no unrelated files or compiler cache entries were removed.

The foreground measurement harness checked `df` for the target and cache
volume and the actual machine queue immediately before each attempt, acquired
`heavy_lock.machine_lock`, rechecked the 70 GiB requirement, and passed
`HEAVY_LOCK_HELD=1` to the nested landing wrapper. This retained heavy exclusion
through each entire gate. It would wait 180 s and recheck if either requirement
failed; neither admission needed to wait. Disk falling below 70 GiB during a
run did not admit another run. No tool was installed for these measurements.

Both commands, from their respective fresh trees, were:

```sh
FARCOOLER_GATE_SCCACHE=1 bash scripts/landing-gate \
  --base aa1be1f49fccce49cc5db03341807e32b9311504 \
  --full-rust --workers 4 --log-dir /tmp/ov-1333-round1-<cold-or-warm>-logs
```

The harness removed inherited engine `FARCOOLER_*` bindings for the standalone
trial and supplied the explicit base. PATH began with `/tmp/ov-1333-bin`, whose
`python3` points to the preinstalled native Python. Warm additionally used a
symlink there to the preinstalled native `/opt/homebrew/bin/openssl`.

Durable evidence is under [ov-1333-evidence](ov-1333-evidence): per-run admission,
revision/start receipt, exit code, full Rust timing records, selection, raw
sccache statistics, disk use and a summary of every family step's result and
duration. Original verbose logs remain in the two `/tmp/...-logs` directories.
The Rust span is last non-skipped end minus first non-skipped start in
`rust-timings.jsonl`, including gaps. Parallel-family wall spans were
1394.75 s cold / 1348.15 s warm; these are not summed job durations.

## Coverage and limits

Both selections contain the same 159 test binaries, zero skipped binaries, and
`--full-rust` as the reason. Both archives contain 162 binaries including three
runtime binaries. The main nextest inventories are identical: 7417 unique
binary/test names, sorted and newline-terminated, SHA-256
`cf74972801b730e8dcae7e76e53e37dceca9647a69d85a3e390eeedb3142e71c`.
Both main runs skipped the same reported count of 38 tests under gate policy.
The main test pools took 541.549 s cold / 543.949 s warm.

Cold ran all 7417 main tests: 7416 passed and
`farcooler-fence::inventory every_key_type_is_fingerprinted_as_ssh_keygen_does`
failed, including its normal isolated retry. The failure was spawning `openssl`
at `crates/fence/tests/inventory.rs:60`: PATH selected the host's x86-only
`/usr/local/bin/openssl` (OS error 86, Bad CPU type). Running the existing
compiled test binary with the preinstalled ARM OpenSSL passed (0.22 s), without
a build or full gate. Warm then passed all 7417 main tests and workspace
doctests (4.87 s). All other cold gate steps passed; fail-fast prevented cold
doctests. Do not describe cold as a green gate or treat its span as a complete
successful gate. The warm/cold difference of 46.59 s is therefore not a clean
whole-gate speedup comparison.

The same source, workers and full selection make this useful bounded evidence,
but two samples, differing test outcomes and a host PATH correction cannot
isolate sccache from load and other shared tool caches. No switch-off control
was run because that would exceed the two-run budget. Increased cache hits
alone do not establish the requested wall-time benefit. **Recommend leaving
the switch off.**

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
cache-session lock serializes access. A stale Unix socket is removed only after a connection probe refuses under the
exclusive session lock; live servers and unexpected files are preserved. The
server socket is cleaned after the child exits. The server runs without daemonization as
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

## Fix-round validation

- Fixed repeat-session startup: the previous smoke left a non-listening Unix
  socket, which the old startup rejected. Connection refusal reproduced the
  cause. A regression test covers live, stale, missing and non-socket paths.
  Two consecutive real server smoke sessions passed before either measured run.
- Gate self-test passed inside both measured gates, including policy fixtures.
- Landing gate tests: 19 passed, including all-step off/on environment comparison
  and socket lifecycle coverage; repeated standalone after measurement.
- Policy suite: 24 passed standalone after measurement.
- All 37 script lint invocations passed again after measurement, including
  self-tests/scans, protocol channels and baseline, actionlint, leak lint and
  quarantine lint. The receipts are [checks.json](ov-1333-evidence/checks.json).
- Warm full landing gate: PASS. Cold failure is disclosed above. No third full
  run, no `cargo fmt`, and no Cargo configuration edits.

The code fix was committed before both gates; final changes only record these
measurements. Standalone check output: `/tmp/ov-1333-round1-checks.log`.
