import Foundation
import Testing

@testable import AgentKit

// The card, over the two grounds it is really drawn on — as PIXELS.
//
// **This suite exists because of a defect that every assertion above the pixels
// passed.** `AgentCardLayout` composed the right words, `GlancePalette` held the
// right twelve values, `GlanceMark` carried the right states, and the Live
// Activity still arrived on a Mac's menu bar as white text on a light ground.
// The card had `.environment(\.colorScheme, .dark)` over the whole of itself
// while leaving its BACKGROUND to the system's material — so on any surface
// where the system draws that material light, the card wrote §01's dark palette
// onto it: `text 2` at L 0.74 landed 1.2:1 against an L 0.80 material, the
// trace's upper half at L 0.88 landed 1.3:1, and a person saw two agents' names
// with nothing under them.
//
// So the shape of every test here is the same: render, read the bytes back, and
// compute the contrast ratio a person's eye is actually offered. None of them
// can pass by the card merely INTENDING the right color.
//
// `#if os(macOS)` for `GlanceMarkTests`' reason exactly — `ImageRenderer` needs
// AppKit to hand the bytes back, and `swift test --package-path
// apps/shared/AgentKit` runs on a Mac in CI.
#if os(macOS)
    import AppKit
    import SwiftUI

    /// One of the two grounds the card is really drawn on, and neither of them
    /// is a color this product chose.
    ///
    /// `.activityBackgroundTint(nil)` hands the background to the system —
    /// a flat fill of ours would sit on top of somebody's photograph like a
    /// sticker — so the ground is whatever material the system draws, and the
    /// card gets no say in it.
    ///
    /// **The pale figure is measured, not invented.** It is the sRGB the
    /// system's material came out at in the owner's screenshot of the Live
    /// Activity in the Mac menu bar: 187, 188, 189. The dark figure is §01's own
    /// `card` surface, L 0.22, which is what the material sits at over a
    /// lock screen photograph.
    private struct Ground {
        let name: String
        let scheme: ColorScheme
        let color: Color

        static let pale = Ground(
            name: "the system's pale material",
            scheme: .light,
            color: Color(.sRGB, red: 187 / 255, green: 188 / 255, blue: 189 / 255))

        static let dark = Ground(
            name: "the lock screen's dark material",
            scheme: .dark,
            color: GlancePalette.card.dark.color)

        static let both = [pale, dark]
    }

    /// A rendered pixel, in the only two forms this file needs it in.
    private struct Pixel {
        let red: Double
        let green: Double
        let blue: Double

        /// WCAG 2.1 relative luminance. The transfer function is the same one
        /// `OKLCH.encode` runs in the other direction; it is written out here
        /// rather than shared because the palette converts a color TO sRGB and
        /// this reads a rendered one back, and a shared helper would tempt
        /// somebody to make the palette depend on a test's arithmetic.
        var luminance: Double {
            func linear(_ channel: Double) -> Double {
                channel <= 0.03928
                    ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
        }

        /// WCAG 2.1 contrast ratio, 1…21.
        func contrast(against other: Pixel) -> Double {
            let mine = luminance, theirs = other.luminance
            return (max(mine, theirs) + 0.05) / (min(mine, theirs) + 0.05)
        }
    }

    /// Every pixel of a view, rendered at the scale a phone draws it.
    ///
    /// Deliberately the same `ImageRenderer` + `CGContext` route
    /// `GlanceMarkTests.fingerprint` takes, and for the same reason: a view
    /// asked what color it is will happily answer correctly while rendering
    /// something else.
    @MainActor
    private func pixels<V: View>(of view: V, size: CGSize, scheme: ColorScheme)
        -> (pixels: [Pixel], width: Int, height: Int)
    {
        let renderer = ImageRenderer(
            content:
                view
                .frame(width: size.width, height: size.height)
                .environment(\.colorScheme, scheme))
        renderer.scale = 3
        guard let image = renderer.cgImage else { return ([], 0, 0) }

        let width = image.width, height = image.height
        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        buffer.withUnsafeMutableBytes { raw in
            let context = CGContext(
                data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(
                image, in: CGRect(origin: .zero, size: CGSize(width: width, height: height)))
        }
        var read: [Pixel] = []
        read.reserveCapacity(width * height)
        for index in stride(from: 0, to: buffer.count, by: 4) {
            read.append(
                Pixel(
                    red: Double(buffer[index]) / 255,
                    green: Double(buffer[index + 1]) / 255,
                    blue: Double(buffer[index + 2]) / 255))
        }
        return (read, width, height)
    }

    /// One ink, as the pixel it actually paints on that ground.
    ///
    /// A filled square rather than a glyph: text is antialiased, so the reading
    /// off a letter is a spread from the ink to the ground and the number that
    /// comes out depends on which pixel of which stem you happened to sample.
    /// A square answers what the ink IS, which is the thing §01 states and the
    /// thing that was wrong.
    @MainActor
    private func inkPixel(_ ink: AnyShapeStyle, on ground: Ground) -> Pixel {
        let swatch = ZStack {
            ground.color
            Rectangle().fill(ink)
        }
        let read = pixels(of: swatch, size: CGSize(width: 12, height: 12), scheme: ground.scheme)
        // The middle of the square, well away from any edge.
        return read.pixels[read.pixels.count / 2 + read.width / 2]
    }

    @MainActor
    private func groundPixel(_ ground: Ground) -> Pixel {
        inkPixel(AnyShapeStyle(ground.color), on: ground)
    }

    /// The card, with two agents on it and a trace apiece — the drawing the
    /// owner photographed.
    private var cardLayout: AgentCardLayout {
        let busy = ActivityTraceTests.encoded(
            code: [0, 0, 0, 0, 0, 8, 2, 900, 40, 7, 0, 0, 0],
            output: [0, 0, 0, 0, 0, 4, 1, 3, 60, 5, 0, 0, 0],
            commits: [0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 0])
        let talky = ActivityTraceTests.encoded(
            code: [0, 12, 40, 90, 60, 30, 110, 200, 90, 20, 5, 0, 0],
            output: [3, 20, 60, 40, 90, 120, 30, 70, 200, 160, 40, 10, 0],
            commits: [0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0])
        let now = Date(timeIntervalSince1970: 1_755_000_000)
        let state = AgentCardState(
            terminal: "term-1", label: "auth-refactor", machine: "studio",
            status: "blocked", detail: "Force-push to origin/main?",
            startedAt: now.addingTimeInterval(-600),
            blocked: 1, review: 1, working: 2,
            insertions: 10104, deletions: 21, commits: 41, more: 1,
            rows: [
                AgentCardRow(
                    terminal: "term-1", label: "auth-refactor", machine: "overnight",
                    status: "blocked", detail: "Verified — the fix is in",
                    insertions: 3321, deletions: 0, commits: 6,
                    startedAt: now.addingTimeInterval(-600),
                    updatedAt: now.addingTimeInterval(-20), trace: busy),
                AgentCardRow(
                    terminal: "term-2", label: "docs-sweep", machine: "studio",
                    status: "working", detail: "Asking Fallback",
                    insertions: 3321, deletions: 0, commits: 6,
                    startedAt: now.addingTimeInterval(-900),
                    updatedAt: now.addingTimeInterval(-5), trace: talky),
            ])
        return AgentCardLayout(state: state, now: now)!
    }

    /// The size the system gives the presentation, near enough — the card is
    /// capped at about 160 points tall and spends the screen's width less its
    /// margins.
    private let cardBox = CGSize(width: 330, height: 132)

    // MARK: - The card, as a whole drawing

    /// **The defect, as one assertion.** A card that forces its own appearance
    /// draws the same bytes whatever the system is doing, and that is exactly
    /// what "white text on a light background" looked like from the inside.
    ///
    /// Stated as a difference rather than as a color so it cannot be satisfied
    /// by a card that follows the environment for its words and forces dark on
    /// its trace, or the other way round: any part of the card left forcing an
    /// appearance leaves the two renders that much closer, and all of it forcing
    /// one leaves them identical.
    @MainActor
    @Test func theCardDrawsSomethingDifferentOnAPaleGroundThanOnADarkOne() {
        let card = GlanceCardView(layout: cardLayout)
        let pale = pixels(of: card, size: cardBox, scheme: .light).pixels
        let dark = pixels(of: card, size: cardBox, scheme: .dark).pixels

        #expect(!pale.isEmpty, "the card rendered nothing at all on a pale ground")
        #expect(pale.count == dark.count)
        let same = zip(pale, dark).allSatisfy {
            $0.red == $1.red && $0.green == $1.green && $0.blue == $1.blue
        }
        #expect(
            !same,
            """
            the card draws byte for byte the same thing in both appearances — it is \
            asserting an appearance the system did not give it, which is how §01's dark \
            palette came to be written onto a pale system material
            """)
    }

    /// **The card must not assert what is behind it**, said as the one thing a
    /// person notices when it does: over a pale ground, nothing the card draws
    /// may be PALER than the ground.
    ///
    /// This is the whole-drawing form of the assertion and it needs no threshold
    /// anybody chose. `text 1` at L 0.97, `commit` at L 0.96 and `code` at L
    /// 0.88 are all lighter than an L 0.80 material, so a card forcing dark
    /// fails this on its title, its figures, its commit marks and half of every
    /// trace at once.
    ///
    /// The tolerance is for antialiasing and nothing else: a glyph's edge blends
    /// ink into ground and the blend can overshoot the ground by a fraction of a
    /// level. One 255th of the range is far below anything a person can see and
    /// far below the 0.17 an ink one step lighter than this ground would move.
    @MainActor
    @Test func overAPaleGroundTheCardWritesNothingPalerThanTheGround() {
        let ground = Ground.pale
        let card = ZStack {
            ground.color
            GlanceCardView(layout: cardLayout)
        }
        let read = pixels(of: card, size: cardBox, scheme: ground.scheme)
        let groundLuminance = groundPixel(ground).luminance
        let tolerance = 1.0 / 255

        let palest = read.pixels.max { $0.luminance < $1.luminance }
        let palestLuminance = palest?.luminance ?? 0
        #expect(
            palestLuminance <= groundLuminance + tolerance,
            """
            the card drew something paler than the ground it is on — luminance \
            \(palestLuminance) against the material's \(groundLuminance). §01's light \
            values exist for exactly this surface; a pale ink on it is the dark palette \
            asserting a backdrop the system did not supply
            """)
    }

    // MARK: - The inks, one at a time

    /// Which floor an ink has to clear, and why that one.
    private struct InkUnderTest {
        let name: String
        let floor: Double
        let ink: (ColorScheme) -> AnyShapeStyle
    }

    /// Everything the card writes with that carries information.
    ///
    /// **The floors are three, and each is argued rather than tuned.**
    ///
    ///   - **4.5 for `ink1`** — WCAG AA for body text, and this is every word a
    ///     person reads first: the header, both row names, both diff figures.
    ///   - **3.0 for `ink2` and for the trace's lit tones** — AA for large text
    ///     and for graphics, and it is the level Apple's own `secondaryLabel`
    ///     sits at, which is what `ink2` resolves to in light mode. A floor of
    ///     4.5 here would be a floor the system's own answer cannot clear.
    ///   - **1.8 for the two state hues** — these are read as shape and, for
    ///     amber, as the one reserved hue in the product; the
    ///     number is only there to catch an ink landing on top of its own
    ///     ground. It is deliberately low and it is still decisive: forcing dark
    ///     onto the pale material puts `text 1` at 1.76, `text 2` at 1.20,
    ///     `code` at 1.34, `commit` at 1.71, amber at 1.07 and review at 1.11.
    ///
    /// **Amber against the pale material is 1.95, and that is a gap in §01
    /// rather than a number chosen to pass.** §01 says amber "darkens to
    /// oklch(0.62 0.13 68) to hold contrast on a pale backdrop", and it does
    /// hold 3.6:1 against WHITE — but the pale backdrop this card really gets is
    /// the system's material at L 0.80, not paper, and against that the same
    /// value is 1.95. The mark carries hue and stroke weight as well as
    /// luminance so it is legible, but the design document is where that is
    /// closed, not here.
    @MainActor
    private func informativeInks() -> [InkUnderTest] {
        [
            InkUnderTest(name: "ink 1", floor: 4.5) { AnyShapeStyle(GlancePalette.ink1($0)) },
            InkUnderTest(name: "ink 2", floor: 3.0) { AnyShapeStyle(GlancePalette.ink2($0)) },
            InkUnderTest(name: "code", floor: 3.0) { AnyShapeStyle(GlancePalette.code($0)) },
            InkUnderTest(name: "chat", floor: 3.0) { AnyShapeStyle(GlancePalette.chat($0)) },
            InkUnderTest(name: "commit", floor: 3.0) { AnyShapeStyle(GlancePalette.commitInk($0)) },
            InkUnderTest(name: "amber", floor: 1.8) { AnyShapeStyle(GlancePalette.amber($0)) },
            InkUnderTest(name: "review", floor: 1.8) { AnyShapeStyle(GlancePalette.review($0)) },
        ]
    }

    /// Every ink on the card, against the ground it is really on.
    @MainActor
    @Test func everyInkTheCardWritesWithClearsTheGroundItIsOn() {
        for ground in Ground.both {
            let base = groundPixel(ground)
            for ink in informativeInks() {
                let ratio = inkPixel(ink.ink(ground.scheme), on: ground).contrast(against: base)
                #expect(
                    ratio >= ink.floor,
                    """
                    \(ink.name) is \(String(format: "%.2f", ratio)):1 against \(ground.name) \
                    and has to clear \(ink.floor):1 — on that ground it is not ink, it is \
                    the ground
                    """)
            }
        }
    }

    /// The three tones that are STRUCTURE rather than information, and the only
    /// two things that can be wrong with them.
    ///
    /// **They must be visible and they must be quieter than the words.** A rule
    /// louder than the sentence it separates turns a card into three boxed
    /// lists, which is what §01's single dark figure for `rule` did the moment it
    /// was drawn on a pale material — L 0.32 is a quiet division against the L
    /// 0.22 card and a near-black bar against an L 0.80 one.
    ///
    /// **And a silent bucket must never be louder than a busy one.** `empty` is
    /// one figure at L 0.42, which sits below both lit tones on a dark ground
    /// and BETWEEN them on a pale one — 4.4:1 where `chat` is 3.1:1 — so
    /// thirteen buckets of nothing shouted over the buckets that had something
    /// in them. That is half of why §04's trace was reported as "a bunch of
    /// random shapes", and it is the half that is arithmetic rather than taste.
    @MainActor
    @Test func aStructuralToneIsVisibleAndQuieterThanWhatItSeparates() {
        for ground in Ground.both {
            let base = groundPixel(ground)
            func ratio(_ ink: AnyShapeStyle) -> Double {
                inkPixel(ink, on: ground).contrast(against: base)
            }
            let words = ratio(AnyShapeStyle(GlancePalette.ink2(ground.scheme)))
            let structure: [(String, AnyShapeStyle)] = [
                ("the card's rule", GlancePalette.ruleInk(ground.scheme)),
                ("the row rule", GlancePalette.rowRuleInk(ground.scheme)),
                ("a silent bucket", GlancePalette.emptyInk(ground.scheme)),
                ("the trace's rule", GlancePalette.axisInk(ground.scheme)),
            ]
            for (name, ink) in structure {
                let measured = ratio(ink)
                #expect(
                    measured > 1.02,
                    "\(name) is \(measured):1 against \(ground.name) — it is not drawn at all")
                #expect(
                    measured < words,
                    """
                    \(name) is \(String(format: "%.2f", measured)):1 against \(ground.name), \
                    louder than the secondary words at \(String(format: "%.2f", words)):1 — \
                    structure drawn heavier than the sentence it separates
                    """)
            }
        }
    }

    /// **The trace is four tones and the ORDER of them is its meaning**: a
    /// silent bucket under the rule, the rule under the talk, the talk under the
    /// code. Read down that ladder and the drawing says "mostly talking, one
    /// burst of code, quiet since" without a word on it.
    ///
    /// On §01's dark card the order holds by arithmetic — 1.7, 1.8, 3.8, 9.7
    /// against the L 0.22 surface. Every one of those four is a single figure
    /// stated for a dark ground, and on the system's pale material the same four
    /// come out 4.4, 4.0, 3.1, 7.6: the silence and the rule are BOTH louder
    /// than the talk they are supposed to sit behind. Two rungs swapped is a
    /// picture that reads as noise, and it is the arithmetic half of the owner's
    /// "it looks like a bunch of random shapes".
    ///
    /// Stated as the ladder rather than as four floors because the ladder is the
    /// claim. Four tones each clearing some threshold would have passed the
    /// whole time.
    @MainActor
    @Test func theTraceIsALadderOfFourTonesAndTheOrderIsItsMeaning() {
        for ground in Ground.both {
            let base = groundPixel(ground)
            func ratio(_ ink: AnyShapeStyle) -> Double {
                inkPixel(ink, on: ground).contrast(against: base)
            }
            let rungs: [(String, Double)] = [
                ("a silent bucket", ratio(GlancePalette.emptyInk(ground.scheme))),
                ("the centre rule", ratio(GlancePalette.axisInk(ground.scheme))),
                ("talk", ratio(AnyShapeStyle(GlancePalette.chat(ground.scheme)))),
                ("code", ratio(AnyShapeStyle(GlancePalette.code(ground.scheme)))),
            ]
            for step in 1..<rungs.count {
                #expect(
                    rungs[step - 1].1 < rungs[step].1,
                    """
                    on \(ground.name), \(rungs[step - 1].0) is \
                    \(String(format: "%.2f", rungs[step - 1].1)):1 and \(rungs[step].0) is \
                    \(String(format: "%.2f", rungs[step].1)):1 — the trace's four tones are \
                    out of order, so the quiet things are shouting over the loud ones
                    """)
            }
        }
    }
#endif
