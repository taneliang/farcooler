# ov-1387: main CI baseline (corrected)

Measured 2026-11-20 using GitHub Actions job and step APIs for the latest 20 `ci.yml` push runs on `main`. Times are minutes rounded to 0.1. Elapsed is workflow creation to last update, a proxy for push-to-finish, not the push timestamp. Queue is job creation to start; dependency wait before job creation is excluded. Jobs are assigned to the first attempt containing their name and start/end timestamps; later carried-forward rows, including those with rewritten job IDs and start times before attempt creation, are omitted. Skipped or unavailable timings are —. Step buckets are approximate: setup includes checkout/toolchain, cache restore includes named cache steps, build/test includes named build, test, install, check and run steps, and other includes post steps. They are not a decomposition of job runtime: gaps and rounded values are omitted, and generic Run steps can include setup. Cache hit/miss cannot be inferred from step durations.

## Runs and jobs

| Run (commit) | Attempts | Elapsed | Conclusion |
|---|---:|---:|---|
| [44684599999](https://github.com/taneliang/farcooler/actions/runs/44684599999) (15d08bec0) | 1 | 22.5 | success |
| [44609213426](https://github.com/taneliang/farcooler/actions/runs/44609213426) (729b7450e) | 1 | 40.3 | success |
| [44552281642](https://github.com/taneliang/farcooler/actions/runs/44552281642) (9c4d20f7a) | 1 | 10.5 | success |
| [44542300252](https://github.com/taneliang/farcooler/actions/runs/44542300252) (b6e460b32) | 2 | 51.2 | success |
| [44508900060](https://github.com/taneliang/farcooler/actions/runs/44508900060) (c2b141a46) | 2 | 48.4 | success |
| [44484977088](https://github.com/taneliang/farcooler/actions/runs/44484977088) (b055801d4) | 1 | 22.4 | success |
| [44467578753](https://github.com/taneliang/farcooler/actions/runs/44467578753) (a323b6576) | 1 | 29.2 | success |
| [44453520666](https://github.com/taneliang/farcooler/actions/runs/44453520666) (5aebef044) | 1 | 23.2 | success |
| [44444058541](https://github.com/taneliang/farcooler/actions/runs/44444058541) (d4a239c45) | 2 | 35.9 | failure |
| [44393561916](https://github.com/taneliang/farcooler/actions/runs/44393561916) (a0aa47fd2) | 1 | 32.6 | success |
| [44382423548](https://github.com/taneliang/farcooler/actions/runs/44382423548) (e54bee246) | 2 | 60.3 | success |
| [44356126259](https://github.com/taneliang/farcooler/actions/runs/44356126259) (22ef4a6d0) | 1 | 28.0 | success |
| [44341222618](https://github.com/taneliang/farcooler/actions/runs/44341222618) (226871c4f) | 2 | 41.4 | success |
| [44313293882](https://github.com/taneliang/farcooler/actions/runs/44313293882) (2f0b5f026) | 4 | 103.0 | success |
| [44289779514](https://github.com/taneliang/farcooler/actions/runs/44289779514) (52a5b1d8c) | 2 | 33.9 | success |
| [44275639329](https://github.com/taneliang/farcooler/actions/runs/44275639329) (d05ba3f97) | 2 | 46.6 | success |
| [44247584655](https://github.com/taneliang/farcooler/actions/runs/44247584655) (7baa3ed87) | 2 | 42.6 | success |
| [44237987362](https://github.com/taneliang/farcooler/actions/runs/44237987362) (dd928aa49) | 1 | 21.4 | success |
| [44225800145](https://github.com/taneliang/farcooler/actions/runs/44225800145) (c62e46f2c) | 2 | 80.2 | success |
| [44221066329](https://github.com/taneliang/farcooler/actions/runs/44221066329) (233feabd8) | 2 | 38.4 | success |

### 44684599999 (15d08bec0)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.1 | 4.2 | 0.1 | 0.0 | 3.8 | 0.1 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Web | 0.0 | 22.1 | 0.3 | 0.1 | 21.4 | 0.3 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust test archive | — | — | — | — | — | — | skipped |
| 1 | Swift (AgentKit + Mac tests) | — | — | — | — | — | — | skipped |
| 1 | Rust native test archive (macOS) | — | — | — | — | — | — | skipped |
| 1 | Swift (app build + bundle checks) | — | — | — | — | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | — | — | — | — | skipped |
| 1 | Rust (${{ matrix.os }}) | — | — | — | — | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | — | — | — | — | skipped |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | — | — | — | — | skipped |
| 1 | Rust tests (${{ matrix.shard }}/4) | — | — | — | — | — | — | skipped |
| 1 | Tunnel helper (Go) | 0.0 | 0.6 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Site | 0.0 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Relay | — | — | — | — | — | — | skipped |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | iOS | — | — | — | — | — | — | skipped |

### 44609213426 (729b7450e)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust test archive | 0.1 | 9.0 | 0.3 | 0.0 | 8.5 | 0.1 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Android unit tests | 0.1 | 7.7 | 0.4 | 0.0 | 6.7 | 0.5 | success |
| 1 | Wire compatibility | 0.1 | 4.7 | 0.2 | 0.0 | 4.1 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 30.0 | 0.5 | 0.1 | 28.6 | 0.8 | success |
| 1 | Android UI tests (first) | 0.1 | 4.6 | 0.2 | 0.1 | 3.9 | 0.3 | success |
| 1 | Android release build | 7.7 | 11.4 | 0.4 | 0.1 | 10.5 | 0.4 | success |
| 1 | Android UI tests (rest) | 4.7 | 9.7 | 0.4 | 0.1 | 8.8 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 13.5 | 0.3 | 0.0 | 12.7 | 0.4 | success |
| 1 | Web | 0.1 | 11.9 | 0.3 | 0.5 | 11.0 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.4 | 0.7 | 0.4 | 4.7 | 0.5 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | iOS | 0.1 | 6.4 | 0.2 | 0.0 | 6.0 | 0.1 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust tests (2/4) | 0.1 | 10.6 | 0.1 | 0.0 | 1.5 | 9.0 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.1 | 0.2 | 0.0 | 1.5 | 9.4 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.6 | 0.1 | 0.0 | 0.4 | 11.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.8 | 0.1 | 0.0 | 0.4 | 11.2 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 12.3 | 0.3 | 0.0 | 11.8 | 0.1 | success |
| 1 | Rust (macos-latest) | 0.1 | 6.9 | 0.2 | 0.0 | 6.3 | 0.3 | success |
| 1 | Android | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Relay | 0.1 | 0.9 | 0.1 | 0.0 | 0.7 | 0.1 | success |
| 1 | Site | 0.0 | 0.4 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.2 | 0.1 | 0.0 | 7.2 | 0.8 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | 0.2 | 0.0 | 9.0 | 0.6 | success |

### 44552281642 (9c4d20f7a)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Web | 0.1 | 10.1 | 0.3 | 0.5 | 9.2 | 0.0 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | 0.3 | 0.0 | 4.3 | 0.2 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Swift (AgentKit + Mac tests) | — | — | — | — | — | — | skipped |
| 1 | Rust native test archive (macOS) | — | — | — | — | — | — | skipped |
| 1 | Rust test archive | — | — | — | — | — | — | skipped |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | — | — | — | — | skipped |
| 1 | Swift (app build + bundle checks) | — | — | — | — | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | — | — | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | — | — | — | — | skipped |
| 1 | Rust tests (${{ matrix.shard }}/4) | — | — | — | — | — | — | skipped |
| 1 | Rust (${{ matrix.os }}) | — | — | — | — | — | — | skipped |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Relay | — | — | — | — | — | — | skipped |
| 1 | iOS | — | — | — | — | — | — | skipped |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |

### 44542300252 (b6e460b32)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.9 | 0.3 | 0.2 | 8.2 | 0.1 | success |
| 1 | Rust test archive | 0.1 | 4.0 | 0.5 | 0.1 | 3.4 | 0.0 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 9.8 | 0.7 | 0.3 | 7.8 | 0.8 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | 0.1 | 0.0 | 4.4 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 14.1 | 0.5 | 0.8 | 11.8 | 0.9 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.1 | 0.0 | success |
| 1 | Android UI tests (rest) | 4.4 | 9.6 | 0.3 | 0.1 | 8.6 | 0.4 | success |
| 1 | Web | 0.1 | 18.4 | 0.3 | 0.1 | 17.7 | 0.3 | success |
| 1 | Android unit tests | 0.1 | 3.4 | 0.3 | 0.1 | 2.6 | 0.3 | success |
| 1 | Android UI tests (first) | 0.1 | 4.3 | 0.3 | 0.1 | 3.4 | 0.4 | success |
| 1 | Android release build | 3.5 | 9.1 | 0.3 | 0.1 | 8.2 | 0.4 | success |
| 1 | iOS | 0.1 | 6.4 | 0.2 | 0.0 | 6.0 | 0.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.1 | 0.1 | 0.0 | 0.6 | 11.4 | success |
| 1 | Rust tests (3/4) | 0.1 | 12.0 | 0.1 | 0.0 | 0.3 | 11.5 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.5 | 0.2 | 0.0 | 0.6 | 11.6 | success |
| 1 | Rust tests (4/4) | 0.0 | 13.2 | 0.1 | 0.0 | 0.5 | 12.5 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust (macos-latest) | 0.1 | 9.0 | 0.6 | 0.2 | 8.0 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.9 | 0.3 | 0.2 | 8.3 | 0.0 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust native tests (macOS 2/2) | 0.0 | 15.0 | — | — | — | — | cancelled |
| 1 | Rust native tests (macOS 1/2) | 5.0 | 9.7 | 0.2 | 0.0 | 8.7 | 0.8 | success |
| 1 | Relay | 0.0 | 1.1 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 1 | Site | 0.0 | 0.4 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | 0.2 | 0.0 | 0.5 | 0.0 | success |
| 2 | Rust native tests (macOS 2/2) | 10.1 | 11.3 | 0.2 | 0.0 | 10.4 | 0.7 | success |
| 2 | iOS UI (smoke) | — | — | — | — | — | — | skipped |

### 44508900060 (c2b141a46)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.0 | 0.7 | 0.5 | 4.4 | 0.4 | success |
| 1 | Rust test archive | 0.1 | 7.8 | 0.1 | 0.0 | 7.5 | 0.1 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 19.8 | 0.7 | 0.3 | 17.8 | 1.0 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 16.9 | 0.4 | 0.0 | 15.8 | 0.6 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | 0.1 | 0.0 | 4.5 | 0.1 | success |
| 1 | Web | 0.1 | 13.0 | 0.3 | 0.6 | 11.9 | 0.1 | success |
| 1 | Android unit tests | 0.1 | 3.4 | 0.5 | 0.1 | 2.3 | 0.4 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Android release build | 3.5 | 14.4 | 0.3 | 0.1 | 13.5 | 0.4 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.2 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android UI tests (first) | 0.1 | 3.9 | 0.3 | 0.1 | 3.1 | 0.3 | success |
| 1 | Android UI tests (rest) | 4.0 | 10.2 | 0.4 | 0.1 | 9.3 | 0.4 | success |
| 1 | iOS | 0.1 | 8.2 | 0.2 | 0.0 | 7.8 | 0.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.6 | 0.3 | 0.0 | 2.1 | 9.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.4 | 0.2 | 0.0 | 2.8 | 9.3 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.2 | 0.3 | 0.0 | 3.7 | 9.2 | failure |
| 1 | Rust tests (3/4) | 0.1 | 10.6 | 0.3 | 0.0 | 1.2 | 9.1 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust (macos-latest) | 0.8 | 16.2 | 0.5 | 0.0 | 14.9 | 0.8 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 14.8 | 0.5 | 0.0 | 14.0 | 0.1 | success |
| 1 | Android | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust native tests (macOS 2/2) | 0.2 | 11.2 | 0.2 | 0.0 | 10.1 | 0.8 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.1 | 0.1 | 0.0 | 7.3 | 0.6 | success |
| 1 | Site | 0.1 | 0.7 | 0.2 | 0.0 | 0.3 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Relay | 0.1 | 1.2 | 0.1 | 0.0 | 1.0 | 0.1 | success |
| 2 | Rust tests (2/4) | 0.1 | 12.7 | 0.2 | 0.0 | 3.4 | 9.1 | success |
| 2 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 2 | Relay | 0.1 | 0.9 | 0.1 | 0.0 | 0.8 | 0.0 | success |
| 2 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.6 | 0.1 | 0.0 | 0.5 | 0.0 | success |

### 44484977088 (b055801d4)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 11.6 | 0.7 | 0.5 | 9.9 | 0.5 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Rust test archive | 0.0 | 4.0 | 0.3 | 0.1 | 3.5 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.5 | 0.6 | 0.2 | 6.0 | 0.6 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 7.8 | 0.3 | 0.2 | 7.2 | 0.0 | success |
| 1 | Android UI tests (rest) | 5.1 | 9.2 | 0.3 | 0.1 | 8.6 | 0.2 | success |
| 1 | Android unit tests | 0.1 | 4.1 | 0.4 | 0.1 | 3.2 | 0.3 | success |
| 1 | Wire compatibility | 0.1 | 5.1 | 0.2 | 0.0 | 4.5 | 0.2 | success |
| 1 | Android UI tests (first) | 0.1 | 5.0 | 0.3 | 0.1 | 4.2 | 0.4 | success |
| 1 | Web | 0.1 | 22.0 | 0.5 | 0.0 | 21.3 | 0.2 | success |
| 1 | Android release build | 4.2 | 11.4 | 0.3 | 0.1 | 9.9 | 0.9 | success |
| 1 | iOS | 0.1 | 6.2 | 0.2 | 0.0 | 5.8 | 0.1 | success |
| 1 | Rust tests (4/4) | 0.0 | 10.1 | 0.1 | 0.0 | 0.8 | 9.1 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.0 | 0.2 | 0.0 | 3.7 | 9.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.3 | 0.1 | 0.0 | 0.5 | 10.8 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.7 | 0.2 | 0.0 | 2.0 | 9.4 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust (macos-latest) | 0.1 | 11.4 | 0.5 | 0.3 | 10.4 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.5 | 0.4 | 0.1 | 7.9 | 0.0 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.5 | 0.2 | 0.0 | 8.6 | 0.7 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.1 | 0.2 | 0.0 | 7.3 | 0.6 | success |
| 1 | Android | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Relay | 0.1 | 0.9 | 0.1 | 0.0 | 0.7 | 0.1 | success |
| 1 | Site | 0.0 | 0.5 | 0.1 | 0.0 | 0.3 | 0.1 | success |

### 44467578753 (a323b6576)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.0 | 4.6 | 0.1 | 0.0 | 4.2 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.8 | 0.7 | 0.8 | 10.9 | 0.4 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 13.9 | 0.3 | 0.0 | 12.8 | 0.6 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.3 | 0.7 | 0.6 | 4.7 | 0.3 | success |
| 1 | Android release build | 4.1 | 9.5 | 0.3 | 0.1 | 8.6 | 0.4 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Android UI tests (rest) | 6.0 | 7.5 | 0.6 | 0.1 | 6.6 | 0.2 | success |
| 1 | Rust test archive | 0.0 | 6.9 | 0.2 | 0.0 | 6.5 | 0.1 | success |
| 1 | Android unit tests | 0.1 | 4.0 | 0.4 | 0.1 | 2.4 | 1.0 | success |
| 1 | Android UI tests (first) | 0.0 | 5.9 | 0.5 | 0.1 | 4.9 | 0.4 | success |
| 1 | Web | 0.0 | 10.3 | 0.3 | 0.6 | 9.4 | 0.0 | success |
| 1 | iOS UI plan | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | iOS | 0.1 | 6.2 | 0.2 | 0.0 | 5.9 | 0.1 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.2 | 0.1 | 0.0 | 9.7 | 0.3 | success |
| 1 | Rust tests (2/4) | 0.1 | 10.7 | 0.2 | 0.0 | 1.1 | 9.4 | success |
| 1 | Rust tests (4/4) | 0.1 | 10.8 | 0.2 | 0.0 | 1.1 | 9.4 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.8 | 0.2 | 0.0 | 1.4 | 9.1 | success |
| 1 | Rust tests (3/4) | 0.0 | 9.7 | 0.2 | 0.0 | 0.6 | 8.9 | success |
| 1 | Android | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.0 | 4.6 | 0.1 | 0.0 | 4.1 | 0.4 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 14.9 | 0.4 | 0.0 | 14.3 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.8 | 0.1 | 0.0 | 8.1 | 0.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.5 | 0.4 | 0.0 | 6.7 | 0.4 | success |
| 1 | Relay | 0.0 | 1.1 | 0.1 | 0.0 | 1.0 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Site | 0.0 | 0.3 | 0.1 | 0.0 | 0.2 | 0.0 | success |

### 44453520666 (5aebef044)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.5 | 0.7 | 0.6 | 10.6 | 0.6 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | 0.1 | 0.0 | 4.4 | 0.2 | success |
| 1 | iOS UI plan | 0.1 | 0.4 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Android UI tests (rest) | 5.0 | 7.2 | 0.3 | 0.1 | 6.5 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 4.0 | 0.3 | 0.1 | 3.6 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.9 | 0.7 | 0.4 | 5.4 | 0.3 | success |
| 1 | Android UI tests (first) | 0.1 | 4.9 | 0.3 | 0.1 | 4.0 | 0.4 | success |
| 1 | Android unit tests | 0.1 | 4.2 | 0.3 | 0.1 | 3.2 | 0.5 | success |
| 1 | Android release build | 4.3 | 13.9 | 0.4 | 0.1 | 12.4 | 0.9 | success |
| 1 | Web | 0.1 | 13.3 | 0.3 | 0.3 | 12.6 | 0.0 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 6.7 | 0.3 | 0.2 | 6.0 | 0.1 | success |
| 1 | iOS | 0.1 | 5.5 | 0.2 | 0.0 | 5.2 | 0.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.2 | 0.1 | 0.0 | 0.6 | 10.4 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.0 | 0.1 | 0.0 | 0.6 | 12.3 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.3 | 0.2 | 0.0 | 1.8 | 9.3 | success |
| 1 | Rust tests (3/4) | 0.1 | 10.6 | 0.1 | 0.0 | 0.7 | 9.7 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.0 | 0.1 | 0.0 | 9.5 | 0.4 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 10.5 | 0.4 | 0.2 | 9.8 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.8 | 0.5 | 0.2 | 7.8 | 0.2 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.5 | 0.1 | 0.0 | 8.8 | 0.5 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 10.3 | 0.2 | 0.0 | 9.3 | 0.8 | success |
| 1 | Android | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Relay | 0.1 | 0.9 | 0.1 | 0.0 | 0.8 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Site | 0.0 | 0.5 | 0.2 | 0.0 | 0.3 | 0.0 | success |

### 44444058541 (d4a239c45)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.1 | 5.8 | 0.2 | 0.0 | 5.3 | 0.2 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.6 | 0.8 | 0.1 | 11.0 | 0.6 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust test archive | 0.1 | 3.9 | 0.3 | 0.1 | 3.5 | 0.0 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.7 | 0.3 | 0.2 | 8.1 | 0.0 | success |
| 1 | Web | 0.1 | 10.5 | 0.3 | 0.5 | 9.6 | 0.0 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 10.2 | 0.4 | 0.5 | 8.9 | 0.4 | success |
| 1 | Android UI tests (first) | 0.1 | 6.8 | 0.2 | 0.0 | 6.0 | 0.5 | success |
| 1 | Android UI tests (rest) | 6.8 | 10.1 | 0.4 | 0.1 | 9.2 | 0.3 | success |
| 1 | Android release build | 8.0 | 8.9 | 0.3 | 0.1 | 8.1 | 0.3 | success |
| 1 | Android unit tests | 0.1 | 7.9 | 0.4 | 0.0 | 6.8 | 0.6 | success |
| 1 | iOS | 0.1 | 6.1 | 0.2 | 0.0 | 5.8 | 0.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 9.3 | 0.1 | 0.0 | 0.4 | 8.7 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.2 | 0.1 | 0.0 | 0.5 | 10.6 | failure |
| 1 | Rust tests (4/4) | 0.0 | 13.2 | 0.1 | 0.0 | 0.5 | 12.6 | success |
| 1 | Rust tests (3/4) | 0.0 | 9.8 | 0.1 | 0.0 | 0.5 | 9.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 14.2 | 0.2 | 0.0 | 13.5 | 0.5 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 10.5 | 0.4 | 0.1 | 9.9 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.1 | 0.5 | 0.3 | 6.2 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.4 | 0.2 | 0.0 | 8.5 | 0.6 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.8 | 0.2 | 0.0 | 8.9 | 0.6 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | 0.2 | 0.0 | 0.5 | 0.0 | success |
| 1 | Relay | 0.1 | 1.1 | 0.2 | 0.0 | 0.7 | 0.1 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.4 | 0.1 | success |
| 2 | Rust tests (2/4) | 0.1 | 11.7 | 0.2 | 0.0 | 2.4 | 9.0 | failure |
| 2 | Relay | 0.0 | 1.0 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 2 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.4 | 0.0 | success |
| 2 | Tunnel helper (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |

### 44393561916 (a0aa47fd2)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust test archive | 0.1 | 8.8 | 0.3 | 0.0 | 8.2 | 0.2 | success |
| 1 | Rust native test archive (macOS) | 5.1 | 8.0 | 0.3 | 0.1 | 7.0 | 0.5 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.8 | 0.6 | 0.5 | 5.8 | 0.7 | success |
| 1 | Android release build | 4.5 | 10.5 | 0.3 | 0.1 | 9.6 | 0.3 | success |
| 1 | Web | 0.1 | 14.4 | 0.2 | 0.3 | 13.0 | 0.8 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android UI tests (first) | 0.1 | 4.0 | 0.3 | 0.1 | 3.0 | 0.5 | success |
| 1 | Android unit tests | 0.1 | 4.4 | 0.4 | 0.1 | 3.2 | 0.6 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | 0.2 | 0.0 | 4.4 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 10.2 | 0.4 | 0.9 | 8.2 | 0.7 | success |
| 1 | Android UI tests (rest) | 4.0 | 11.1 | 0.3 | 0.1 | 10.1 | 0.4 | success |
| 1 | iOS | 0.1 | 6.6 | 0.2 | 0.0 | 5.9 | 0.4 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.2 | 0.1 | 0.0 | 10.6 | 0.4 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.7 | 0.2 | 0.0 | 3.1 | 9.3 | success |
| 1 | Rust tests (4/4) | 0.0 | 13.1 | 0.1 | 0.0 | 2.3 | 10.6 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.9 | 0.2 | 0.0 | 2.6 | 9.0 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.1 | 0.1 | 0.0 | 0.5 | 11.4 | success |
| 1 | Rust native tests (macOS 1/2) | 10.1 | 9.1 | 0.2 | 0.0 | 8.2 | 0.7 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | 0.2 | 0.0 | 8.9 | 0.6 | success |
| 1 | Rust (macos-latest) | 0.1 | 10.8 | 0.5 | 0.3 | 9.2 | 0.7 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 13.8 | 0.3 | 0.1 | 13.3 | 0.1 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Relay | 0.1 | 1.0 | 0.2 | 0.0 | 0.8 | 0.1 | success |
| 1 | Site | 0.1 | 0.6 | 0.1 | 0.0 | 0.4 | 0.1 | success |

### 44382423548 (e54bee246)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust test archive | 0.1 | 3.7 | 0.4 | 0.1 | 3.2 | 0.0 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 17.2 | 0.6 | 0.4 | 15.3 | 0.8 | success |
| 1 | Wire compatibility | 0.1 | 5.9 | 0.2 | 0.0 | 5.4 | 0.2 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.0 | 0.8 | 0.4 | 5.5 | 0.3 | success |
| 1 | Android UI tests (rest) | 4.2 | 9.9 | 0.4 | 0.1 | 9.0 | 0.4 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android release build | 3.7 | 13.1 | 0.3 | 0.1 | 12.3 | 0.4 | success |
| 1 | Web | 0.1 | 18.4 | 0.3 | 0.1 | 17.6 | 0.3 | success |
| 1 | Android UI tests (first) | 0.1 | 4.1 | 0.4 | 0.1 | 3.1 | 0.3 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 15.1 | — | — | — | — | cancelled |
| 1 | Android unit tests | 0.1 | 3.6 | 0.4 | 0.1 | 2.7 | 0.4 | success |
| 1 | iOS | 0.1 | 6.5 | 0.2 | 0.0 | 6.2 | 0.0 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.0 | 0.1 | 0.0 | 0.7 | 12.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.7 | 0.2 | 0.0 | 1.1 | 9.4 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.1 | 0.2 | 0.0 | 3.7 | 9.1 | success |
| 1 | Rust tests (3/4) | 0.1 | 13.2 | 0.1 | 0.0 | 1.0 | 12.0 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.9 | 0.4 | 0.2 | 9.3 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 11.1 | 0.3 | 0.0 | 10.2 | 0.5 | success |
| 1 | Android | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Relay | 0.1 | 1.1 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | — | — | — | — | skipped |
| 2 | Rust native test archive (macOS) | 10.1 | 12.0 | 0.4 | 0.0 | 11.0 | 0.5 | success |
| 2 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 2 | Rust native tests (macOS 2/2) | 0.1 | 10.1 | 0.2 | 0.0 | 9.0 | 0.9 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 11.1 | 0.2 | 0.1 | 10.0 | 0.9 | success |
| 2 | Rust (ubuntu-24.04) | 0.1 | 9.3 | 0.4 | 0.1 | 8.7 | 0.0 | success |
| 2 | Rust (macos-latest) | 0.1 | 8.8 | 0.4 | 0.4 | 7.9 | 0.1 | success |

### 44356126259 (22ef4a6d0)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 14.1 | 0.3 | 0.0 | 13.0 | 0.6 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Android release build | 4.2 | 10.1 | 0.5 | 0.1 | 9.1 | 0.3 | success |
| 1 | Android unit tests | 0.1 | 4.1 | 0.4 | 0.1 | 3.2 | 0.4 | success |
| 1 | Android UI tests (first) | 0.1 | 4.5 | 0.5 | 0.1 | 3.3 | 0.5 | success |
| 1 | Wire compatibility | 0.1 | 4.6 | 0.2 | 0.0 | 4.1 | 0.1 | success |
| 1 | Web | 0.1 | 13.9 | 0.4 | 0.3 | 13.0 | 0.1 | success |
| 1 | Android UI tests (rest) | 4.6 | 7.9 | 0.3 | 0.1 | 7.0 | 0.4 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 16.1 | 0.9 | 0.8 | 13.9 | 0.5 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.1 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 8.3 | 0.5 | 0.2 | 6.4 | 1.1 | success |
| 1 | Rust test archive | 0.1 | 7.5 | 0.2 | 0.1 | 6.9 | 0.2 | success |
| 1 | iOS | 0.1 | 9.0 | 0.2 | 0.0 | 8.6 | 0.1 | success |
| 1 | Rust tests (1/4) | 0.0 | 12.2 | 0.1 | 0.0 | 0.4 | 11.6 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.6 | 0.2 | 0.0 | 2.4 | 8.9 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.1 | 0.1 | 0.0 | 1.0 | 11.9 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.7 | 0.2 | 0.0 | 0.5 | 12.9 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust (ubuntu-24.04) | 0.0 | 13.5 | 0.4 | 0.0 | 13.0 | 0.1 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.5 | 0.4 | 0.2 | 7.7 | 0.1 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 10.3 | 0.2 | 0.0 | 9.4 | 0.7 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | 0.2 | 0.0 | 8.5 | 1.0 | success |
| 1 | Relay | 0.1 | 1.6 | 0.1 | 0.0 | 1.4 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Site | 0.1 | 0.6 | 0.1 | 0.0 | 0.3 | 0.1 | success |

### 44341222618 (226871c4f)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 9.4 | 0.3 | 0.2 | 8.8 | 0.1 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.7 | 0.7 | 0.3 | 5.2 | 0.3 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 21.6 | 0.7 | 0.6 | 19.1 | 1.1 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | 0.1 | 0.0 | 4.5 | 0.2 | success |
| 1 | Android unit tests | 0.1 | 4.2 | 0.4 | 0.1 | 3.2 | 0.4 | success |
| 1 | Android UI tests (rest) | 5.8 | 11.9 | 0.3 | 0.1 | 11.0 | 0.5 | success |
| 1 | Web | 0.1 | 18.5 | 0.3 | 0.1 | 17.8 | 0.3 | success |
| 1 | Android UI tests (first) | 0.1 | 5.8 | 0.5 | 0.1 | 4.3 | 0.8 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Android release build | 4.2 | 13.8 | 0.4 | 0.1 | 12.8 | 0.4 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.0 | success |
| 1 | Rust test archive | 0.1 | 3.9 | 0.5 | 0.1 | 3.3 | 0.0 | success |
| 1 | iOS | 0.1 | 7.4 | 0.2 | 0.0 | 7.0 | 0.1 | success |
| 1 | Rust tests (2/4) | 0.1 | 12.8 | 0.1 | 0.0 | 0.5 | 12.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.3 | 0.1 | 0.0 | 1.8 | 9.4 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.9 | 0.1 | 0.0 | 0.4 | 10.3 | success |
| 1 | Rust tests (3/4) | 0.1 | 12.2 | 0.1 | 0.0 | 0.5 | 11.5 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.8 | 0.1 | 0.0 | 11.2 | 0.4 | success |
| 1 | Rust (macos-latest) | 5.1 | 7.7 | 0.5 | 0.2 | 6.9 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.8 | 0.6 | 0.2 | 8.9 | 0.0 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Relay | 0.1 | 1.1 | 0.1 | 0.0 | 1.0 | 0.0 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.6 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.0 | 0.2 | 0.0 | 8.0 | 0.7 | failure |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.6 | 0.1 | 0.0 | 7.8 | 0.6 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 9.8 | 0.2 | 0.0 | 8.8 | 0.7 | success |

### 44313293882 (2f0b5f026)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 8.8 | 0.8 | 0.2 | 7.1 | 0.7 | success |
| 1 | Wire compatibility | 0.1 | 4.7 | 0.3 | 0.1 | 4.1 | 0.1 | success |
| 1 | Tunnel (Go) | 0.1 | 0.6 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Rust native test archive (macOS) | 10.1 | 12.7 | 0.4 | 0.0 | 11.7 | 0.5 | success |
| 1 | Android UI tests (first) | 0.0 | 4.8 | 0.3 | 0.1 | 3.9 | 0.5 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.3 | 0.6 | 0.6 | 10.7 | 0.4 | success |
| 1 | Rust test archive | 0.1 | 6.8 | 0.3 | 0.0 | 6.3 | 0.1 | success |
| 1 | Android unit tests | 0.1 | 4.7 | 0.5 | 0.2 | 3.6 | 0.4 | success |
| 1 | Web | 0.1 | 10.6 | 0.3 | 0.5 | 9.7 | 0.0 | success |
| 1 | Android UI tests (rest) | 4.9 | 7.7 | 0.4 | 0.1 | 6.7 | 0.3 | success |
| 1 | iOS UI plan | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Android release build | 4.8 | 10.6 | 0.3 | 0.1 | 9.6 | 0.4 | success |
| 1 | iOS | 0.1 | 9.9 | 0.3 | 0.0 | 9.4 | 0.1 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.4 | 0.2 | 0.0 | 2.0 | 9.1 | success |
| 1 | Rust tests (3/4) | 0.0 | 8.8 | 0.1 | 0.0 | 0.5 | 8.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.7 | 0.1 | 0.0 | 0.9 | 11.7 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.2 | 0.2 | 0.0 | 2.8 | 9.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.2 | 0.1 | 0.0 | 9.7 | 0.4 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Relay | 0.1 | 1.3 | 0.1 | 0.0 | 1.1 | 0.1 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.4 | 0.1 | success |
| 1 | Rust (macos-latest) | 0.1 | 31.5 | 0.8 | 0.1 | 30.1 | 0.6 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 12.6 | 0.4 | 0.0 | 12.0 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 10.2 | 4.9 | 0.1 | 0.0 | 4.4 | 0.4 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 4.4 | 0.1 | 0.0 | 3.9 | 0.4 | failure |
| 2 | Rust native tests (macOS 1/2) | 0.0 | 15.0 | — | — | — | — | cancelled |
| 3 | Rust native tests (macOS 1/2) | 5.1 | 8.9 | 0.2 | 0.0 | 8.0 | 0.7 | failure |
| 4 | Rust native tests (macOS 1/2) | 5.0 | 6.1 | 0.1 | 0.0 | 5.5 | 0.4 | success |

### 44289779514 (52a5b1d8c)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.7 | 0.3 | 0.2 | 8.0 | 0.1 | success |
| 1 | Rust test archive | 0.0 | 4.2 | 0.3 | 0.1 | 3.7 | 0.0 | success |
| 1 | Web | 0.1 | 22.0 | 0.3 | 0.1 | 21.2 | 0.3 | success |
| 1 | Android release build | 3.6 | 9.9 | 0.3 | 0.1 | 9.1 | 0.4 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android UI tests (first) | 0.1 | 4.9 | 0.3 | 0.1 | 4.0 | 0.4 | success |
| 1 | Wire compatibility | 0.1 | 5.0 | 0.1 | 0.0 | 4.6 | 0.2 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.8 | 0.6 | 0.4 | 5.3 | 0.4 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 13.9 | 0.6 | 0.7 | 11.9 | 0.6 | success |
| 1 | Android UI tests (rest) | 5.0 | 9.6 | 0.3 | 0.1 | 8.6 | 0.5 | success |
| 1 | Android unit tests | 0.0 | 3.5 | 0.3 | 0.1 | 2.6 | 0.4 | success |
| 1 | iOS | 0.1 | 8.9 | 0.2 | 0.0 | 8.6 | 0.1 | success |
| 1 | Rust tests (3/4) | 0.0 | 10.9 | 0.1 | 0.0 | 0.4 | 10.4 | success |
| 1 | Rust tests (4/4) | 0.0 | 11.6 | 0.1 | 0.0 | 0.4 | 11.1 | success |
| 1 | Rust tests (1/4) | 0.0 | 11.2 | 0.1 | 0.0 | 0.4 | 10.7 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.9 | 0.1 | 0.0 | 1.0 | 11.8 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.8 | 0.6 | 0.2 | 9.0 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.0 | 4.5 | 0.3 | 0.1 | 4.0 | 0.1 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.4 | 0.1 | 0.0 | 9.2 | 1.9 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 11.3 | 0.2 | 0.0 | 10.4 | 0.7 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.9 | 0.1 | 0.0 | 7.9 | 0.8 | failure |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Site | 0.1 | 0.3 | 0.1 | 0.0 | 0.2 | 0.0 | success |
| 1 | Relay | 0.1 | 1.1 | 0.1 | 0.0 | 0.9 | 0.1 | success |
| 2 | Rust native tests (macOS 1/2) | 0.2 | 7.6 | 0.1 | 0.0 | 6.9 | 0.5 | success |

### 44275639329 (d05ba3f97)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | 0.2 | 0.1 | 4.2 | 0.1 | success |
| 1 | Tunnel (Go) | 0.0 | 0.6 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Rust test archive | 0.1 | 6.6 | 0.3 | 0.0 | 6.2 | 0.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 5.3 | 0.3 | 0.0 | 4.7 | 0.3 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 25.5 | 0.7 | 0.6 | 23.1 | 1.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.4 | 0.6 | 0.2 | 10.8 | 0.8 | success |
| 1 | Web | 0.1 | 10.5 | 0.3 | 0.5 | 9.6 | 0.0 | success |
| 1 | Android unit tests | 0.1 | 4.0 | 0.3 | 0.1 | 3.2 | 0.3 | success |
| 1 | Android release build | 4.1 | 13.3 | 0.3 | 0.1 | 12.5 | 0.4 | success |
| 1 | Android UI tests (rest) | 2.9 | 7.0 | 0.3 | 0.1 | 6.4 | 0.2 | success |
| 1 | Android UI tests (first) | 0.1 | 2.8 | 0.3 | 0.1 | 2.2 | 0.2 | failure |
| 1 | iOS | 0.0 | 7.0 | 0.2 | 0.0 | 6.7 | 0.0 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.7 | 0.4 | 0.1 | 8.1 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 16.1 | 0.4 | 0.0 | 14.9 | 0.7 | success |
| 1 | Rust tests (3/4) | 0.0 | 12.5 | 0.1 | 0.0 | 0.3 | 12.0 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.8 | 0.2 | 0.0 | 2.4 | 9.1 | success |
| 1 | Rust tests (1/4) | 0.0 | 9.9 | 0.2 | 0.0 | 0.7 | 9.1 | success |
| 1 | Rust tests (4/4) | 0.0 | 10.9 | 0.1 | 0.0 | 0.4 | 10.4 | success |
| 1 | iOS UI (smoke) | 0.0 | 9.4 | 0.1 | 0.0 | 9.0 | 0.3 | success |
| 1 | Android | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | failure |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Relay | 0.1 | 1.1 | 0.2 | 0.0 | 0.7 | 0.1 | success |
| 1 | Site | 0.1 | 0.5 | 0.1 | 0.0 | 0.3 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 10.8 | 0.2 | 0.0 | 9.7 | 0.7 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.9 | 0.1 | 0.0 | 8.1 | 0.6 | failure |
| 2 | Android UI tests (first) | 0.0 | 2.6 | 0.3 | 0.1 | 1.8 | 0.4 | success |
| 2 | Rust native tests (macOS 1/2) | 0.3 | 9.2 | 0.2 | 0.0 | 8.2 | 0.8 | success |
| 2 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.7 | 0.2 | 0.0 | 0.5 | 0.0 | success |
| 2 | Relay | 0.1 | 0.9 | 0.2 | 0.0 | 0.7 | 0.0 | success |
| 2 | Site | 0.1 | 0.7 | 0.2 | 0.0 | 0.4 | 0.0 | success |

### 44247584655 (7baa3ed87)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.0 | 4.5 | 0.1 | 0.0 | 4.2 | 0.1 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 18.5 | 0.6 | 0.8 | 16.6 | 0.4 | success |
| 1 | Swift (app build + bundle checks) | 0.0 | 10.8 | 0.6 | 0.3 | 9.7 | 0.2 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 5.3 | 0.2 | 0.1 | 4.8 | 0.1 | success |
| 1 | Web | 0.0 | 18.3 | 0.3 | 0.0 | 17.6 | 0.3 | success |
| 1 | Rust test archive | 0.0 | 3.9 | 0.3 | 0.1 | 3.5 | 0.0 | success |
| 1 | Android unit tests | 0.0 | 6.2 | 0.3 | 0.0 | 5.3 | 0.5 | success |
| 1 | iOS UI plan | 0.0 | 0.3 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Android release build | 6.3 | 13.2 | 0.3 | 0.1 | 12.4 | 0.4 | success |
| 1 | Android UI tests (first) | 0.0 | 4.8 | 0.3 | 0.1 | 4.0 | 0.4 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.1 | 0.0 | 0.5 | 0.0 | success |
| 1 | Android UI tests (rest) | 4.9 | 9.3 | 0.3 | 0.1 | 8.6 | 0.3 | success |
| 1 | iOS | 0.0 | 4.9 | 0.1 | 0.0 | 4.6 | 0.0 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.5 | 0.2 | 0.0 | 2.2 | 9.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.7 | 0.2 | 0.0 | 1.1 | 9.3 | failure |
| 1 | Rust tests (4/4) | 0.1 | 12.3 | 0.1 | 0.0 | 0.3 | 11.8 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.8 | 0.1 | 0.0 | 0.4 | 12.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 13.7 | 0.1 | 0.0 | 12.5 | 1.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.0 | 0.3 | 0.2 | 6.4 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.3 | 0.5 | 0.4 | 7.3 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 10.2 | 0.1 | 0.0 | 9.1 | 0.8 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.7 | 0.1 | 0.0 | 8.0 | 0.6 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Site | 0.1 | 0.4 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Relay | 0.1 | 1.2 | 0.1 | 0.0 | 1.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | 0.2 | 0.0 | 0.5 | 0.0 | success |
| 2 | Rust tests (1/4) | 0.1 | 12.1 | 0.1 | 0.0 | 1.1 | 10.9 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 2 | Relay | 0.0 | 1.0 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 2 | Site | 0.0 | 0.4 | 0.1 | 0.0 | 0.2 | 0.0 | success |

### 44237987362 (dd928aa49)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Wire compatibility | 0.0 | 4.7 | 0.1 | 0.0 | 4.3 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.0 | 12.0 | 0.6 | 0.7 | 10.2 | 0.5 | success |
| 1 | Rust test archive | 0.0 | 9.1 | 0.3 | 0.0 | 8.5 | 0.2 | success |
| 1 | iOS UI plan | 0.1 | 0.4 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | 0.2 | 0.0 | 0.4 | 0.0 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 3.1 | 0.2 | 0.1 | 2.7 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.4 | 0.7 | 0.5 | 5.9 | 0.2 | success |
| 1 | Android | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Web | — | — | — | — | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | — | — | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | — | — | — | — | skipped |
| 1 | iOS | 0.1 | 7.9 | 0.2 | 0.0 | 7.6 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 7.1 | 0.3 | 0.1 | 6.6 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.3 | 0.5 | 0.3 | 7.3 | 0.1 | success |
| 1 | iOS UI (smoke) | — | — | — | — | — | — | skipped |
| 1 | Rust tests (1/4) | 0.1 | 10.4 | 0.2 | 0.0 | 1.2 | 8.9 | success |
| 1 | Rust tests (2/4) | 0.0 | 9.7 | 0.1 | 0.0 | 0.4 | 9.2 | success |
| 1 | Rust tests (3/4) | 0.0 | 10.4 | 0.1 | 0.0 | 0.4 | 9.8 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.0 | 0.2 | 0.0 | 1.7 | 9.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.0 | 0.2 | 0.0 | 8.2 | 0.5 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.4 | 0.1 | 0.0 | 7.6 | 0.6 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Site | 0.0 | 0.4 | 0.1 | 0.0 | 0.2 | 0.0 | success |
| 1 | Relay | — | — | — | — | — | — | skipped |

### 44225800145 (c62e46f2c)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | 0.1 | 0.0 | 0.0 | 0.1 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.8 | 0.7 | 0.4 | 5.5 | 0.2 | success |
| 1 | Wire compatibility | 0.1 | 5.0 | 0.1 | 0.0 | 4.6 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 45.4 | 0.5 | 0.8 | 44.0 | 0.1 | cancelled |
| 1 | Rust native test archive (macOS) | 0.1 | 9.1 | 0.3 | 0.3 | 8.4 | 0.0 | success |
| 1 | Rust test archive | 0.1 | 4.6 | 0.3 | 0.1 | 4.1 | 0.0 | success |
| 1 | Tunnel (Go) | 0.0 | 0.6 | 0.1 | 0.0 | 0.4 | 0.0 | success |
| 1 | Android release build | 3.3 | 13.4 | 0.4 | 0.1 | 12.1 | 0.6 | success |
| 1 | Android unit tests | 0.1 | 3.2 | 0.4 | 0.1 | 2.3 | 0.3 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.1 | 0.0 | 0.0 | 0.0 | success |
| 1 | Android UI tests (rest) | 4.9 | 10.1 | 0.4 | 0.1 | 9.2 | 0.3 | success |
| 1 | Web | 0.1 | 9.5 | 0.3 | 0.4 | 8.8 | 0.0 | success |
| 1 | Android UI tests (first) | 0.1 | 4.8 | 0.3 | 0.1 | 4.1 | 0.3 | success |
| 1 | iOS | 0.1 | 6.3 | 0.2 | 0.0 | 5.9 | 0.1 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.3 | 0.2 | 0.0 | 2.1 | 9.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.0 | 0.2 | 0.0 | 1.7 | 9.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.2 | 0.1 | 0.0 | 0.5 | 11.6 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.4 | 0.2 | 0.0 | 2.2 | 8.9 | success |
| 1 | iOS UI (smoke) | 0.1 | 14.5 | 0.2 | 0.0 | 13.1 | 1.2 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.8 | 0.5 | 0.2 | 7.0 | 0.1 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.5 | 0.3 | 0.1 | 8.1 | 0.0 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Relay | 0.0 | 1.0 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 1 | Site | 0.0 | 0.5 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.2 | 0.1 | 0.0 | 8.2 | 0.8 | failure |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.0 | 0.2 | 0.0 | 8.2 | 0.6 | success |
| 2 | Swift (AgentKit + Mac tests) | 0.1 | 12.9 | 0.5 | 0.6 | 11.2 | 0.5 | success |
| 2 | Rust native tests (macOS 2/2) | 0.1 | 9.5 | 0.2 | 0.0 | 8.6 | 0.7 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 10.5 | 0.3 | 0.0 | 9.4 | 0.8 | success |

### 44221066329 (233feabd8)

| Attempt | Job | Queue | Run | Setup | Cache restore | Build/test | Other | Result |
|---:|---|---:|---:|---:|---:|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | 0.2 | 0.0 | 0.0 | 0.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.8 | 0.7 | 0.4 | 11.3 | 0.3 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 15.6 | 0.5 | 0.6 | 13.8 | 0.6 | success |
| 1 | Android release build | 3.7 | 13.2 | 0.3 | 0.1 | 12.3 | 0.4 | success |
| 1 | Wire compatibility | 0.0 | 4.9 | 0.1 | 0.0 | 4.6 | 0.2 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 9.6 | 0.3 | 0.3 | 8.8 | 0.1 | success |
| 1 | Android unit tests | 0.1 | 3.6 | 0.5 | 0.2 | 2.5 | 0.3 | success |
| 1 | Web | 0.0 | 13.5 | 0.3 | 0.1 | 12.8 | 0.2 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Android UI tests (first) | 0.1 | 3.8 | 0.3 | 0.1 | 2.9 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 3.9 | 0.4 | 0.1 | 3.3 | 0.0 | success |
| 1 | Android UI tests (rest) | 3.9 | 7.5 | 0.3 | 0.1 | 6.6 | 0.5 | success |
| 1 | iOS | 0.1 | 8.3 | 0.2 | 0.0 | 8.0 | 0.0 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.3 | 0.1 | 0.0 | 0.4 | 11.8 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.2 | 0.1 | 0.0 | 0.6 | 12.5 | failure |
| 1 | Rust tests (1/4) | 0.1 | 12.0 | 0.1 | 0.0 | 2.3 | 9.6 | success |
| 1 | Rust tests (3/4) | 0.1 | 13.0 | 0.1 | 0.0 | 1.3 | 11.6 | success |
| 1 | iOS UI (smoke) | 0.1 | 9.3 | 0.1 | 0.0 | 8.5 | 0.6 | success |
| 1 | Rust (ubuntu-24.04) | 0.0 | 8.4 | 0.4 | 0.1 | 7.9 | 0.0 | success |
| 1 | Rust (macos-latest) | 0.1 | 9.3 | 0.5 | 0.3 | 8.4 | 0.1 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.7 | 0.1 | 0.0 | 8.8 | 0.7 | success |
| 1 | Rust native tests (macOS 2/2) | 0.0 | 7.9 | 0.2 | 0.0 | 7.2 | 0.6 | success |
| 1 | Android | 0.1 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | 0.1 | 0.0 | 0.6 | 0.0 | success |
| 1 | Relay | 0.0 | 1.1 | 0.1 | 0.0 | 1.0 | 0.0 | success |
| 1 | Site | 0.0 | 0.4 | 0.1 | 0.0 | 0.3 | 0.0 | success |
| 2 | Rust tests (2/4) | 0.1 | 11.3 | 0.2 | 0.0 | 2.0 | 9.1 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.6 | 0.1 | 0.0 | 0.4 | 0.0 | success |
| 2 | Relay | 0.1 | 1.0 | 0.1 | 0.0 | 0.9 | 0.0 | success |
| 2 | Site | 0.1 | 0.4 | 0.1 | 0.0 | 0.3 | 0.0 | success |

## Findings and limits

* In 44609213426, Swift tests ran 30.0 minutes. Its job log records Rust, SwiftPM downloads, Swift compilation and tmux cache misses; native targets took 9.2 minutes, AgentKit 4.8, Mac tests 12.3 and tmux installation 0.9. In 44484977088, the same steps took 1.8, 1.3 and 5.5 minutes respectively, and the job ran 11.6. These observations identify cold builds as a contributor, not a controlled attribution of all variance.
* The 44684599999 Web job alone ran 22.1 minutes. In 44609213426, Android release queued 7.7 minutes and ran 11.4. Native macOS shards also wait for both Swift jobs and the native archive before starting. These are distinct critical paths; fixing Swift alone cannot establish the end-to-end goal.
* Rerun delay below is the interval from the latest completed actual job in the previous attempt to the earliest creation time of a new job in the next attempt. Negative values reflect overlapping attempts and are retained; this is not necessarily human wait time.

| Run | Next attempt | Delay (min) |
|---|---:|---:|
| 44542300252 | 2 | 0.4 |
| 44508900060 | 2 | 0.5 |
| 44444058541 | 2 | 0.3 |
| 44382423548 | 2 | 0.4 |
| 44341222618 | 2 | 0.5 |
| 44313293882 | 2 | 0.3 |
| 44313293882 | 3 | 0.5 |
| 44313293882 | 4 | 7.4 |
| 44289779514 | 2 | 0.5 |
| 44275639329 | 2 | 0.4 |
| 44247584655 | 2 | 0.3 |
| 44225800145 | 2 | 1.6 |
| 44221066329 | 2 | 0.3 |

No post-change five-run Swift sample or five consecutive push-to-last-job measurements are available here. GitHub-hosted runner/cache availability is external, but this sample does not establish it as the single limiting factor. The acceptance targets therefore remain unverified.

## Capacity constraint on proposed parallelism

The checked-in capacity model (`python3 scripts/ci-macos-capacity.py`) reports a conservative peak of 14 landing jobs, including four Macs, and 20 total jobs with the shared background reservation, recorder and Nightly yield gate. It reserves five Macs in all. The native test shards deliberately wait for both Swift jobs as well as their archive; removing either Swift dependency fails the model's regression checks. The Android build and UI matrices each use `max-parallel: 1`; those waits are workflow-imposed serialization, not evidence of a GitHub runner queue. More simultaneous shards or an additional Swift job require a revised capacity plan, not just changing those settings. This model is an upper bound for overlapping workflows, not proof that external runner capacity is the single limit on these observed runs.

The cold Swift job's named steps add 26.3 minutes (9.2 native build, 4.8 AgentKit, 12.3 Mac tests), versus 8.6 minutes for the corresponding warm steps in 44484977088. The cache misses and differences are observed together; they do not isolate the effect of each cache. A safe split must preserve the required checks and test guards, fit the capacity reservation, and then be measured on five executions. No such split or post-change measurements have been validated in this branch.
