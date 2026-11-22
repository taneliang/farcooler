# ov-1422 engine disk cleanup verification

On 22 November 2026, the engine completed an isolated build, review, send-back, rebuild, second review, and grouped landing on the macOS runner. The fixture ran the configured adapters in real Git worktrees and pushed the landed candidate to a local bare repository. The adapters generated representative output files so the test could verify what the engine reclaimed.

| Sample | Available bytes |
| --- | ---: |
| Before the first build | 85,064,511,488 |
| After landing and cleanup | 85,143,945,216 |
| Change in available space | +79,433,728 |
| Logical bytes reported by engine cleanup | 150,995,274 |

The test sampled the scratch runtime's volume with `statvfs`. Available space includes concurrent activity on the shared host volume. The engine's cleanup records measure the logical size of the files it removed.

Both builds passed the check that reads the rebuilt worktree output. The builder and reviewer scratch directories disappeared after settlement, and the ticket's `target/` disappeared at each join wait. The landing worktree and its output were released while the group record remained on the board. Advancing the cleanup clock through seven days removed the engine-recorded gate log and exposed its three reclaimed bytes in `workflow get`.

The startup recovery regression held a reader on a finished group's retained checkout. The first sweep preserved the checkout, and a retry after the file closed released it. Historical `/tmp/lane-gate-logs-*` directories lack the engine ownership records required for automatic deletion.

Run the disk fixture with `cargo test -p farcooler-daemon --lib build_review_send_back_and_landing_release_owned_output -- --nocapture`.
