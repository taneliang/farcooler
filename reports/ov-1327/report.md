# ov-1327 — refuse a landing when disk is short

Landing admission requires **70 GB free**: the named `GATE_BUDGET` is 45 GiB,
plus a 25 GiB daemon reserve. Messages label GiB (2^30 bytes) as GB.
The budget uses ov-1322's initial sampled footprint of **38.335506 GiB**
(tree, scratch and logs), recorded in `0f39e9e38:reports/ov-1322/report.md`.
The 45 GiB allowance leaves 6.66 GiB above that observation. **The doctest tail
is unmeasured in this budget's source observation**; sampling also does not
bound transient allocations or shared caches. No full gate was run for ov-1327.
This is an admission budget, not a disk reservation.

The public wrapper checks before screen/load waits or the heavy queue. The
landing runner checks before toolchain work and rechecks after acquiring the
queue, before launching families. Admission reads capacity and deletes nothing.
Landing's old size-based target trim is removed; no stale output or log cleanup
is added. Cheap-only plans and dry runs do not require the heavy gate budget.

After successful extraction and metadata checks, the Rust wrapper unlinks only
`target/rust-tests.tar.zst` in its gate tree. The suite and isolated retries use
nextest metadata. Other archive names and symlink escapes are refused; extraction
failure keeps the archive. A pre-existing unrelated file beside it survives.

Validation uses Python 3.14, foreground fixtures and at most four build jobs.
The low-disk real-wrapper test injects 21 GB free, asserts free/needed GB, has a
60-second timeout and uses an audit hook that fails on any deletion. Boundary,
archive retention, unrelated-file preservation and path-guard tests also pass.
Sandbox restrictions on process inspection/local sockets required unrestricted
fixture reruns; inherited engine bindings were cleared for standalone self-tests.

Final checks: gate self-test **PASS** (including 21 landing-policy tests),
landing gate tests **13 PASS**, every scripts lint plus its self-test
**16 entry points / 32 commands PASS**, `bash -n scripts/landing-gate` **PASS**,
and `git diff --check` **PASS**. Raw validation logs are temporary and uncommitted.

## Landing-gate return repair

The real preflight on this host refused with **49.21 GB free; 70.00 GB needed**
before heavy work. That is the required short-disk behavior; the budget remains
unchanged and no full gate was run or stale output/log folders removed.

The deletion audit had masked Python's temporary-directory discovery by setting
`tempfile.tempdir` in the test. Without that bypass, the test reproduced an
`os.remove` before disk admission: `gettempdir()` creates and unlinks a probe
file. Planning now uses an inert path placeholder and allocates real scratch
only after successful admission, rebuilding the steps with that scratch path.
The unmodified temporary-directory discovery remains active in the regression
test, which failed before the fix and passes afterward (under one second).

Fresh validation: gate self-test **PASS** (including **21 policy tests**),
landing gate tests **13 PASS**, all scripts lints and their self-tests
**16 entry points / 32 commands PASS**, wrapper syntax and diff whitespace
checks **PASS**. Native Python 3.14 is first on PATH; this host's
`/usr/local/bin/python3` is incompatible with its CPU. Gate self-tests require
process inspection/local sockets outside the restricted sandbox.
