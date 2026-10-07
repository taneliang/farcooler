import CoreGraphics
import Foundation

/// How much of the screen the keyboard and the composer docked on it cover,
/// from the three things that report it (ov-386).
///
/// The phone's `KeyboardInset` feeds this and publishes `height`; the rules
/// live here because the iOS target has no unit tests, and each one is a
/// way the number was once, or could be, wrong.
///
/// - A keyboard frame says where the keyboard's top is, accessory included,
///   so its overlap with the screen is the cover. A frame whose overlap is
///   nearly the whole screen is iOS 26's synthetic one, with an origin of
///   zero, and reads as no keyboard.
/// - The docked composer says where its own top is when UIKit resizes it
///   with no frame to say so (a banner above the field, a queued message).
/// - While the keyboard is hiding the composer is mid-animation, so the top
///   it reports is wherever the slide has got to, not where it will rest.
///   Its reports are dropped until the hide's own frame arrives, which says
///   what is left.
public struct KeyboardCover: Equatable, Sendable {
    /// Points of the screen covered, from its bottom.
    public private(set) var height: CGFloat = 0

    /// Between "the keyboard is going away" and the frame that ends it.
    public private(set) var hiding = false

    public init() {}

    /// Beyond this share of the screen a keyboard frame is not a keyboard.
    static let impossibleShare: CGFloat = 0.8

    /// The keyboard is going away. What it leaves, the docked composer,
    /// isn't known until the next frame.
    public mutating func willHide() {
        hiding = true
        height = 0
    }

    /// The hide's frame never came, so what the composer last said stands.
    /// The caller asks it to say again.
    public mutating func hideTimedOut() {
        hiding = false
    }

    /// A keyboard frame, as `overlap` points of a screen `screenHeight` tall.
    public mutating func frame(overlap: CGFloat, screenHeight: CGFloat) {
        hiding = false
        height = overlap >= screenHeight * Self.impossibleShare ? 0 : overlap
    }

    /// The composer's top is `cover` points up a window `screenHeight` tall.
    ///
    /// Anything between nothing and the whole window counts. A cover of
    /// 80% of the window was once dropped as impossible, and on a short
    /// phone (667 pt, so 534) a tall composer over the keys really does
    /// reach that far; dropping it left the stale height that report exists
    /// to correct. The impossible one is a composer not yet placed, whose
    /// top is the window's own: a cover of the whole window.
    public mutating func accessory(cover: CGFloat, screenHeight: CGFloat) {
        guard !hiding, cover > 0, cover < screenHeight else { return }
        height = cover
    }
}

/// Names one docked composer, for the inset that listens to it alone.
public final class AccessoryScope {
    public init() {}
}

/// Where a docked composer says its top has moved, and who hears it.
///
/// Scoped to the composer's own inset (ov-386): the report was once posted
/// for anything to hear, so a second docked composer would have moved a
/// chat's inset it had nothing to do with. An inset with no scope hears
/// every composer, which is the shell's (it cancels what the framework
/// applies for whichever is up).
public enum AccessoryCoverChannel {
    public static let didResize = Notification.Name("FarCooler.AccessoryHostView.didResize")

    /// `scope`'s composer's top is `cover` points up a window `screen` tall.
    public static func post(
        cover: CGFloat, screen: CGFloat, scope: AccessoryScope?, center: NotificationCenter = .default
    ) {
        center.post(name: didResize, object: scope, userInfo: ["cover": cover, "screen": screen])
    }

    /// Calls `handler` with a cover and the window's height, for `scope`'s
    /// composer, or for every one when `scope` is nil.
    public static func observe(
        scope: AccessoryScope?, center: NotificationCenter = .default, queue: OperationQueue? = .main,
        handler: @escaping @Sendable (CGFloat, CGFloat) -> Void
    ) -> NSObjectProtocol {
        center.addObserver(forName: didResize, object: scope, queue: queue) { note in
            guard let cover = note.userInfo?["cover"] as? CGFloat, let screen = note.userInfo?["screen"] as? CGFloat
            else { return }
            handler(cover, screen)
        }
    }
}
