# ov-926 source handoff

Baseline: exact reviewed72ccadf5c9987b851972780409fcccc9cf6edc37
(shared presentation source791345ee8). Signed source: `db43f6393aa4c480b4e12a4365e9302819209fcb`.
Exact changed-path hashes and signature verification are in
`source-sha256.json` and `source-signature.log`. This evidence commit updates
api.md ownership to927 per manager decisions; source manifest hashes describe
exactdb43 source, while evidence manifest hashes include the corrected API.

## Implemented finite shell surface

Mac/iOS share `WorkflowRuns.swift` + `WorkflowRunsView.swift` in AgentKit;
iOS's canonical generator includes both files. Existing workspace toolbars open
a standard sheet with a selected run retained per runner/workspace. Android's
phone AND wide workspace toolbar open the Compose dialog, saving visibility
and selected item via existing saveable/preferences mechanisms. Web's current
workspace has an additive dialog button, HTML dialog focus/escape semantics,
local selection memory and existing reconnect callbacks. Terminal/chat panes
and their routes/mounting are untouched. No old feature/authority was removed.

All four clients fill finite slots from Rust's label/phase/stage/progression/
gate/reason/action strings. They show immutable run/definition/policy/candidate,
retained evidence candidate/manifest/receipt/policy, pending uncertain effect,
fence/attempt/iteration and rework budget identities. Semantic IDs are not
native Reason/Action enums and never choose copy, permission or availability.
The supplied action list is the only source of buttons. Local readiness/busy
flags concern transport/view mechanics only. No native reducer, Grant, command
mapping, direct store access, ACK or runtime starts were introduced.

Feature A's EvidenceInvalidated + InspectLatestEvidence passes through the
same slots, including retained invalidated evidence, without a feature branch
in any renderer. Inspection results use selectable text; no remote artifact
fetch is claimed. Changing selection/disconnect clears inspection attribution.

## Pending authenticated integration

Read [api.md](api.md) for exact consumer methods/injection points. ov-925 owns
protocol/capabilities/daemon/CLI. Manager decisions c424de3e/504b425b assign
actual Session/ONE Rust Intent-to-protocol adapter to ov-927, after corrected925
security review. Read its concrete proposed (not implemented) contract in the
925 tree at reports/ov-925/session-contract.md. The Session adapter was not
available at this source handoff. Default ports are explicitly absent and the real workspace
entry shows that runs are not connected. No invented RPC or caller-trusted
Capabilities/Grant bridge fills that gap.

Host integration must resolve CURRENT authenticated Session, independently
source capabilities, re-read latest State and validate the entire opaque
Action guard before authorized execution. Swift session id rotates/nil on
reconnect/disconnect; Kotlin port is replaced/nil; Web resolves current port
and calls the sheet's connection hook. UI cancellation must not cancel accepted
runtime work or discard stable command identity/durable reply retry in Session.
Display controllers never automatically repeat a mutation after a lost reply.

## Evidence and limits

All compiler/test commands are under normal heavy FIFO, native Python,
jobs <=6, 180-second owned command timeouts and >=10GiB reserve. Work product
is ~274MiB temporary focused outputs; no dependency downloads, full-platform
rebuilds, display activity, simulator/device/app launch or owner data access.
See [check-plan.md](check-plan.md) for the corrected SDK/cache provenance.

* Web: strict TS5.9.3 compile of new modules; **10** Node fixture/controller/
  simulated-DOM cases pass against actual five Rust golden responses. Tests
  cover shared copy, retained evidence, exact descriptor/object forwarding,
  long/hostile text as text, lossless u64 identity, absent/changed actions,
  delayed read after disconnect, reconnect/selection restoration and a lost
  action reply without automatic duplicate mutation.
* Swift: Swift6.4/SDK27 compiles shared model AND SwiftUI view into a focused
  executable (macOS26 target). **Five** golden cases plus pending-port,
  delayed disconnected read and long opaque descriptor checks pass. Normal
  AgentKit test source has a separate focused typecheck receipt; full AgentKit
  suite/application linking and iOS target build are not claimed.
* Android: cached Kotlin2.4.10 compiles production model + test; JUnit **3**
  tests pass across the golden cases, disconnected delayed read, pending/wrong
  workspace and lossless u64/long copy. Cached Compose compiler + AndroidX
  classes + installed Android37 jar compile only the two new production files.
  This is not a Gradle app/JNI build, Robolectric layout capture or actual device.
* Canonical iOS generator succeeds and generated source membership is checked.
  Generated `.pbxproj` remains ignored as the repository requires.
* Full Web app typecheck has the SAME **8** diagnostics at exact72cc and current:
  absent local QR/xterm packages and consequential terminal parameter types.
  New modules pass; full app typecheck is **not** green. No install/download.
* File-size budget, doc-comment diff and diff whitespace checks pass. Attempt
  logs retain initial sandbox/Testing-classpath/Kotlin-classpath setup failures,
  corrected by bounded checks; they are not relabelled product regressions.

No real browser narrow layout/screenshot, native input/IME, accessibility,
actual workspace navigation or live reconnect/runtime/ACK behavior was run.
Source shows uncapped wrapping/scrollable finite slots; that is not graphical
proof. Ordinary renderer source changes total six new production files (two
Swift shared by both Apple shells, two Kotlin, two TS) plus seven entry/style/
generator files; **zero shell feature-specific policy/copy switches**.

## Acceptance disposition / review request

1. Finite all-four renderer/model source and shared-copy fixtures implemented;
   actual authenticated data integration/device behavior remains pending.
2. Additive navigation seams and opaque dispatch hooks implemented; actual
   execution explicitly pending reviewed927 Session adapter after925 review as brief permits.
3. Feature A/controller/golden/long-content/data tests implemented; real narrow
   layout/input/accessibility/reconnect/navigation proof remains parent work.
4. Signed source/API/evidence pins supplied; request independent scoped source
   review of opaque action fidelity, stale/disconnected fencing, all-four
   entry-point wiring and lifecycle isolation. Review has NOT yet been received.

Stop at this source pin. Full release acceptance remains ov-921 until transport,
independent review, actual platform proof and the later cutover gates close.
