# ov-1096: iPad chat after rotation

Status: diagnostic change; neither acceptance item is complete yet.

## Evidence

- Read ov-1096 and ov-1101's intent through the Canary CLI.
- [Nov 10 nightly](https://github.com/taneliang/farcooler/actions/runs/42877991253):
  `iOS UI (pad)` selected iPad Pro 11-inch (M5), **iOS 27.2**. After rotation,
  the log reported the header hittable and the `pad-chat` frame at
  `(375.5, 97.75, 476.0, 1112.25)` in an `(834, 1210)` window. Its right edge
  is 17.5 points outside the window. The test had already observed `twoColumns`.
  The log contains no child-frame dump at this failure.
- The retained `ios-ui-xcresult-pad` recording shows the header, Agents card,
  and terminal failure state still visible in portrait. It does not establish
  which child enlarged the measured frame. The failure state's transcript is
  `not in the harness`.
- [Nov 11 nightly](https://github.com/taneliang/farcooler/actions/runs/43080833359):
  this method failed at `PhoneHarnessLaunch.swift:61`, during `launch()`, with
  `Application 'com.farcooler.ios.local' does not have a process ID`. It never
  reached the rotation check. This is separate evidence from the layout failure.
- The rotation test passes locally on iPad Pro 11-inch (M5), **iOS 27.0**.
  This machine does not have CI's iOS 27.2 runtime installed.

## Change

When `chatIsShown` detects a bad frame, it prints the OS version and the chat's
accessibility subtree, including each child's identifier, label and frame.
The same report is kept as a `Chat Layout` xcresult attachment. This implements
ov-703's requested next diagnostic step without weakening the containment
assertion, hiding overflow, adding a retry, or changing an unproven layout cause.

## Remaining acceptance work

1. Read the next failing CI run's `CHAT children` output and name the child
   responsible for the extra width. Compare its frame with the column and window;
   distinguish an actual layout overflow from an accessibility frame overflow.
2. Reproduce on CI's runtime, fix the identified cause, and verify the next
   nightly pad shard passes. A green local iOS 27.0 run does not prove this.

The Nov 11 launch failure has no evidence here supporting a launch-harness fix.

## Validation

- Built simulator xcframeworks from this checkout with
  `scripts/build-ios-frameworks.sh`; the pre-existing cached frameworks lacked
  `farcooler_client_plan_view` and `farcooler_client_watch_start`.
- Temporarily added `window.width > 1000` to the visibility condition. The
  landscape check passed and the portrait check failed, printing the full
  subtree with `pad-chat-header`, `agents-card`, `orchestrator-pane`, terminal
  failure text, and mount probes. The `Chat Layout` text attachment was exported
  from that failed run's xcresult and inspected. Removed the temporary condition.
- `git diff --check` and `scripts/file-size-budget.py` pass.
- Final run through `scripts/ios-ui-tests.sh` on the booted iPad Pro 11-inch
  (M5), iOS 27.0: **3 executed, 0 skipped, 0 failures**. Ran every method
  that uses `chatIsShown`: `testAPickInTheTreeShowsInThePlanColumn`,
  `testTheChatIsShownAndSurvivesATurn`, and `testTwoColumnsShowTheTreeOnDemand`.
