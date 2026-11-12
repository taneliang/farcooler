# ov-1096: iPad chat after rotation

## CI root cause

[Nightly 43252796734, pad job 131307587775](https://github.com/taneliang/farcooler/actions/runs/43252796734/job/131307587775)
ran `4bacc02d1`, including diagnostic commit `b484be421`, on **iPad Pro
11-inch (M5), iOS 27.2**. Its only failing test was
`testTheChatIsShownAndSurvivesATurn`: 23 collected, 1 capture test skipped,
22 ran, 1 failure. The relevant raw log is [ci-child-frames.log](ci-child-frames.log).

The overflowing child is **`agents-card`**, the outer accessibility button
around its context menu. After rotation:

| Element | x | width | right edge |
| --- | ---: | ---: | ---: |
| Window | 0 | 834 | 834 |
| `pad-chat` accessibility container | 375.5 | 476 | 851.5 |
| `agents-card` outer button | 395.5 | 456 | 851.5 |
| Its inner button and text | 395.5 | 418.5 | 814 |
| `orchestrator-pane` | 375.5 | 458.5 | 834 |

The actual column and terminal fit. The card's accessibility wrapper retains
its landscape width (456 points, the landscape chat less its 40-point
horizontal insets), while its inner button has resized to portrait. Its
17.5-point overflow enlarges the containing accessibility frame. Neither the
terminal failure state nor the column's width allocation causes it.

## Fix

Rebuild the iPad's `WorkspaceCards` strip when its column width changes,
using the column width already calculated by `PadLayout` as its identity.
This rebuilds the `FloatingGroup` (`GlassEffectContainer`) and the strip's
glass namespace together with the button and context-menu host. Replacing
only the button leaves the surrounding glass group and namespace alive.
The conversation is a sibling and retains its session and mount. The card
keeps its label, count, hint, identifier, glass, context-menu preview and
actions. The column allocation is unchanged; nothing clamps or hides overflow.

[Run 43258934525](https://github.com/taneliang/farcooler/actions/runs/43258934525/job/131328132225)
proved that merely moving the accessibility modifiers before the context
menu did not repair the cached frame: the same 456-point wrapper persisted.
That ineffective modifier reorder was removed. This run also had a separate
app-launch failure in `testCancelPutsTheSidebarAway`.
[Run 43264601695](https://github.com/taneliang/farcooler/actions/runs/43264601695/job/131348958465)
on `8b8f6dee2` proved that replacing only the button's identity was also
insufficient: on iOS 27.2 the outer button still measured 456 points and
its inner button 418.5 points. The revision moves the identity to the
strip, including its glass group and namespace. That run also failed the
sidebar keyboard shortcut test, separately from the chat assertion.
The existing `chatIsShown` window-containment assertion is unchanged. The
rotation test additionally checks chat visibility after turning back to
landscape and opens the resized card's context-menu preview and Agents sheet,
alongside its existing mount-preservation checks. The failed CI
run above is the regression's before evidence.

## Validation

With the strip-identity change, local iOS 27.0: the three methods using
`chatIsShown` passed (3 ran, 0 skipped, 0 failures), including the added
post-rotation card interaction. On an iPhone, `GlassUITests/testHoldingAgentsPreviewsThem`
and `testATapOpensEachCardsSheet` passed (2 ran, 0 skipped, 0 failures).
The first invocation mistakenly selected the phone card tests on an iPad;
those require the phone's Now/Next card and failed at that prerequisite.
The separate runs above use the appropriate devices.
`testTheKeysShowTheBoardThePlanAndTheTree` also passed (1 ran, 0 skipped,
0 failures), covering the column-width changes when the tree is toggled.
`git diff --check` and `scripts/file-size-budget.py` pass.

CI validation of the fix is in progress. Acceptance requires the subsequent
Nightly pad shard on iOS 27.2; local success is supplementary only.
The remaining jobs of the diagnostic Nightly were cancelled after the pad
result and raw log were saved, freeing the Nightly queue for the fix.
