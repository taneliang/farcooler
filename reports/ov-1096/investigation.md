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

Put the Agents card's semantic accessibility element on the button **before**
applying its context menu, rather than wrapping the context-menu host in a
second semantic button. Keep its label, count, hint, identifier, glass,
context-menu preview and actions. Do not clamp or hide its frame.

The existing `chatIsShown` window-containment assertion is unchanged. The
rotation test additionally checks chat visibility after turning back to
landscape, alongside its existing mount-preservation checks. The failed CI
run above is the regression's before evidence.

## Validation

Local iOS 27.0: the three methods using `chatIsShown` passed (3 ran,
0 skipped, 0 failures), as did `GlassUITests/testHoldingAgentsPreviewsThem`
and `testATapOpensEachCardsSheet` (2 ran, 0 skipped, 0 failures).
`git diff --check` and `scripts/file-size-budget.py` pass.

CI validation of the fix is in progress. Acceptance requires the subsequent
Nightly pad shard on iOS 27.2; local success is supplementary only.
The remaining jobs of the diagnostic Nightly were cancelled after the pad
result and raw log were saved, freeing the Nightly queue for the fix.
