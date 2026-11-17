# ov-1324 selection report

Replay of every commit landed on Nov 17, 2026 (Asia/Singapore), through main's
`06fa623ab` (the branch base). Landing time is the Git committer timestamp;
individual commits within folded landing groups are listed separately. Each
input is `git diff --no-renames --name-only <commit>^ <commit>`: no stale runner
base or cumulative diff. Before uses `jobs_for` from `06fa623ab`; after uses the
ov-1324 planner, with the same current CI family rules and command plans on both
sides. This compares selection only, not executed gates or Rust package counts.
No builds or full gate were run for this replay.

**all 11** = wire, doc-comments, relay, site, tunnel, rust, swift-tests, swift, ios, android, web. These are the gate's job names;
Swift jobs and iOS share the Apple scheduling family. `wire` includes script
self-tests/lints; workflow changes also add actionlint and the iOS shard check.

| Commit | Landed (+08:00) | Subject | Before | After |
| --- | --- | --- | --- | --- |
| `33aeefffd` | 02:27:21 | ov-1304: every xcode-27 job selects Xcode 27 explicitly; the Mac partitioner names DEVELOPER_DIR when xcrun finds no platform | **all 11** | **all 11** |
| `70a38a27a` | 02:27:21 | ov-1304: temporary CI trigger for this branch (to be removed) | **all 11** | **all 11** |
| `57a90cb91` | 02:27:21 | ov-1304: temporary runner diagnostics (to be removed) | **all 11** | **all 11** |
| `adadea6ae` | 02:27:21 | ov-1304: take only an Xcode whose own xcrun finds the platform, runner-declared path first; drop diagnostics | **all 11** | **all 11** |
| `95d8e29c1` | 02:27:21 | ov-1304: temporary trigger touch, run 2 of 3 | **all 11** | **all 11** |
| `281207a92` | 02:27:21 | ov-1304: remove the temporary CI trigger; the report | **all 11** | **all 11** |
| `b1f87744a` | 03:21:05 | ov-1301: an Uncertain effect recovery can't settle is looked at once, then backed off | **all 11** | **all 11** |
| `d64e6d07b` | 03:21:09 | WIP ov-1301: native turn and resolve changes | **all 11** | **all 11** |
| `e43a25ce4` | 03:21:10 | ov-1301: a busy runner never ends a build silently, and an Uncertain build can be settled | **all 11** | **all 11** |
| `d2906bed3` | 03:21:10 | ov-1301: the backoff test names what it uses, for the layers check | **all 11** | **all 11** |
| `a0c7a0298` | 03:21:10 | ov-1301: a resolve wakes the backed-off recovery, and the steering test says why it failed | **all 11** | **all 11** |
| `b4cf8a0c0` | 03:21:10 | ov-1301: a builder a landing gate holds stopped keeps its time, and running out of it says so | **all 11** | **all 11** |
| `da29626b1` | 05:13:20 | ov-1310: retry macOS runner acquisition failures | **all 11** | **all 11** |
| `6e1ea85f0` | 05:13:21 | ov-1310: retry all cancelled Mac shards once | **all 11** | wire, doc-comments |
| `eefe9eb5b` | 05:13:22 | ov-1310: track automatic retries across CI attempts | **all 11** | wire, doc-comments |
| `91c9c5bb5` | 05:13:22 | ov-1311: record runner retry carry verification | wire, doc-comments | wire, doc-comments |
| `6bdf5161c` | 07:43:08 | ov-1305: gate folded candidates against their recorded fold base | **all 11** | **all 11** |
| `8112dfb33` | 07:43:08 | ov-1305: document stale base receipts and replay ten landed plans | wire, doc-comments | wire, doc-comments |
| `04f7ef6f7` | 07:43:08 | ov-1305: assert light-job lock behavior without a timing threshold | **all 11** | **all 11** |
| `19188513b` | 07:43:08 | ov-1305: include landed candidates without retained push facts | wire, doc-comments | wire, doc-comments |
| `ea58046de` | 07:43:08 | ov-1305: record passing validation and isolate evidence fixture context | **all 11** | **all 11** |
| `b156baa55` | 07:43:08 | ov-1307: record gate verification | wire, doc-comments | wire, doc-comments |
| `2c88c0dd4` | 07:43:08 | ov-1307: record full Rust suite result | wire, doc-comments | wire, doc-comments |
| `432913ced` | 07:43:08 | ov-1307: capture native test diagnostics and verification | **all 11** | **all 11** |
| `695b3ff7c` | 07:43:08 | ov-1307: record review run flaky result | wire, doc-comments | wire, doc-comments |
| `da398eaaf` | 07:43:08 | ov-1313: record combined branch verification | wire, doc-comments | wire, doc-comments |
| `9a5c7e9b2` | 08:47:59 | ov-1308: bound Codex launches and retain window refusal reasons | **all 11** | **all 11** |
| `6858b8caa` | 08:47:59 | ov-1308: exercise the generated Codex launch through tmux | **all 11** | **all 11** |
| `18bb8e01b` | 08:47:59 | ov-1308: verify generated shell and pinned hooks in startup test | **all 11** | **all 11** |
| `ac2bfd884` | 08:47:59 | ov-1308: record full daemon and gate verification | wire, doc-comments | wire, doc-comments |
| `9460c3e89` | 08:47:59 | ov-1308: isolate Swift scratch daemon shells | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `762f65b17` | 08:47:59 | ov-1308: record repaired Swift gate validation | wire, doc-comments | wire, doc-comments |
| `3033e382e` | 08:47:59 | ov-1312: wait for resize restoration before the next test step | wire, doc-comments, site, tunnel, swift-tests, swift, ios | wire, doc-comments, site, tunnel, swift-tests, swift, ios |
| `63374831f` | 08:47:59 | ov-1312: record CI evidence and resize test verification | wire, doc-comments | wire, doc-comments |
| `5f328d4fa` | 12:42:34 | ov-1318: reproduce stale landing base selection | **all 11** | **all 11** |
| `54f2d170f` | 12:42:34 | ov-1318: recover landing base from exact durable fold record | **all 11** | **all 11** |
| `869f56103` | 13:44:38 | ov-1317: check Canary wire history in CI and the landing gate | **all 11** | **all 11** |
| `ef9ba0fdf` | 13:44:40 | ov-1317: distinguish pagination from board pages and record passing tests | **all 11** | **all 11** |
| `86a87f170` | 13:44:40 | ov-1317: preserve the recovery anchor through cherry-pick landing | **all 11** | **all 11** |
| `ff2fb0c55` | 13:44:40 | ov-1317: record passing fix-round validation | wire, doc-comments | wire, doc-comments |
| `068444023` | 13:44:40 | ov-1317: keep next_after and recover the unshipped wire baseline | **all 11** | **all 11** |
| `b9ac58fca` | 13:44:40 | ov-1317: record passing layer lints and TaskList tests | wire, doc-comments | wire, doc-comments |
| `cda08e591` | 13:44:40 | ov-1317: restore the original TaskList cursor name and recovery snapshot | **all 11** | **all 11** |
| `0d8d720fa` | 13:44:40 | ov-1317: distinguish task pagination in page lint and record passing checks | **all 11** | **all 11** |
| `6f5e85d1f` | 13:44:40 | ov-1315: wait for uncertain CI recovery effect | **all 11** | **all 11** |
| `1e39652af` | 13:44:40 | ov-1315: record daemon integration verification | wire, doc-comments | wire, doc-comments |
| `83c9d8e59` | 13:44:40 | ov-1315: record complete daemon verification | wire, doc-comments | wire, doc-comments |
| `06fa623ab` | 15:57:45 | ov-1320: retry failed main CI jobs once | **all 11** | wire, doc-comments |

48 landed commits replayed. 3 change selection.

The reviewed retry files select `wire, doc-comments` individually and together.
`ci-runner-retry-test.py` imports the helper and reads/executes the workflow's
commands; CI's wire job runs that test. The retry workflow operates only the
GitHub API. No gate platform build consumes these inputs. The map includes only
these reviewed paths; absence from command text never establishes safety.
A newly named direct gate user is added to the selection. Unknown scripts,
unknown workflows, and changes to gate selection itself retain full fallback.
`apps/macos/test.sh` retains the Apple jobs, while `scripts/lane-gate.py` and
`scripts/macos-rust-tests.py` retain full coverage.

## Validation

The retry regression failed before the policy change: all three individual
paths and their combined diff selected all 11 jobs (four failed assertions).
The failing regression is committed as `46d0e32`.

| Check | Result |
| --- | --- |
| `python3 scripts/lane-gate.py --self-test` | PASS, including policy, wrapper, benchmark, process and flaky-runner fixtures |
| `python3 scripts/landing-policy-test.py` | PASS, 21 tests |
| Every `scripts/*-lint.py --self-test` and each lint's default scan | PASS, 15 self-tests + 15 scans |
| Proto lint: preview, stable, canary `--since-baseline` | PASS |
| `bash scripts/actionlint.sh --self-test` and default lint | PASS |
| `python3 scripts/test-leak-check.py --lint` | PASS |
| `python3 scripts/ios-ui-shards.py --check` | PASS |
| `python3 scripts/ci-runner-retry-test.py` | PASS, 6 tests |
| `python3 scripts/gate-evidence-test.py` | PASS, 18 tests |
| `python3 scripts/gate-evidence-mutation-test.py` | PASS, all four mutations detected |
| `python3 scripts/engine-gate-test.py` | PASS, 9 tests |
| `python3 scripts/file-size-budget.py` | PASS |
| `git diff --check` | PASS |

Tests used Python 3.14 with a temporary `python3` PATH entry (the host's
`/usr/local/bin/python3` is an incompatible binary). Fixture processes did not
inherit `FARCOOLER_*` from the real engine; Cargo builds and nextest threads
were capped at four. Gate self-test ran its synthetic repository as designed.
No full gate, cargo fmt, background shell jobs, or platform builds were run.
The evidence fixture initially lacked the newly required policy module; its
copy list now includes that module, and the canonical evidence planner binds
its bytes and includes the same workflow validation steps as the gate.
