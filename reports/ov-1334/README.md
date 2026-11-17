**Recommendation: keep the current conservative selector and decline widening its full-suite fallbacks.** Against a full main run, the duration-weighted model estimates **about 0.50 minutes saved for a typical daemon-only change**, **0 minutes for protocol**, and **0.045 minutes for core**. These are planning estimates, not measured wall-time savings: the retained paired daemon run showed no saving. Selection already offers these modeled savings when eligible; accepting this recommendation changes no policy and saves zero additional minutes. The evidence does not support a multi-minute reduction of the 25-minute gate.

# ov-1334 — changed-crate selection cost

## Snapshot and current rules

Read-only analysis at `65b87d65b` (2026-11-18, +08:00). Main history means `origin/main` at that SHA; local `main` is stale at `3f78754f9` (October 1). No fetch, gate, Cargo invocation, build, or test was run. All source citations below refer to this snapshot.

Fix-round audit: the task's complete text and JSON history contain status transitions but no reviewer finding or comment. This revision independently checks the earlier report's numbers, adds omitted gate catches and executable reproduction instructions, and corrects date and uncertainty wording.

- Enabled by default in both plain lane and landing gates, when Rust is reached; `--full-rust` overrides it (`scripts/lane-gate.py:471–477,977–988`). Older opt-in/shadow descriptions in `reports/ov-1268/README.md` are superseded by ov-1278.
- Base must be a verified successful main-push CI ancestor, from `CANDIDATES` or GitHub's last 100 successful runs. Selection unions changes since that green base and the requested merge base, including staged/dirty and untracked inputs. No green ancestor, unavailable GitHub/network, invalid metadata or other caught selection error means full (`scripts/landing_policy.py:360–399`). A clean single-commit replay is therefore a lower coverage bound, not a reconstruction of an actual multi-ticket gate.
- Graph unions normal/dev/build/optional/all-target workspace edges, plus explicit CLI → daemon runtime consumption; closes in the reverse direction (`scripts/rust-select.py:30–53,162–169`). Dev dependencies and the runtime edge make daemon changes reach CLI, client and JNI as well.
- Narrowable inputs: added/modified `.rs` under a workspace crate's `src/`, `tests/`, or `examples/`, except protocol and custom build targets. Literal `include!`, `include_str!`, `include_bytes!`, and `#[path]` consumers also seed the graph (`scripts/rust-select.py:96–103,131–156`).
- Full: non-doc deletion/rename (diff uses `--no-renames`), manifests/lockfiles, build scripts, protocol/generated wire inputs, fixtures, config and unknown paths. Any full reason wins; empty change range is full. Docs (`reports/`, `docs/`, any `.md`) select nothing unless literally included by Rust; docs are processed before the deletion fallback (`scripts/rust-select.py:91–94,118–161`). Arbitrary non-Rust app/script files are **not** generally ignored by this selector.
- Modified `ci.yml` is exempt only if all changed jobs are in the explicit Swift/iOS/Android/relay/site/web allowlist. Rust/shared/unknown jobs, workflow header edits, or missing old/new workflow text mean full (`scripts/rust-select.py:56–88,123–130`). Outer family selection is separate; reviewed script consumers in `landing_policy.py:19–27,248–260` do not make an unknown path safe for the Rust selector.
- Only `rust` / `Full Rust workspace (local)` is filtered, by exact package expression; empty package selection skips that step. An unexpected command shape raises an error rather than narrowing. Every gate records selected/skipped binaries (`scripts/landing_policy.py:402–423,449–476`; `scripts/lane-gate.py:480–485`).

## Timing model and inventory

Timing source T: `/tmp/lane-gate-logs-j3pvafx3/rust-7988707.log`, November 18, nextest run `9c4fead0-ec78-4716-b28f-bf910a4a85dc`. First run: **7,417 passed, 38 skipped, 159 binaries, 533.608 s**. SHA-256: `981dff427c13053e51bca43ab6e807ad32da49b1f7a42be25d7b7b6516390fae`. Its sibling `rust-selection.json` lists the 159 binary IDs. The identical binary list is retained in `reports/ov-1333/ov-1333-evidence/warm/rust-selection.json`.

For each binary b, sum the elapsed seconds of its first-run PASS records to obtain S(b); these are test process durations, **not serial binary wall times**. Sum is 3,133.370 s. For selected packages P: binaries = count of listed IDs owned by P; tests = count of PASS records owned by P; **estimated main-run seconds = 533.608 × Σ S(selected binaries) / 3133.370**. Rounded to 0.1 s. Reproduce by parsing `PASS [ <seconds>s] (<ordinal>/7417) <binary> <test>` in T and grouping binary IDs before `::`. No proportional test-count extrapolation is used.

This is a fixed-throughput planning proxy, not measured selected-run wall time. Nextest overlaps binaries and has serial/exclusive groups (`.config/nextest.toml:18–136`); duration, load, scheduling and retries can change after filtering. It excludes extraction, wrappers, builds, feature tests, lints and doctests. Do not add skipped binaries' elapsed durations as wall-time savings. Actual paired daemon/CLI evidence saved nothing: 11m26s selected vs 11m09s full under load (`reports/ov-1268/README.md:18–27`).

Sensitivity check using the same formula on the first-run PASS/FAIL records of the three logs named in the ov-1329 failure row below: full wall times 536.442 / 552.586 / 547.939 s; daemon modeled savings 31.501 / 30.194 / 30.202 s, core 2.817 / 2.827 / 2.842 s. Together with T this gives daemon 29.7–31.5 s and core 2.7–2.9 s. This checks sensitivity to the source log, not model validity or a confidence interval; none is a measured selected run.

Counts include six binary IDs with no executed tests (JNI, core/live_agents, and four daemon tunnel targets); 38 skipped tests are excluded from 7,417. Inventory below permits auditing package sums; S is summed test seconds from T.

| Package suffix (`farcooler-`) | Binaries | Executed tests | S (s) |
|---|---:|---:|---:|
| agent-core | 1 | 8 | 0.069 |
| agent-hooks | 1 | 38 | 0.512 |
| ceremony | 1 | 104 | 0.986 |
| cli | 14 | 705 | 30.208 |
| client | 13 | 454 | 35.447 |
| core | 2 | 793 | 12.906 |
| daemon | 79 | 3875 | 2893.353 |
| fence | 3 | 77 | 1.462 |
| ffi-guard | 2 | 5 | 0.995 |
| jni | 1 | 0 | 0.000 |
| protocol | 1 | 77 | 0.938 |
| review | 1 | 44 | 0.431 |
| store | 1 | 667 | 103.468 |
| tailcat | 1 | 10 | 0.097 |
| tmux | 2 | 104 | 38.606 |
| transport | 6 | 54 | 3.365 |
| vt | 3 | 139 | 1.146 |
| workflow | 27 | 263 | 9.381 |

The daemon closure is CLI/client/daemon/JNI: **107 binaries, 5,034 tests, 503.9 s**, retaining 94.4% of timed work while selecting 67.9% of tests. Core adds core/store/tmux/transport: **118 / 6,652 / 530.9 s**. Protocol's raw graph closure is **124 / 6,920 / 531.5 s**, but protocol always forces **159 / 7,417 / 533.6 s**. Workflow's closure is **151 / 7,183 / 533.1 s**. Current manifests (all dependency tables, including target tables, with inherited paths resolved through root `Cargo.toml`) supply the graph; the selector's explicit CLI edge is included. No historical test counts are substituted for today's inventory.

## Every Rust-touching commit since November 15

Window: `git log 65b87d65b --since='2026-11-15T00:00:00+08:00'`, using **committer** dates (landed/cherry-picked commits often have older author dates). Dates below are normalized to Singapore time, including `f9afc6472` (Nov 14 21:31:54 UTC = Nov 15 05:31:54 SGT). 215 distinct commits: 207 touch `crates/` or root Cargo inputs; eight additional commits touch only shared Rust schema/test inputs. Scope includes `crates/**` (also non-workspace web), `proto/**`, `.cargo/**`, any `.rs`, root Cargo/toolchain files and `.config/{nextest.toml,quarantine.txt,flaky-log.tsv}`; script/workflow-only changes are not Rust source landings.

All 215 are enumerated below, grouped only when date, changed crate set and results match (up to five hashes per row). Crate names are package suffixes; JNI lives in `crates/android`. `shared` means no workspace crate changed. Raw = graph closure of changed workspace crates **alone**; shared schema/config inputs and non-workspace web have no raw graph estimate (—). Policy = current selector replay of **every changed path** from that commit's parent, including non-Rust paths. `F` is 159 binaries / 7,417 tests / 533.6 s; `=` means the raw result. Protocol, shared or mixed-path fallbacks explain why Policy can be F despite a small raw closure. No green-base/network fallback is injected into these optimistic clean-parent replays.

| Date (Nov) | Commit(s) | Crates changed | Raw binaries / tests / estimated s | Policy |
|---|---|---|---:|---|
| 17 | `6f5e85d1f`, `18bb8e01b`, `6858b8caa`, `432913ced`, `b4cf8a0c0` | daemon | 107 / 5034 / 503.9 | = |
| 17 | `a0c7a0298`, `d2906bed3`, `d64e6d07b`, `b1f87744a` | daemon | 107 / 5034 / 503.9 | = |
| 17 | `cda08e591` | cli, client, daemon, protocol | 124 / 6920 / 531.5 | F |
| 17 | `068444023`, `ef9ba0fdf` | protocol | 124 / 6920 / 531.5 | F |
| 17 | `86a87f170`, `869f56103` | shared | — | F |
| 17 | `9a5c7e9b2` | daemon, tmux | 109 / 5138 / 510.5 | = |
| 17 | `6bdf5161c` | daemon | 107 / 5034 / 503.9 | F |
| 17 | `e43a25ce4` | daemon, workflow | 151 / 7183 / 533.1 | = |
| 16 | `0009b4798`, `60ddb56c7` | cli, client, daemon, protocol | 124 / 6920 / 531.5 | F |
| 16 | `5eb552699` | cli, daemon, protocol, store | 124 / 6920 / 531.5 | F |
| 16 | `77a7feb1a` | cli, daemon | 107 / 5034 / 503.9 | F |
| 16 | `228f0b516` | client, daemon, workflow | 151 / 7183 / 533.1 | F |
| 16 | `70270f3b3`, `ee6e3e01e` | workflow | 151 / 7183 / 533.1 | F |
| 16 | `d014f61ee` | daemon | 107 / 5034 / 503.9 | F |
| 16 | `fc57a8825`, `3cdb5b23e`, `24e99d771`, `891c980a6`, `d84272d8f` | daemon | 107 / 5034 / 503.9 | = |
| 16 | `a2d96bc54`, `31646caf5`, `8953bb06c`, `bb3066503`, `041878c4c` | daemon | 107 / 5034 / 503.9 | = |
| 16 | `33acdb088`, `3ca677227`, `69021450e`, `6b7c6fe61`, `bc839729f` | daemon | 107 / 5034 / 503.9 | = |
| 16 | `e7ee62b0a`, `763d05499`, `3ad5e4efa`, `e2bf8c94f`, `a9db3c1aa` | daemon | 107 / 5034 / 503.9 | = |
| 16 | `10c457160` | daemon | 107 / 5034 / 503.9 | = |
| 16 | `d262a7345`, `712fa86ee` | shared | — | F |
| 16 | `2416236d7`, `309102b9a`, `d5c8c6bea` | daemon, store | 108 / 5701 / 521.5 | = |
| 16 | `1d0bc8a83` | store | 108 / 5701 / 521.5 | F |
| 16 | `bc3d41803`, `155e1bd88` | client | 107 / 5034 / 503.9 | F |
| 16 | `858ad16ee` | client, daemon | 107 / 5034 / 503.9 | F |
| 16 | `c65f00916`, `42522c5ee`, `7ef431e6b`, `242b471e7`, `c9e0dd15c` | workflow | 151 / 7183 / 533.1 | = |
| 16 | `a2a2e0b49`, `1520b5cd1`, `0b8b5bdf4`, `e87889e99`, `2c37f5e0c` | workflow | 151 / 7183 / 533.1 | = |
| 16 | `07c814db6`, `63096ce22`, `5d855b688`, `4e38a5b90` | workflow | 151 / 7183 / 533.1 | = |
| 16 | `7b49c542c` | client, daemon, store | 108 / 5701 / 521.5 | F |
| 16 | `d52a887d6` | cli, daemon | 107 / 5034 / 503.9 | = |
| 16 | `e9dce63e0` | cli | 107 / 5034 / 503.9 | = |
| 16 | `a04476d5b` | cli, client, core | 118 / 6652 / 530.9 | F |
| 16 | `457172123` | daemon, workflow | 151 / 7183 / 533.1 | F |
| 15 | `8b4ed22bd` | cli, daemon, protocol, store | 124 / 6920 / 531.5 | F |
| 15 | `d612b7e42`, `8120e56cf`, `6ada144fc` | cli, daemon | 107 / 5034 / 503.9 | = |
| 15 | `e89ba5b88`, `46ac89254`, `9e4b72a0a`, `388a42104`, `355ee5e22` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `07080462f`, `466cabc4c`, `ee9d7fff4`, `b8e408444`, `0cb96ab46` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `092ad24ca`, `fdf45461d`, `673b23531`, `e7db93961`, `e4bba0659` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `04dbcaa25`, `05f0f50c8`, `9c1c9a92d`, `e9e3ded73`, `0dfdac9a1` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `a88544abb`, `256f1e834`, `ea5805fa1`, `39ba65c8f`, `d4fc81ca3` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `8eb5dff03`, `d4e8f39a1`, `16a92fd10`, `dca40e307`, `2362368d9` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `3024b9303`, `58f4de357`, `d61ab5ee5`, `de81675d9`, `65a89a027` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `0effa8940`, `333604b14`, `9933012aa`, `1156583cd`, `aa143e179` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `4d9c6a1b9`, `d7c989a32`, `ae3c31a8c`, `426d27265`, `ee37df140` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `729dd2d58`, `1a5642112`, `c998c91b1`, `db13c429f`, `08386df36` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `a1b2ea88e`, `e90b3b150`, `60fdf27cc`, `e7b5531a3`, `5a2b78d98` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `84a452ca1`, `a12992add`, `4ccac812b`, `142ecff5e`, `4fa8a7614` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `1709de7c3`, `bd8d0409b`, `bc821aca1`, `486c32217`, `cfec71418` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `4cb08a7cb` | daemon | 107 / 5034 / 503.9 | = |
| 15 | `0fb1f289c`, `a7f4be432`, `e770e2109` | client | 107 / 5034 / 503.9 | = |
| 15 | `d4e952356`, `ab37e450f` | client | 107 / 5034 / 503.9 | F |
| 15 | `b75c39e13` | cli, client, daemon, store | 108 / 5701 / 521.5 | F |
| 15 | `b39374a24`, `47762159b`, `71e5fd150`, `b101a33ae` | daemon | 107 / 5034 / 503.9 | F |
| 15 | `f1c0e3d80`, `a89ab2d7a`, `b78add05d`, `b525af5e6`, `7294138b8` | daemon, store | 108 / 5701 / 521.5 | = |
| 15 | `46baa8af6`, `08e8b4395`, `cffbdea21`, `733d19efc` | daemon, store | 108 / 5701 / 521.5 | = |
| 15 | `67627cab8`, `0bd059ce6` | cli, client, daemon, protocol, store, workflow | 151 / 7183 / 533.1 | F |
| 15 | `0227720ec`, `e6f2ab279`, `623e004cd`, `4c1dea207`, `88a84980c` | workflow | 151 / 7183 / 533.1 | = |
| 15 | `94f1ab700`, `77fea85ef` | workflow | 151 / 7183 / 533.1 | = |
| 15 | `760524c27`, `5e060bf1a`, `9217b51eb`, `c7b0a788d`, `764aad4f0` | daemon, workflow | 151 / 7183 / 533.1 | = |
| 15 | `5a6404b88` | daemon, workflow | 151 / 7183 / 533.1 | = |
| 15 | `53451d7aa` | agent-hooks, cli, daemon | 108 / 5072 / 504.0 | = |
| 15 | `98875f082` | store | 108 / 5701 / 521.5 | = |
| 15 | `564869709` | daemon, protocol, store | 124 / 6920 / 531.5 | F |
| 15 | `80d0c3acf` | daemon, protocol | 124 / 6920 / 531.5 | F |
| 15 | `355bb94d1` | cli, client, daemon, protocol, store | 124 / 6920 / 531.5 | F |
| 15 | `06a1782c8`, `42ebd0536` | store, workflow | 151 / 7183 / 533.1 | = |
| 15 | `a5e7ebe82`, `2c33cb0f3` | cli, client, daemon, protocol | 124 / 6920 / 531.5 | F |
| 15 | `36dfedf3b`, `5253f9945`, `180462ad2`, `e99c6964a`, `c71dc243a` | daemon, store, workflow | 151 / 7183 / 533.1 | = |
| 15 | `94ed7b95a` | cli | 107 / 5034 / 503.9 | = |
| 15 | `64c4057e1` | cli, daemon, workflow | 151 / 7183 / 533.1 | = |
| 15 | `172e6ae17` | cli, core, daemon, workflow | 151 / 7183 / 533.1 | = |
| 15 | `65bfc72c8` | client, daemon, workflow | 151 / 7183 / 533.1 | F |
| 15 | `69f6dadbf` | cli, client | 107 / 5034 / 503.9 | F |
| 15 | `19de9fb3d` | client, daemon, store, workflow | 151 / 7183 / 533.1 | F |
| 15 | `1a3666b57` | cli, client, core, daemon, protocol, store, workflow | 151 / 7183 / 533.1 | F |
| 15 | `0b23ed68b`, `0d55f602b` | cli, daemon, store, workflow | 151 / 7183 / 533.1 | = |
| 15 | `adfde91c0` | cli, transport | 113 / 5088 / 504.5 | = |
| 15 | `f977e3fc7`, `e5265d99c`, `1fe14084d` | shared | — | F |
| 15 | `bd9826d7d` | cli, daemon, protocol | 124 / 6920 / 531.5 | F |
| 15 | `88b248a19`, `f4c18e9ef` | cli, daemon | 107 / 5034 / 503.9 | F |
| 15 | `f9afc6472` | shared | — | F |
| 15 | `d7e780074` | cli, client, daemon | 107 / 5034 / 503.9 | F |
| 15 | `79e384a37` | daemon, transport | 113 / 5088 / 504.5 | = |
| 15 | `0025bde68` | ceremony, client | 108 / 5138 / 504.1 | F |
| 15 | `8d7c4b4d1`, `9f4015f4e` | web (non-workspace) | — | F |
| 15 | `eec4d7757` | ceremony, client, daemon, jni, protocol | 124 / 6920 / 531.5 | F |

Audit each row with `git diff-tree --root --no-commit-id --name-status -r --no-renames <hash>`. Replay `select()` using those statuses and workflow text from `<hash>^` / `<hash>`, with the graph/inventory pinned above. For example `7b49c542c` is full because of `crates/client/src/testdata/workflow-labels.json`; `6bdf5161c` includes gate scripts; `ef9ba0fdf` touches protocol. Parent-only replays cannot establish the packages actually run in a landing: verified-green history, dirty inputs, mixed groups and flags can only broaden these results. Across the 215 replays, 54 are full and 161 partial; median estimated main time is 504.5 s, and mean modeled saving versus full is 15.9 s. These are commit-weighted, not landing-group-weighted statistics.

## Reproduce the commit table without Cargo

Run this read-only Python 3.11+ block from this report's branch (source is unchanged from `65b87d65b`). It rebuilds the all-table manifest graph, calls the real selector, verifies every table row and the log digest, and prints each commit's raw and policy counts/times plus full reasons. No metadata/build/test command runs; `sys.dont_write_bytecode` prevents import output. T and the retained binary inventory above are its only timing/count inputs.

```python
import sys
sys.dont_write_bytecode = True
import hashlib, importlib.util, json, re, subprocess, tomllib
from pathlib import Path
root = Path.cwd()
def git(*args):
    return subprocess.check_output(['git', *args], text=True)
workspace = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']
packages = []
for member in workspace['members']:
    path = root / member
    manifest = tomllib.loads((path / 'Cargo.toml').read_text())
    deps = []
    def walk(table):
        for key, value in table.items():
            if key in ('dependencies', 'dev-dependencies', 'build-dependencies'):
                for alias, dep in value.items():
                    if not isinstance(dep, dict): continue
                    if dep.get('workspace'): dep = workspace['dependencies'][alias]
                    if 'path' in dep: deps.append(dict(name=dep.get('package', alias), path=dep['path']))
            elif isinstance(value, dict): walk(value)
    walk(manifest)
    build = manifest['package'].get('build', 'build.rs')
    targets = [dict(kind=['custom-build'], src_path=str(path / build))] if build and (path / build).exists() else []
    name = manifest['package']['name']
    packages.append(dict(id=name, name=name, manifest_path=str(path / 'Cargo.toml'), dependencies=deps, targets=targets))
meta = dict(workspace_members=[p['id'] for p in packages], packages=packages)
spec = importlib.util.spec_from_file_location('selector', root / 'scripts/rust-select.py')
selector = importlib.util.module_from_spec(spec); spec.loader.exec_module(selector)
_, reverse = selector.graph(meta)
roots = {p["id"]: str(Path(p["manifest_path"]).parent.relative_to(root)) + "/" for p in packages}
consumers = selector.includes(root, roots)
selector.includes = lambda *args: consumers
bins = json.loads((root / 'reports/ov-1333/ov-1333-evidence/warm/rust-selection.json').read_text())['selected_binaries']
log = Path('/tmp/lane-gate-logs-j3pvafx3/rust-7988707.log').read_bytes()
assert hashlib.sha256(log).hexdigest() == '981dff427c13053e51bca43ab6e807ad32da49b1f7a42be25d7b7b6516390fae'
rows = re.findall(r'PASS \[\s*([\d.]+)s\] \(\s*\d+/7417\) (\S+) (.+)', log.decode())
total = sum(float(t) for t, b, n in rows)
assert len(bins) == 159 and len(rows) == 7417 and round(total, 3) == 3133.370
def stats(names):
    return (sum(b.split('::')[0] in names for b in bins), sum(b.split('::')[0] in names for t,b,n in rows), round(533.608 * sum(float(t) for t,b,n in rows if b.split('::')[0] in names) / total, 1))
report = (root / 'reports/ov-1334/README.md').read_text()
table = {h: line.split('|')[4:6] for line in report.splitlines() if re.match(r'\| \d+ \|', line) for h in re.findall(r'`([a-f0-9]{9})`', line)}
seen, full_count = set(), 0
for sha in git('log', '65b87d65b', '--since=2026-11-15T00:00:00+08:00', '--format=%H').splitlines():
    changes = [tuple(s.split('\t', 1)) for s in git('diff-tree', '--root', '--no-commit-id', '--name-status', '-r', '--no-renames', sha).splitlines()]
    paths = [p for status,p in changes]
    if not any(p.startswith(('crates/', 'proto/', '.cargo/')) or p.endswith('.rs') or p in ('Cargo.toml','Cargo.lock','rust-toolchain','rust-toolchain.toml','.config/nextest.toml','.config/quarantine.txt','.config/flaky-log.tsv') for p in paths): continue
    reached = {p['name'] for p in packages if any(f.startswith(str(Path(p['manifest_path']).parent.relative_to(root)) + '/') for f in paths)}
    while (broader := reached | set().union(*(reverse[p] for p in reached))) != reached: reached = broader
    selected, reasons, full = selector.select(root, meta, changes, lambda p: (git('show', sha+'^:'+p), git('show', sha+':'+p)))
    raw, policy = stats(reached), stats(selected)
    expected, flag = table[sha[:9]]
    assert expected.strip() == '—' or raw == tuple(float(v) for v in expected.split('/'))
    assert policy == ((159,7417,533.6) if flag.strip() == 'F' else raw)
    seen.add(sha[:9]); full_count += full
    print(sha[:9], raw, policy, '; '.join(reasons) if full else '')
assert seen == set(table) and len(seen) == 215 and full_count == 54
```

## Real failures and the coverage cost

The tracked `.config/flaky-log.tsv` is absent now. `git log -p origin/main -- .config/flaky-log.tsv` shows it was added by `712fa86ee` with **header only**, then deleted by `d262a7345`; there are no retained failure rows there. Records moved to gate log directories (`scripts/gate-flaky.py:12–29`). This absence is not a zero-miss record. The following are actual gate/CI catches found in reports dated to this window, supplemented by the reports' named raw logs. Builder-only failures, intentional mutation reds and older undated CI catches are not counted as landing catches.

| Catch and evidence | Would changed-crate selection still execute it? |
|---|---|
| Nov 15 main CI `43699070430`, head `99b2a020f`: daemon `builder_resources::tests::{adopted_preexisting_output_is_preserved_through_build_and_cleanup,low_disk_refusal_never_touches_adopted_output}`; cleanup deleted adopted output. `reports/ov-1263/README.md:3–17`. | **Yes**, daemon is reached from offending cleanup commits `5253f9945,36dfedf3b` (daemon/store/workflow). Still Linux-specific: executing the test on Mac originally passed accidentally because `/var` was noncanonical. |
| Same CI: daemon `watch::answer_wake::engine::tests::{engine_message_reaches_a_scratch_agent_and_acceptance_revision_notifies_it,engine_message_waits_behind_a_draft_and_does_not_answer_a_dialog}` never submitted messages on tmux 3.4. `reports/ov-1263/README.md:19–31`; full names at `crates/daemon/src/watch/answer_wake/engine.rs:403,494`. | **Yes**, daemon/CLI delivery changes (`092ad24ca,53451d7aa`) reach daemon. Mac selection cannot substitute for Linux/tmux-version coverage. |
| Parallel gate's daemon `git_sandbox::alone::a_planted_hook_is_refused_with_the_pins_off`, quarantined in `f977e3fc7`; `reports/ov-1249/README.md:1`. Additional quarantine evidence (original gate log unavailable): provision-race test `workflow_runtime::coding_tests::workflow_coding_two_tickets_start_with_different_models_and_each_build_launches_its_own`, quarantined by `71e5fd150`; `.config/quarantine.txt:17–18`. | **Yes for daemon-affecting work** (including git-sandbox/provision changes); quarantine inputs themselves force full. Retention does not mean strict first-failure rejection: quarantine retries remain in place. Original complete gate ranges are not retained in these reports. |
| ov-1268 full/selected comparison: client real-daemon, CLI hook and daemon script-hook failures under load; `reports/ov-1268/README.md:22–27` records four failures but not all exact names. | **Yes**, the named change `d612b7e42` reaches all three packages, and the report observes these failures in the selected run too. Cannot independently name/attribute all four without the original logs. |
| Nov 16 trial CI `43804704834`, Rust shard 1/4: daemon integration `workflow_service_over_socket::workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration`; `reports/ov-1294/README.md:54`, recovery investigation `reports/ov-1315/README.md`. | **Yes under current policy for the trial's workflow edits (full)**; yes for daemon source changes. A truly unrelated leaf closure would omit it. CI trial also hit iOS simulator boot failure: unaffected by Rust selection. |
| ov-1329 landing runs: daemon unit `workflow_runtime::native_control::tests::a_native_pause_freezes_processes_and_the_steps_deadline_then_resumes_the_same_pane`, and daemon integration `a_cutover_survives_every_crash::a_person_takes_a_real_runner_from_prepare_to_ordinary_service`. `reports/ov-1329/README.md:34–38`; `/tmp/lane-gate-logs-{mwr8k8gf,cqalrde0,abft9e9e}/rust-{99889518,92315277,33783785}.log` respectively. | **Yes**, all three recorded selections were full; both tests are also retained by daemon, core and protocol closures. One/two/two first-run failures passed isolated retries. This is a gate catch, not a final red gate. |
| ov-1333 cold full gate: fence integration `inventory::every_key_type_is_fingerprinted_as_ssh_keygen_does` failed twice because PATH chose Intel OpenSSL. `reports/ov-1333/ov-1333-report.md:78–88`. | **Yes for the actual cache-script change**, which forces full. **MISSED by a daemon-only or core-only closure** (fence is outside both). The raw protocol closure includes fence, and current protocol policy is full. This is a concrete lost environment-regression probe, not evidence of a daemon dependency bug. |
| Swift resize test: CI `43756701004` attempt 1 and Nov 16 `43962297903`, head `91c9c5bb5`, `TranscriptScrollerTests` reader/resize assertion. `reports/ov-1270/README.md:36`; `reports/ov-1312/README.md:3`, `ci-excerpt.log`. | **Unaffected**: Swift tests are outside the filtered Rust main run. The latter head changed only a report, illustrating environmental failures on unrelated changes. |
| ov-1302 landing gate's Swift shard: `NativeAgentRunnerTests` returned `not_running` instead of `not_an_agent`; `NativeOfferTests` reported `resource not found` from `terminal send`. `reports/ov-1302/README.md:45–52`; report landed Nov 16 with `c3d51caf2`. | **Unaffected**: both are Swift tests, outside the filtered nextest command. The focused retry passed all nine tests; this is a retained first-attempt gate catch. |
| Nov 15 train gate at `f47b682e1`: Go/Wasm `farcooler/tailcat` could not start because argv plus environment exceeded its limit. `reports/ov-1247/README.md:3–9`, `orchestrator-gate-full.out:216–221,404`. | **Unaffected**: Rust package filtering cannot prevent this web-job failure. The clean-environment web-only retry passed; the report does not retain an individual Go test name because execution failed before one ran. |
| CI `43751761408`: native bundle fixture `opt-level=3` assertion and lane-gate self-test rejected new release overrides (`reports/ov-1270/README.md:9–16`). ov-1184 gate: `ios-ui-shards.py --check` rejected Android shard matrix (`reports/ov-1242/README.md:54–63`). ov-1254 partial gate: `macos-rust-tests.py` lacked Python `tomllib` (`reports/ov-1254/README.md:48–53`). | **Unaffected**: bundle/wire/script checks are not package-filtered. No nextest binary is involved. |

Reports also retain catches without enough detail to classify a test: `reports/ov-1294/README.md:56` says CI `43812224858` failed Swift tests but gives no name (unaffected); `reports/ov-1291/README.md:12–27` records main CI red, a 40-minute gate timeout and a gate modifying a tracked file. The tracked-file problem is unaffected; **the unnamed CI failure and timeout are unclassifiable**, not proved retained or missed. Reports of native failures in standalone suites (ov-1299, ov-1307, ov-1315), intentional mutation reds (ov-1284/ov-1317), and older ov-1137/ov-1200 investigations do not establish additional since-Nov-15 landing catches. The compose-held CI flake in `reports/ov-1182/README.md:117–122` predates the window (`symbols-run2-jobs.json` dates run `43575477705` to Nov 14 UTC/SGT).

No attributable daemon/core/protocol **product regression** in this retained sample would be omitted by its offending change's current selection. That is bounded evidence, not proof of completeness: missing original green-base ranges and failure logs prevent an exhaustive miss rate. Unselected tests would lose opportunistic checks of host tools, unrelated crates and cross-crate runtime contracts absent from Cargo/includes/manual edges. The fence example proves that cost can be real even without a source dependency.

## Checks that remain full and owner decision

Selection changes only the local main nextest expression. Workspace runtime-bin and archive builds remain full (`.github/workflows/ci.yml:815–820`; `scripts/lane-gate.py:343–380`; `scripts/rust-archive.sh:25–26`); compilation savings are not credited. Default and feature clippy stay as configured (`ci.yml:652,692–704`); linked host/musl and daemon tailcat checks stay (`727–728,797`); helper and five daemon tailcat-helper nextest commands stay (`741–775`); workspace doctests stay (`843`). “Full” here means their entire configured scope, not every possible feature combination. Wire, leak/quarantine checks and all reached non-Rust families remain unchanged.

Main CI keeps full Linux archive coverage across four hash shards (`ci.yml:973`), plus configured macOS-specific Rust coverage (`1117`), features/lints/doctests and other jobs. Local package filtering does not alter CI commands (`landing_policy.py:449–476`). Main CI is later detection; it is not equivalent to catching a missed test before landing.

**Owner yes/no:** keep the existing conservative selection, retain full main CI and `--full-rust`, and reject a broad fallback relaxation for daemon/core/protocol work. Even dropping the protocol exception buys only ~2.1 weighted seconds; core buys ~2.7, daemon ~29.7. These estimates do not justify losing the fence/host probe or assuming unknown inputs are safe. A future proposal should name a specific leaf/input and its complete consumers; the present evidence does not support a multi-minute promise for typical Rust landings.

Validation for this ticket was document/data review only: inventory totals, unique commit coverage, graph arithmetic, source citations and report length. Per the explicit constraint, **no gates, builds or tests were run**. Only this report is committed.
