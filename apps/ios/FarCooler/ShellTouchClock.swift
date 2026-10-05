import SwiftUI
import UIKit

/// When the finger under the shell actually stopped and lifted, by the
/// touch's own clock rather than by when SwiftUI got round to saying so.
///
/// **`DragGesture.Value.time` is when the value was DELIVERED, not when the
/// finger did the thing it describes**, and that is the whole reason this
/// exists. Measured on a simulator by stalling the main thread for 1.2 seconds
/// part-way through a 60-point lift that then held still for half a second
/// before letting go: the gap between the last movement and the release, read
/// from `value.time`, fell from 500 ms to 0. Every event the stall had queued
/// arrived in one burst, all stamped with the moment they were handed over, so
/// a release from a standstill read as a release mid-throw and carried the
/// estimator's 249 points per second into the projection. On a fast phone the
/// stall never happens; on CI's simulator one slow frame at the end of a drag
/// does it, and `ShellGestureTests.testAFlickUpFromOverAMenuRowChoosesNoRow`
/// and `ShellPaneScrollTests.testACodeLineAtItsEndHandsThePageTurnBack` both
/// failed on CI exactly that way: a held lift escaped, and a held sixty-point
/// drag turned the page.
///
/// `UITouch.timestamp` is the time the digitizer reported the touch, and it is
/// the same number however late the main thread reads it. So the question
/// `ShellRootView.releaseVelocity` asks — had the finger been still for
/// longer than `ShellRootView.stillFor` when it lifted? — is answered here, off
/// two of those timestamps, and a slow frame cannot change the answer.
///
/// A reference type held in `@State`, because it is written from UIKit on
/// every touch and read once per release: nothing about it should ever cause
/// a render. `@MainActor` because UIKit writes it and SwiftUI's gesture
/// callbacks read it, both on the main thread.
@MainActor
final class ShellTouchClock {
    /// The timestamp of the last sample that moved more than half a point —
    /// the same slop `ShellRootView.noteMovement` allows — or of touch-down,
    /// for a finger that has not moved yet.
    fileprivate var lastMoved: TimeInterval?
    /// Where that sample was, in the window's coordinates.
    fileprivate var lastPoint: CGPoint = .zero
    /// The release's timestamp, once UIKit has delivered it; nil while the
    /// finger is down.
    fileprivate var lifted: TimeInterval?
    /// Whether another finger was on the glass at any point during this one.
    ///
    /// The clock times one touch, and with two down it can't know which of
    /// them the shell's drag was following. A thumb resting on the terminal
    /// while the other hand flicks the bar would otherwise lend the flick the
    /// resting thumb's stillness. So a crowded touch answers nothing, and the
    /// release falls back to `ShellRootView.lastMoved`, which reads the
    /// drag's own deliveries.
    fileprivate var crowded = false

    /// How long the finger had been still when it lifted. Nil when this
    /// touch's release hasn't been seen yet, or when another finger was down
    /// during it.
    var stillBeforeLift: TimeInterval? {
        guard !crowded, let lifted, let lastMoved else { return nil }
        return lifted - lastMoved
    }

    fileprivate func down(_ touch: UITouch) {
        lastMoved = touch.timestamp
        lastPoint = touch.location(in: nil)
        lifted = nil
        crowded = false
    }

    fileprivate func sample(_ touch: UITouch) {
        let point = touch.location(in: nil)
        guard abs(point.x - lastPoint.x) > 0.5 || abs(point.y - lastPoint.y) > 0.5 else { return }
        lastMoved = touch.timestamp
        lastPoint = point
    }

    /// The release, stamped and nothing more.
    ///
    /// Its location is deliberately not a sample. A real finger rolls a
    /// point or so as it leaves the glass, and counting that as movement
    /// would make a held lift look like it moved at the instant it lifted —
    /// the defect this type exists to fix. A finger that really was moving
    /// produced a `touchesMoved` within a frame of lifting, well inside
    /// `ShellRootView.stillFor`.
    fileprivate func up(_ touch: UITouch) {
        lifted = touch.timestamp
    }

    fileprivate func crowd() { crowded = true }
}

/// Installs the clock's recognizer on the window the shell is in.
///
/// On the WINDOW rather than on a view of the shell's own, so it sees the
/// bar's touches and the content's alike without being in the hit-test path
/// of either. It never recognizes and it never delays or cancels a touch, so
/// every gesture under it — SwiftUI's, a terminal's pan, a hunk's scroll view
/// — receives exactly what it received before.
struct ShellTouchClockInstaller: UIViewRepresentable {
    let clock: ShellTouchClock

    func makeUIView(context: Context) -> InstallerView {
        let view = InstallerView()
        view.isUserInteractionEnabled = false
        view.recognizer.clock = clock
        return view
    }

    func updateUIView(_ view: InstallerView, context: Context) {
        view.recognizer.clock = clock
    }

    final class InstallerView: UIView {
        let recognizer = ShellTouchClockRecognizer()

        override func didMoveToWindow() {
            super.didMoveToWindow()
            recognizer.view?.removeGestureRecognizer(recognizer)
            window?.addGestureRecognizer(recognizer)
        }
    }
}

/// A recognizer that only listens.
///
/// It stays `.possible` for the whole of a touch and fails when the touch
/// ends, which is how a recognizer watches without competing:
/// `cancelsTouchesInView`, `delaysTouchesBegan` and `delaysTouchesEnded` are
/// all off, and it recognizes simultaneously with everything and is never a
/// failure requirement for anything.
///
/// One finger: the first touch down is the one timed. Any other touch that
/// lands while it is down marks it crowded, and a crowded touch gives no
/// answer (see `ShellTouchClock.crowded`).
final class ShellTouchClockRecognizer: UIGestureRecognizer, UIGestureRecognizerDelegate {
    weak var clock: ShellTouchClock?
    private weak var tracked: UITouch?

    init() {
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard tracked == nil else {
            clock?.crowd()
            return
        }
        guard let touch = touches.first else { return }
        tracked = touch
        clock?.down(touch)
        if touches.count > 1 { clock?.crowd() }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let tracked, touches.contains(tracked) else { return }
        // Every sample the digitizer took, not only the last of a frame's:
        // a slow frame coalesces them, and the last movement is the sample
        // this is looking for.
        for sample in event.coalescedTouches(for: tracked) ?? [tracked] {
            clock?.sample(sample)
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let tracked, touches.contains(tracked) else { return }
        clock?.up(tracked)
        if (event.allTouches?.count ?? 1) > 1 { clock?.crowd() }
        state = .failed
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let tracked, touches.contains(tracked) else { return }
        state = .failed
    }

    override func reset() {
        super.reset()
        tracked = nil
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool { true }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldBeRequiredToFailBy other: UIGestureRecognizer
    ) -> Bool { false }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRequireFailureOf other: UIGestureRecognizer
    ) -> Bool { false }
}
