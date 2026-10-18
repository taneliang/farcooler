import Foundation
import SwiftUI
import Testing

@testable import AgentKit

// The person's bubble (ov-610): the site's blue, and white text on it above
// WCAG AA in every appearance.
struct MessageBubbleTests {
    @Test func whiteTextMeetsAAOnEveryFace() {
        for scheme in [ColorSchemeCase.light, .dark] {
            for increased in [false, true] {
                let fill = MessageBubble.packed(scheme.value, contrast: increased ? .increased : .standard)
                let ratio = MessageBubble.contrast(MessageBubble.textPacked, fill)
                #expect(ratio >= 4.5, "white on \(String(fill, radix: 16)) is \(ratio):1 (\(scheme), increased \(increased))")
                // Increase Contrast never lowers it.
                if increased {
                    let standard = MessageBubble.contrast(MessageBubble.textPacked, MessageBubble.packed(scheme.value, contrast: .standard))
                    #expect(ratio > standard, "Increase Contrast lowered it (\(scheme))")
                }
            }
        }
    }

    @Test func theRatiosAreWhatTheyShouldBe() {
        // WCAG's own examples: white on black is 21:1, and a color on itself 1:1.
        #expect(abs(MessageBubble.contrast(0xFFFFFF, 0x000000) - 21) < 0.01)
        #expect(abs(MessageBubble.contrast(0x0A59C7, 0x0A59C7) - 1) < 0.01)
        #expect(abs(MessageBubble.contrast(0xFFFFFF, MessageBubble.siteBlue) - 6.43) < 0.01)
    }

    /// The light face is the site's own `.fe-msg` background, read from the
    /// page itself, so the two can't drift apart.
    @Test func theLightFaceIsTheSitesBlue() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { url.deleteLastPathComponent() }
        let page = try String(contentsOf: url.appendingPathComponent("services/site/public/index.html"), encoding: .utf8)
        let rule = try #require(page.components(separatedBy: "\n").first { $0.contains(".fe-msg {") })
        let hex = String(format: "#%06x", MessageBubble.siteBlue)
        #expect(rule.contains("background: \(hex)"), "the site's bubble is \(rule)")
        #expect(rule.contains("border-radius: 13px 13px 4px 13px"), "the site's bubble changed shape: \(rule)")
    }
}

/// `ColorScheme`, iterable and printable in a test.
enum ColorSchemeCase: CustomStringConvertible {
    case light, dark
    var value: ColorScheme { self == .light ? .light : .dark }
    var description: String { self == .light ? "light" : "dark" }
}

