# Main baseline: compilation and fan-out

Compile: controlled ov-1110 capture 8a7ce8759, with build inputs identical to main ancestor d986f3083. Graph/lines: main snapshot 73974f898.
Normal dependencies union platform predicates. Selector consumers include dev/build/optional/platform edges and explicit CLI -> daemon.
Compiler seconds aggregate that package’s library, binary and build-script invocations; overlap prevents interpreting these as additive wall time.

| Crate | src / tests lines now | Compiler seconds measured | Normal deps | Selector direct / transitive consumers |
| --- | ---: | ---: | ---: | ---: |
| farcooler-agent-core | 828 / 0 | 0.36 | 0 | 3 / 5 |
| farcooler-agent-hooks | 1,566 / 0 | 0.31 | 1 | 2 / 4 |
| farcooler-ceremony | 3,315 / 0 | 0.57 | 1 | 1 / 4 |
| farcooler-cli | 39,496 / 1,881 | 4.44 | 10 | 1 / 3 |
| farcooler-client | 25,501 / 4,106 | 4.40 | 8 | 3 / 3 |
| farcooler-core | 34,412 / 518 | 3.33 | 2 | 6 / 7 |
| farcooler-daemon | 167,406 / 27,623 | 19.61 | 12 | 2 / 3 |
| farcooler-fence | 2,681 / 766 | 0.40 | 1 | 3 / 4 |
| farcooler-ffi-guard | 130 / 299 | 0.05 | 0 | 6 / 10 |
| farcooler-jni | 1,384 / 0 | 0.43 | 3 | 0 / 0 |
| farcooler-protocol | 3,348 / 0 | 5.04 | 1 | 10 / 11 |
| farcooler-review | 1,582 / 0 | 0.45 | 1 | 1 / 4 |
| farcooler-store | 39,227 / 0 | 4.36 | 3 | 3 / 4 |
| farcooler-tailcat | 1,642 / 0 | 0.12 | 1 | 3 / 4 |
| farcooler-tmux | 4,117 / 946 | 0.73 | 2 | 2 / 4 |
| farcooler-transport | 3,163 / 1,472 | 0.39 | 2 | 3 / 4 |
| farcooler-vt | 4,266 / 184 | 0.75 | 1 | 2 / 4 |
| farcooler-workflow | 1,970 / 2,999 | 1.39 | 0 | 4 / 12 |

Transitive consumers exclude the changed package itself, even in a test cycle.
The independent web workspace and Swift/Go/Android application suites are outside this Rust graph; no omitted-test savings are assigned to them.
