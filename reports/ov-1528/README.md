# ov-1528 Empty scratch folders are swept

The runner now sweeps empty scratch folders at start and hourly. One sweep with
this code took this machine from **7,397** scratch folders to **1,782**.
`dependency_seed` is still off. The lines that turn it on are
[below](#turning-dependency_seed-on) for the owner to publish.

## The rule

Two rules delete scratch, and neither deletes anything else under `/private/tmp`.

- **Recorded folders** follow their cleanup record, as ov-1516 left it
  (`owned_directories.rs`). That includes an orphan with output in it, a day
  after it was recorded. It already ran at runner start and hourly.
- **The new sweep** (`workflow_runtime/scratch_sweep.rs`) removes a folder only
  when all of these hold:
  - its name is `farcooler-scratch-wf1-` and exactly 64 lowercase hex digits;
  - it's a real folder, not a link, and belongs to the runner's user;
  - it's empty, and has not changed for a day;
  - no pending cleanup record names it, on any board in this runner's database;
  - `lsof` shows no process using it as a working directory or holding it open.

Removal is `rmdir`, so a folder that gains a file after the check stays. The
open-file check passes up to 500 folder names to one `lsof` call. If `lsof` exits
with an unexpected status, writes to stderr, names a path it wasn't asked about,
or reports success without naming a folder, the whole batch stays.

It runs in the cleanup round on the same hourly timer as process-log pruning,
whose first turn is runner start. Only the service runner sweeps.

## Counts on this machine

One sweep on the evening of Nov 28, in `/private/tmp`. It ran the new code with the
real `lsof`, through the ignored test `one_sweep_of_a_named_scratch_base`. Its
keep list was the 233 pending recorded paths read, read-only, from the canary
database; 5 of them are scratch folders. The stable and local databases have no
cleanup table. The installed runner doesn't have this code yet.

| `farcooler-scratch-*` folders | Before | After |
| --- | ---: | ---: |
| Empty, a day old or more | 5,615 | 0 |
| Empty, under a day old | 1,777 | 1,777 |
| With files in them | 5 | 5 |
| **Total** | **7,397** | **1,782** |

It removed 5,615 folders in 3.2 s. All 5 recorded scratch folders were still there.

## Something still makes empty folders

Empty folders were still appearing after ov-1516 landed. Its fix covers the
daemon's unit tests. The integration tests in `crates/daemon/tests/` link the
daemon without `cfg(test)`, so their scratch still goes to the real `/tmp`. I
counted `/private/tmp` around each file: `workflow_service_over_socket` added 22
folders and `engine_flow_per_skill` added 1. `workflow_draft_approval` added
none. The hourly counts here often came in multiples of about 23. I didn't trace
the other folders. I left the tests alone, which is outside this ticket; it's
worth a follow-up. My own run left its 23 behind. The sweep takes them after a day.

## Limits

- The pending-record check reads only this runner's database. A second runner on
  the machine (another channel, or a scratch daemon) could have a scratch folder
  that is still recorded, empty, unchanged for a day and not held open. This
  sweep would remove it. No output can be lost, since the folder is empty. A
  build that then made the folder again would leave it for a person to delete,
  because its record no longer matches.
- For the same reason, an integration test or scratch daemon that runs the
  service runner sweeps the real `/tmp`, under the same conditions.

## Turning `dependency_seed` on

In the workspace's adapter, add this key inside `adapter.config` (the `coding/v1`
config, beside `harness`, `checks` and `ci`), then publish:

```json
"dependency_seed": {
  "argv": ["/usr/bin/python3", "scripts/dependency-seed.py"],
  "environment": {"HOME": "/Users/e-liang", "PATH": "/Users/e-liang/.cargo/bin:/usr/bin:/bin"},
  "timeout_ms": 5400000
}
```

Add a comma to match where you put it. `workflow bundle <workspace>` exports the
published JSON to edit, and `workflow-draft --file` takes it back. The runner
must include ov-1516, or it refuses the unknown key.

I checked these lines against `CodingConfig` and its command validation in the
source: an absolute, executable `argv[0]`, and a timeout under the 3-hour limit.
`scripts/dependency-seed.py` is on this branch's base. I couldn't check them
against the published config: `workflow bundle` answered this step with
"scope denied: known caller required". The first seed is made by the next
landing, or the next build if that comes first. It waits until 91 GB is free
(ov-1516).

## Verification

Tests are in `workflow_runtime/scratch_sweep/tests.rs`. With a stub sweep, the
four that expect a removal failed. I then broke the finished code eight ways, and
each break failed a test: no record exclusion, holders ignored, a failed check
read as free, no age rule, a loose name rule, success with no names accepted, no
runner call, and a sweep on every round.

- Open files: a real child process's working directory and a real open handle
  each keep their folder, and both go once released.
- `cargo test -p farcooler-daemon --lib -- workflow_runtime::`: 621 passed, 1
  failed, 13 ignored. The failure is
  `the_native_launch_refuses_the_quarantined_binary_behind_an_npm_codex_launcher`:
  this machine's `node` gives "Bad CPU type in executable", as ov-1516 reported.
- `cargo test -p farcooler-store`: 661 passed, 3 ignored.
- Integration tests `engine_flow_per_skill`, `workflow_draft_approval` and
  `workflow_service_over_socket`: 34 passed, 1 ignored.
- After the one clippy fix, in a test: `scratch_sweep`, `owned_directories`,
  `dependency_seed`, `builder_resources` and `idle_cleanup` passed (30, 2
  ignored), and `cargo clippy` on the daemon and store with `--tests` was clean.
- `doc-comment-check.py`, `daemon-layers.py`, `copy-lint.py` and
  `file-size-budget.py` passed.
