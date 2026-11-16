# ov-1304: Mac tests find Xcode on every runner

## What changed

- Every `xcode-27` job that builds for Apple platforms (ci.yml iOS and Swift, canary.yml, release.yml, nightly.yml, and the two iOS UI actions) now selects Xcode in its "Check the runner has Xcode 27" step.
- The step tries the runner-declared `XCODE_27_DEVELOPER_DIR`, then `xcode-select -p`, then `/Applications/Xcode*.app`. It takes the first Xcode 27 whose own `xcrun --show-sdk-platform-path` succeeds, then exports `DEVELOPER_DIR` to the rest of the job through `GITHUB_ENV`.
- If none qualifies, the step fails with "No usable Xcode 27 on this runner; set DEVELOPER_DIR."
- `scripts/macos-test-partitions.py` fails with one line naming `DEVELOPER_DIR` when `xcrun --show-sdk-platform-path` fails. A self-test with a stubbed `xcrun` covers it.
- `apps/macos/test.sh` runs after that step in the same job, so it inherits `DEVELOPER_DIR`. What the jobs test is unchanged.

## Checks

- `macos-test-partitions.py --self-test`, `lane-gate.py --self-test`, every `scripts/*-lint.py`, `file-size-budget.py` and `daemon-layers.py` all pass.
- CI run 43924635548: job "Swift (AgentKit + Mac tests)" passed.
- Run 43916193057 (the previous commit) also had that job pass before a later push cancelled the run.
- The orchestrator asked to stop at one passing run instead of three, because main is red until this lands.
- The temporary branch trigger in ci.yml is removed.
