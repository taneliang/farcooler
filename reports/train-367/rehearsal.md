# Train 367 rehearsal (private synthetic fixtures only)

No owner database, app, daemon or provider was touched. Run at the train-367 tip, under `scripts/heavy`.

| Step | Result |
| --- | --- |
| `cargo test -p farcooler-store --lib engine_cutover` | 105 passed, 2 ignored. Includes the whole `released_board` module: wait, block, unblock, timed hold, subagent record, tracked-ticket wait and blocks, and the stage moves at `released`. |
| `engine_cutover_rehearsal` example | Reached `committed`, incarnation 1, manifest equal to the baseline, an independent restore made; send, push and Enter counters all 0. |
| `cargo test -p farcooler-daemon --test a_cutover_survives_every_crash` | 7 passed in 85.7 s (the real-crash matrix). |
| `reports/ov-940/old-reader.py` | The pinned registry-60 reader at d625aaeb9 opens the raw v60 file and refuses the file Prepare leaves with `NewerData`. |

Not covered: the owner-copy rehearsal, power-loss durability, Linux for the `/proc` holder scan, and the Robolectric and iOS UI classes beyond those named in the train report.
