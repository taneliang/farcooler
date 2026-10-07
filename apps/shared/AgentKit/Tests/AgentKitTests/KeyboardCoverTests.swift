import CoreGraphics
import Foundation
import Testing

@testable import AgentKit

/// ov-386: the rules `KeyboardInset` publishes its number by.
struct KeyboardCoverTests {
    private let phone: CGFloat = 932
    private let small: CGFloat = 667

    @Test func aKeyboardFrameIsItsOverlap() {
        var cover = KeyboardCover()
        cover.frame(overlap: 336, screenHeight: phone)
        #expect(cover.height == 336)
    }

    @Test func aSyntheticFullScreenFrameReadsAsNoKeyboard() {
        var cover = KeyboardCover()
        cover.frame(overlap: 336, screenHeight: phone)
        cover.frame(overlap: phone, screenHeight: phone)
        #expect(cover.height == 0)
    }

    @Test func theComposerResizingMovesTheCoverWithNoFrame() {
        var cover = KeyboardCover()
        cover.frame(overlap: 456, screenHeight: phone)
        cover.accessory(cover: 516, screenHeight: phone)
        #expect(cover.height == 516)
    }

    /// The hide's animation lays the composer out at a place it won't rest.
    @Test func aComposerReportDuringAHideIsDropped() {
        var cover = KeyboardCover()
        cover.frame(overlap: 400, screenHeight: phone)
        cover.willHide()
        #expect(cover.height == 400, "the hide read as nothing before its frame")
        cover.accessory(cover: 260, screenHeight: phone)
        #expect(cover.height == 400, "a transient mid-slide cover was taken")
        cover.frame(overlap: 90, screenHeight: phone)
        #expect(cover.height == 90)
        cover.accessory(cover: 120, screenHeight: phone)
        #expect(cover.height == 120, "the hide's frame left the cover deaf to the composer")
    }

    /// `willChangeFrame` first, with the keyboard gone and the bar left: the
    /// hide that follows mustn't take the bar's cover away.
    @Test func aHideAfterItsFrameKeepsTheBarsCover() {
        var cover = KeyboardCover()
        cover.frame(overlap: 400, screenHeight: phone)
        cover.frame(overlap: 90, screenHeight: phone)
        cover.willHide()
        #expect(cover.height == 90, "the bar's cover went to \(cover.height)")
    }

    /// 260 pt of keys under a 300 pt composer: 84% of a 667 pt phone.
    @Test func aTallKeyboardAndComposerOnAShortPhoneIsTakenAsAFrame() {
        var cover = KeyboardCover()
        cover.frame(overlap: 560, screenHeight: small)
        #expect(cover.height == 560)
    }

    @Test func aHideThatSendsNoFrameEndsOnItsTimeout() {
        var cover = KeyboardCover()
        cover.willHide()
        cover.hideTimedOut()
        cover.accessory(cover: 90, screenHeight: phone)
        #expect(cover.height == 90)
    }

    /// A tall composer over the keys on a 667 pt phone reaches past 80% of it.
    @Test func aTallComposerOnAShortPhoneIsTaken() {
        var cover = KeyboardCover()
        cover.accessory(cover: 600, screenHeight: small)
        #expect(cover.height == 600)
    }

    @Test func aComposerNotYetPlacedIsNotTaken() {
        var cover = KeyboardCover()
        cover.accessory(cover: 90, screenHeight: small)
        cover.accessory(cover: small, screenHeight: small)
        cover.accessory(cover: 0, screenHeight: small)
        #expect(cover.height == 90)
    }
}

/// ov-386: a chat's inset hears its own composer, and only that one.
struct AccessoryCoverChannelTests {
    /// Calls a handler recorded for one observer, from a center of this
    /// test's own so nothing else's posts reach it.
    final class Heard: @unchecked Sendable {
        private let lock = NSLock()
        private var covers: [CGFloat] = []
        func add(_ cover: CGFloat) { lock.withLock { covers.append(cover) } }
        var all: [CGFloat] { lock.withLock { covers } }
    }

    @Test func aScopedInsetHearsOnlyItsOwnComposer() {
        let center = NotificationCenter()
        let mine = AccessoryScope()
        let other = AccessoryScope()
        let heard = Heard()
        let token = AccessoryCoverChannel.observe(scope: mine, center: center, queue: nil) { cover, _ in heard.add(cover) }
        defer { center.removeObserver(token) }

        AccessoryCoverChannel.post(cover: 100, screen: 900, scope: other, center: center)
        AccessoryCoverChannel.post(cover: 200, screen: 900, scope: mine, center: center)
        AccessoryCoverChannel.post(cover: 300, screen: 900, scope: nil, center: center)
        #expect(heard.all == [200], "heard \(heard.all)")
    }

    @Test func anUnscopedInsetHearsEveryComposer() {
        let center = NotificationCenter()
        let heard = Heard()
        let token = AccessoryCoverChannel.observe(scope: nil, center: center, queue: nil) { cover, _ in heard.add(cover) }
        defer { center.removeObserver(token) }

        AccessoryCoverChannel.post(cover: 100, screen: 900, scope: AccessoryScope(), center: center)
        AccessoryCoverChannel.post(cover: 200, screen: 900, scope: nil, center: center)
        #expect(heard.all == [100, 200])
    }
}
