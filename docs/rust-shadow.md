# Rust changed-crate shadow (ov-1116)

`scripts/rust-shadow.py` observes the default-host Rust suite in both CI Rust
jobs. It cannot enable a skip. The existing ov-1021 job-family selection,
feature/target checks, full Mac suite, nightly coverage and doctests remain.

The baseline is the nearest earlier ancestor with a successful main CI run,
using the same green-run listing and ancestor rule as `ci-changes.py`. A PR's
base SHA and the previous commit are not proof that tests passed. Missing or
failed listings, dirty tracked inputs and metadata errors propose full coverage.

The graph comes from locked Cargo metadata with all features and no platform
filter. Both resolve edges and dependency declarations contribute normal, dev,
build, optional and foreign-target dependencies. Selection closes transitively
over reverse dependencies. The daemon is an explicit consumer of the CLI:
daemon tests spawn it despite there being no Cargo dependency edge. Mac real
CLI tests remain in the full Mac suite, outside this Rust-only proposal.

Only added/modified Rust source under a member's src, tests or examples can
narrow the proposal. Literal includes and `#[path]` references add their owning
consumers. Everything else is full: deleted/renamed files, fixtures, headers,
build scripts (including custom paths from metadata), manifests, lockfiles,
generated protocol, toolchain/config, scripts and unknown paths. This is an
intentionally conservative input model, not a claim to discover arbitrary
runtime file reads. Extending it requires a consumer test first.

CI uploads `rust-shadow-<os>` with:

- `plan.json`: green base/head, changes, selected packages and reasons.
- `full-inventory.json` and `selected-inventory.json`: nextest's complete JSON
  inventories, including ignored tests and filter matches.
- `measurement.json`: actual full/replay wall seconds, exits, sampled load and
  acceptance. Failed inventory collection is recorded and rejects the result.
- `summary.txt`, also published to the job summary.

The existing leak-checked full command always runs unchanged. Only after it
passes does a narrow selection replay the same command with a package filter.
Both runs wait for one- and five-minute load below 20. Load is sampled during
execution; any sample at or above 20 invalidates the savings figure. A failure
in either run rejects selection and fails the step. The test suite plants a CLI
contract failure that passes a CLI-only run and fails its omitted daemon and
Mac-style consumers; that result must reject selection.

The difference is **warm sequential replay wall time**, including the wrappers,
not summed per-test durations or an end-to-end CI speedup. Full fallback does
not replay or claim a saving. Feature/target variants and external suites have
no proposed omissions and no claimed savings. No skip should be enabled from
the ticket's earlier 688/6973 coverage model; review real CI artifacts for the
relevant host/change, consumer mutation evidence, and the remaining full gates
before a separate change authorizes any skip.

Run the planner's offline tests with:

```sh
python3 scripts/rust-shadow-test.py
```

For a local measurement, provide the same verified-success NDJSON listing CI
uses and run under the machine queue (never stop another task to get a slot):

```sh
CANDIDATES=/tmp/green.ndjson scripts/heavy -- python3 scripts/rust-shadow.py \
  --out /tmp/rust-shadow -- ./scripts/test-leak-check.py -- \
  ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --workspace --no-fail-fast
```
