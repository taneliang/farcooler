# ov-1247: CI train gate evidence

The orchestrator ran the gate on commit `f47b682e1` in a scratch worktree on November 15, 2026, 08:28–09:17 SGT, with a cold build. The full run used:

```sh
./scripts/lane-gate.py --base origin/main --fail-fast
```

The full run passed the executed steps in wire, doc-comments, relay, site, tunnel, rust (including `cargo clippy --workspace --all-targets -- -D warnings`), swift-tests, swift, ios and android. It stopped in web at `GOOS=js GOARCH=wasm go test` with `total length of command line and environment variables exceeds limit`, an artifact of the orchestrator session's oversized environment. The orchestrator reran the web job alone using `env -i` with a minimal environment and all 12 executed steps passed. The web rerun was a partial gate; the full output also records steps left to CI and iOS UI smoke not run locally. See [full output](orchestrator-gate-full.out) and [web output](orchestrator-gate-web.out) for the individual results and skips. The exact environment assignments used for the rerun were not recorded in the supplied evidence; the corresponding web-only invocation is `env -i … ./scripts/lane-gate.py --base origin/main --only web`; the exact command line is not recorded in the supplied outputs.

| Job | Seconds |
| --- | ---: |
| wire | 306 |
| doc-comments | 2 |
| relay | 27 |
| site | 9 |
| tunnel | 30 |
| rust | 1003 |
| swift-tests | 577 |
| swift | 299 |
| ios | 91 |
| android | 396 |

Full-run wall clock: **49 minutes 37 seconds**.

## Tickets carried

From [ov-1242's train report](../ov-1242/README.md):

- Queued tickets: ov-1211, ov-1216, ov-1209, ov-1184, ov-1219, ov-1183, ov-1215, ov-1225, ov-1230, ov-1223.
- Train tickets: ov-1218, ov-1224, ov-1189, ov-1182, ov-1226, ov-1156, ov-1237.
- Also carried at that report: ov-1186, ov-1200, ov-1188, ov-1205, ov-1207, ov-1185, ov-1190, ov-1194, ov-1229, ov-1239, ov-1240; the report notes that the final rebase drops any that land first.
- ov-1242 itself added an Android shard matrix self-test.
