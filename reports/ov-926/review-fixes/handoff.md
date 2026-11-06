# ov-926 focused review corrections

Source pin: **32a00811271f4b56cda5b0592dcb32759dad932f**, signed.
Base: **26b1665acad8d20e9cf4027cf0eedfab66716985** (source db43f6393).
Branch: `ov-926-renderer-review-fixes`.
Review request: the same independent reviewer audits only the nine-path source
correction against 26b. Original review:
`/Users/e-liang/Dev/overnight/.claude/agent/reports/ov-926/independent-review.md`.
Original reports/source/evidence remain unchanged. Product source is frozen.

## Findings and fixes

The three findings were confirmed against the exact source; no disagreement
with the review's lifecycle/focus analysis was found.

* R1: connection/read generation did not change on selection. A delayed reply
  could repopulate the common inspection slot after selecting another run.
  AgentKit `WorkflowRuns.swift:105,107,139-147`, Kotlin
  `WorkflowRuns.kt:49-50,84-91`, and Web `workflow.ts:30,34,75-84` now capture a
  separate display selection token plus item/run identity. Selection clears
  increment that token. Swift's actual selection buttons clear synchronously
  (`WorkflowRunsView.swift:41,47`), as Android's existing callback already did.
  A refreshed replacement/removal of any previously displayed item/run pair
  conservatively clears retained inspection on every shell. Host/workspace
  remain immutable model/board scope. Display invalidation does not change the
  action/read generation: busy cleanup and post-action refresh still run, and
  no accepted work is cancelled or replayed by selection.
* R2: redraw removed the focused control, and text-based recovery could not
  restore absent/disabled controls. Web `workflow-view.ts:9-43,61` now uses
  structural control keys: refresh/navigation keys, item identity for rows,
  item/run/kind/guard identity for supplied actions. Shared labels never choose
  focus identity. An unavailable control falls back to enabled All Runs or the
  first enabled control. A disabled action retains its intended key across
  redraws and restores it when ready; a move to another control replaces that
  intention. Focus outside the content, including dialog Close, is preserved.
* R3: an old failed-action caller wrote its notice after an awaited read that
  had already discarded its stale result. Each refresh now returns its current
  read ticket or nil/null; the action caller checks that returned ticket AGAIN
  against the current generation after its await. Disconnect/newer-session
  notices cannot be overwritten by the old caller. This is controller-local
  bookkeeping, not a transport/authentication token or API change to the port.

The opaque action descriptor path, shared Rust copy/policy, schema 2, navigation
mounts, Session integration ownership and engine/ACK boundaries are unchanged.
No proto/capability/daemon/CLI/Rust files were edited.

## Fresh checks and failing-before controls

Reproduction command, using native Python throughout:

```
/usr/bin/python3 scripts/heavy -- /usr/bin/python3 reports/ov-926/review-fixes/run-checks.py suite
```

All entries below completed through the normal shared heavy FIFO, load5 <20,
compiler jobs <=6, no dependency downloads, 180-second owned compiler/test
limits (60 seconds for headless browser), >=10GiB disk reserve. Matching
installed Swift6.4/SDK27 prebuilt modules, cached Kotlin2.4.10/AndroidX/Android37
and TS5.9.3 were reused. Owned temporary work is about 171MiB; about 37GiB disk
remained. No full native app build, Gradle resolution, simulator or device.

| Check | Result / saved receipt |
| --- | --- |
| Strict new Web modules | exit 0, `web-typecheck.log` |
| Web fixtures/controllers/simulated DOM | **15 pass**, `web-fixtures.log` |
| Swift production model + SwiftUI + harness | exit 0, `swift-compile.log` |
| Swift five shared goldens, each with lifecycle regressions | pass, `swift-fixtures.log` |
| Focused Swift test module + test-source typecheck | exit 0, `swift-test-{module,typecheck}.log` |
| Kotlin model + JUnit | **7 pass**, `kotlin-{compile,fixtures}.log` |
| Two-file Compose compilation | exit 0, `kotlin-compose.log` |
| Actual cached headless Chromium keyboard/focus | six pass statements, `browser-focus.log` |
| Whitespace / doc comments / file-size budget | pass, corresponding logs |

Deterministic delayed continuations/promises cover selection changes including
return to the original selection, same-item run replacement during ordinary
and post-action refresh, delayed action completion after disconnect/reconnect,
and an old failure waiting on its read when disconnect/new-session refresh wins.
They assert busy cleanup and required refresh, and no automatic action replay.
The Swift lifecycle helper executes in the standalone harness and is also
called by the normal Swift Testing test source. Full AgentKit tests were not run.

Negative controls compile original db43 production files into owned temporary
outputs while running the NEW tests. Expected nonzero exits are asserted by
the runner, so an accidentally passing negative control fails the suite:

* Web: four isolated failures for selection, run replacement, keyboard run-entry
  focus, and stale failure notice (`negative-r*.log`). The focus control fails
  on actual missing fallback focus, rather than a test dependency on new keys.
* Swift: compilation succeeds, lifecycle helper reports `staleDisplay`, exit 1
  (`negative-swift-{compile,fixtures}.log`). It stops at the first new failure.
* Kotlin: compilation succeeds; all seven tests run and exactly three new
  selection/replacement/stale-failure regressions fail, exit 1
  (`negative-kotlin-{compile,fixtures}.log`).

Browser fixture uses the cached Chromium headless shell1248 with a fresh owned
profile, localhost-only HTTP server, disabled background network and external
DNS, and built-in CDP control. It runs the compiled production renderer/model
with Rust golden fixture DTOs and a delayed fake port. Real Enter key events
verify run entry, busy fallback, action restoration; additional checks cover
colliding/changed labels, failed read/recovery, action removal and preserving
Close focus. Browser/profile/server are closed/reaped. It does not open a
visible window or attach to an existing browser. The initial CDP harness omitted
Enter's carriage-return text and failed to activate the button; that setup
attempt is retained in `browser-key-event-attempt.log`. Corrected input produces
the saved green receipt without a product change.

Initial signed commit was stopped by an incompatible `/usr/local/bin/python3`
selected by the repository hook. Repeating the normal signed commit with
`/usr/bin` first in PATH ran the hook successfully. No hook/signing bypass.
`source-signature.log` verifies the source pin; the key-certification trust
warning is preserved. `source-sha256.json` covers exactly the nine source/test
paths; `evidence-sha256.json` covers this handoff, runner/harness and raw logs.

## Acceptance and remaining work

Scoped correction status: **DONE_WITH_CONCERNS**, pending independent review.
Request the same reviewer to verify R1/R2/R3 and the saved proof limits. No new
owner decision is needed. Mark ov-926 in_review at this source/evidence handoff.

This does not close parent release acceptance: actual authenticated Session
adapter attachment belongs to ov-927 after ov-925 security closure. The port
still defaults unavailable. Actual workspace/application navigation, native
keyboard/IME/VoiceOver/TalkBack, scene restoration, narrow graphical layouts,
live reconnect/runtime/ACK independence, full app builds and parent integration
checks remain open. The inherited full-Web baseline diagnostics were not rerun
or declared green; focused new modules pass. This headless fixture provides real
browser focus evidence only, not real application or authenticated execution.
