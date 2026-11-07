# ov-932 source-only rebind preparation

Prepared 2026-11-07. This package implements the latest task phase, not the full runtime acceptance. No build, test, toolchain/capability probe, network fetch, application execution, profile access or launch was performed. Only source/Git/known evidence inspection and board coordination occurred. Final reviewed ov-936 Store/bootstrap composition and a concrete phase allocation remain prerequisites.

## Exact composition

Dedicated branch: `ov-932-runtime-verification-prep`. Start: `4c645bb788ec320fb19704ed042507adeccfd911`. Four normal clean picks, no conflicts:

| Approved original | Local replay |
| --- | --- |
| e1203d8c6bbeb4bf7eaecb51a08fd0ab2c6884e2 | 89b0ffea086752714a34ee8995b4c782d743e2b8 |
| f4020a91daeda03219dbcfeda2ff112e62a20764 | f1ae3a41b6ea9f2092b9c57e6e3c8887a3975745 |
| 58734b6792ea7883962c7f142efca711382325f6 | 059ed92c5bcb227c232e0fb4a609165cc7f89d1b |
| 264a7670d4149fbc54bb7cff3af385b2906e8e8a | 6949bcc33cb7ab432131923ce9f680db4e7e252d |

No manual product edits, Rust changes beyond those exact approved picks, or changes to old reports. Pre-existing untracked `.codex/` and `.cursor/` remain untouched. `source-sha256.txt` binds inspected source; `composition.txt` binds ancestry and product diff. Replay identities are not new independent review or current build evidence.

Historical compile/resource/tool inventories in `../inventory.json`, `../build-plan.md`, Mac/Android/Web logs and artifact manifests retain their original source/time meaning. They do not establish current tool availability, free space, output presence, build compatibility or current native identity. Historical sandbox failures and FIFO history remain at their original refs, including `8ff5524e2969532ef84a2b3749c0e264652f2408:reports/ov-932/launch-postbaseline-outcome.md` and `427ff8da9:reports/ov-932/runtime-root-source-audit.md`; no copies are rewritten as success. No more Python sandbox retries or speculative permission expansion.

## Normal CLI/main route and remaining effects

`crates/cli/src/main.rs:1070` captures validated runtime selection before normal dispatch. `paths.rs:55` refuses malformed explicit roots; selection caches success/failure. Validation computes paths, but does not prove UID, symlink safety or ownership. `daemon_link.rs:92/165/217` resolves the selected socket; `:394` honors exact `FARCOOLERD_BIN`, otherwise sibling/channel search applies. `--json daemon ensure` uses `ensure_local` and may start or replace a daemon; it is not a passive read. Never run it until socket/peer identity belongs to this phase.

`crates/daemon/src/main.rs:36` selects before kept logging; normal `run` ensures socket directory, probes existing socket and acquires lock before sshd offer, Service startup, workflows, recovery/migrations, tunnel and Watcher/socket work. `runtime_selection.rs:6` calls `open_in(selected.ensure_dir())`; explicit selection retains root-local authorized keys, agent home, Claude paths and login state. Installed selection preserves existing OS-user defaults. Real sibling `rep_cli` is restored for both. Private `log_roots/log_home` exclude ambient HOME/CODEX_HOME. `Service::open_in` still creates install identity, opens/migrates Store, refreshes tmux inventory, loads registry and persisted Sessions. Thus this composition alone does not satisfy pre-Store cutover/admission: see `reports/ov-937/startup-handoff.md`. `--stream` and standalone `--stdio` also open Service and must be covered by final936; a normal-run-only check is insufficient.

Mandatory future private absolute `FARCOOLER_CONFIG` and `CLAUDE_CONFIG_DIR` remain. Explicit runtime selection is not an all-effects sandbox. Private root/keys/agent state do not authorize enrollment, new pairing, provider launch, external push, default profile access or changing the real-home read-only-folder ceiling. Keep HOME/CODEX_HOME unchanged. Config, tmux/sshd child, shell/login PATH discovery, tunnel/rendezvous and background tasks require exact source-backed dispositions in the final launch package. Do not infer those dispositions from the runtime-root fix.

## Real application navigation and shared bridge

| Existing root and current connection | Actual call route and journal |
| --- | --- |
| Mac workspace detail `ContentView+WorkspaceDetail.swift:102-103` → WorkflowRunsButton/port; `NativeAgents.swift:47-64` resolves `links[target]` on each call and guards connected current core; host established from daemon hello. | `RunnerCore.swift:69` derives local socket from AppControl; `:124` → `farcooler_client_call` → `crates/client/src/ffi.rs:1588` → shared Session workflow call. Shared Apple journal is Application Support / bundle identifier / WorkflowIntents. |
| iOS `WorkspaceScreen.swift:77-78` → port; `ConnectionWorkflow.swift:6` requires answering requested host and actual daemon runnerId, calling current `self.core`. | `ClientCore.swift:182` → same C FFI/Session; same Apple journal logic, under its actual app container. A Mac compile cannot prove this binary or iOS lifecycle. |
| Android `ui/WorkspaceScreen.kt:741-749` → ANSWERING port; `ui/AppModel.kt:76-89` resolves fleet connection/current core each invocation with daemon runnerId and account/client identity. | `core/ClientCore.kt` → NativeClient → `crates/android/src/lib.rs:139` → real C FFI/Session; `filesDir/workflow-intents` journal. Both current JNI and Go tunnel libraries must be packaged and loaded. |
| Web `services/web/src/client/app.ts:1029-1032` workspace dialog; `:688` uses `reconnector.current`; `:730` installs only selected target/current port. | `workflow-session.ts` → `WebRunner.workflow_call` → `crates/web/src/js.rs:320` → `wire.rs:177` → shared Session. Origin localStorage and navigator locks retain intents. Existing root dials Tailcat/SSH with device/account/key state; an arbitrary HTTP fixture cannot replace it. |

Apple `WorkflowSession.swift:103/134/144/152/164` forwards snapshot, complete action JSON with observed revision/epoch, stores opaque prepared object, reloads retained intent and resolves fresh submit core. Android `WorkflowSession.kt:61/90/101/108/122` and Web `workflow-session.ts` implement corresponding read/prepare/persist/retry paths. Preparing-without-reply requires refreshed inspection instead of guessed submit. Receipt/result display is generic. Shared Rust `workflow_presentation.rs` and `workflow_presentation/display.rs` own EvidenceInvalidated and InspectLatestEvidence semantics; native/Web shells must not map domain commands/reasons independently.

Current CLI publication `crates/cli/src/workflow.rs:29-41` uses the closed `{definition, adapter?, notifications?}` envelope, with `coding/v1` adapter validation and definition-only support; historical early931 description omits notifications. Final936 composition must be rebound as a whole, preserving current policy/definition pins and shared presentation schema2, not adapted piecemeal per platform.

## Authentication and private-profile gaps

Mac local Unix connection uses real Session socket handshake. Remote/mobile/Web paths require existing supported identity/host trust and credentials; these are not supplied by this report. `crates/client/src/session/workflow/tests/socket.rs:15-31` creates an isolated Service/Runtime and injects `Peer { client_id: phone, scope: Control }` with workflow fixture handle. It is a truthful fixture peer, not real enrollment/production authority. Its direct Session dispatch tests cannot prove native root navigation, real C-FFI callbacks, app journal lifecycle or production callers. Reusing it for future app testing requires a concrete supported root/auth route and a separately scoped fixture allocation; none is claimed here.

Mac `ContentView.swift:350-377` hooks device registration/sign-in and calls LocalDaemon.ensure; NativeAgents initialization prunes draft/image storage. `CLI.swift:23-54` selects bundled CLI ahead of FARCOOLER_BIN and inherits process environment, replacing PATH after login-shell discovery. Therefore FARCOOLER_BIN alone cannot select a bundle's CLI. Bundle resource CLI/daemon must both match final source. `FARCOOLER_REGISTER_APP=0` only suppresses Launch Services registration during private bundle assembly, not app runtime effects. Shared Account uses UserDefaults.standard and channel keychain/token migration; workflow journal uses Bundle identity. A renamed bundle or FARCOOLER_HOME alone does not prove private account/keychain/defaults/draft/profile ownership. Smallest missing Mac prerequisite is a source-backed supported private app-profile/account/credential route with bounded startup effects; do not invent a generic framework or launch to discover it.

Android runner identity/preferences are application-context SharedPreferences (`data/Identity.kt`, `RunnerStore.kt`); isolated package/container plus supported pre-existing test auth is required. iOS needs an owned simulator/container and supported credentials without owner device/keychain access. Web needs an owned origin/profile, current compiled WASM, supported device credentials and a private reachable real peer; injected workflow-only port is insufficient. Where normal auth cannot be supplied within constraints, report the exact gap and stop that platform phase.

## Evidence required before acceptance

All full current app builds, real root inputs/captures and runtime receipt rows are OPEN. No current FeatureA/FeatureB, lost-reply, disconnect, AX or restoration pass is claimed. See `next-phase.md` for staged proposals. The source-only package is ready for the same ports review; runtime work remains held until orchestrator supplies final reviewed936 and a concrete allocation. No release/full-cutover claim.
