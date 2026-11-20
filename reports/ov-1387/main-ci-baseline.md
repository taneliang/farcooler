# ov-1387: main CI baseline

Measured 2026-11-20 from GitHub Actions REST API: the latest 20 `ci.yml` push runs on `main`, including every attempt. Times are minutes, rounded to 0.1. Run elapsed is workflow creation to last update, including time between attempts; it is a proxy for push-to-finish, since the push timestamp was not collected. Job queue is job `created_at` to `started_at`, and runtime is `started_at` to `completed_at`; dependency wait before job creation is not included. GitHub sometimes reports skipped jobs with equal timestamps: these are marked skipped rather than treated as zero-duration work. Attempt endpoints can include jobs from earlier attempts, so job IDs are deduplicated. Missing timings are shown as —.

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

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.3 | success |
| 1 | Wire compatibility | 0.1 | 4.2 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | success |
| 1 | Web | 0.0 | 22.1 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Android | 0.0 | 0.1 | success |
| 1 | Rust test archive | — | — | skipped |
| 1 | Swift (AgentKit + Mac tests) | — | — | skipped |
| 1 | Rust native test archive (macOS) | — | — | skipped |
| 1 | Swift (app build + bundle checks) | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | skipped |
| 1 | Rust (${{ matrix.os }}) | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | skipped |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | skipped |
| 1 | Rust tests (${{ matrix.shard }}/4) | — | — | skipped |
| 1 | Tunnel helper (Go) | 0.0 | 0.6 | success |
| 1 | Site | 0.0 | 0.5 | success |
| 1 | Relay | — | — | skipped |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | iOS | — | — | skipped |

### 44609213426 (729b7450e)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 9.0 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Android unit tests | 0.1 | 7.7 | success |
| 1 | Wire compatibility | 0.1 | 4.7 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 30.0 | success |
| 1 | Android UI tests (first) | 0.1 | 4.6 | success |
| 1 | Android release build | 7.7 | 11.4 | success |
| 1 | Android UI tests (rest) | 4.7 | 9.7 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 13.5 | success |
| 1 | Web | 0.1 | 11.9 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.4 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | iOS | 0.1 | 6.4 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust tests (2/4) | 0.1 | 10.6 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.6 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.8 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 12.3 | success |
| 1 | Rust (macos-latest) | 0.1 | 6.9 | success |
| 1 | Android | 0.0 | 0.0 | success |
| 1 | Relay | 0.1 | 0.9 | success |
| 1 | Site | 0.0 | 0.4 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.2 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | success |

### 44552281642 (9c4d20f7a)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Web | 0.1 | 10.1 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Swift (AgentKit + Mac tests) | — | — | skipped |
| 1 | Rust native test archive (macOS) | — | — | skipped |
| 1 | Rust test archive | — | — | skipped |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | skipped |
| 1 | Swift (app build + bundle checks) | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | skipped |
| 1 | Rust tests (${{ matrix.shard }}/4) | — | — | skipped |
| 1 | Rust (${{ matrix.os }}) | — | — | skipped |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 1 | Relay | — | — | skipped |
| 1 | iOS | — | — | skipped |
| 1 | iOS UI (smoke) | — | — | skipped |

### 44542300252 (b6e460b32)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.9 | success |
| 1 | Rust test archive | 0.1 | 4.0 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 9.8 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 14.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Android UI tests (rest) | 4.4 | 9.6 | success |
| 1 | Web | 0.1 | 18.4 | success |
| 1 | Android unit tests | 0.1 | 3.4 | success |
| 1 | Android UI tests (first) | 0.1 | 4.3 | success |
| 1 | Android release build | 3.5 | 9.1 | success |
| 1 | iOS | 0.1 | 6.4 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.1 | success |
| 1 | Rust tests (3/4) | 0.1 | 12.0 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.5 | success |
| 1 | Rust tests (4/4) | 0.0 | 13.2 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust (macos-latest) | 0.1 | 9.0 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.9 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.0 | 15.0 | cancelled |
| 1 | Rust native tests (macOS 1/2) | 5.0 | 9.7 | success |
| 1 | Relay | 0.0 | 1.1 | success |
| 1 | Site | 0.0 | 0.4 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 2 | Rust native tests (macOS 2/2) | 10.1 | 11.3 | success |
| 2 | Changes | -29.8 | 0.2 | success |
| 2 | Android unit tests | -29.5 | 3.4 | success |
| 2 | Android release build | -26.1 | 9.1 | success |
| 2 | Web | -29.5 | 18.4 | success |
| 2 | Rust test archive | -29.5 | 4.0 | success |
| 2 | Rust tests (2/4) | -25.4 | 12.5 | success |
| 2 | iOS UI plan | -29.5 | 0.3 | success |
| 2 | Rust native tests (macOS 1/2) | -10.3 | 9.7 | success |
| 2 | Rust (macos-latest) | -20.5 | 9.0 | success |
| 2 | Rust tests (1/4) | -25.4 | 12.1 | success |
| 2 | Wire compatibility | -29.5 | 4.8 | success |
| 2 | Rust (ubuntu-24.04) | -20.5 | 8.9 | success |
| 2 | iOS | -29.1 | 6.4 | success |
| 2 | Rust tests (3/4) | -25.4 | 12.0 | success |
| 2 | Android | -15.5 | 0.1 | success |
| 2 | Rust tests (4/4) | -25.4 | 13.2 | success |
| 2 | iOS UI (smoke) | — | — | skipped |
| 2 | Tunnel helper (Go) | -12.2 | 0.8 | success |
| 2 | Relay | -12.2 | 1.1 | success |
| 2 | Site | -12.2 | 0.4 | success |
| 2 | Tunnel (Go) | -29.6 | 0.8 | success |
| 2 | Swift (app build + bundle checks) | -29.6 | 9.8 | success |
| 2 | Rust native test archive (macOS) | -29.5 | 8.9 | success |
| 2 | Android UI tests (rest) | -25.2 | 9.6 | success |
| 2 | Android UI tests (first) | -29.6 | 4.3 | success |
| 2 | Swift (AgentKit + Mac tests) | -29.6 | 14.1 | success |

### 44508900060 (c2b141a46)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.0 | success |
| 1 | Rust test archive | 0.1 | 7.8 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 19.8 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 16.9 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | success |
| 1 | Web | 0.1 | 13.0 | success |
| 1 | Android unit tests | 0.1 | 3.4 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Android release build | 3.5 | 14.4 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Android UI tests (first) | 0.1 | 3.9 | success |
| 1 | Android UI tests (rest) | 4.0 | 10.2 | success |
| 1 | iOS | 0.1 | 8.2 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.6 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.4 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.2 | failure |
| 1 | Rust tests (3/4) | 0.1 | 10.6 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust (macos-latest) | 0.8 | 16.2 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 14.8 | success |
| 1 | Android | 0.2 | 0.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.2 | 11.2 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.1 | success |
| 1 | Site | 0.1 | 0.7 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | success |
| 1 | Relay | 0.1 | 1.2 | success |
| 2 | Changes | -34.7 | 0.2 | success |
| 2 | Tunnel (Go) | -34.4 | 0.8 | success |
| 2 | Rust tests (2/4) | 0.1 | 12.7 | success |
| 2 | Android UI tests (first) | -34.4 | 3.9 | success |
| 2 | Android unit tests | -34.4 | 3.4 | success |
| 2 | Web | -34.4 | 13.0 | success |
| 2 | Android UI tests (rest) | -30.4 | 10.2 | success |
| 2 | Swift (AgentKit + Mac tests) | -34.4 | 19.8 | success |
| 2 | Swift (app build + bundle checks) | -34.4 | 6.0 | success |
| 2 | Rust tests (1/4) | -26.6 | 11.6 | success |
| 2 | Android | -16.4 | 0.1 | success |
| 2 | Rust (ubuntu-24.04) | -17.4 | 14.8 | success |
| 2 | iOS | -34.1 | 8.2 | success |
| 2 | Rust native tests (macOS 1/2) | -14.6 | 8.1 | success |
| 2 | Rust (macos-latest) | -16.7 | 16.2 | success |
| 2 | Rust tests (4/4) | -26.6 | 12.4 | success |
| 2 | Rust tests (3/4) | -26.6 | 10.6 | success |
| 2 | Rust native tests (macOS 2/2) | -14.4 | 11.2 | success |
| 2 | iOS UI (smoke) | — | — | skipped |
| 2 | Rust native test archive (macOS) | -34.5 | 16.9 | success |
| 2 | Rust test archive | -34.5 | 7.8 | success |
| 2 | iOS UI plan | -34.5 | 0.3 | success |
| 2 | Android release build | -31.1 | 14.4 | success |
| 2 | Wire compatibility | -34.6 | 4.8 | success |
| 2 | Relay | 0.1 | 0.9 | success |
| 2 | Site | 0.1 | 0.5 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.6 | success |

### 44484977088 (b055801d4)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 11.6 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Rust test archive | 0.0 | 4.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.5 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 7.8 | success |
| 1 | Android UI tests (rest) | 5.1 | 9.2 | success |
| 1 | Android unit tests | 0.1 | 4.1 | success |
| 1 | Wire compatibility | 0.1 | 5.1 | success |
| 1 | Android UI tests (first) | 0.1 | 5.0 | success |
| 1 | Web | 0.1 | 22.0 | success |
| 1 | Android release build | 4.2 | 11.4 | success |
| 1 | iOS | 0.1 | 6.2 | success |
| 1 | Rust tests (4/4) | 0.0 | 10.1 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.3 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.7 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust (macos-latest) | 0.1 | 11.4 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.5 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.5 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.1 | success |
| 1 | Android | 0.0 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Relay | 0.1 | 0.9 | success |
| 1 | Site | 0.0 | 0.5 | success |

### 44467578753 (a323b6576)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Wire compatibility | 0.0 | 4.6 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.8 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 13.9 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.3 | success |
| 1 | Android release build | 4.1 | 9.5 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Android UI tests (rest) | 6.0 | 7.5 | success |
| 1 | Rust test archive | 0.0 | 6.9 | success |
| 1 | Android unit tests | 0.1 | 4.0 | success |
| 1 | Android UI tests (first) | 0.0 | 5.9 | success |
| 1 | Web | 0.0 | 10.3 | success |
| 1 | iOS UI plan | 0.0 | 0.2 | success |
| 1 | iOS | 0.1 | 6.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.2 | success |
| 1 | Rust tests (2/4) | 0.1 | 10.7 | success |
| 1 | Rust tests (4/4) | 0.1 | 10.8 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.8 | success |
| 1 | Rust tests (3/4) | 0.0 | 9.7 | success |
| 1 | Android | 0.0 | 0.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.0 | 4.6 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 14.9 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.8 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.5 | success |
| 1 | Relay | 0.0 | 1.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Site | 0.0 | 0.3 | success |

### 44453520666 (5aebef044)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.5 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | success |
| 1 | iOS UI plan | 0.1 | 0.4 | success |
| 1 | Android UI tests (rest) | 5.0 | 7.2 | success |
| 1 | Rust test archive | 0.1 | 4.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.9 | success |
| 1 | Android UI tests (first) | 0.1 | 4.9 | success |
| 1 | Android unit tests | 0.1 | 4.2 | success |
| 1 | Android release build | 4.3 | 13.9 | success |
| 1 | Web | 0.1 | 13.3 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 6.7 | success |
| 1 | iOS | 0.1 | 5.5 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.2 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.0 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.3 | success |
| 1 | Rust tests (3/4) | 0.1 | 10.6 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.0 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 10.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.8 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.5 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 10.3 | success |
| 1 | Android | 0.0 | 0.1 | success |
| 1 | Relay | 0.1 | 0.9 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 1 | Site | 0.0 | 0.5 | success |

### 44444058541 (d4a239c45)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Wire compatibility | 0.1 | 5.8 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.6 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 3.9 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.7 | success |
| 1 | Web | 0.1 | 10.5 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 10.2 | success |
| 1 | Android UI tests (first) | 0.1 | 6.8 | success |
| 1 | Android UI tests (rest) | 6.8 | 10.1 | success |
| 1 | Android release build | 8.0 | 8.9 | success |
| 1 | Android unit tests | 0.1 | 7.9 | success |
| 1 | iOS | 0.1 | 6.1 | success |
| 1 | Rust tests (1/4) | 0.1 | 9.3 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.2 | failure |
| 1 | Rust tests (4/4) | 0.0 | 13.2 | success |
| 1 | Rust tests (3/4) | 0.0 | 9.8 | success |
| 1 | iOS UI (smoke) | 0.1 | 14.2 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 10.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.4 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.8 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 1 | Relay | 0.1 | 1.1 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 2 | Changes | -23.1 | 0.3 | success |
| 2 | Wire compatibility | -22.7 | 5.8 | success |
| 2 | Tunnel (Go) | -22.7 | 0.8 | success |
| 2 | Rust native test archive (macOS) | -22.7 | 8.7 | success |
| 2 | Rust tests (2/4) | 0.1 | 11.7 | failure |
| 2 | Rust test archive | -22.7 | 3.9 | success |
| 2 | Android UI tests (first) | -22.7 | 6.8 | success |
| 2 | Android UI tests (rest) | -15.9 | 10.1 | success |
| 2 | Swift (app build + bundle checks) | -22.7 | 12.6 | success |
| 2 | Rust tests (1/4) | -18.7 | 9.3 | success |
| 2 | Rust tests (4/4) | -18.7 | 13.2 | success |
| 2 | Android unit tests | -22.7 | 7.9 | success |
| 2 | Rust native tests (macOS 1/2) | -10.1 | 9.8 | success |
| 2 | Rust (macos-latest) | -13.9 | 7.1 | success |
| 2 | Android release build | -14.8 | 8.9 | success |
| 2 | Rust tests (3/4) | -18.7 | 9.8 | success |
| 2 | Rust native tests (macOS 2/2) | -10.1 | 9.4 | success |
| 2 | Rust (ubuntu-24.04) | -13.9 | 10.5 | success |
| 2 | iOS | -22.4 | 6.1 | success |
| 2 | Android | -5.8 | 0.1 | success |
| 2 | iOS UI (smoke) | -16.2 | 14.2 | success |
| 2 | Web | -22.8 | 10.5 | success |
| 2 | iOS UI plan | -22.8 | 0.3 | success |
| 2 | Swift (AgentKit + Mac tests) | -22.8 | 10.2 | success |
| 2 | Relay | 0.0 | 1.0 | success |
| 2 | Site | 0.1 | 0.5 | success |
| 2 | Tunnel helper (Go) | 0.0 | 0.7 | success |

### 44393561916 (a0aa47fd2)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | success |
| 1 | Rust test archive | 0.1 | 8.8 | success |
| 1 | Rust native test archive (macOS) | 5.1 | 8.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.8 | success |
| 1 | Android release build | 4.5 | 10.5 | success |
| 1 | Web | 0.1 | 14.4 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | success |
| 1 | Android UI tests (first) | 0.1 | 4.0 | success |
| 1 | Android unit tests | 0.1 | 4.4 | success |
| 1 | iOS UI plan | 0.1 | 0.2 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 10.2 | success |
| 1 | Android UI tests (rest) | 4.0 | 11.1 | success |
| 1 | iOS | 0.1 | 6.6 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.2 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.7 | success |
| 1 | Rust tests (4/4) | 0.0 | 13.1 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.9 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.1 | success |
| 1 | Rust native tests (macOS 1/2) | 10.1 | 9.1 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | success |
| 1 | Rust (macos-latest) | 0.1 | 10.8 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 13.8 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Relay | 0.1 | 1.0 | success |
| 1 | Site | 0.1 | 0.6 | success |

### 44382423548 (e54bee246)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 3.7 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 17.2 | success |
| 1 | Wire compatibility | 0.1 | 5.9 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.0 | success |
| 1 | Android UI tests (rest) | 4.2 | 9.9 | success |
| 1 | Tunnel (Go) | 0.1 | 0.8 | success |
| 1 | Android release build | 3.7 | 13.1 | success |
| 1 | Web | 0.1 | 18.4 | success |
| 1 | Android UI tests (first) | 0.1 | 4.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 15.1 | cancelled |
| 1 | Android unit tests | 0.1 | 3.6 | success |
| 1 | iOS | 0.1 | 6.5 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.0 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.7 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.1 | success |
| 1 | Rust tests (3/4) | 0.1 | 13.2 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.9 | success |
| 1 | Rust (macos-latest) | 0.1 | 11.1 | success |
| 1 | Android | 0.1 | 0.0 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 1 | Relay | 0.1 | 1.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Rust native tests (macOS ${{ matrix.shard }}/2) | — | — | skipped |
| 2 | Rust native test archive (macOS) | 10.1 | 12.0 | success |
| 2 | Changes | -26.9 | 0.3 | success |
| 2 | iOS UI plan | -26.5 | 0.3 | success |
| 2 | Android unit tests | -26.5 | 3.6 | success |
| 2 | Swift (AgentKit + Mac tests) | -26.5 | 17.2 | success |
| 2 | Android UI tests (rest) | -22.4 | 9.9 | success |
| 2 | Rust tests (1/4) | -22.8 | 10.7 | success |
| 2 | Web | -26.5 | 18.4 | success |
| 2 | Rust tests (3/4) | -22.8 | 13.2 | success |
| 2 | Rust tests (2/4) | -22.8 | 13.1 | success |
| 2 | iOS | -26.2 | 6.5 | success |
| 2 | Android | -9.7 | 0.0 | success |
| 2 | Rust tests (4/4) | -22.8 | 13.0 | success |
| 2 | Relay | -9.6 | 1.1 | success |
| 2 | iOS UI (smoke) | — | — | skipped |
| 2 | Site | -9.6 | 0.5 | success |
| 2 | Tunnel helper (Go) | -9.6 | 0.7 | success |
| 2 | Android release build | -23.0 | 13.1 | success |
| 2 | Android UI tests (first) | -26.6 | 4.1 | success |
| 2 | Tunnel (Go) | -26.6 | 0.8 | success |
| 2 | Rust test archive | -26.6 | 3.7 | success |
| 2 | Swift (app build + bundle checks) | -26.6 | 7.0 | success |
| 2 | Wire compatibility | -26.7 | 5.9 | success |
| 2 | Rust native tests (macOS 2/2) | 0.1 | 10.1 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 11.1 | success |
| 2 | Rust (ubuntu-24.04) | 0.1 | 9.3 | success |
| 2 | Rust (macos-latest) | 0.1 | 8.8 | success |

### 44356126259 (22ef4a6d0)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 14.1 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | success |
| 1 | Android release build | 4.2 | 10.1 | success |
| 1 | Android unit tests | 0.1 | 4.1 | success |
| 1 | Android UI tests (first) | 0.1 | 4.5 | success |
| 1 | Wire compatibility | 0.1 | 4.6 | success |
| 1 | Web | 0.1 | 13.9 | success |
| 1 | Android UI tests (rest) | 4.6 | 7.9 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 16.1 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 8.3 | success |
| 1 | Rust test archive | 0.1 | 7.5 | success |
| 1 | iOS | 0.1 | 9.0 | success |
| 1 | Rust tests (1/4) | 0.0 | 12.2 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.6 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.1 | success |
| 1 | Rust tests (4/4) | 0.1 | 13.7 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust (ubuntu-24.04) | 0.0 | 13.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.5 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 10.3 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.8 | success |
| 1 | Relay | 0.1 | 1.6 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Site | 0.1 | 0.6 | success |

### 44341222618 (226871c4f)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 9.4 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.7 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 21.6 | success |
| 1 | Wire compatibility | 0.1 | 4.9 | success |
| 1 | Android unit tests | 0.1 | 4.2 | success |
| 1 | Android UI tests (rest) | 5.8 | 11.9 | success |
| 1 | Web | 0.1 | 18.5 | success |
| 1 | Android UI tests (first) | 0.1 | 5.8 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Android release build | 4.2 | 13.8 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust test archive | 0.1 | 3.9 | success |
| 1 | iOS | 0.1 | 7.4 | success |
| 1 | Rust tests (2/4) | 0.1 | 12.8 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.3 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.9 | success |
| 1 | Rust tests (3/4) | 0.1 | 12.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.8 | success |
| 1 | Rust (macos-latest) | 5.1 | 7.7 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.8 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Relay | 0.1 | 1.1 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.6 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.0 | failure |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.6 | success |
| 2 | Changes | -31.5 | 0.2 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 9.8 | success |
| 2 | Web | -31.3 | 18.5 | success |
| 2 | Android UI tests (first) | -31.3 | 5.8 | success |
| 2 | Rust tests (1/4) | -27.3 | 10.9 | success |
| 2 | Tunnel (Go) | -31.3 | 0.7 | success |
| 2 | Rust tests (2/4) | -27.3 | 12.8 | success |
| 2 | Rust (macos-latest) | -16.8 | 7.7 | success |
| 2 | Rust tests (3/4) | -27.3 | 12.2 | success |
| 2 | Rust (ubuntu-24.04) | -21.8 | 9.8 | success |
| 2 | iOS | -30.8 | 7.4 | success |
| 2 | Android | -13.2 | 0.1 | success |
| 2 | Rust tests (4/4) | -27.3 | 11.3 | success |
| 2 | Rust native tests (macOS 2/2) | -9.5 | 8.6 | success |
| 2 | iOS UI (smoke) | -23.4 | 11.8 | success |
| 2 | Tunnel helper (Go) | -13.1 | 0.6 | success |
| 2 | Site | -13.1 | 0.5 | success |
| 2 | Relay | -13.1 | 1.1 | success |
| 2 | Swift (app build + bundle checks) | -31.4 | 6.7 | success |
| 2 | Rust native test archive (macOS) | -31.4 | 9.4 | success |
| 2 | Swift (AgentKit + Mac tests) | -31.3 | 21.6 | success |
| 2 | Android unit tests | -31.4 | 4.2 | success |
| 2 | iOS UI plan | -31.4 | 0.3 | success |
| 2 | Android release build | -27.2 | 13.8 | success |
| 2 | Android UI tests (rest) | -25.6 | 11.9 | success |
| 2 | Wire compatibility | -31.4 | 4.9 | success |
| 2 | Rust test archive | -31.4 | 3.9 | success |

### 44313293882 (2f0b5f026)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 8.8 | success |
| 1 | Wire compatibility | 0.1 | 4.7 | success |
| 1 | Tunnel (Go) | 0.1 | 0.6 | success |
| 1 | Rust native test archive (macOS) | 10.1 | 12.7 | success |
| 1 | Android UI tests (first) | 0.0 | 4.8 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 12.3 | success |
| 1 | Rust test archive | 0.1 | 6.8 | success |
| 1 | Android unit tests | 0.1 | 4.7 | success |
| 1 | Web | 0.1 | 10.6 | success |
| 1 | Android UI tests (rest) | 4.9 | 7.7 | success |
| 1 | iOS UI plan | 0.0 | 0.2 | success |
| 1 | Android release build | 4.8 | 10.6 | success |
| 1 | iOS | 0.1 | 9.9 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.4 | success |
| 1 | Rust tests (3/4) | 0.0 | 8.8 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.7 | success |
| 1 | Rust tests (1/4) | 0.1 | 12.2 | success |
| 1 | iOS UI (smoke) | 0.1 | 10.2 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Relay | 0.1 | 1.3 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 31.5 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 12.6 | success |
| 1 | Rust native tests (macOS 2/2) | 10.2 | 4.9 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 4.4 | failure |
| 2 | Rust native tests (macOS 1/2) | 0.0 | 15.0 | cancelled |
| 2 | Changes | -54.9 | 0.2 | success |
| 2 | Android release build | -49.8 | 10.6 | success |
| 2 | Swift (app build + bundle checks) | -54.6 | 8.8 | success |
| 2 | Rust (macos-latest) | -31.8 | 31.5 | success |
| 2 | Rust tests (4/4) | -47.7 | 12.7 | success |
| 2 | Rust (ubuntu-24.04) | -31.9 | 12.6 | success |
| 2 | Rust tests (2/4) | -47.7 | 11.4 | success |
| 2 | iOS | -54.3 | 9.9 | success |
| 2 | Rust tests (3/4) | -47.8 | 8.8 | success |
| 2 | Android | -39.1 | 0.1 | success |
| 2 | Rust tests (1/4) | -47.7 | 12.2 | success |
| 2 | iOS UI (smoke) | -44.4 | 10.2 | success |
| 2 | Relay | -35.0 | 1.3 | success |
| 2 | Site | -35.0 | 0.5 | success |
| 2 | Tunnel helper (Go) | -35.0 | 0.7 | success |
| 2 | iOS UI plan | -54.7 | 0.2 | success |
| 2 | Android UI tests (first) | -54.7 | 4.8 | success |
| 2 | Wire compatibility | -54.7 | 4.7 | success |
| 2 | Rust native test archive (macOS) | -44.7 | 12.7 | success |
| 2 | Rust native tests (macOS 2/2) | -21.9 | 4.9 | success |
| 2 | Rust test archive | -54.7 | 6.8 | success |
| 2 | Swift (AgentKit + Mac tests) | -54.7 | 12.3 | success |
| 2 | Android UI tests (rest) | -49.9 | 7.7 | success |
| 2 | Android unit tests | -54.7 | 4.7 | success |
| 2 | Tunnel (Go) | -54.8 | 0.6 | success |
| 2 | Web | -54.8 | 10.6 | success |
| 3 | Changes | -70.4 | 0.2 | success |
| 3 | Swift (AgentKit + Mac tests) | -70.2 | 12.3 | success |
| 3 | Rust native tests (macOS 1/2) | 5.1 | 8.9 | failure |
| 3 | iOS UI plan | -70.2 | 0.2 | success |
| 3 | Rust test archive | -70.2 | 6.8 | success |
| 3 | Swift (app build + bundle checks) | -70.2 | 8.8 | success |
| 3 | Rust (ubuntu-24.04) | -47.4 | 12.6 | success |
| 3 | Rust native tests (macOS 2/2) | -37.3 | 4.9 | success |
| 3 | Rust tests (1/4) | -63.3 | 12.2 | success |
| 3 | Rust tests (3/4) | -63.3 | 8.8 | success |
| 3 | Rust (macos-latest) | -47.4 | 31.5 | success |
| 3 | Rust tests (2/4) | -63.3 | 11.4 | success |
| 3 | iOS | -69.8 | 9.9 | success |
| 3 | Rust tests (4/4) | -63.3 | 12.7 | success |
| 3 | Android | -54.7 | 0.1 | success |
| 3 | iOS UI (smoke) | -59.9 | 10.2 | success |
| 3 | Tunnel helper (Go) | -50.5 | 0.7 | success |
| 3 | Relay | -50.5 | 1.3 | success |
| 3 | Site | -50.5 | 0.5 | success |
| 3 | Web | -70.3 | 10.6 | success |
| 3 | Android UI tests (rest) | -65.4 | 7.7 | success |
| 3 | Android release build | -65.5 | 10.6 | success |
| 3 | Wire compatibility | -70.3 | 4.7 | success |
| 3 | Rust native test archive (macOS) | -60.3 | 12.7 | success |
| 3 | Tunnel (Go) | -70.3 | 0.6 | success |
| 3 | Android UI tests (first) | -70.3 | 4.8 | success |
| 3 | Android unit tests | -70.3 | 4.7 | success |
| 4 | Changes | -91.8 | 0.2 | success |
| 4 | Rust native tests (macOS 1/2) | 5.0 | 6.1 | success |
| 4 | Swift (AgentKit + Mac tests) | -91.5 | 12.3 | success |
| 4 | Tunnel (Go) | -91.5 | 0.6 | success |
| 4 | iOS UI plan | -91.5 | 0.2 | success |
| 4 | Rust tests (2/4) | -84.7 | 11.4 | success |
| 4 | Android unit tests | -91.5 | 4.7 | success |
| 4 | Android release build | -86.8 | 10.6 | success |
| 4 | Android UI tests (rest) | -86.7 | 7.7 | success |
| 4 | Rust tests (1/4) | -84.7 | 12.2 | success |
| 4 | Swift (app build + bundle checks) | -91.5 | 8.8 | success |
| 4 | Rust (ubuntu-24.04) | -68.8 | 12.6 | success |
| 4 | Rust native tests (macOS 2/2) | -58.7 | 4.9 | success |
| 4 | Rust tests (4/4) | -84.7 | 12.7 | success |
| 4 | iOS | -91.2 | 9.9 | success |
| 4 | Relay | -71.9 | 1.3 | success |
| 4 | iOS UI (smoke) | -81.3 | 10.2 | success |
| 4 | Android | -76.1 | 0.1 | success |
| 4 | Rust (macos-latest) | -68.8 | 31.5 | success |
| 4 | Site | -71.9 | 0.5 | success |
| 4 | Rust tests (3/4) | -84.7 | 8.8 | success |
| 4 | Tunnel helper (Go) | -71.9 | 0.7 | success |
| 4 | Rust native test archive (macOS) | -81.6 | 12.7 | success |
| 4 | Rust test archive | -91.7 | 6.8 | success |
| 4 | Wire compatibility | -91.7 | 4.7 | success |
| 4 | Android UI tests (first) | -91.7 | 4.8 | success |
| 4 | Web | -91.7 | 10.6 | success |

### 44289779514 (52a5b1d8c)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 8.7 | success |
| 1 | Rust test archive | 0.0 | 4.2 | success |
| 1 | Web | 0.1 | 22.0 | success |
| 1 | Android release build | 3.6 | 9.9 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | success |
| 1 | Android UI tests (first) | 0.1 | 4.9 | success |
| 1 | Wire compatibility | 0.1 | 5.0 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.8 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 13.9 | success |
| 1 | Android UI tests (rest) | 5.0 | 9.6 | success |
| 1 | Android unit tests | 0.0 | 3.5 | success |
| 1 | iOS | 0.1 | 8.9 | success |
| 1 | Rust tests (3/4) | 0.0 | 10.9 | success |
| 1 | Rust tests (4/4) | 0.0 | 11.6 | success |
| 1 | Rust tests (1/4) | 0.0 | 11.2 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.9 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 9.8 | success |
| 1 | Rust (macos-latest) | 0.0 | 4.5 | success |
| 1 | iOS UI (smoke) | 0.1 | 11.4 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 11.3 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.9 | failure |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 1 | Site | 0.1 | 0.3 | success |
| 1 | Relay | 0.1 | 1.1 | success |
| 2 | Changes | -26.0 | 0.2 | success |
| 2 | Rust native tests (macOS 1/2) | 0.2 | 7.6 | success |
| 2 | Rust test archive | -25.8 | 4.2 | success |
| 2 | Swift (AgentKit + Mac tests) | -25.8 | 13.9 | success |
| 2 | Android UI tests (rest) | -20.9 | 9.6 | success |
| 2 | Tunnel (Go) | -25.8 | 0.7 | success |
| 2 | Rust (ubuntu-24.04) | -17.0 | 9.8 | success |
| 2 | Rust tests (2/4) | -21.6 | 12.9 | success |
| 2 | Rust tests (1/4) | -21.6 | 11.2 | success |
| 2 | Rust tests (3/4) | -21.6 | 10.9 | success |
| 2 | Rust tests (4/4) | -21.6 | 11.6 | success |
| 2 | Rust (macos-latest) | -17.0 | 4.5 | success |
| 2 | Rust native tests (macOS 2/2) | -11.8 | 11.3 | success |
| 2 | iOS | -25.4 | 8.9 | success |
| 2 | Android | -11.3 | 0.1 | success |
| 2 | iOS UI (smoke) | -16.4 | 11.4 | success |
| 2 | Tunnel helper (Go) | -8.6 | 0.7 | success |
| 2 | Site | -8.6 | 0.3 | success |
| 2 | Relay | -8.6 | 1.1 | success |
| 2 | Wire compatibility | -25.9 | 5.0 | success |
| 2 | Web | -25.9 | 22.0 | success |
| 2 | iOS UI plan | -25.9 | 0.3 | success |
| 2 | Rust native test archive (macOS) | -25.9 | 8.7 | success |
| 2 | Android release build | -22.4 | 9.9 | success |
| 2 | Android UI tests (first) | -25.9 | 4.9 | success |
| 2 | Swift (app build + bundle checks) | -25.9 | 6.8 | success |
| 2 | Android unit tests | -25.9 | 3.5 | success |

### 44275639329 (d05ba3f97)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | Wire compatibility | 0.1 | 4.8 | success |
| 1 | Tunnel (Go) | 0.0 | 0.6 | success |
| 1 | Rust test archive | 0.1 | 6.6 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 5.3 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 25.5 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.4 | success |
| 1 | Web | 0.1 | 10.5 | success |
| 1 | Android unit tests | 0.1 | 4.0 | success |
| 1 | Android release build | 4.1 | 13.3 | success |
| 1 | Android UI tests (rest) | 2.9 | 7.0 | success |
| 1 | Android UI tests (first) | 0.1 | 2.8 | failure |
| 1 | iOS | 0.0 | 7.0 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.7 | success |
| 1 | Rust (macos-latest) | 0.1 | 16.1 | success |
| 1 | Rust tests (3/4) | 0.0 | 12.5 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.8 | success |
| 1 | Rust tests (1/4) | 0.0 | 9.9 | success |
| 1 | Rust tests (4/4) | 0.0 | 10.9 | success |
| 1 | iOS UI (smoke) | 0.0 | 9.4 | success |
| 1 | Android | 0.0 | 0.1 | failure |
| 1 | Tunnel helper (Go) | 0.0 | 0.7 | success |
| 1 | Relay | 0.1 | 1.1 | success |
| 1 | Site | 0.1 | 0.5 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 10.8 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 8.9 | failure |
| 2 | Android UI tests (first) | 0.0 | 2.6 | success |
| 2 | Rust native tests (macOS 1/2) | 0.3 | 9.2 | success |
| 2 | Changes | -37.0 | 0.3 | success |
| 2 | Android UI tests (rest) | -33.9 | 7.0 | success |
| 2 | Rust tests (1/4) | -30.1 | 9.9 | success |
| 2 | Rust (macos-latest) | -31.4 | 16.1 | success |
| 2 | Rust test archive | -36.7 | 6.6 | success |
| 2 | iOS UI (smoke) | -29.3 | 9.4 | success |
| 2 | Rust tests (2/4) | -30.0 | 11.8 | success |
| 2 | Rust tests (3/4) | -30.1 | 12.5 | success |
| 2 | Android unit tests | -36.8 | 4.0 | success |
| 2 | Web | -36.8 | 10.5 | success |
| 2 | Rust native test archive (macOS) | -36.8 | 5.3 | success |
| 2 | Rust tests (4/4) | -30.2 | 10.9 | success |
| 2 | Tunnel (Go) | -36.9 | 0.6 | success |
| 2 | Rust native tests (macOS 2/2) | -11.2 | 10.8 | success |
| 2 | iOS | -36.5 | 7.0 | success |
| 2 | Swift (app build + bundle checks) | -36.8 | 12.4 | success |
| 2 | Wire compatibility | -36.8 | 4.8 | success |
| 2 | Swift (AgentKit + Mac tests) | -36.8 | 25.5 | success |
| 2 | Android release build | -32.9 | 13.3 | success |
| 2 | iOS UI plan | -36.9 | 0.3 | success |
| 2 | Rust (ubuntu-24.04) | -31.5 | 8.7 | success |
| 2 | Android | 0.1 | 0.1 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 2 | Relay | 0.1 | 0.9 | success |
| 2 | Site | 0.1 | 0.7 | success |

### 44247584655 (7baa3ed87)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Wire compatibility | 0.0 | 4.5 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 18.5 | success |
| 1 | Swift (app build + bundle checks) | 0.0 | 10.8 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 5.3 | success |
| 1 | Web | 0.0 | 18.3 | success |
| 1 | Rust test archive | 0.0 | 3.9 | success |
| 1 | Android unit tests | 0.0 | 6.2 | success |
| 1 | iOS UI plan | 0.0 | 0.3 | success |
| 1 | Android release build | 6.3 | 13.2 | success |
| 1 | Android UI tests (first) | 0.0 | 4.8 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Android UI tests (rest) | 4.9 | 9.3 | success |
| 1 | iOS | 0.0 | 4.9 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.5 | success |
| 1 | Rust tests (1/4) | 0.1 | 10.7 | failure |
| 1 | Rust tests (4/4) | 0.1 | 12.3 | success |
| 1 | Rust tests (2/4) | 0.0 | 12.8 | success |
| 1 | iOS UI (smoke) | 0.1 | 13.7 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.0 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.3 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 10.2 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.7 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Site | 0.1 | 0.4 | success |
| 1 | Relay | 0.1 | 1.2 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.7 | success |
| 2 | Rust tests (1/4) | 0.1 | 12.1 | success |
| 2 | Changes | -29.3 | 0.2 | success |
| 2 | Android UI tests (first) | -29.0 | 4.8 | success |
| 2 | Swift (app build + bundle checks) | -29.0 | 10.8 | success |
| 2 | Web | -29.0 | 18.3 | success |
| 2 | Rust (ubuntu-24.04) | -23.7 | 8.3 | success |
| 2 | iOS UI (smoke) | -23.8 | 13.7 | success |
| 2 | Rust tests (4/4) | -25.1 | 12.3 | success |
| 2 | Rust tests (2/4) | -25.1 | 12.8 | success |
| 2 | Rust native tests (macOS 1/2) | -10.4 | 10.2 | success |
| 2 | Rust native tests (macOS 2/2) | -10.4 | 8.7 | success |
| 2 | Rust tests (3/4) | -25.1 | 11.5 | success |
| 2 | Rust test archive | -29.1 | 3.9 | success |
| 2 | Android | -9.6 | 0.1 | success |
| 2 | Tunnel (Go) | -29.1 | 0.7 | success |
| 2 | Rust (macos-latest) | -23.8 | 7.0 | success |
| 2 | iOS | -28.8 | 4.9 | success |
| 2 | Wire compatibility | -29.1 | 4.5 | success |
| 2 | Android UI tests (rest) | -24.2 | 9.3 | success |
| 2 | Rust native test archive (macOS) | -29.1 | 5.3 | success |
| 2 | iOS UI plan | -29.2 | 0.3 | success |
| 2 | Android release build | -22.9 | 13.2 | success |
| 2 | Swift (AgentKit + Mac tests) | -29.1 | 18.5 | success |
| 2 | Android unit tests | -29.2 | 6.2 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 2 | Relay | 0.0 | 1.0 | success |
| 2 | Site | 0.0 | 0.4 | success |

### 44237987362 (dd928aa49)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.0 | 0.2 | success |
| 1 | Wire compatibility | 0.0 | 4.7 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.0 | 12.0 | success |
| 1 | Rust test archive | 0.0 | 9.1 | success |
| 1 | iOS UI plan | 0.1 | 0.4 | success |
| 1 | Tunnel (Go) | 0.0 | 0.7 | success |
| 1 | Rust native test archive (macOS) | 0.0 | 3.1 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 7.4 | success |
| 1 | Android | 0.0 | 0.1 | success |
| 1 | Web | — | — | skipped |
| 1 | Android ${{ matrix.part }} | — | — | skipped |
| 1 | Android UI tests (${{ matrix.shard }}) | — | — | skipped |
| 1 | iOS | 0.1 | 7.9 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 7.1 | success |
| 1 | Rust (macos-latest) | 0.1 | 8.3 | success |
| 1 | iOS UI (smoke) | — | — | skipped |
| 1 | Rust tests (1/4) | 0.1 | 10.4 | success |
| 1 | Rust tests (2/4) | 0.0 | 9.7 | success |
| 1 | Rust tests (3/4) | 0.0 | 10.4 | success |
| 1 | Rust tests (4/4) | 0.1 | 11.0 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.0 | success |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 8.4 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 1 | Site | 0.0 | 0.4 | success |
| 1 | Relay | — | — | skipped |

### 44225800145 (c62e46f2c)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.2 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 6.8 | success |
| 1 | Wire compatibility | 0.1 | 5.0 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 45.4 | cancelled |
| 1 | Rust native test archive (macOS) | 0.1 | 9.1 | success |
| 1 | Rust test archive | 0.1 | 4.6 | success |
| 1 | Tunnel (Go) | 0.0 | 0.6 | success |
| 1 | Android release build | 3.3 | 13.4 | success |
| 1 | Android unit tests | 0.1 | 3.2 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Android UI tests (rest) | 4.9 | 10.1 | success |
| 1 | Web | 0.1 | 9.5 | success |
| 1 | Android UI tests (first) | 0.1 | 4.8 | success |
| 1 | iOS | 0.1 | 6.3 | success |
| 1 | Rust tests (3/4) | 0.1 | 11.3 | success |
| 1 | Rust tests (1/4) | 0.1 | 11.0 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.2 | success |
| 1 | Rust tests (2/4) | 0.1 | 11.4 | success |
| 1 | iOS UI (smoke) | 0.1 | 14.5 | success |
| 1 | Rust (macos-latest) | 0.1 | 7.8 | success |
| 1 | Rust (ubuntu-24.04) | 0.1 | 8.5 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.0 | 0.8 | success |
| 1 | Relay | 0.0 | 1.0 | success |
| 1 | Site | 0.0 | 0.5 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.2 | failure |
| 1 | Rust native tests (macOS 2/2) | 0.1 | 9.0 | success |
| 2 | Swift (AgentKit + Mac tests) | 0.1 | 12.9 | success |
| 2 | Swift (app build + bundle checks) | -56.3 | 6.8 | success |
| 2 | Rust tests (2/4) | -51.6 | 11.4 | success |
| 2 | iOS | -55.9 | 6.3 | success |
| 2 | Rust test archive | -56.2 | 4.6 | success |
| 2 | Rust tests (4/4) | -51.6 | 12.2 | success |
| 2 | Site | -39.4 | 0.5 | success |
| 2 | Android | -39.5 | 0.1 | success |
| 2 | Rust (ubuntu-24.04) | -47.1 | 8.5 | success |
| 2 | Rust tests (3/4) | -51.6 | 11.3 | success |
| 2 | Tunnel helper (Go) | -39.4 | 0.8 | success |
| 2 | Relay | -39.4 | 1.0 | success |
| 2 | Android UI tests (first) | -56.3 | 4.8 | success |
| 2 | iOS UI (smoke) | -49.5 | 14.5 | success |
| 2 | Rust tests (1/4) | -51.6 | 11.0 | success |
| 2 | Android release build | -53.1 | 13.4 | success |
| 2 | Rust native test archive (macOS) | -56.4 | 9.1 | success |
| 2 | Web | -56.4 | 9.5 | success |
| 2 | Android unit tests | -56.4 | 3.2 | success |
| 2 | Rust (macos-latest) | -47.2 | 7.8 | success |
| 2 | Android UI tests (rest) | -51.5 | 10.1 | success |
| 2 | Tunnel (Go) | -56.4 | 0.6 | success |
| 2 | iOS UI plan | -56.4 | 0.3 | success |
| 2 | Wire compatibility | -56.4 | 5.0 | success |
| 2 | Changes | -56.7 | 0.2 | success |
| 2 | Rust native tests (macOS 2/2) | 0.1 | 9.5 | success |
| 2 | Rust native tests (macOS 1/2) | 0.1 | 10.5 | success |

### 44221066329 (233feabd8)

| Attempt | Job | Queue | Run | Result |
|---:|---|---:|---:|---|
| 1 | Changes | 0.1 | 0.3 | success |
| 1 | iOS UI plan | 0.1 | 0.3 | success |
| 1 | Swift (app build + bundle checks) | 0.1 | 12.8 | success |
| 1 | Swift (AgentKit + Mac tests) | 0.1 | 15.6 | success |
| 1 | Android release build | 3.7 | 13.2 | success |
| 1 | Wire compatibility | 0.0 | 4.9 | success |
| 1 | Rust native test archive (macOS) | 0.1 | 9.6 | success |
| 1 | Android unit tests | 0.1 | 3.6 | success |
| 1 | Web | 0.0 | 13.5 | success |
| 1 | Tunnel (Go) | 0.1 | 0.7 | success |
| 1 | Android UI tests (first) | 0.1 | 3.8 | success |
| 1 | Rust test archive | 0.1 | 3.9 | success |
| 1 | Android UI tests (rest) | 3.9 | 7.5 | success |
| 1 | iOS | 0.1 | 8.3 | success |
| 1 | Rust tests (4/4) | 0.1 | 12.3 | success |
| 1 | Rust tests (2/4) | 0.1 | 13.2 | failure |
| 1 | Rust tests (1/4) | 0.1 | 12.0 | success |
| 1 | Rust tests (3/4) | 0.1 | 13.0 | success |
| 1 | iOS UI (smoke) | 0.1 | 9.3 | success |
| 1 | Rust (ubuntu-24.04) | 0.0 | 8.4 | success |
| 1 | Rust (macos-latest) | 0.1 | 9.3 | success |
| 1 | Rust native tests (macOS 1/2) | 0.1 | 9.7 | success |
| 1 | Rust native tests (macOS 2/2) | 0.0 | 7.9 | success |
| 1 | Android | 0.1 | 0.1 | success |
| 1 | Tunnel helper (Go) | 0.1 | 0.8 | success |
| 1 | Relay | 0.0 | 1.1 | success |
| 1 | Site | 0.0 | 0.4 | success |
| 2 | Changes | -25.9 | 0.3 | success |
| 2 | Rust tests (2/4) | 0.1 | 11.3 | success |
| 2 | Rust test archive | -25.6 | 3.9 | success |
| 2 | Tunnel (Go) | -25.6 | 0.7 | success |
| 2 | Rust (macos-latest) | -15.9 | 9.3 | success |
| 2 | Android unit tests | -25.6 | 3.6 | success |
| 2 | Rust (ubuntu-24.04) | -16.0 | 8.4 | success |
| 2 | Rust tests (3/4) | -21.6 | 13.0 | success |
| 2 | iOS UI (smoke) | -16.8 | 9.3 | success |
| 2 | Rust native tests (macOS 1/2) | -9.9 | 9.7 | success |
| 2 | Rust native tests (macOS 2/2) | -10.0 | 7.9 | success |
| 2 | iOS | -25.2 | 8.3 | success |
| 2 | Rust tests (1/4) | -21.6 | 12.0 | success |
| 2 | Rust tests (4/4) | -21.6 | 12.3 | success |
| 2 | Android | -8.7 | 0.1 | success |
| 2 | Rust native test archive (macOS) | -25.7 | 9.6 | success |
| 2 | Swift (app build + bundle checks) | -25.7 | 12.8 | success |
| 2 | Web | -25.7 | 13.5 | success |
| 2 | Android release build | -22.1 | 13.2 | success |
| 2 | Android UI tests (rest) | -21.8 | 7.5 | success |
| 2 | Android UI tests (first) | -25.6 | 3.8 | success |
| 2 | Swift (AgentKit + Mac tests) | -25.7 | 15.6 | success |
| 2 | iOS UI plan | -25.8 | 0.3 | success |
| 2 | Wire compatibility | -25.8 | 4.9 | success |
| 2 | Tunnel helper (Go) | 0.1 | 0.6 | success |
| 2 | Relay | 0.1 | 1.0 | success |
| 2 | Site | 0.1 | 0.4 | success |

## Findings and limits

* Run 44609213426 (729b7450e) spent 40.3 minutes from workflow creation to last update; Swift tests ran 30.0 minutes. Its job log records misses for Rust, SwiftPM downloads, Swift compilation, and tmux caches. Native test targets took 9.2 minutes, AgentKit 4.8, Mac tests 12.3, and tmux installation 0.9. Cache restore steps themselves took seconds on this miss. The cache misses explain a cold build contributing to the variance; the log does not establish why all entries were absent, or quantify every source of the Mac test slowdown.
* In run 44484977088, the same Swift job ran 11.6 minutes: native targets 1.8, AgentKit 1.3 and Mac tests 5.5. Comparing steps across runs establishes where time changed, but is not a controlled experiment on cache state or runner performance.
* Run 44609213426 also shows Android release waiting behind the unit-test matrix member: matrix `max-parallel: 1` serializes them. Relay and Site depend on both Rust tests and Android. The tables include each job’s own queue, but the dependency delay must be read from job timestamps rather than added to that queue.
* Reruns are material: the elapsed table includes manual or automatic delay between attempts; jobs are listed once per job ID, including failed and cancelled attempts. A rerun of failed jobs need not rerun successful Swift jobs.
* GitHub-hosted runner/cache availability is outside this branch’s control. The cold-cache example alone does not prove it is the only external limit, and the baseline does not establish five consecutive runs below 15 minutes. No five post-change Swift runs exist here, so a 90th percentile below 15 minutes cannot be claimed. Do not unblock goal 2 based on this note.

The data above is observational. No required checks or test coverage were changed. A follow-up should collect cache hit/miss and build/test timing on five comparable post-change main runs, including cold-cache behavior, and verify both the Swift percentile and push-to-last-job time before declaring the target met.
