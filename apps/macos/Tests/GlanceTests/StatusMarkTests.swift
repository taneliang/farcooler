import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

// What the Mac's status column actually paints.
//
// **This suite exists because the claim it checks is a claim about pixels.**
// The glance vocabulary is a difference in ring weight, ring hue, dash and
// core — every one of which is invisible to a unit test that asks a view what
// it is. So this renders the mark and reads the ink back out of the bitmap,
// which is the only way "the Mac's amber is the same amber as the phone's" can
// be said with anything behind it. Reasoning about it is exactly the failure
// this repository keeps having: a check that passes because it cannot fail.
//
// It also writes the sheets a person looks at. Point `FARCOOLER_GLANCE_OUT` at
// a directory (default `.build/glance`) and run the suite; every state, at
// every size, in both appearances, lands there as a PNG.
//
// **Deliberately not in `CeremonyTests`.** That target's charter is "the rules
// with teeth, and only those — none of it is visible by looking". This is the
// opposite: it is looking, automated, and mixing the two would blur a boundary
// `Package.swift` argues for at length.

// MARK: - Rendering

/// One SwiftUI view, rasterised, in a named appearance.
///
/// `scale` is 16 rather than 2 on purpose. A quiet mark is a ONE POINT ring;
/// at 2× that is two device pixels, both of them antialiased against the
/// backdrop, and no sample taken from it is the colour the palette named. At
/// 16× the same ring is sixteen pixels across and its middle ones are the
/// literal value, so a mismatch is a mismatch and not a rounding argument.
@MainActor
private func raster<V: View>(
    _ view: V, size: CGSize, scheme: ColorScheme, scale: CGFloat = 16
) -> Raster {
    let renderer = ImageRenderer(
        content:
            view
            .frame(width: size.width, height: size.height)
            .background(scheme == .dark ? Color.black : Color.white)
            .environment(\.colorScheme, scheme)
    )
    renderer.scale = scale
    guard let cg = renderer.cgImage else {
        Issue.record("ImageRenderer produced no image")
        return Raster(width: 0, height: 0, pixels: [])
    }
    return Raster(cg)
}

/// A bitmap, in straight 8-bit sRGB, so a pixel is four bytes and nothing else.
///
/// Drawn into a context this file owns rather than read out of whatever
/// `cgImage` happened to produce: the renderer's own bitmap may be premultiplied,
/// float, or in the display's colour space, and every one of those turns a
/// comparison against an `oklch()`-derived value into a comparison against a
/// conversion of it.
private struct Raster {
    let width: Int
    let height: Int
    /// RGBA, row-major, straight alpha.
    let pixels: [UInt8]

    init(width: Int, height: Int, pixels: [UInt8]) {
        self.width = width
        self.height = height
        self.pixels = pixels
    }

    init(_ cg: CGImage) {
        width = cg.width
        height = cg.height
        let w = cg.width
        let h = cg.height
        var buffer = [UInt8](repeating: 0, count: w * h * 4)
        buffer.withUnsafeMutableBytes { raw in
            let context = CGContext(
                data: raw.baseAddress,
                width: w, height: h,
                bitsPerComponent: 8, bytesPerRow: w * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
        }
        pixels = buffer
    }

    func rgb(x: Int, y: Int) -> (Double, Double, Double) {
        let i = (y * width + x) * 4
        return (Double(pixels[i]) / 255, Double(pixels[i + 1]) / 255, Double(pixels[i + 2]) / 255)
    }

    /// The colour that most of the ink is, ignoring the backdrop.
    ///
    /// A histogram over every pixel, with anything within `nearBackdrop` of the
    /// given ground thrown away and the survivors bucketed at 1/255. Taking a
    /// single sample from a chosen coordinate was tried first and is the wrong
    /// tool: a 2pt ring on an 8pt mark leaves the sampler chasing a curve, and
    /// a test that fails because the geometry moved half a point is a test
    /// nobody keeps. The MODE is stable under any change that keeps the mark
    /// the same colour, which is the only thing this is asserting.
    func dominantInk(against ground: (Double, Double, Double), nearBackdrop: Double = 0.12)
        -> (Double, Double, Double)?
    {
        var counts: [Int: Int] = [:]
        for y in 0..<height {
            for x in 0..<width {
                let p = rgb(x: x, y: y)
                if distance(p, ground) < nearBackdrop { continue }
                let key =
                    (Int(p.0 * 255) << 16) | (Int(p.1 * 255) << 8) | Int(p.2 * 255)
                counts[key, default: 0] += 1
            }
        }
        guard let (key, _) = counts.max(by: { $0.value < $1.value }) else { return nil }
        return (
            Double((key >> 16) & 255) / 255,
            Double((key >> 8) & 255) / 255,
            Double(key & 255) / 255
        )
    }

    /// How much of this bitmap is not the backdrop.
    func inkFraction(against ground: (Double, Double, Double), nearBackdrop: Double = 0.12)
        -> Double
    {
        var lit = 0
        for y in 0..<height {
            for x in 0..<width where distance(rgb(x: x, y: y), ground) >= nearBackdrop { lit += 1 }
        }
        return Double(lit) / Double(width * height)
    }

    func png() -> Data? {
        var buffer = pixels
        return buffer.withUnsafeMutableBytes { raw -> Data? in
            guard
                let context = CGContext(
                    data: raw.baseAddress,
                    width: width, height: height,
                    bitsPerComponent: 8, bytesPerRow: width * 4,
                    space: CGColorSpace(name: CGColorSpace.sRGB)!,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
                let image = context.makeImage()
            else { return nil }
            let rep = NSBitmapImageRep(cgImage: image)
            return rep.representation(using: .png, properties: [:])
        }
    }
}

private func distance(_ a: (Double, Double, Double), _ b: (Double, Double, Double)) -> Double {
    max(abs(a.0 - b.0), max(abs(a.1 - b.1), abs(a.2 - b.2)))
}

/// A `Color`, in the same straight sRGB the raster is in.
@MainActor
private func components(_ color: Color) -> (Double, Double, Double) {
    let ns = NSColor(color).usingColorSpace(.sRGB) ?? .black
    return (Double(ns.redComponent), Double(ns.greenComponent), Double(ns.blueComponent))
}

/// Where the sheets land, so a person can open them.
private var outputDirectory: URL {
    let path =
        ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
        ?? FileManager.default.currentDirectoryPath + "/.build/glance"
    let url = URL(fileURLWithPath: path)
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

@MainActor
private func write(_ raster: Raster, _ name: String) {
    guard let png = raster.png() else {
        Issue.record("could not encode \(name)")
        return
    }
    let url = outputDirectory.appendingPathComponent(name)
    try? png.write(to: url)
    print("wrote \(url.path) — \(raster.width)×\(raster.height)")
}

// MARK: - The mapping

@Suite("The Mac's statuses, in the glance vocabulary")
struct StatusMarkMappingTests {
    /// The seven §03 has a slot for, and exactly what it is.
    ///
    /// Written out one by one rather than as a loop over a table, because a
    /// table is the mapping and asserting a mapping against itself is the
    /// check that cannot fail.
    @Test("A blocked agent is the heavy amber ring, with no core")
    func blockedIsNeedsYouAtAPrompt() {
        #expect(Status.blocked.glanceMark == GlanceMark(attention: .needsYou, core: .atAPrompt))
    }

    @Test("Working and starting are the quiet ring with a filled core")
    func producingStates() {
        let producing = GlanceMark(attention: .quiet, core: .producing)
        #expect(Status.working.glanceMark == producing)
        #expect(Status.starting.glanceMark == producing)
    }

    @Test("Idle, running and exited are the quiet hairline — drawn, not omitted")
    func quietStates() {
        let quiet = GlanceMark(attention: .quiet, core: .atAPrompt)
        #expect(Status.idle.glanceMark == quiet)
        #expect(Status.running.glanceMark == quiet)
        #expect(Status.exited.glanceMark == quiet)
    }

    /// The core is `nil` and not `.atAPrompt`, which draws the same and says
    /// something different. See `GlanceMark.core`.
    @Test("A runner that did not answer breaks the ring and states no core")
    func unreadableIsABrokenLink() {
        #expect(Status.unreadable.glanceMark == GlanceMark(attention: .quiet, core: nil, link: .broken))
        #expect(Status.unreadable.glanceMark?.core == nil)
        #expect(Status.unreadable.glanceMark?.link == .broken)
    }

    /// The reported disagreement, pinned so it cannot be closed by accident.
    ///
    /// If somebody later maps these onto the quiet hairline, this fails — and
    /// it should, because the hairline's own phrase is "Nothing wanted", which
    /// about a build that died overnight is a false statement in a vocabulary
    /// three platforms now trust.
    @Test("The five outcomes §03 has no slot for refuse the mark rather than borrowing one")
    func outcomesHaveNoMark() {
        for status in [Status.done, .failed, .failedRun, .failedTurn, .lost] {
            #expect(status.glanceMark == nil, "\(status.label) borrowed a mark it has no right to")
        }
    }

    /// `toReview` is fleet-level or Diff-level and nothing else — the rule
    /// `ShellScreen` and `ShellNavigation` both refuse to break. Nothing on
    /// this platform may reach it from a per-terminal status either.
    @Test("No per-agent status can ever produce the review ring")
    func noPerAgentReview() {
        let every: [Status] = [
            .starting, .running, .idle, .working, .blocked, .done, .exited,
            .failed, .lost, .failedRun, .failedTurn, .unreadable,
        ]
        for status in every {
            #expect(status.glanceMark?.attention != .toReview, "\(status.label) claimed to-review")
        }
    }
}

// MARK: - The pixels

@Suite("What the status column actually paints")
@MainActor
struct StatusMarkPixelTests {
    private static let dark = (0.0, 0.0, 0.0)
    private static let light = (1.0, 1.0, 1.0)

    private func ground(_ scheme: ColorScheme) -> (Double, Double, Double) {
        scheme == .dark ? Self.dark : Self.light
    }

    /// The whole point of the exercise: the Mac's amber IS the phone's amber.
    ///
    /// Not "matches the spec" by reading two numbers side by side — the ink is
    /// read back out of a rendered bitmap and compared against
    /// `GlancePalette.amber`, which is the same value `GlanceMarkView` draws on
    /// iOS, the widgets and the wrist.
    @Test("A blocked agent's ring is GlancePalette.amber, in both appearances")
    func blockedRingIsAmber() {
        for scheme in [ColorScheme.dark, .light] {
            let sheet = raster(
                StatusGlyph(status: .blocked), size: CGSize(width: 8, height: 8), scheme: scheme)
            let ink = sheet.dominantInk(against: ground(scheme))
            #expect(ink != nil, "nothing was drawn for a blocked agent in \(scheme)")
            guard let ink else { continue }
            let expected = components(GlancePalette.amber(scheme))
            #expect(
                distance(ink, expected) < 0.04,
                "\(scheme) ring is \(ink), GlancePalette.amber is \(expected)")
        }
    }

    /// Not amber, at any opacity. §01's first rule, checked from the pixels.
    @Test("A working agent's ring is the quiet ink and carries no amber at all")
    func workingRingIsQuiet() {
        for scheme in [ColorScheme.dark, .light] {
            let sheet = raster(
                StatusGlyph(status: .working), size: CGSize(width: 8, height: 8), scheme: scheme)
            let amber = components(GlancePalette.amber(scheme))
            for y in 0..<sheet.height {
                for x in 0..<sheet.width where distance(sheet.rgb(x: x, y: y), amber) < 0.06 {
                    Issue.record("amber at \(x),\(y) on a working mark in \(scheme)")
                    return
                }
            }
        }
    }

    /// The behaviour change, stated as a measurement rather than as a promise.
    ///
    /// An idle terminal used to render an empty frame. It now renders §03's
    /// hairline, so there is ink where there was none — and there is still far
    /// less of it than a blocked agent's ring, which is what "quiet" has to
    /// mean for the column to stay scannable.
    @Test("Idle is drawn, and is lighter than anything that wants you")
    func idleIsAHairlineAndNotAShout() {
        let size = CGSize(width: 8, height: 8)
        let idle = raster(StatusGlyph(status: .idle), size: size, scheme: .dark)
            .inkFraction(against: Self.dark)
        let blocked = raster(StatusGlyph(status: .blocked), size: size, scheme: .dark)
            .inkFraction(against: Self.dark)
        #expect(idle > 0.02, "an idle terminal drew nothing at all")
        #expect(idle < blocked / 1.5, "idle (\(idle)) is not quieter than blocked (\(blocked))")
    }

    /// A dashed ring, which is to say: less ink than the same ring solid.
    @Test("An unanswering runner's ring is broken")
    func unreadableRingIsDashed() {
        let size = CGSize(width: 8, height: 8)
        let solid = raster(StatusGlyph(status: .idle), size: size, scheme: .dark)
            .inkFraction(against: Self.dark)
        let dashed = raster(StatusGlyph(status: .unreadable), size: size, scheme: .dark)
            .inkFraction(against: Self.dark)
        #expect(dashed > 0, "an unanswering runner drew nothing")
        #expect(dashed < solid * 0.9, "dashed (\(dashed)) is not broken against solid (\(solid))")
    }

    /// The two states the mark cannot say still say it, in the colours they
    /// always did. If this starts failing, somebody folded an outcome into the
    /// hairline.
    @Test("A finished turn is still green and a dead one still red")
    func outcomesKeepTheirInk() {
        for (status, expected) in [(Status.done, Color.green), (Status.failedTurn, Color.red)] {
            let sheet = raster(
                StatusGlyph(status: status), size: CGSize(width: 8, height: 8), scheme: .dark)
            let ink = sheet.dominantInk(against: Self.dark)
            #expect(ink != nil, "\(status.label) drew nothing")
            guard let ink else { continue }
            #expect(
                distance(ink, components(expected)) < 0.08,
                "\(status.label) is \(ink), expected \(components(expected))")
        }
    }

    /// The sheets. Not an assertion — a rendering, so the change can be looked
    /// at rather than argued about.
    @Test("Write the specimen sheets")
    func writeSheets() {
        for scheme in [ColorScheme.dark, .light] {
            let name = scheme == .dark ? "dark" : "light"
            write(
                raster(StatusSpecimen(), size: StatusSpecimen.size, scheme: scheme, scale: 4),
                "status-column-\(name).png")
            write(
                raster(
                    SidebarSpecimen(title: "SIDEBAR") { StatusGlyph(status: $0) },
                    size: SidebarSpecimen<EmptyView>.size, scheme: scheme, scale: 4),
                "sidebar-\(name).png")
        }
    }
}

/// Every status this app can be in, at the two sizes it draws, with its word.
private struct StatusSpecimen: View {
    static let size = CGSize(width: 320, height: 380)

    private static let every: [Status] = [
        .blocked, .working, .starting, .idle, .running, .exited, .unreadable,
        .done, .failedTurn, .failedRun, .failed, .lost,
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("STATUS COLUMN")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.bottom, 8)
            ForEach(Array(Self.every.enumerated()), id: \.offset) { _, status in
                HStack(spacing: 9) {
                    StatusGlyph(status: status)
                    Text(status.label)
                        .font(.system(size: 12.5))
                        .frame(width: 110, alignment: .leading)
                    StatusGlyph(status: status, inAppDiameter: 6)
                    StatusGlyph(status: status, size: .lone)
                    Text(status.glanceMark.map(\.phrase) ?? "— no slot in §03 —")
                        .font(.system(size: 9))
                        .foregroundStyle(.tertiary)
                    Spacer(minLength: 0)
                }
                .frame(height: 26)
            }
        }
        .padding(14)
        .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
    }
}

// MARK: - The sidebar, which is where the owner looks

/// A terminal in a given state, decoded rather than constructed.
///
/// `Terminal` is `Decodable` and its initialiser is the synthesised one, so
/// JSON is how a test builds one without a second constructor existing in the
/// app purely for tests to call. The states are produced the way the app
/// produces them — through `StateKind` and `AgentActivity` — so this specimen
/// exercises `Terminal.status` rather than sidestepping it.
func terminal(_ label: String, _ json: String) -> Terminal {
    let body = """
        {"id":"t-\(label)","short":"\(label)","title":"\(label)","preset":"\(label)",
         "epoch":1,\(json)}
        """
    // swiftlint:disable:next force_try
    return try! JSONDecoder().decode(Terminal.self, from: Data(body.utf8))
}

let specimenTerminals: [Terminal] = [
    terminal("claude", #""state":"running","activity":"blocked""#),
    terminal("codex", #""state":"running","activity":"working""#),
    terminal("shell", #""state":"running","activity":"idle""#),
    terminal("cursor", #""state":"running","activity":"done","turnFailed":false"#),
    terminal("amp", #""state":"running","activity":"done","turnFailed":true"#),
    terminal("build", #""state":"exited","exitCode":1"#),
    terminal("server", #""state":"exited","exitCode":0"#),
    terminal("gone", #""state":"LOST""#),
    terminal("quiet", #""state":"unknown""#),
]

/// The sidebar's terminal rows, drawn with a glyph of the caller's choosing.
///
/// The row's own header layout — 8pt marker column, `markerGap` of 7, the name
/// at 12.5, the status word where `showsMeta` allows one — reproduced rather
/// than reused, and reproduced ONCE so the before sheet and the after sheet
/// differ in the glyph and in nothing else.
///
/// `TerminalRow` itself cannot be rasterised for this: it carries `.onDrag`,
/// and `ImageRenderer` resolves a drag source into its placeholder — a yellow
/// band with a `nosign` over it — which buries the one thing the sheet is for.
/// The marks in it are the real ones either way; what this reconstruction buys
/// is a comparison with nothing else moving.
struct SidebarSpecimen<Glyph: View>: View {
    static var size: CGSize { CGSize(width: 280, height: 300) }

    let title: String
    @ViewBuilder let glyph: (Status) -> Glyph

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.bottom, 6)
            ForEach(specimenTerminals) { t in
                HStack(spacing: 7) {
                    glyph(t.status)
                    Text(t.label).font(.system(size: 12.5)).lineLimit(1)
                    Spacer(minLength: 6)
                    if t.status.wantsAttention || t.status == .working {
                        Text(t.status.label)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                .padding(.vertical, 3)
                .padding(.horizontal, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
    }
}
