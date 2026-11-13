# ov-1097: the pan test needs vertical zoom room

## CI evidence

[Nightly 43303932801, phone6](https://github.com/taneliang/farcooler/actions/runs/43303932801/job/131507653767), November 12, ran on iPhone 17 / iOS 27.2. The complete execution log for `testASwipeDownClosesAnOpenImage` is preserved in [ci-test.log](ci-test.log):

- At 42.54 seconds, XCTest pinches with scale 3 in `QLPreviewControllerView`, using rect `{{50, 50}, {302, 774}}`.
- At 45.07 seconds, it starts a 300-point downward drag at 750 points/second.
- At 48.90 seconds, the viewer is gone; the test fails at line 434 with “a drag on the zoomed image closed it rather than panning”.

The retained test recording proves that the pinch **did** enlarge the picture. Its pale circle grows from 150 to 392 points in a 402 × 874 screen. The circle is half the fixture's height, so the enlarged image is still only about 784 points tall: it remains letterboxed, without vertical content to scroll. The next gesture is a dismissal flick, not a bounded pan within vertically overflowing content. Viewer existence alone did not establish the precondition the assertion depended on.

Frames from the recording, at 42.0, 44.4, and 45.2 seconds:

| Before pinch | After pinch | After flick |
| --- | --- | --- |
| ![Fitted image](ci-before-pinch.png) | ![Enlarged but letterboxed image](ci-after-pinch.png) | ![Dismissal returns to conversation](ci-after-flick.png) |

The shard was subsequently cancelled, but this test and the entire NativeAgentViewTests suite finished and reported their failures before cancellation. The uploaded xcresult lacks its final Info.plist. These frames come from its retained `kXCTAttachmentScreenRecording` payload (7,054,760 bytes), whose test summary identifies `NativeAgentViewTests/testASwipeDownClosesAnOpenImage()` and timestamp `2026-11-12T22:45:30.082+0000`. They are CI evidence, not local screenshots or a claim that the whole shard finished.

## Change

Wait for the fixture's pale orange circle to establish that the picture has decoded, excluding both white and black loading backgrounds. Use a larger pinch, then measure the circle in a screenshot. Require its height to exceed 70% of the screen, establishing vertical image content for the pan. Use a slow 80-point drag with a hold before release so the pan check does not fling the image into its edge. Keep the original 300-point fast dismissal swipe, and first verify that Quick Look's double tap returns the marker to its original fitted size. Preserve the existing before/after screenshot attachments.

The production Quick Look viewer is unchanged. The test continues to check short-drag cancellation, a drag while zoomed, dismissal while fitted, and return to the conversation.

## Validation

On the local iPhone 17 / iOS 27.0 simulator, using `scripts/ios-ui-tests.sh`:

- The unchanged test passed, confirming the failure is not reproduced by the old test on this runtime.
- With the larger zoom, the old pinch-in left the image zoomed and the final dismissal failed. Even scale 0.05 did not restore fitted size. Double tap plus the fitted-marker check passed.
- After rebuilding all simulator Rust XCFrameworks from the rebased source, the revised swipe test passed in the full class run (22.119 seconds).
- Rebuilt all simulator Rust XCFrameworks after rebasing before the final class/smoke run. The unrelated `testNoticeTurnsRenderAsNotices` fails both before and after that rebuild with “no notice turn”. Its unchanged harness emits `origin: Notification` without `notice`; the base's `NativeRows.swift` now requires `turn.notice == true`. This failure is outside image-swipe handling, and neither that harness nor its notice assertion is changed here.
- Full affected class plus `scripts/ios-ui-shards.py smoke`: 33 collected, 0 skipped, 32 passed, 1 failure (the notice test above). All smoke tests passed. See [local-validation.log](local-validation.log). The harness's `ClientCore.call` returns `standInCalls` directly, so rebuilding Rust cannot backfill the missing field in this Swift fixture.
- After tightening the initial decode guard to reject both white and black backgrounds, the final focused swipe test passed: 1 collected, 0 skipped, 1 actually ran. See [local-focused-test.log](local-focused-test.log).

The marker check rejects the failed CI precondition: 392 / 874 = 44.9%, below the required 70%. A passing **next nightly** remains a separate acceptance gate; local results cannot establish it.
