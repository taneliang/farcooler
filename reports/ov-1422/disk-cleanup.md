# ov-1422 engine disk cleanup verification

On 22 November 2026, the native probe completed a build, review, and grouped landing on the macOS runner with Codex 0.169.0 in `workspace-write` mode. Codex used a disposable home with approvals disabled. The builder made a one-line change in a small Rust project, ran `cargo build --offline` with `CARGO_TARGET_DIR` under the engine-provided `FARCOOLER_SCRATCH`, and ran the compiled binary. The harness committed the change in the disposable repository because the sandbox protects linked worktree Git metadata. A second Codex agent reviewed the change, ran `cargo test --offline` in its own scratch directory, and recorded a passing verdict. The grouped landing built and tested the candidate in a new worktree and pushed commit `3e77b9152bc2f07d3c73e4af23e32259ad64a9c8` to the local bare repository. The test supplied a simulated CI observation for the local remote.

The engine records the scratch directories by path and filesystem identity and reclaims each after its owner settles. Codex receives no additional writable root for scratch. On macOS the engine uses the canonical `/private/tmp` spelling. Agent-invoked lane gates write logs under scratch, while the engine's landing gate supplies a separately recorded log directory with seven-day retention.

| Measurement | Bytes |
| --- | ---: |
| Volume space available before build | 93,926,993,920 |
| Volume space available after landing and cleanup | 95,503,253,504 |
| Change in volume space available | +1,576,259,584 |
| Builder scratch reclaimed | 1,084,261 |
| Reviewer scratch reclaimed | 1,652,555 |
| Total logical bytes reclaimed by the engine | 6,039,184 |

The probe sampled the scratch runtime's volume with `statvfs`. The volume samples include concurrent activity on the shared machine. The engine totals count the logical size of the files it removed. Both agent scratch directories were gone at the join wait.

## Regression coverage

The deterministic build and review fixture exercises a send-back and checks that the rebuilt candidate passes with a cold target. It verifies that waiting tickets release their target, that the landed group retains its record after its checkout disappears, and that `workflow get` reports bytes freed when the gate log expires. The startup sweep test holds an open reader on a settled group and verifies that cleanup waits for the reader to close. Historical log directories require an engine ownership record to qualify for automatic deletion.

Run the deterministic case with `cargo test -p farcooler-daemon --lib build_review_send_back_and_landing_release_owned_output`. For the native probe, build the runner and CLI with `cargo build -p farcooler-cli -p farcooler-daemon` and the daemon test binary with `cargo test -p farcooler-daemon --lib --no-run`. Set `OV1422_TEST_BINARY` to the reported executable, and `OV1422_CLI_BINARY` and `OV1422_DAEMON_BINARY` to the absolute paths of the built `farcooler` and `farcoolerd`. Stage them in a disposable directory so the probe resolves the verdict CLI and terminal output helper beside the test binary.

```sh
probe=$(mktemp -d /private/tmp/ov-1422-live.XXXXXX)
mkdir "$probe/deps"
cp -c "$OV1422_TEST_BINARY" "$probe/deps/daemon-tests"
ln -s "$OV1422_CLI_BINARY" "$probe/farcooler"
ln -s "$probe/farcooler" "$probe/deps/farcooler-local"
ln -s "$OV1422_DAEMON_BINARY" "$probe/deps/farcoolerd-local"
TMPDIR=/private/tmp FARCOOLER_TEST_SHELL=/bin/sh FARCOOLER_LIVE_AGENTS=1 \
OV1422_CODEX_AUTH=/path/to/scratch/auth.json \
OV1422_RUSTUP_HOME="$HOME/.rustup" \
"$probe/deps/daemon-tests" native_codex_build_review_and_landing_release_owned_output --ignored --nocapture
```
