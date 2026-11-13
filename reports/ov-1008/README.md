# ov-1008: UICollectionView transcript spike

This directory is an experiment, excluded from the production project. `prepare.py`
copies the Apple sources to a fresh directory, injects `CollectionTranscript.swift`
there, gives the app the private identifier `com.farcooler.spike.ov1008`, and adds a
launch-argument switch. No production source, source list, entitlement, or app
route changes in this branch.

## Reproduce

Build frameworks from this checkout first (`./scripts/build-ios-frameworks.sh`).
Use a dedicated, booted iPhone simulator and substitute its UDID below:

```sh
python3 reports/ov-1008/prepare.py /tmp/ov1008-repro --frameworks "$PWD/apps/ios/Frameworks"
xcodebuild build -project /tmp/ov1008-repro/apps/ios/FarCooler.xcodeproj \
  -scheme FarCooler -configuration Release \
  -destination 'platform=iOS Simulator,id=UDID' \
  -derivedDataPath /tmp/ov1008-repro-dd CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES ARCHS=arm64
python3 reports/ov-1008/run.py UDID \
  /tmp/ov1008-repro-dd/Build/Products/Release-iphonesimulator/FarCooler.app \
  /tmp/ov1008-repro-results.json
```

The same optimized binary runs both variants. `DEBUG` is defined in the copied
Release project so the existing runnerless harness and frame recorder are
available; optimization remains `-O`. This is deliberately **not** a shipping
Release configuration. Both modes include the same debug probes.

For the separate functional test (never while timing):

```sh
xcodebuild test -project /tmp/ov1008-repro/apps/ios/FarCooler.xcodeproj \
  -scheme FarCooler -configuration Release \
  -destination 'platform=iOS Simulator,id=UDID' \
  -derivedDataPath /tmp/ov1008-repro-dd CODE_SIGNING_ALLOWED=NO ONLY_ACTIVE_ARCH=YES ARCHS=arm64 \
  -parallel-testing-enabled NO -only-testing:FarCoolerUITests/TranscriptSpikeUITests
```

## What is compared

The SwiftUI side retains the `NativeAgentView.conversation` body with the
measurement-only reader-scrolling notification described below. The UIKit
side uses a compositional list, self-sizing `UICollectionViewListCell`s,
`UIHostingConfiguration`, and diffable snapshots keyed by `ConversationItem.id`.
Both use `NativeRowView`, `NativeToolGroupRow`, the real observable row boxes,
markdown, folded report views, range-selectable text, and the real composer. The
fixture, decoding, store and streaming loop are the existing `NativeLiveRows` /
`RealSession` harness, including the long-report flag. For the measured comparison,
the disposable copy preloads all 400 rows in both modes and disables older-page
availability. This isolates scrolling from different paging/anchor policies.
The collection has a head-page request hook, but paging is not measured here. Group membership changes
reconfigure the surviving group cell; changes within a row use observation.

Timing reuses `FrameWatch`: 16 × 1.2-second flings, 5,000 pt/s starting velocity,
0.998/ms decay, eight away from the initial end and eight back, driven and timed
by the same CADisplayLink. At start, a local notification explicitly unpins the
SwiftUI transcript and releases its place hold, and the common store enters its
scrolling state; finish releases that state. Without this, direct `contentOffset`
changes do not mimic a finger and incoming rows keep pulling SwiftUI to the tail.
Each new process settles for five seconds. The runner
makes no XCUI query during timing. It alternates S/U/U/S/S/U to reduce a simple
warmup/order bias and saves each raw report. Added fields identify the actual
scroll container, row counts and script-driven distance, to expose an empty,
wrong, or stationary comparison.

This isolates the container with production row rendering; it does not implement
all the scroll policy of the production SwiftUI transcript. In particular, it
is not eligible to ship merely because it builds or times better.

## Migration cost and risk

A container swap leaves the row renderers intact, but replaces most of the
transcript controller in `NativeAgentView.swift`. Budget approximately **one to
two engineer-weeks** for a production candidate and **another week of device,
accessibility and scroll-regression QA**, subject to the measured result below.
This is an engineering estimate, not measured implementation time.

| Area | Prototype behavior and production work |
| --- | --- |
| Selection | Uses the existing `SelectableText` renderer and enables range selection. A selected UITextView can be recycled offscreen; selection/menu lifetime, gesture arbitration with collection scrolling, Copy, VoiceOver and cross-row behavior need device checks. Hosting configuration alone does not preserve a selection across reuse. |
| Folding | Uses the same tool groups and folded report views. Row-local `@State` is tied to a hosted cell's lifetime. Move disclosure state to a dictionary keyed by stable row/group id, explicitly reset reuse identity, and test expansion, scrolling offscreen/back, group merges and self-sizing invalidation. Disclosure heights can cause another layout/commit hitch. |
| Live updates | Existing row-box observation renders changed text; diffable snapshots insert/remove/regroup items. The prototype does not reproduce the store's drag/deceleration deferral hooks, pinned-tail reanchoring or pending-send settlement. Snapshot application and changed hosted sizes need coordinated anchors; never reload a selected cell indiscriminately. |
| Older pages | The head requests the same source's older page. There is no equivalent of `PlaceHold`'s measured visible-row anchor/correction ladder. Estimated cell heights and prepending can shift the reader. |
| Scroll and keyboard | Initial placement at the tail only. Production needs follow/unfollow intent, send-to-tail, Jump to Latest, short-session bottom alignment, safe-area/edge effects, composer growth, keyboard/split-window changes and background cancellation. The existing SwiftUI `.scrollPosition`/geometry/phase modifiers cannot control a UICollectionView. |
| Row actions | Normal tool and report buttons are reused. This prototype supplies no ask/send-now actions or open-subagent environment to hosted cells; production must explicitly propagate these and the correct screen environment. No claim of action parity. |
| Runtime cost | SwiftUI graph/layout and text allocation still occur inside every hosted row; UIKit removes the outer lazy-stack graph, not markdown/TextKit/Core Animation. Diffable item lookup is linear in this spike; use an item dictionary at scale. Track memory and allocations as well as time before committing to a rewrite. |

Keep the existing conversation selection, folding, live-update, paging, keyboard,
Dynamic Type and VoiceOver regression tests as migration gates. Include physical
60/120 Hz iPhones and a cold and warm pass. A simulator result alone cannot prove
that the motivating 26–40 ms device stalls are gone.

## Measured result and recommendation

**Keep the SwiftUI transcript. Do not pursue a production migration based on this
prototype.** The collection prototype is slower in every measured run. This does
not rule out a more specialized UIKit renderer; it rejects the assumption that
putting the existing SwiftUI rows in a collection is itself a performance win.

Measured 2026-11-14, base `6afab149ab10a5548dc6033537914945cbd93c0b`, Xcode 27.1
(27A9275), simulator SDK 27.1, Apple M5 (Mac17,2). The dedicated device was an
iPhone 17 simulator running iOS 26.5, with a 16.7 ms measured display slot.
[Raw reports and environment](results.json) preserve the actual execution order.
No compilation or UI test ran concurrently with the final timing batch; other
host/simulator activity was not controlled. Three samples per mode do not support
statistical significance claims.

| Mode / sample | Hitches | Over one frame | Worst late ms | Late ms/s | Text views made | Script distance pt |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| SwiftUI 1 | 38 | 27 | 40.4 | 42.2 | 46 | 35,697 |
| SwiftUI 2 | 35 | 27 | 34.2 | 40.5 | 57 | 35,680 |
| SwiftUI 3 | 46 | 29 | 38.0 | 50.6 | 59 | 35,687 |
| UIKit 1 | 83 | 59 | 106.6 | 164.8 | 20 | 35,160 |
| UIKit 2 | 71 | 50 | 105.6 | 143.4 | 12 | 35,264 |
| UIKit 3 | 75 | 58 | 100.9 | 154.5 | 22 | 35,234 |

Median late time is **42.2 → 154.5 ms/s (3.66×, 266% worse)**; median worst hitch
is **38.0 → 105.6 ms (2.78×)**. Median text-view creation falls **57 → 20**,
so reduced allocation alone does not translate into smoother frames. All runs
moved roughly 35,000 points in 19.3 seconds. UIKit's median distance is 1.3% less;
this is the same time-driven script, not exactly the same visible pixels, because
missed frames and container layout affect its integration and viewport.

Both modes start with the same 400 fixture rows plus 3–4 timed live arrivals and
finish with 421–422 rows / 301–302 grouped items. The one-row difference is the
fixture's wall-clock follow scheduling, not different source text. This is a
static corpus plus live-update comparison; it makes no claim about prepending
older pages or a user's real pan/deceleration gesture. The recorder moves offsets
programmatically and measures missed display slots, not Instruments' full
render-server animation-hitch pipeline. No CPU profile was collected, so the
regression cannot be attributed specifically to self-sizing, hosting graph work,
or Core Animation commits. The prototype also reads `store.items.last` in each
hosted row to choose streaming rendering; an append can reevaluate visible hosted
rows. A tuned version should pass this flag explicitly and reconfigure only the
old/new last cells. This is a measured implementation choice, not a proof that all
UICollectionView designs are slower.

The original 26–40 ms issue remains worth improving, but this result offers no
positive win to justify the estimated two to three weeks of migration and QA.
If revisited, first profile a physical iPhone Release build of this isolated
prototype and test cached measured heights / targeted row invalidation. Proceed
to a migration only after a repeatable device win with selection, disclosure and
live-update behavior retained. Nothing from this spike ships.

## Validation

- Matching Rust simulator frameworks built successfully from this checkout.
- The isolated app built in Release (`-O`, DEBUG fixtures enabled).
- All six timing reports passed the container, ≥400 initial rows, ≥30,000 points
  of movement, and ≥19 seconds of measurement checks.
- `TranscriptSpikeUITests.testCollectionStreamsAndDisclosesTools` passed in
  Release: **1 executed, 0 failures, no skips**. It observes a changed collection
  snapshot after live arrivals, scrolls to a production tool group, opens it, and
  verifies the production child tool row appears. This is a functional smoke
  check, not selection, offscreen disclosure persistence or full scroll parity.
- Python compile checks and the repository file-size budget passed. The production
  generator contains neither spike Swift file. Only this report directory is
  committed; the source project and production app are unchanged.

The UI test uses the disposable project's test scheme directly; the repository's
standard UI runner targets the production checkout. Executed test counts were
checked explicitly so a zero-test or skipped-only run cannot count as validation.
The initial locked launch, the uncorrected scroll-policy diagnostics, and failed
scheme/universal-architecture test attempts are excluded from the timing data.
