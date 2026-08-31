import Foundation
import SwiftUI

// The glance surfaces' twelve colours, written once, in the notation the spec
// wrote them in.
//
// **This file is the only place in the codebase where any of these values may
// appear.** The design project's `Spec.dc.html` §01 states the rule that made
// this file necessary, and states it about itself:
//
// > Do not copy values out of them into your own constants file and then edit
// > them there — that duplication is what produced eight rounds of drift while
// > these documents were being written, and the same failure mode will hit the
// > codebase.
//
// So a surface that wants amber asks `GlancePalette.amber` for it. It does not
// write `Color.orange`, and it does not write `oklch(0.78 0.16 70)` a second
// time. Three copies of that rule already existed when this file was written —
// `glanceTint` in `FleetWidget.swift`, its acknowledged copy in
// `WatchFleetWidget.swift`, and `ShellMarkView`'s four literals in
// `ShellBar.swift` — and the comment beside each of the first two said, in
// so many words, that a colour is a SwiftUI type and `FleetSnapshot.swift` has
// no business importing SwiftUI. That is still true (see
// `FleetSnapshot.swift:466-470`); this file is where the SwiftUI half goes
// instead, so the wire's shape stays free of views and the tint still lives in
// exactly one place.
//
// **It is in AgentKit and not beside any view** because five binaries draw
// these surfaces: the app, the widget extension, the notification service
// extension, the watch app and the watch's complication. See the comment at
// `apps/ios/generate-project.py:198-205` — "THREE targets, three binaries, one
// file" — for why a shared file needs a build id per target rather than a copy
// per target.
//
// **It must typecheck for `arm64_32-apple-watchos`.** The watch compiles
// AgentKit one file at a time and never the package
// (`apps/ios/generate-project.py:274-281`), so nothing here may reach for a
// type the watch does not have: no `UIColor`, no `NSColor`, no asset catalog,
// no dynamic colour provider. That is the reason the conversion below is plain
// arithmetic on `Double` and the reason light mode is a second literal rather
// than a system trait lookup.

/// One colour, in the co-ordinates the spec uses.
///
/// OKLCH is kept as the stored form rather than converted by hand into sRGB
/// components, and that is the whole point of this type. The spec's §09 review
/// pass records what happened when the swatch table printed bare numbers
/// without their function — "A spec exists to be copied; all twelve are now
/// complete `oklch()` strings" — and a table of pre-converted hex here would
/// reintroduce exactly that gap, since nobody comparing this file against the
/// design document could tell whether `#f7a224` was still `oklch(0.78 0.16 70)`
/// or had been nudged.
///
/// Lightness is 0…1, chroma is absolute (roughly 0…0.4 for anything a screen
/// can show), hue is degrees.
public struct OKLCH: Sendable, Equatable {
    public let lightness: Double
    public let chroma: Double
    public let hue: Double
    public let alpha: Double

    public init(_ lightness: Double, _ chroma: Double, _ hue: Double, alpha: Double = 1) {
        self.lightness = lightness
        self.chroma = chroma
        self.hue = hue
        self.alpha = alpha
    }

    /// The sRGB colour, converted here rather than at any call site.
    ///
    /// Björn Ottosson's Oklab matrices, in the order the transform runs:
    /// polar → Oklab, Oklab → cone responses, cubed, cone responses → linear
    /// sRGB, then the sRGB transfer function. Written out rather than pulled
    /// from a dependency because the watch's complication compiles four files
    /// and links no package, and because sixteen constants that never change
    /// are cheaper to read than a dependency's version.
    ///
    /// **Out-of-gamut components are clamped, not gamut-mapped**, which is a
    /// deliberate simplification and safe for exactly these twelve values: all
    /// sixteen colours in this file (twelve dark, four light) were checked to
    /// land inside sRGB with no component beyond ±0.0005, so no clamp fires
    /// today. It is here so that a value edited in the design document to
    /// something sRGB cannot hold degrades to the nearest displayable colour
    /// instead of to whatever a negative component renders as.
    public var color: Color {
        let a = chroma * cos(hue * .pi / 180)
        let b = chroma * sin(hue * .pi / 180)

        let lRoot = lightness + 0.3963377774 * a + 0.2158037573 * b
        let mRoot = lightness - 0.1055613458 * a - 0.0638541728 * b
        let sRoot = lightness - 0.0894841775 * a - 1.2914855480 * b

        let l = lRoot * lRoot * lRoot
        let m = mRoot * mRoot * mRoot
        let s = sRoot * sRoot * sRoot

        let red = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
        let green = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
        let blue = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

        return Color(
            .sRGB,
            red: Self.encode(red), green: Self.encode(green), blue: Self.encode(blue),
            opacity: alpha)
    }

    /// Linear light to sRGB, clamped into the unit interval.
    private static func encode(_ channel: Double) -> Double {
        let c = min(1, max(0, channel))
        return c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1 / 2.4) - 0.055
    }
}

/// One colour of the system, in both appearances.
///
/// Two literals rather than one colour and a filter, because §01 is explicit
/// that light mode is "Not a filter flip": amber DARKENS on a pale backdrop to
/// hold its contrast, the surfaces invert to translucent black, and the two
/// trace tones swap ends of the scale. A single value adjusted at draw time
/// cannot express any of that.
///
/// Resolved against `ColorScheme` at the call site rather than by a dynamic
/// colour provider, because the provider APIs are `UIColor`/`NSColor` and the
/// watch has neither. `@Environment(\.colorScheme)` is the one mechanism all
/// three platforms share.
public struct GlanceInk: Sendable, Equatable {
    public let dark: OKLCH
    public let light: OKLCH

    public init(dark: OKLCH, light: OKLCH) {
        self.dark = dark
        self.light = light
    }

    /// Same value in both appearances — the nine neutrals §01 gives one figure
    /// for.
    public init(_ both: OKLCH) {
        self.dark = both
        self.light = both
    }

    public func callAsFunction(_ scheme: ColorScheme) -> Color {
        (scheme == .dark ? dark : light).color
    }

    /// The dark value, for the surfaces that are dark whatever the phone is
    /// set to — a lock screen card sits over a wallpaper and takes no part in
    /// the appearance setting.
    public var darkColor: Color { dark.color }
}

/// §01, transcribed. Twelve values, and nothing else in the product may hold a
/// colour that belongs to a glance surface.
public enum GlancePalette {
    // MARK: - The one saturated hue, and the one that is not allowed to be loud

    /// Needs you. **Nothing else in the product may be amber, at any opacity.**
    ///
    /// This is the whole colour system: one reserved hue, so that "does this
    /// need me" is answered before a word is read. `FleetSnapshot.Glance`
    /// (`FleetSnapshot.swift:471-477`) is the enum that decides which surface
    /// earns it; this is the value.
    ///
    /// The light value is a genuinely different colour rather than the same one
    /// dimmed — §01: "amber darkens to oklch(0.62 0.13 68) to hold contrast on
    /// a pale backdrop."
    public static let amber = GlanceInk(
        dark: OKLCH(0.78, 0.16, 70), light: OKLCH(0.62, 0.13, 68))

    /// A finished diff, unread. Deliberately low chroma so amber stays the only
    /// loud thing.
    ///
    /// It reads as a pale blue rather than the cyan this codebase used to draw
    /// (`ShellBar.swift`'s `Color.cyan`), and the low chroma is the point: a
    /// saturated second hue competes with amber, and the moment two things are
    /// loud neither one is.
    public static let review = GlanceInk(
        dark: OKLCH(0.76, 0.055, 235), light: OKLCH(0.5, 0.06, 235))

    // MARK: - The activity trace's four tones
    //
    // Kept here in full although nothing draws a trace yet — the data for it
    // does not exist at any layer, which is a daemon question rather than a
    // view one. §09 argues for keeping all twelve rows of the table even though
    // a strict reading would cut it to the four that vary: "a value absent from
    // a spec gets invented at build time, and inventing an amber is the one
    // mistake that breaks the whole system." The same holds one level down. A
    // trace built later against a palette missing its own four tones is a trace
    // whose author picks four greys.

    /// Upper half of the trace — lines touched. Inverts in light mode.
    public static let code = GlanceInk(
        dark: OKLCH(0.88, 0.002, 250), light: OKLCH(0.28, 0.002, 250))

    /// Lower half of the trace — output to the person. Inverts in light mode.
    public static let chat = GlanceInk(
        dark: OKLCH(0.62, 0.002, 250), light: OKLCH(0.5, 0.002, 250))

    /// Commit marks on the trace axis.
    public static let commit = GlanceInk(OKLCH(0.96, 0.002, 250))

    /// The trace centre rule. Continuous, never dotted.
    public static let axis = GlanceInk(OKLCH(0.44, 0.002, 250))

    /// A bucket with no activity. Drawn, not omitted.
    public static let empty = GlanceInk(OKLCH(0.42, 0.002, 250))

    // MARK: - Ink

    /// Names, counts, anything you read first.
    public static let text1 = GlanceInk(OKLCH(0.97, 0.002, 250))

    /// Secondary lines inside a frame. **Floor for on-device text** — §01's
    /// contrast rule is that nothing below L 0.7 goes on the card, and this is
    /// L 0.74.
    public static let text2 = GlanceInk(OKLCH(0.74, 0.004, 250))

    // MARK: - Surfaces
    //
    // §01 gives one figure per surface for dark and a RANGE for light —
    // "Surfaces invert to black at 8% → 3%" — over the three surfaces in the
    // order the table lists them. The two ends are the spec's; the widget's
    // 5.5% is the midpoint of a two-point range read across three rows, and is
    // the one number in this file that is derived rather than quoted. If it is
    // wrong it is wrong by a percentage point of a translucent black, which is
    // the least consequential place for the gap to be — but it IS a gap, and
    // the design document is where it should be closed.

    /// Live Activity surface over the lock screen.
    public static let card = GlanceInk(
        dark: OKLCH(0.22, 0.004, 250, alpha: 0.9), light: OKLCH(0, 0, 0, alpha: 0.08))

    /// Home-screen widget surface.
    public static let widget = GlanceInk(
        dark: OKLCH(0.19, 0.004, 250, alpha: 0.95), light: OKLCH(0, 0, 0, alpha: 0.055))

    /// Dynamic Island fill.
    public static let island = GlanceInk(
        dark: OKLCH(0.12, 0.002, 250), light: OKLCH(0, 0, 0, alpha: 0.03))

    // MARK: - The two inks, where §01 stops

    /// The ink a mark's core is filled with, and anything else read first.
    ///
    /// **§01 gives `text 1` and `text 2` one figure each, and both are light
    /// inks for a dark surface.** Every light-mode value the spec DOES state is
    /// an inversion or a darkening — surfaces go to translucent black, amber
    /// darkens, review darkens, "Trace tones invert" — so the neutrals plainly
    /// invert too. It just does not say to what, and §09 already knows this
    /// section is unfinished: "Light mode is specified as values but not drawn.
    /// Worth one pass before build."
    ///
    /// Rendering the mark in light mode is what turned that from a note into a
    /// defect: `text 1` is L 0.97, so a present core on a pale widget is a
    /// near-white disc on a near-white ground, and "producing" and "at a
    /// prompt" — two of §03's three axes' worth of difference — become the same
    /// drawing.
    ///
    /// **So light mode defers to the system's own label colours rather than
    /// inventing two numbers.** `Color.primary` and `Color.secondary` are
    /// correct on any appearance by construction and are the same deferral the
    /// working rung already makes with `HierarchicalShapeStyle.tertiary`. Dark
    /// mode keeps the spec's figures exactly, which is the half the spec
    /// actually specifies. When the design document draws light mode, these two
    /// arms become literals like every other value in this file.
    public static func ink1(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? text1.darkColor : .primary
    }

    /// The secondary ink, on the same argument as `ink1`. Secondary lines,
    /// notes, and the quiet hairline ring.
    public static func ink2(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? text2.darkColor : .secondary
    }

    // MARK: - What a surface with room for one thing is about

    /// The tint for one rung of `FleetSnapshot.Glance`.
    ///
    /// **The single replacement for the two `glanceTint` functions** that used
    /// to live in `FleetWidget.swift` and `WatchFleetWidget.swift`, each of
    /// which carried a comment explaining that it had to be a copy because a
    /// colour is a SwiftUI type. It does not have to be a copy any more; it has
    /// to be in a SwiftUI file, and this is one.
    ///
    /// `AnyShapeStyle` because the three tints are not all colours: the working
    /// rung stays a hierarchical style, which is what lets the all-clear case
    /// recede against whatever wallpaper or watch face is behind it rather than
    /// sitting at a fixed grey that vanishes on one and shouts on the other.
    /// That reasoning survives the palette — §01 has no colour for "getting on
    /// with it", and it should not, because the answer is "whatever is behind
    /// this, slightly".
    public static func tint(_ glance: FleetSnapshot.Glance, _ scheme: ColorScheme) -> AnyShapeStyle
    {
        switch glance {
        case .blocked: AnyShapeStyle(amber(scheme))
        case .review: AnyShapeStyle(review(scheme))
        case .working: AnyShapeStyle(HierarchicalShapeStyle.tertiary)
        }
    }
}
