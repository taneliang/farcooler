import CoreText
import FarCoolerVT
import PhotosUI
import SwiftUI
import UIKit

/// The cell grid a font produces, and the insets the canvas draws inside.
///
/// Measured rather than assumed, the same reasoning as the Mac app's: this
/// device's Dynamic Type and Accessibility settings can change what "13pt
/// monospaced" measures to, and a hard-coded guess would send the host a
/// column count the screen cannot actually show.
private enum TerminalMetrics {
    static let padding = UIEdgeInsets(top: 6, left: 6, bottom: 6, right: 6)

    static func cell(_ font: UIFont) -> CGSize {
        // Every cell is the same box in a monospaced face, so one glyph's
        // measured width defines the grid.
        let width = ("M" as NSString).size(withAttributes: [.font: font]).width
        return CGSize(
            width: width.rounded(.up),
            height: (font.ascender - font.descender + font.leading).rounded(.up))
    }
}

/// Where the grid sits inside the canvas, and at what scale.
///
/// The same numbers `TerminalRenderer.draw` lays glyphs out with, and what
/// `TerminalView.cell(at:grid:size:)` inverts to turn a touch back into a
/// column and row. Kept as one calculation because the two had a habit of
/// drifting apart the moment they were two.
struct TerminalGridLayout {
    var scale: CGFloat
    var cell: CGSize
    var origin: CGPoint
}

/// Which character sits at which index inside which typeface, worked out once
/// and then never again.
///
/// This is the whole of why the terminal draws at frame rate. The renderer
/// used to hand SwiftUI a `Text` per cell — 2,090 of them on a 55×38 grid —
/// and each one is a full text pipeline: an attributed string, a `CTLine`,
/// shaping, a run, a rasterisation. Nothing about that work changes between
/// frames, or between the thousand cells on screen showing the letter `e`.
///
/// A glyph *index* does not depend on point size, so one cache serves every
/// size the pane is ever drawn at, including the fractional ones a reflow
/// produces. Only the sized `CTFont` handles are per-size, and there are two
/// of those.
///
/// ## Why this is also the ligature answer
///
/// A terminal must never show `!=` as `≠`: the grid is one glyph per cell at
/// an exact position, and a program that printed two characters must get two
/// characters. Batching cells into a shared `Text` — the obvious way to cut
/// 2,090 draws down to 38 — is exactly what turns those two characters into a
/// ligature.
///
/// Measured against the font this app actually bundles, `kCTLigatureAttributeName
/// = 0` does NOT prevent it. Iosevka Nerd Font Mono carries its coding
/// ligatures in `calt`, contextual alternates, not in `liga`; `CTLineCreate`
/// on `"!="` returns glyphs 28119 and 27864 — the two halves of `≠` — with
/// ligatures explicitly disabled just as it does without. The attribute
/// governs `liga`, and this font does not have a `liga` table at all.
///
/// Placing glyphs by index sidesteps the question rather than arguing with
/// it. `CTFontGetGlyphsForCharacters` on `"!"` and `"="` gives 4 and 32, the
/// plain forms, because it is a `cmap` lookup: there is no shaper in the path
/// to substitute anything, and no context for it to substitute on. See
/// `TerminalLigatureTests`.
@MainActor
final class TerminalGlyphCache {
    static let shared = TerminalGlyphCache()

    /// A character's glyph, and the face it had to be found in.
    struct Resolved: Equatable {
        /// nil for the terminal's own face; a PostScript name when the
        /// character was not in it and CoreText named a fallback.
        var fallback: String?
        var glyph: CGGlyph
    }

    private struct GlyphKey: Hashable {
        var character: Character
        var bold: Bool
        var choice: TerminalFontChoice
    }

    private struct FontKey: Hashable {
        var name: String?
        var bold: Bool
        var size: CGFloat
        var choice: TerminalFontChoice
    }

    /// `Resolved?` inside the dictionary, so a character that resolves to
    /// nothing is remembered as resolving to nothing rather than being
    /// re-asked on every frame — which is the case that would otherwise be
    /// the slowest one.
    private var glyphs: [GlyphKey: Resolved?] = [:]
    private var fonts: [FontKey: CTFont] = [:]
    private var colors: [UInt32: CGColor] = [:]
    private var cells: [FontKey: CGSize] = [:]
    private var baselines: [FontKey: CGFloat] = [:]

    /// The box one character occupies, measured once per font and size.
    ///
    /// `TerminalMetrics.cell` lays a glyph out to measure it, which is a text
    /// layout — and the renderer asks for this on every draw, as do the
    /// accessibility value and the keystroke sink beside it. Three text
    /// layouts a frame to re-derive a number that only changes when somebody
    /// opens Settings.
    func cell(bold: Bool = false, size: CGFloat, choice: TerminalFontChoice) -> CGSize {
        let key = FontKey(name: nil, bold: bold, size: size, choice: choice)
        if let cached = cells[key] { return cached }
        let measured = TerminalMetrics.cell(.terminal(choice, size: size, bold: bold))
        if cells.count > 24 { cells.removeAll(keepingCapacity: true) }
        cells[key] = measured
        return measured
    }

    /// How far below the top of a row its glyphs sit, for the same font and
    /// size `cell(bold:size:choice:)` measured the row from — the two are the
    /// same arithmetic read in opposite directions, so they are answered from
    /// the same place.
    func baseline(size: CGFloat, choice: TerminalFontChoice) -> CGFloat {
        let key = FontKey(name: nil, bold: false, size: size, choice: choice)
        if let cached = baselines[key] { return cached }
        let face = UIFont.terminal(choice, size: size)
        let answer = face.leading + face.ascender
        if baselines.count > 24 { baselines.removeAll(keepingCapacity: true) }
        baselines[key] = answer
        return answer
    }

    /// The sized face to draw a run with.
    func font(
        fallback: String?, bold: Bool, size: CGFloat, choice: TerminalFontChoice
    ) -> CTFont {
        let key = FontKey(name: fallback, bold: bold, size: size, choice: choice)
        if let cached = fonts[key] { return cached }
        let font: CTFont
        if let fallback {
            font = CTFontCreateWithName(fallback as CFString, size, nil)
        } else {
            font = UIFont.terminal(choice, size: size, bold: bold) as CTFont
        }
        // A reflow walks `size` through a range of fractional values, and
        // every one of them would otherwise be remembered for the life of the
        // app. Two dozen is more than any pane needs and small enough that
        // throwing the lot away costs nothing.
        if fonts.count > 24 { fonts.removeAll(keepingCapacity: true) }
        fonts[key] = font
        return font
    }

    /// The colour the core packed, as CoreGraphics wants it.
    ///
    /// Cached because a terminal has sixteen colours, or two hundred and
    /// fifty-six of them, and building a `CGColor` per run per frame is a
    /// per-frame allocation for a value that never changes.
    func color(_ packed: UInt32) -> CGColor {
        if let cached = colors[packed] { return cached }
        // sRGB by name, not `CGColor(red:green:blue:alpha:)`, which is generic
        // device RGB. `Color(packed:)` is a SwiftUI `Color(red:green:blue:)`
        // and that is sRGB, so the device variant drew every cell of the grid
        // in a colour a few values off the one the rest of the app uses — a
        // difference too small to notice by eye and large enough that a
        // pixel comparison of the two drawing paths found 88% of the screen
        // disagreeing.
        let color = CGColor(
            colorSpace: Self.srgb,
            components: [
                CGFloat((packed >> 16) & 0xFF) / 255,
                CGFloat((packed >> 8) & 0xFF) / 255,
                CGFloat(packed & 0xFF) / 255,
                1,
            ]) ?? CGColor(gray: 0, alpha: 1)
        colors[packed] = color
        return color
    }

    private static let srgb = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    /// The glyph one character maps to, or nil when no single glyph can stand
    /// for it.
    ///
    /// nil is not a failure: a ZWJ emoji or a base-plus-combining-mark pair is
    /// genuinely more than one glyph, and the renderer sends those back through
    /// SwiftUI's own text drawing, which is slow and correct. There are never
    /// many on a terminal screen.
    func glyph(
        for character: Character, bold: Bool, choice: TerminalFontChoice
    ) -> Resolved? {
        let key = GlyphKey(character: character, bold: bold, choice: choice)
        if let cached = glyphs[key] { return cached }
        let answer = resolve(character, bold: bold, choice: choice)
        // A ceiling, because the key is a `Character` and the alphabet a
        // terminal can print is the whole of Unicode. Four thousand is every
        // character any pane has ever shown and then some; the cost of being
        // wrong about that is re-resolving them, which is a `cmap` lookup.
        if glyphs.count > 4096 { glyphs.removeAll(keepingCapacity: true) }
        glyphs[key] = answer
        return answer
    }

    private func resolve(
        _ character: Character, bold: Bool, choice: TerminalFontChoice
    ) -> Resolved? {
        let units = Array(String(character).utf16)
        // One UTF-16 unit, or a surrogate pair. Anything longer is a sequence,
        // and a sequence is not one glyph.
        guard units.count == 1 || units.count == 2 else { return nil }
        // Any size at all: a glyph index is a property of the typeface, not of
        // the size it is drawn at.
        let base = font(fallback: nil, bold: bold, size: 16, choice: choice)
        if let glyph = single(of: units, in: base) {
            return Resolved(fallback: nil, glyph: glyph)
        }
        // Not in the terminal face — a CJK ideograph, a box-drawing character
        // some faces lack. Ask CoreText which face does have it, once, and
        // remember the answer by name so the sized handle can be rebuilt at
        // any scale.
        let fallback = CTFontCreateForString(
            base, String(character) as CFString, CFRangeMake(0, units.count))
        guard
            let glyph = single(of: units, in: fallback),
            let name = CTFontCopyPostScriptName(fallback) as String?
        else { return nil }
        return Resolved(fallback: name, glyph: glyph)
    }

    /// The one non-zero glyph these units map to, or nil.
    ///
    /// A surrogate pair reports its glyph in the first slot and zero in the
    /// second, so "exactly one non-zero" is the test for both lengths. Zero is
    /// `.notdef`, which is CoreText's way of saying this face does not have
    /// the character.
    private func single(of units: [UTF16.CodeUnit], in font: CTFont) -> CGGlyph? {
        var found = [CGGlyph](repeating: 0, count: units.count)
        _ = CTFontGetGlyphsForCharacters(font, units, &found, units.count)
        let real = found.filter { $0 != 0 }
        return real.count == 1 ? real[0] : nil
    }
}

/// How the grid becomes pixels.
///
/// Production always draws `.glyphs`. The other two exist so the cost of this
/// one can be stated as a number rather than asserted, and so the ligature
/// guard has something that actually fires a ligature to be broken against —
/// see `TerminalBenchHarness`.
enum TerminalDrawPath: String {
    /// Glyph indices placed at cell positions. One CoreText call per distinct
    /// (face, colour) on the whole screen, and no shaper anywhere in the path.
    case glyphs
    #if DEBUG
    /// One `context.draw(Text)` per cell, which is what this renderer did
    /// before. Kept only as the measurement's other end.
    case cellText
    /// One `Text` per row — the obvious batching, and the wrong one. This
    /// fires Iosevka's `calt` ligatures, so a row reading `a != b` draws `≠`.
    /// It is here to be failed against.
    case rowText
    #endif
}

/// The grid, in points.
///
/// Split out of `TerminalView` so that a benchmark and a ligature fixture can
/// drive exactly the drawing the app ships, rather than a copy of it that can
/// drift.
@MainActor
struct TerminalRenderer {
    var fontChoice: TerminalFontChoice
    var fontSize: CGFloat
    var path: TerminalDrawPath = .glyphs

    /// The cell grid the current font and size produce.
    var cellSize: CGSize {
        TerminalGlyphCache.shared.cell(size: fontSize, choice: fontChoice)
    }

    /// Almost never a straight reflow: the pane resizes itself to roughly fit
    /// (see `TerminalSession.configure`), but the two round trips that takes —
    /// asking the host, the host reflowing tmux — leave a gap where the grid
    /// on screen is still the OLD size, and this scales that leftover
    /// mismatch down rather than cropping it. `scale` is 1 the rest of the
    /// time.
    func layout(for grid: TerminalGrid, in size: CGSize) -> TerminalGridLayout {
        let padding = TerminalMetrics.padding
        let cell = cellSize
        let content = CGSize(
            width: padding.left + padding.right + CGFloat(grid.columns) * cell.width,
            height: padding.top + padding.bottom + CGFloat(grid.rows) * cell.height)
        guard content.width > 0, content.height > 0 else {
            return TerminalGridLayout(scale: 1, cell: cell, origin: .zero)
        }
        let scale = min(size.width / content.width, size.height / content.height, 1)
        return TerminalGridLayout(
            scale: scale,
            cell: CGSize(width: cell.width * scale, height: cell.height * scale),
            origin: CGPoint(x: padding.left * scale, y: padding.top * scale))
    }

    /// `scrolledBy` is the sub-row nudge the scroll driver is asking for, in
    /// points, measured from the row the emulator is on. Everything below is
    /// laid out from `originY`, so adding it there moves the glyphs, the
    /// background runs and the cursor as one thing — which is the only way the
    /// grid can be between two rows without coming apart.
    ///
    /// Clamped to the canvas, because a number that large could only come from
    /// a bug and the failure mode of not clamping is a pane that draws nothing
    /// at all. The value in ordinary use is under half a row, or — past an end
    /// — whatever the rubberband is currently allowing.
    ///
    /// Scaled by hand — every position and font size below is multiplied by
    /// `scale` directly — rather than through `GraphicsContext.scaleBy`. The
    /// cell size comes from a fixed font, which is right for a Mac window sized
    /// to its own terminal and wrong here: the phone renders whatever grid the
    /// HOST has — 75 columns is normal — and at 13pt that is far wider than any
    /// phone. `scaleBy` looks like the right tool and reads as one, but it did
    /// not touch glyphs drawn through `context.draw(_ text: Text, at:anchor:)`:
    /// a scale of 0.69 was computed correctly and applied to the context on
    /// every frame while the glyphs kept landing at their full, untransformed
    /// size. So the fills and the text agree by construction: both are laid out
    /// in coordinates that already have `scale` baked in.
    func draw(
        grid: TerminalGrid, into context: GraphicsContext, size: CGSize,
        scrolledBy: CGFloat = 0
    ) {
        context.fill(
            Path(CGRect(origin: .zero, size: size)),
            with: .color(TerminalPalette.background))
        let layout = layout(for: grid, in: size)
        guard layout.cell.width > 0, layout.cell.height > 0 else { return }
        let origin = CGPoint(
            x: layout.origin.x,
            y: layout.origin.y + min(max(scrolledBy, -size.height), size.height))

        switch path {
        case .glyphs:
            drawByGlyph(grid: grid, into: context, layout: layout, origin: origin)
        #if DEBUG
        case .cellText:
            drawByCellText(grid: grid, into: context, layout: layout, origin: origin)
        case .rowText:
            drawByRowText(grid: grid, into: context, layout: layout, origin: origin)
        #endif
        }
    }

    // MARK: - The glyph path

    /// One character that no single glyph stands for, on its way back to
    /// SwiftUI's text drawing.
    private struct Leftover {
        var character: Character
        var point: CGPoint
        var color: Color
        var bold: Bool
    }

    private struct GlyphBatch: Hashable {
        var fallback: String?
        var bold: Bool
        var color: UInt32
    }

    /// Where a glyph goes, in the space CoreText will read it in.
    ///
    /// The y is negated because the text matrix below is, and the positions
    /// are transformed by it: see the note on `cg.textMatrix`. Kept as one
    /// named function so the negation happens in exactly one place — the
    /// cursor's glyph is placed by the same rule as every other one, and a
    /// cursor half a screen from its own block is the bug this prevents.
    static func textSpace(x: CGFloat, baseline: CGFloat) -> CGPoint {
        CGPoint(x: x, y: -baseline)
    }

    /// The glyphs of one batch and where each of them goes.
    ///
    /// Appended to through `Dictionary`'s defaulting subscript, which mutates
    /// the stored value in place. Reading the pair out, appending to it and
    /// writing it back would hold a second reference to both arrays across the
    /// append and so copy them — every glyph, every frame.
    private struct GlyphRun {
        var glyphs: [CGGlyph] = []
        var positions: [CGPoint] = []

        mutating func append(_ glyph: CGGlyph, at position: CGPoint) {
            glyphs.append(glyph)
            positions.append(position)
        }
    }

    private func drawByGlyph(
        grid: TerminalGrid, into context: GraphicsContext, layout: TerminalGridLayout,
        origin: CGPoint
    ) {
        let cache = TerminalGlyphCache.shared
        let ground = TerminalPalette.backgroundPacked
        let cell = layout.cell
        let scaledSize = fontSize * layout.scale
        // The baseline the glyphs sit on. `TerminalMetrics.cell` builds the
        // row's height out of ascent, descent and leading, and CoreText puts
        // the leading above the ascent, so this is the same arithmetic read
        // downwards from the top of the row.
        let baseline = cache.baseline(size: scaledSize, choice: fontChoice)

        // Filled by colour across the WHOLE grid rather than per row: a screen
        // has a handful of distinct colours and thousands of cells, and
        // `CGContext.fill(_ rects:)` takes the lot in one call.
        var fills: [UInt32: [CGRect]] = [:]
        var batches: [GlyphBatch: GlyphRun] = [:]
        var leftovers: [Leftover] = []

        for row in 0..<grid.rows {
            let y = origin.y + CGFloat(row) * cell.height
            var column = 0
            while column < grid.columns {
                let background = grid[row, column].backgroundPacked
                var end = column + 1
                while end < grid.columns, grid[row, end].backgroundPacked == background {
                    end += 1
                }
                if background != ground {
                    fills[background, default: []].append(
                        CGRect(
                            x: origin.x + CGFloat(column) * cell.width, y: y,
                            width: CGFloat(end - column) * cell.width, height: cell.height))
                }
                column = end
            }

            column = 0
            while column < grid.columns {
                let occupant = grid[row, column]
                let x = origin.x + CGFloat(column) * cell.width
                if let character = occupant.character {
                    if let resolved = cache.glyph(
                        for: character, bold: occupant.bold, choice: fontChoice)
                    {
                        let key = GlyphBatch(
                            fallback: resolved.fallback, bold: occupant.bold,
                            color: occupant.foregroundPacked)
                        batches[key, default: GlyphRun()].append(
                            resolved.glyph, at: Self.textSpace(x: x, baseline: y + baseline))
                    } else {
                        leftovers.append(
                            Leftover(
                                character: character, point: CGPoint(x: x, y: y),
                                color: occupant.foreground, bold: occupant.bold))
                    }
                }
                // A wide character's neighbour is its own spacer; drawing it
                // too would put a second, blank glyph where the wide one
                // already reaches.
                column += occupant.wide ? 2 : 1
            }
        }

        // The cursor, resolved before the CoreGraphics pass so the block and
        // the glyph inside it can be drawn in the same one.
        var cursor: (rect: CGRect, glyph: TerminalGlyphCache.Resolved?, character: Character?)?
        if grid.cursorRow < grid.rows, grid.cursorColumn < grid.columns {
            let occupant = grid[grid.cursorRow, grid.cursorColumn]
            let rect = CGRect(
                x: origin.x + CGFloat(grid.cursorColumn) * cell.width,
                y: origin.y + CGFloat(grid.cursorRow) * cell.height,
                width: occupant.wide ? cell.width * 2 : cell.width,
                height: cell.height)
            let resolved = occupant.character.flatMap {
                cache.glyph(for: $0, bold: false, choice: fontChoice)
            }
            cursor = (rect, resolved, occupant.character)
        }

        context.withCGContext { cg in
            for (packed, rects) in fills {
                cg.setFillColor(cache.color(packed))
                cg.fill(rects)
            }
            // A flipped text matrix, because a `Canvas` hands out a context
            // whose y grows downwards and CoreText draws glyphs upwards from
            // their baseline. Without it every glyph is mirrored about its own
            // baseline.
            //
            // The flip is not free of consequence, and the consequence is the
            // whole of `textSpace(x:baseline:)`: glyph positions handed to
            // CoreText are in TEXT space, so they go through this matrix too,
            // and a baseline of 40 lands at user-space y of -40. The first
            // version of this drew a perfectly correct screenful of glyphs
            // just above the top edge of the canvas, which looks exactly like
            // drawing nothing at all.
            cg.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
            for (key, run) in batches {
                cg.setFillColor(cache.color(key.color))
                let font = cache.font(
                    fallback: key.fallback, bold: key.bold, size: scaledSize,
                    choice: fontChoice)
                CTFontDrawGlyphs(font, run.glyphs, run.positions, run.glyphs.count, cg)
            }
            if let cursor {
                cg.setFillColor(cache.color(TerminalPalette.cursorPacked))
                cg.fill([cursor.rect])
                if let resolved = cursor.glyph {
                    cg.setFillColor(cache.color(ground))
                    let font = cache.font(
                        fallback: resolved.fallback, bold: false, size: scaledSize,
                        choice: fontChoice)
                    var glyph = resolved.glyph
                    var position = Self.textSpace(
                        x: cursor.rect.minX, baseline: cursor.rect.minY + baseline)
                    CTFontDrawGlyphs(font, &glyph, &position, 1, cg)
                }
            }
        }

        // After the CoreGraphics pass, so a character SwiftUI has to draw
        // still lands on top of the background behind it. Empty on almost
        // every frame of almost every pane, which is why the two fonts are
        // built here rather than at the top: nothing about this branch should
        // cost anything on a screen that never enters it.
        //
        // Written out with `if let` rather than as `cursor?.glyph == nil`,
        // which reads correctly and is not: optional chaining on an optional
        // tuple gives `Resolved??`, and comparing THAT to nil asks whether the
        // cursor exists, not whether its glyph resolved. It is false whenever
        // there is a cursor at all, so the branch below would never run.
        var cursorNeedsText = false
        if let cursor, cursor.glyph == nil, cursor.character != nil { cursorNeedsText = true }
        guard !leftovers.isEmpty || cursorNeedsText else { return }
        let regular = Font.terminal(fontChoice, size: scaledSize)
        let bold = Font.terminal(fontChoice, size: scaledSize, bold: true)
        for leftover in leftovers {
            context.draw(
                Text(String(leftover.character))
                    .font(leftover.bold ? bold : regular)
                    .foregroundColor(leftover.color),
                at: leftover.point, anchor: .topLeading)
        }
        if let cursor, cursor.glyph == nil, let character = cursor.character {
            context.draw(
                Text(String(character)).font(regular)
                    .foregroundColor(TerminalPalette.background),
                at: cursor.rect.origin, anchor: .topLeading)
        }
    }

    #if DEBUG
    // MARK: - The two paths that exist to be compared against

    /// What this renderer did before: a `Text` per cell.
    private func drawByCellText(
        grid: TerminalGrid, into context: GraphicsContext, layout: TerminalGridLayout,
        origin: CGPoint
    ) {
        let cell = layout.cell
        let regularFont = Font.terminal(fontChoice, size: fontSize * layout.scale)
        let boldFont = Font.terminal(fontChoice, size: fontSize * layout.scale, bold: true)
        for row in 0..<grid.rows {
            let y = origin.y + CGFloat(row) * cell.height
            var column = 0
            while column < grid.columns {
                let background = grid[row, column].background
                var end = column + 1
                while end < grid.columns, grid[row, end].background == background { end += 1 }
                if background != TerminalPalette.background {
                    context.fill(
                        Path(
                            CGRect(
                                x: origin.x + CGFloat(column) * cell.width, y: y,
                                width: CGFloat(end - column) * cell.width,
                                height: cell.height)),
                        with: .color(background))
                }
                column = end
            }
            column = 0
            while column < grid.columns {
                let occupant = grid[row, column]
                if let character = occupant.character {
                    context.draw(
                        Text(String(character))
                            .font(occupant.bold ? boldFont : regularFont)
                            .foregroundColor(occupant.foreground),
                        at: CGPoint(x: origin.x + CGFloat(column) * cell.width, y: y),
                        anchor: .topLeading)
                }
                column += occupant.wide ? 2 : 1
            }
        }
        drawCursorAsText(grid: grid, into: context, layout: layout, origin: origin, font: regularFont)
    }

    /// The obvious batching, and the wrong one: one `Text` per row. Fewer
    /// draws, and Iosevka's `calt` turns `!=` into `≠` along the way.
    private func drawByRowText(
        grid: TerminalGrid, into context: GraphicsContext, layout: TerminalGridLayout,
        origin: CGPoint
    ) {
        let cell = layout.cell
        let font = Font.terminal(fontChoice, size: fontSize * layout.scale)
        for row in 0..<grid.rows {
            let y = origin.y + CGFloat(row) * cell.height
            var line = ""
            for column in 0..<grid.columns {
                line.append(grid[row, column].character ?? " ")
            }
            guard !line.allSatisfy({ $0 == " " }) else { continue }
            context.draw(
                Text(line).font(font).foregroundColor(grid[row, 0].foreground),
                at: CGPoint(x: origin.x, y: y), anchor: .topLeading)
        }
        drawCursorAsText(grid: grid, into: context, layout: layout, origin: origin, font: font)
    }

    private func drawCursorAsText(
        grid: TerminalGrid, into context: GraphicsContext, layout: TerminalGridLayout,
        origin: CGPoint, font: Font
    ) {
        guard grid.cursorRow < grid.rows, grid.cursorColumn < grid.columns else { return }
        let occupant = grid[grid.cursorRow, grid.cursorColumn]
        let rect = CGRect(
            x: origin.x + CGFloat(grid.cursorColumn) * layout.cell.width,
            y: origin.y + CGFloat(grid.cursorRow) * layout.cell.height,
            width: occupant.wide ? layout.cell.width * 2 : layout.cell.width,
            height: layout.cell.height)
        context.fill(Path(rect), with: .color(TerminalPalette.cursor))
        if let character = occupant.character {
            context.draw(
                Text(String(character)).font(font)
                    .foregroundColor(TerminalPalette.background),
                at: rect.origin, anchor: .topLeading)
        }
    }
    #endif
}

/// The physics a scroll on this platform is expected to have.
///
/// Every number here is borrowed rather than invented, and that is the point of
/// the file it lives in. WWDC 2018 "Designing Fluid Interfaces" makes the
/// argument directly, about a picture-in-picture window being thrown: *"I
/// already have a lot of muscle memory for doing exactly that with scrolling…
/// by taking advantage of that here, we're reinforcing things that people have
/// learned elsewhere."* A terminal that decelerated on a curve of its own would
/// be a fifth motion vocabulary in an app that already has too many, and every
/// thumb arriving at it has spent years learning `UIScrollView`'s.
///
/// So: `UIScrollView.DecelerationRate.normal` for the throw, UIScrollView's own
/// progressive resistance curve for the edges, and a critically damped spring
/// for the return.
enum TerminalScrollPhysics {
    /// `UIScrollView.DecelerationRate.normal`, as a plain number.
    ///
    /// It is 0.998, and the unit is "fraction of the velocity surviving one
    /// millisecond" — which is why every formula below has a 1000 in it.
    static let decelerationRate = UIScrollView.DecelerationRate.normal.rawValue

    /// Where content thrown at `velocity` would come to rest.
    ///
    /// The talk's projection function, in the units it was given in: velocity
    /// per second in, distance out. It is what makes a flick travel further
    /// than the finger did — without it a swipe moves the content exactly as
    /// far as the thumb moved and no further, which the talk names as the
    /// counter-example: *"those same swipes wouldn't get you very far… you'd
    /// have to do these long, laborious swipes."*
    static func projection(
        of velocity: CGFloat, decelerationRate rate: CGFloat = decelerationRate
    ) -> CGFloat {
        guard rate > 0, rate < 1 else { return 0 }
        return (velocity / 1000) * rate / (1 - rate)
    }

    /// One `dt`-second step of that same deceleration.
    ///
    /// Integrated in closed form rather than stepped as `v *= rate` per frame,
    /// so a dropped frame costs the same distance as two delivered ones. The
    /// distance this returns over an infinite `dt` is exactly `projection(of:)`
    /// — the coast and the throw distance are the same curve, stated twice.
    static func decelerate(
        velocity: CGFloat, dt: CGFloat, decelerationRate rate: CGFloat = decelerationRate
    ) -> (travelled: CGFloat, velocity: CGFloat) {
        guard rate > 0, rate < 1, dt > 0 else { return (0, velocity) }
        let decay = pow(rate, dt * 1000)
        return (velocity * (decay - 1) / (1000 * log(rate)), velocity * decay)
    }

    /// `UIScrollView`'s resistance past an edge.
    ///
    /// Progressive, not a linear multiplier: the first point of over-drag is
    /// nearly free and the hundredth is nearly immovable, so the boundary
    /// announces itself by getting harder rather than by arriving. The talk,
    /// on what happens without it: *"you actually wouldn't know the difference
    /// between a frozen phone, and phone that's just at the top of the edge of
    /// the screen."*
    ///
    /// `dimension` is the viewport the content is being dragged across, which
    /// is what makes the curve feel the same on a phone and on an iPad.
    static func rubberband(
        _ overshoot: CGFloat, dimension: CGFloat, coefficient: CGFloat = 0.55
    ) -> CGFloat {
        guard dimension > 0 else { return 0 }
        let resisted =
            (1 - (1 / ((abs(overshoot) * coefficient / dimension) + 1))) * dimension
        return overshoot < 0 ? -resisted : resisted
    }

    /// One `dt`-second step of a critically damped spring towards `target`.
    ///
    /// Critically damped — no bounce — because nothing this settles has any
    /// bounce to earn: content returning from an over-drag is elastic, not
    /// springy, and content landing on a row boundary at the end of a coast
    /// must not announce that it arrived. Solved analytically for the same
    /// reason `decelerate` is: it is exact at any `dt`, including the long one
    /// after a dropped frame.
    static func settle(
        value: CGFloat, velocity: CGFloat, toward target: CGFloat, response: CGFloat,
        dt: CGFloat
    ) -> (value: CGFloat, velocity: CGFloat) {
        guard response > 0, dt > 0 else { return (target, 0) }
        let omega = 2 * CGFloat.pi / response
        let displacement = value - target
        let slope = velocity + omega * displacement
        let decay = exp(-omega * dt)
        return (
            target + (displacement + slope * dt) * decay,
            (velocity - omega * slope * dt) * decay
        )
    }
}

/// What a scroll in progress is doing to the drawn grid, published so the
/// `Canvas` can offset itself by it and so a UI test can see it at all.
///
/// **Relative to the row the core is on, never absolute.** The emulator's
/// `display_offset` is the authority on WHICH lines are on screen; this is only
/// the sub-row nudge between them, and stating it that way is what makes the
/// two structurally unable to drift apart — the worst a stale reading can be is
/// half a row, and there is nowhere for an error to accumulate.
///
/// The three peaks exist because a `Canvas` is pixels: there is no child view
/// for a test to read, and "the content tracked the thumb between two rows" and
/// "the content quantised to rows" produce the same emulator offset and the
/// same screenshot. Only a number the drawing itself was given can tell them
/// apart. See `TerminalScrollTests`.
struct TerminalScrollReadout: Equatable {
    /// How far the grid is drawn from the row the core is on, in points.
    var offset: CGFloat = 0
    /// The largest `offset` yet drawn while INSIDE the scrollback's own
    /// bounds — that is, the deepest the grid has ever sat between two rows.
    /// Zero for the whole lifetime of a pane that quantises.
    var grainPeak: CGFloat = 0
    /// The largest over-drag past either end, in points.
    var bandPeak: CGFloat = 0
    /// How far past an end the content is right now. Returns to zero, which is
    /// the half of rubberbanding that is not resistance.
    var over: CGFloat = 0
}

/// Shows one terminal, live.
///
/// Everything about what an escape sequence means happened before this view
/// ever sees a byte — `TerminalSession` hands it a grid of already-resolved
/// cells. What is left here is genuinely platform work: laying that grid out
/// in points, and turning touches into the bytes a program is waiting to
/// read. It never parses an escape sequence and never decides what an arrow
/// key sends, matching the contract the C header states for every renderer.
@MainActor
struct TerminalView: View {
    @ObservedObject var connection: Connection
    @Environment(\.scenePhase) private var scenePhase

    @StateObject private var session: TerminalSession
    /// The terminal this pane shows. Fixed for the pane's lifetime.
    ///
    /// This used to be `@State` that the tab strip reassigned, because one
    /// `TerminalView` was reused for every pane in the workspace. It is a `let`
    /// now: `ShellPaneTrack` keeps one of these per retained pane and shows the
    /// current one, so a pane never becomes a different pane.
    let terminal: Terminal
    /// Whether this pane is the one on screen.
    ///
    /// The pane stays MOUNTED when it is not — that is the whole point, and
    /// what keeps its scroll offset, its folds and its grid — but nothing that
    /// costs the network or the host may keep running. `onAppear`/`onDisappear`
    /// cannot express that: they stop firing once a view is merely hidden
    /// rather than removed. Everything that used to hang off them hangs off
    /// this instead.
    let isVisible: Bool
    @State private var ctrlArmed = false
    @State private var altArmed = false
    @State private var focusRequest = 0
    @State private var dismissRequest = 0
    /// The URL a long press landed on, which is also what presents the dialog.
    ///
    /// One value rather than a flag plus a string: a dialog that can be shown
    /// with nothing to show is a dialog that will eventually be shown with
    /// nothing to show.
    @State private var heldLink: String?

    /// How far the grid is drawn from the row the emulator is on, plus the
    /// peaks a test reads. Written by the scroll driver in `KeystrokeSink`,
    /// once per frame while anything is moving and never otherwise.
    @State private var scrollReadout = TerminalScrollReadout()

    // User-configurable, unlike the Mac's fixed terminal font: see
    // Settings.swift. Read directly from the same `UserDefaults` key
    // `SettingsView`'s controls write to, so a change there is picked up the
    // next time SwiftUI redraws this screen — no delegate, no notification,
    // nothing to keep in sync by hand.
    @AppStorage(TerminalSettings.fontKey) private var fontChoice: TerminalFontChoice = .iosevka
    @AppStorage(TerminalSettings.fontSizeKey) private var fontSize: Double = TerminalSettings.defaultFontSize

    /// The cell grid the CURRENT font and size produce.
    ///
    /// Computed, not cached: a cached size is exactly what went stale the
    /// moment the settings screen changed either `@AppStorage` value out from
    /// under it, and a stale cell size is a grid that no longer lines up with
    /// what `columns(for:)`/`rows(for:)` and `draw(grid:into:size:)` assume
    /// about it.
    private var cellSize: CGSize { renderer.cellSize }

    /// The drawing, as a value.
    ///
    /// Built fresh on each access for the same reason `cellSize` was computed
    /// rather than cached: both `@AppStorage` values behind it can change out
    /// from under this view at any moment, and a renderer holding a stale
    /// point size is a grid that no longer lines up with what
    /// `columns(for:)`/`rows(for:)` assume about it. It carries no state — the
    /// glyphs and the sized faces live in `TerminalGlyphCache.shared`, which
    /// is what makes rebuilding this per frame free.
    private var renderer: TerminalRenderer {
        TerminalRenderer(fontChoice: fontChoice, fontSize: fontSize)
    }

    /// Images on their way into this pane, owned by `ShellScreen` so a transfer
    /// started on one pane survives switching to another.
    @ObservedObject var pastes: ImagePasteQueue

    init(
        terminal: Terminal, isVisible: Bool, connection: Connection, pastes: ImagePasteQueue
    ) {
        self.connection = connection
        self.terminal = terminal
        self.isVisible = isVisible
        self.pastes = pastes
        _session = StateObject(
            wrappedValue: TerminalSession(terminalID: terminal.id, core: connection.core))
    }

    /// The workspace `current` actually lives in, looked up fresh each time
    /// rather than carried in from wherever `current` was set — a workspace
    /// the tab strip or the switcher sheet points at is only ever known to
    /// this screen by its terminal's id.
    /// The terminal as the daemon describes it RIGHT NOW.
    ///
    /// `current` is a `@State` copy, taken when this screen opened and updated
    /// by the tab strip. That is right for identity and wrong for anything that
    /// changes underneath it — pane mode above all. Switching to chat left the
    /// copy still saying "terminal", so the button asked for the same switch
    /// every time, the screen kept drawing a VT grid, and the change only
    /// appeared after navigating away and back, which rebuilt the copy.
    private var live: Terminal {
        guard let workspace = currentWorkspace?.id else { return terminal }
        return connection.terminal(terminal.id, in: workspace) ?? terminal
    }

    private var currentWorkspace: Workspace? {
        connection.fleet.workspaces.first { $0.terminals.contains { $0.id == terminal.id } }
    }

    /// Which of several identically-labeled siblings `current` is — the
    /// same numbering the shell's ribbon and column use, so a terminal
    /// reads as "claude 2" everywhere or nowhere.
    private var currentOrdinal: Int? {
        currentWorkspace?.ordinals()[terminal.id]
    }

    private var currentName: String { terminal.displayName(ordinal: currentOrdinal) }

    var body: some View {
        VStack(spacing: 0) {
            // Chosen by `terminal.isAgentPane`, which the daemon sets — never
            // derived here, the same rule that keeps `activity` and
            // `agentMode` as reported rather than guessed at. `AgentView` is
            // given `terminal.id` as its SwiftUI identity: a tab switch
            // between two agent panes recreates it rather than retargeting
            // an existing `AgentStream`, which is simpler than the terminal
            // side's `TerminalSession.switchTo` and correct here because an
            // agent session has no live ssh channel to hand off — a fresh
            // subscribe from seq 0 costs one round trip, not a stream.
            if live.isAgentPane {
                AgentView(
                    terminalID: terminal.id, workspaceID: currentWorkspace?.id,
                    connection: connection, isVisible: isVisible)
                    .id(terminal.id)
            } else if live.isChangesPane, let workspace = currentWorkspace {
                // A review of the worktree, not a tty.
                //
                // This branch did not exist, so a `changes` pane fell through
                // to the VT renderer below and drew whatever bytes were on a
                // pane that has none — the mode has been in the fleet all
                // along, with nothing on this platform able to show it.
                //
                // No longer the ordinary way a diff is reached: the shell
                // gives every workspace a Changes tab that needs no pane behind
                // it — see `ShellFleetMap.of` — and folds a host-side `changes`
                // pane into that tab rather than mounting it here. What is left for this branch is a pane
                // that BECOMES a `changes` pane while it is mounted — the Mac
                // can do that to a worktree this phone is looking at — and the
                // alternative for that case is the VT grid and the original bug.
                //
                // Keyed on the WORKSPACE rather than the terminal: what is
                // being reviewed is the worktree, and two changes panes in one
                // workspace are the same review.
                // The store comes from `Connection`, so the scroll position,
                // which files are folded, and the diffs already read all
                // survive switching to another tab and back. Held in the view,
                // they were rebuilt from nothing on every return.
                // The agent panes a review note can be sent to, as plain
                // values. Resolved here because this is where the fleet is
                // already in hand, and handed over as values rather than as
                // the `Connection` so that reviewing a diff does not
                // re-evaluate a forty-card lazy stack on every three-second
                // poll. See `ChangesView.agents`. The filter itself lives on
                // `Workspace` because the inbox reaches the same review by a
                // different door — see `Workspace.reviewAgentTargets()`.
                ChangesView(
                    store: connection.changesStores.store(for: workspace.id),
                    workspaceName: workspace.task,
                    agents: workspace.reviewAgentTargets())
                    .id(workspace.id)
            } else {
                GeometryReader { geo in
                    phaseContent(size: geo.size)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        // Size is asserted only by the pane you are LOOKING at.
                        //
                        // tmux sizes a pane to its smallest attached client, so
                        // several mounted panes all telling the host their
                        // geometry would have them fighting over it — which is
                        // the content jumping around on open, made worse rather
                        // than better by keeping panes alive. A hidden pane is
                        // laid out but says nothing.
                        .task(
                            id: GridSize(
                                width: geo.size.width, height: geo.size.height, cell: cellSize,
                                visible: isVisible)
                        ) {
                            guard isVisible else { return }
                            await session.configure(
                                columns: columns(for: geo.size), rows: rows(for: geo.size))
                        }
                }
                // NO KEYBOARD INSET OF THIS VIEW'S OWN, AND THAT IS A CHANGE.
                //
                // There used to be one here — `Color.clear.frame(height:
                // keyboard.height)` outside the reader, so that the reader
                // itself was proposed the smaller height and `columns/rows`
                // asked tmux for the pane the screen could actually show. It
                // was correct while its premise was: the shell took no
                // automatic avoidance, so the grid had to ask for its own room.
                //
                // The premise died when the pane grew a `NavigationStack` for
                // its own bar (`ShellScreen.ShellPaneRealView.body`). A
                // navigation stack is a `UINavigationController`, and the
                // framework re-derives keyboard avoidance from the window
                // inside it — an `ignoresSafeArea(.keyboard)` outside the stack
                // does not reach in. So this inset became the SECOND
                // subtraction of the same 360 points, and 874 − 116 (bar and
                // status bar) − 450 (the shell's furniture plus the keyboard)
                // − 360 is negative: the grid came out **0 points tall**, the
                // pane rendered nothing at all with the keyboard up, and three
                // UI tests could not swipe an element with no visible frame.
                //
                // What replaces it is the framework's own number, and it is
                // the same number: SwiftUI's avoidance counts the input
                // accessory, so the key row is included exactly as
                // `KeyboardInset` counted it. This view no longer observes
                // `KeyboardInset` at all — `AgentView` still does, because a
                // DOCKED composer is on screen with the keyboard down and posts
                // no keyboard frame at all, which is a state the terminal's key
                // row never reaches: its accessory exists only while the field
                // it belongs to is first responder.
            }
        }
        // THE PANE'S OWN GROUND, PAINTED FROM INSIDE THE NAVIGATION STACK.
        //
        // `ShellPaneRealView` already says `.background(TerminalPalette
        // .background.ignoresSafeArea())`, and it is OUTSIDE the pane's
        // `NavigationStack` — which is a `UINavigationController`, and a
        // navigation controller's view carries its own opaque
        // `systemBackground`. Under the terminal's dark scheme that is pure
        // black, and it is drawn straight over the ground the shell asked for.
        //
        // A diff never showed it: `ChangesView` puts `TerminalPalette
        // .background` on its own scroll view, and a scroll view fills the
        // whole pane including the safe areas. A VT grid does not — it is a
        // `GeometryReader` sized to the SAFE region — so everything the grid
        // does not cover was the navigation controller's black.
        //
        // Measured on an iPhone 17, keyboard down: the grid ran 116…750 in
        // this app's `#2E3440` and the two strips either side of it — 0…116
        // under the pane's bar, and 750…874 behind the shell's — read
        // (0, 0, 0). That is the owner's "black bars above and below
        // terminals". Keyboard up the lower strip is 424…513, between the grid
        // and the key row, and just as black.
        //
        // Inside the stack rather than a second copy outside it, because
        // outside is where the one that loses already is.
        .background(TerminalPalette.background.ignoresSafeArea())
        // The link a long press landed on. Titled with the URL itself, because
        // "Open Link" without saying which link is a button that asks you to
        // trust output an agent produced.
        .confirmationDialog(
            heldLink ?? "",
            isPresented: Binding(get: { heldLink != nil }, set: { if !$0 { heldLink = nil } }),
            titleVisibility: .visible
        ) {
            if let link = heldLink, let url = URL(string: link) {
                Button("Open Link") { UIApplication.shared.open(url) }
            }
            Button("Copy Link") { UIPasteboard.general.string = heldLink }
            Button("Cancel", role: .cancel) {}
        }
        // What this pane costs while it is not on screen: nothing.
        //
        // The stream is a second ssh channel and the poll is traffic to the
        // host, so a mounted-but-hidden pane must hold neither. Driven by
        // `isVisible` rather than `onDisappear`, which no longer fires — the
        // pane is hidden, not removed, and that is what keeps its grid.
        .task(id: isVisible) {
            if isVisible {
                // `resume`, not `relink`. Relinking rebuilt the pane from
                // nothing every time it won its race with `configure`, which
                // is the "Loading…" on a tab you had already opened — and the
                // exact opposite of what mounting hidden panes is for. See
                // `TerminalSession.resume`.
                session.resume()
                Notifier.shared.visibleTerminal = terminal.id
                await connection.markVisibleSeen()
            } else {
                session.stop()
                // AND IT GIVES UP THE KEYBOARD.
                //
                // A pane that is merely hidden is still MOUNTED — that is what
                // `ShellPaneTrack` is for — and `KeystrokeSink` went on holding
                // first responder after the pane left the screen. So the
                // keyboard, and the key row docked in its window, stayed up
                // over a workspace whose pane has no tty at all, covering the
                // bottom of the display and everything on it: measured, the
                // shell's bar sat at y=784 under a keyboard whose top edge was
                // at 514, and a swipe aimed at the bar went into the keyboard
                // instead. `DockedBar.swift:34-41` is the same hazard said
                // about a composer; this is its terminal half.
                //
                // It was invisible until the shell stopped letting the keyboard
                // into its own safe area (`ShellRootView.body`), because the
                // bar used to be shoved up clear of the keyboard — which is the
                // bug, not the reason it worked.
                //
                // Through `dismissRequest` rather than by calling
                // `resignFirstResponder` from here: the request is the one
                // channel `KeystrokeField.updateUIView` reads, and a second
                // route into UIKit's responder state is how the two get out of
                // step.
                dismissRequest += 1
            }
        }
        // Returning to the foreground carries no geometry of its own — this
        // pane's size has not changed — but the pane is shared, and someone on
        // the Mac could have resized the shared window while this device was
        // backgrounded and not watching. `reassertSize` re-asks for the size
        // already on file rather than computing a new one.
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active, isVisible else { return }
            session.reassertSize()
            Task { await connection.markVisibleSeen() }
        }
        // The link under this pane was replaced.
        //
        // A stream is a second ssh channel on the session that just died, so
        // everything this pane had open went with it. `TerminalSession`
        // survives that on its own by falling back to polling, which is the
        // right behavior and the slower path; this puts it back on the stream
        // now that there is one to be on.
        .onChange(of: connection.reconnectGeneration) { _, _ in
            guard isVisible else { return }
            session.relink()
        }
    }

    @ViewBuilder
    private func phaseContent(size: CGSize) -> some View {
        switch session.phase {
        case .connecting:
            status(spinner: true, title: "Loading \(currentName)…")
        case .notLive:
            // Not a failure and not an alarm: a pane that isn't running is the
            // ordinary end of a pane. Amber on this screen said "an agent is
            // waiting on you", which is the one thing it means everywhere else
            // in the product and is not what this is.
            status(
                symbol: "moon.zzz", mark: .secondary, title: "Not live",
                message: "\(currentName) has no running pane right now.")
        case .failed(let message, let transcript):
            status(
                symbol: "exclamationmark.triangle", mark: .red, title: "Could not load",
                message: message, transcript: transcript)
        case .live:
            if let grid = session.grid {
                live(grid: grid, size: size)
            } else {
                status(spinner: true, title: "Loading \(currentName)…")
            }
        }
    }

    /// `message` is prose this app wrote; `transcript` is what the host said.
    ///
    /// They are drawn as two different kinds of thing on purpose. The host's
    /// answer used to arrive as `message` and be set in the same callout, so a
    /// lowercase fragment from an ssh channel read as Far Cooler's own account
    /// of the pane. Nothing is dropped — those words are the whole diagnosis of
    /// a runner that cannot be reached — they just go in the box that means
    /// "output", the one `DaemonUpdateCard`, `RunnersSettings` and
    /// `ChangesPane` already use for exactly this.
    ///
    /// Semantic colors throughout, and they resolve correctly here:
    /// `ShellPaneRealView` puts this whole pane in the terminal theme's own color
    /// scheme. That was already written down beside the transcript box, and the
    /// rest of this view had drifted past it — a hardcoded `.white` headline
    /// over the two `#FFFFFF` themes this app ships was a screen that said
    /// nothing at all on a ground it was told the polarity of.
    ///
    /// The proportions are `FleetView.failure`'s — a 42pt thin mark, a
    /// `.title2` headline, a `.callout` sentence, 22 and 8 between them. This
    /// was `.largeTitle` over `.headline`, and `.headline` is 17pt: a list row's
    /// title standing in for a full-screen one.
    private func status(
        spinner: Bool = false, symbol: String? = nil, mark: Color = .secondary, title: String,
        message: String? = nil, transcript: String? = nil
    ) -> some View {
        VStack(spacing: 0) {
            if spinner {
                // The spinner is the mark in this state, so it is sized like
                // one rather than left at the 20pt a row would use.
                ProgressView()
                    .controlSize(.large)
                    .padding(.bottom, 22)
            } else if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 42, weight: .thin))
                    .foregroundStyle(mark)
                    .padding(.bottom, 22)
            }
            Text(title)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
                .padding(.bottom, 8)
            if let message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
            }
            if let transcript, !transcript.isEmpty {
                DetailBox(text: transcript)
                    .frame(maxWidth: 320)
                    .padding(.top, 14)
            }
        }
        .padding(.horizontal, 32)
    }

    private func live(grid: TerminalGrid, size: CGSize) -> some View {
        ZStack(alignment: .topLeading) {
            Canvas { context, canvasSize in
                renderer.draw(
                    grid: grid, into: context, size: canvasSize,
                    scrolledBy: scrollReadout.offset)
            }
                // The one way to ask this pane what it is showing.
                //
                // A `Canvas` is pixels: there are no child views for a UI test
                // to read, so without this "did the swipe scroll anything"
                // cannot be answered except by a person looking. Both numbers
                // are here because both are needed to tell the two failures
                // apart — a gesture that never fired leaves `offset` at 0 with
                // history behind it, and a pane the host sent no scrollback for
                // leaves `history` at 0, which is a swipe that correctly did
                // nothing.
                //
                // On the `Canvas` rather than the `ZStack`, deliberately:
                // giving the stack an accessibility value would make the STACK
                // the element and hide the keyboard sink inside it.
                .accessibilityElement()
                .accessibilityIdentifier("terminal-surface")
                // `mouse=` last, and every reader of this value parses it by
                // KEY rather than by position — see `field` in
                // TerminalScrollTests. The first version of that parser
                // required exactly three space-separated parts and matched
                // `history=0` with ENDSWITH, so appending a fourth field would
                // have silently turned three guards into no-ops. Naming the
                // fields is what makes this string safe to extend.
                //
                // The last four are the physics, and they are here for the same
                // reason `offset` is: a `Canvas` is pixels, and "the grid
                // tracked the thumb between two rows" and "the grid quantised to
                // rows" leave the emulator on the same line and the screenshot
                // identical. Only a number the drawing itself was given can tell
                // them apart. `cell` is what turns points into rows, so a test
                // can say "further than the finger travelled" in the units the
                // finger travelled in. See `TerminalScrollReadout`.
                .accessibilityValue(
                    "offset=\(session.scrollPosition.offset) "
                        + "history=\(session.scrollPosition.history) "
                        + "source=\(session.scrollPosition.streaming ? "stream" : "poll") "
                        + "mouse=\(session.scrollPosition.wantsMouse ? "on" : "off") "
                        + "cell=\(Int(gridLayout(for: grid, in: size).cell.height.rounded())) "
                        + "grain=\(Int(scrollReadout.grainPeak.rounded())) "
                        + "band=\(Int(scrollReadout.bandPeak.rounded())) "
                        + "over=\(Int(scrollReadout.over.rounded()))")
            // A UIKit view whose only job is to be the thing the software
            // keyboard is attached to. It carries no visible state of its
            // own — every character it reports goes straight to the host and
            // back through the poll, which is the only place this view ever
            // draws what was typed.
            // Filling the terminal rather than hiding in a corner.
            //
            // It was a 1×1 view at 1% opacity with the tap handled by the SwiftUI
            // stack around it, and it never reliably took first responder: a view
            // that small and that transparent is not something UIKit is eager to
            // give the keyboard to, and the tap had to travel through a Canvas to
            // reach it. Sizing it to the area it represents makes it an ordinary
            // view that can be tapped and focused, and it is still invisible
            // because it draws nothing.
            //
            // It also carries the scroll gesture, for the same reason: it is
            // the view that already owns every touch landing on the terminal.
            // A `DragGesture` layered on top in SwiftUI would be racing this
            // view's own `UITapGestureRecognizer` for the same touches rather
            // than cooperating with it, so the pan recognizer lives on the
            // same UIView and the tap is told to lose that race explicitly —
            // see `KeystrokeSink.setup()`.
            KeystrokeField(
                focusRequest: focusRequest,
                dismissRequest: dismissRequest,
                cellHeight: gridLayout(for: grid, in: size).cell.height,
                coreOffset: session.scrollPosition.offset,
                historyLines: session.scrollPosition.history,
                alternateScreen: session.scrollPosition.alternate,
                onInsertText: insert,
                onDeleteBackward: { sendKey(UInt32(FARCOOLER_VT_KEY_BACKSPACE)) },
                // Synchronous, and it answers with where the emulator ended up.
                //
                // There is no `Task` here any more and that is the whole of the
                // actor-hop answer: `TerminalSession.scroll` never suspended on
                // this path, so the hop bought nothing and cost a frame — which
                // a finger could afford and a throw crossing fifty rows in a
                // quarter second cannot. Reading `scrollPosition.offset` back on
                // the same line is what makes the drawn offset and the
                // emulator's `display_offset` unable to disagree: the driver is
                // told the truth on every commit rather than assuming its own
                // arithmetic won.
                onScroll: { lines, point in
                    let target = cell(at: point, grid: grid, size: size)
                    session.scroll(lines: lines, column: target.column, row: target.row)
                    return session.scrollPosition.offset
                },
                onReadout: { scrollReadout = $0 },
                onHold: { point in
                    // Nothing happens away from a link. There is no terminal
                    // paste on this platform for a long press to displace, so
                    // inventing a second meaning here would be a gesture nobody
                    // asked for on a screen where every touch matters.
                    let target = cell(at: point, grid: grid, size: size)
                    heldLink = session.url(atRow: target.row, column: target.column)
                },
                accessory: AnyView(
                    TerminalKeyRow(
                        ctrlArmed: ctrlArmed,
                        altArmed: altArmed,
                        onToggleCtrl: { ctrlArmed.toggle() },
                        onToggleAlt: { altArmed.toggle() },
                        onKey: sendKey,
                        onDismiss: { dismissRequest += 1 }
                    )
                )
            )
        }
        // Bounded to the terminal, and clipped to it.
        //
        // `KeystrokeSink` is an invisible `UIView` with no intrinsic size, so
        // nothing stops it being handed more room than the grid it belongs to —
        // and it owns every touch that lands on it. That was harmless while the
        // tab strip sat above the terminal, and became a bug the moment the
        // strip moved below: the sink covered the chips, so tapping a terminal
        // to switch to it did nothing at all.
        .frame(width: size.width, height: size.height)
        .clipped()
        .onAppear { focusRequest += 1 }
    }

    // MARK: - Drawing

    /// The grid cell under a point in the canvas's own coordinates, clamped
    /// onto the grid so a touch a pixel past the last row still lands
    /// somewhere real rather than off the edge of `grid`'s backing array.
    private func cell(at point: CGPoint, grid: TerminalGrid, size: CGSize) -> (column: Int, row: Int) {
        let layout = gridLayout(for: grid, in: size)
        guard layout.cell.width > 0, layout.cell.height > 0 else { return (0, 0) }
        let column = Int(((point.x - layout.origin.x) / layout.cell.width).rounded(.down))
        let row = Int(((point.y - layout.origin.y) / layout.cell.height).rounded(.down))
        return (
            min(max(column, 0), max(grid.columns - 1, 0)),
            min(max(row, 0), max(grid.rows - 1, 0))
        )
    }

    private func gridLayout(for grid: TerminalGrid, in size: CGSize) -> TerminalGridLayout {
        renderer.layout(for: grid, in: size)
    }

    // MARK: - Geometry

    private func columns(for size: CGSize) -> Int {
        let padding = TerminalMetrics.padding
        let usable = size.width - padding.left - padding.right
        return max(1, Int((usable / cellSize.width).rounded(.down)))
    }

    private func rows(for size: CGSize) -> Int {
        let padding = TerminalMetrics.padding
        let usable = size.height - padding.top - padding.bottom
        return max(1, Int((usable / cellSize.height).rounded(.down)))
    }
    // `select` is gone. Switching panes is `ShellPaneTrack`'s job now, and it does
    // it by showing a different, already-mounted pane rather than by pointing
    // this one somewhere else — which is what makes a pane's grid, scroll
    // offset and fold state survive the switch.


    // MARK: - Input

    /// Route one burst of typed text, special-casing the newline the
    /// software keyboard's Return key produces.
    ///
    /// `insertText("\n")` is what UIKit sends for a keyboard Return tap, but
    /// passing the scalar straight through would send a bare 0x0A — a literal
    /// newline character rather than the Enter *keystroke* a program expects,
    /// and the two are not always interchangeable under a raw pty. Routing it
    /// through the same encoder the Return button uses keeps the keyboard's
    /// own Return key and the accessory row's agreeing with each other.
    private func insert(_ text: String) {
        if text == "\n" {
            sendKey(UInt32(FARCOOLER_VT_KEY_ENTER))
            return
        }
        let modifiers = consumeModifiers()
        Task { await session.send(text: text, modifiers: modifiers) }
    }

    private func sendKey(_ key: UInt32) {
        let modifiers = consumeModifiers()
        Task { await session.send(key: key, modifiers: modifiers) }
    }

    /// Ctrl is a toggle, not a held key — there is nothing on a touchscreen
    /// that behaves like holding a modifier down. So it applies to exactly
    /// the next key and then clears itself, the same shape as Shift-lock on
    /// a physical keyboard with only one hand.
    /// Ctrl and Alt are toggles, not held keys — there is nothing on a
    /// touchscreen that behaves like holding a modifier down. So each applies
    /// to exactly the next key and then clears itself, the same shape as
    /// Shift-lock on a physical keyboard with only one hand.
    private func consumeModifiers() -> VTModifiers {
        defer {
            ctrlArmed = false
            altArmed = false
        }
        var mods: VTModifiers = []
        if ctrlArmed { mods.insert(.control) }
        if altArmed { mods.insert(.alt) }
        return mods
    }

    /// What `columns(for:)`/`rows(for:)` depend on — the view's own size, AND
    /// the cell a font produces. `cell` is here so a font or size change in
    /// Settings re-runs `session.configure`, not just the drawing: a viewport
    /// that fit 80 columns at 13pt fits fewer at 18pt, and the pane this
    /// screen asks the host to resize to should reflect the font actually on
    /// screen, not whatever it was measured at when this screen first
    /// appeared.
    private struct GridSize: Equatable {
        var width: Double
        var height: Double
        var cell: CGSize
        /// Part of the key so that becoming visible re-asserts the size, and
        /// becoming hidden stops asserting it, without the geometry changing.
        var visible: Bool
    }
}

/// The row of keys a terminal needs and a phone's keyboard does not have.
///
/// Styled as a keyboard accessory, not a strip of custom chrome: system
/// materials rather than a hand-picked grey, so it reads as part of iOS rather
/// than as a widget floating on top of it.
///
/// It used to say `.bordered`/`.borderedProminent` gave every key its
/// pressed-state animation for free, and that there was no `isPressed`-driven
/// fill anywhere here. Both halves stopped being true when the keys went to
/// `.buttonStyle(.plain)` with a fill of their own — see the note on the shape
/// below — and what was left was a key row with NO pressed state, on the one
/// control in this app where the difference between a keystroke that landed and
/// one that might have is the whole question. `TerminalKeyStyle` is the fill
/// the comment was describing, written out.
private struct TerminalKeyRow: View {
    let ctrlArmed: Bool
    let altArmed: Bool
    let onToggleCtrl: () -> Void
    let onToggleAlt: () -> Void
    let onKey: (UInt32) -> Void
    let onDismiss: () -> Void

    /// Keys share the width rather than each claiming their own.
    ///
    /// Nine keys sized from their own content overflowed a phone once — a
    /// bordered button pads whatever you hand it, so a 34pt legend became a
    /// 58pt key. The row did not clip on its own: it widened the stack it was
    /// in, so the terminal ABOVE it lost characters off both edges.
    /// `maxWidth: .infinity` makes overflow impossible to express.
    ///
    /// 44 tall, the guideline's floor. The width stays whatever nine keys
    /// divide into — around 31 points — because that is keyboard density and
    /// the row directly above this one is doing the same thing.
    private static let keyHeight: CGFloat = 44

    var body: some View {
        HStack(spacing: 5) {
            key { onKey(UInt32(FARCOOLER_VT_KEY_ESCAPE)) } label: { glyph("escape") }
            key { onKey(UInt32(FARCOOLER_VT_KEY_TAB)) } label: { glyph("arrow.right.to.line") }
            key(filled: ctrlArmed, action: onToggleCtrl) { glyph("control") }
            key(filled: altArmed, action: onToggleAlt) { glyph("option") }
            // Held, each arrow becomes the jump it is the small version of.
            // A phone has no room for eight more keys and no modifier to hide
            // them behind, and holding a direction to go further in it is the
            // gesture people already have for exactly this.
            // Solid arrows rather than chevrons, because `control` IS a
            // chevron: ⌃ beside a chevron-up meant two keys with the same
            // glyph sitting four apart in the same row.
            arrow("arrow.left", tap: FARCOOLER_VT_KEY_LEFT, hold: FARCOOLER_VT_KEY_HOME)
            arrow("arrow.down", tap: FARCOOLER_VT_KEY_DOWN, hold: FARCOOLER_VT_KEY_PAGE_DOWN)
            arrow("arrow.up", tap: FARCOOLER_VT_KEY_UP, hold: FARCOOLER_VT_KEY_PAGE_UP)
            arrow("arrow.right", tap: FARCOOLER_VT_KEY_RIGHT, hold: FARCOOLER_VT_KEY_END)
            // Putting the keyboard away, which this row is otherwise the only
            // thing standing in the way of: it lives above the keyboard, so it
            // goes when the keyboard does, and without a way to dismiss from
            // here there is nowhere else to ask from.
            key(action: onDismiss) { glyph("keyboard.chevron.compact.down") }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 7)
        // Glass, like the composer next door.
        //
        // This used to take the `UIInputView`'s own backdrop, so that it read
        // as the keyboard's top edge rather than a slab resting on it. That was
        // right when the keyboard's edge was a flat bar; on iOS 26 the thing
        // resting above a keyboard is a floating glass surface, and a squared
        // strip against a rounded composer looked like two releases of the app
        // at once.
        //
        // No Return key. The software keyboard already has one, and the row's
        // whole job is the keys a phone keyboard does not have.
        //
        // Radius 22, inset 10 — the tab strip's pair and the composer's, so the
        // three floating surfaces on this screen share one edge and one corner
        // instead of three of each.
        .modifier(GlassSurface(radius: 22))
        .padding(.horizontal, 10)
        // Clear of the keyboard, not resting on it.
        //
        // A floating bar whose bottom edge is flush against the keyboard's top
        // edge is not floating — it reads as a strip welded to the keyboard
        // with rounded corners drawn on, which is worse than either honest
        // option. The gap is what says the two are different surfaces.
        .padding(.bottom, 10)
        .padding(.top, 2)
    }

    /// An arrow that means one thing tapped and a bigger version of the same
    /// thing held.
    private func arrow(_ symbol: String, tap: UInt32, hold: UInt32) -> some View {
        key(action: { onKey(tap) }, onHold: { onKey(hold) }) { glyph(symbol) }
    }

    /// One key: a rounded rectangle, because that is what a key looks like
    /// here. The system's bordered style rounds to a capsule at these
    /// proportions, which reads as a row of pills rather than a keyboard.
    private func key<Label: View>(
        filled: Bool = false,
        action: @escaping () -> Void,
        onHold: (() -> Void)? = nil,
        @ViewBuilder label: () -> Label
    ) -> some View {
        Button(action: action) { label() }
            .buttonStyle(TerminalKeyStyle(filled: filled, height: Self.keyHeight))
            // A long press that fires once, rather than a repeat: these send a
            // jump, and a jump that repeats while your thumb rests on it would
            // scroll somewhere nobody asked to be.
            .onLongPressGesture(minimumDuration: 0.35) { onHold?() }
    }

    private func glyph(_ symbol: String) -> some View {
        Image(systemName: symbol).font(.system(size: 15, weight: .medium))
    }
}

/// One key's fill, and what it does under a thumb.
///
/// A style rather than a `.background` inside the label, because the pressed
/// state is the point: `configuration.isPressed` is only reachable from here,
/// and a key with no pressed state gives a typist nothing to tell a keystroke
/// that landed from one that missed. On a row whose keys send Escape and
/// Control to a shell, that is not a polish item.
///
/// The unpressed fill is `.quaternary` and not `systemGray3`. Grey 3 is
/// OPAQUE, and nine opaque slabs on a glass bar cancel exactly the material
/// they are sitting on — the bar stops refracting the terminal behind it and
/// goes back to being the flat strip the glass replaced.
private struct TerminalKeyStyle: ButtonStyle {
    /// An armed modifier — Control or Option — which stays lit between presses.
    let filled: Bool
    let height: CGFloat

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(maxWidth: .infinity, minHeight: height)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(fill(pressed: configuration.isPressed))
            )
            // White on an opaque accent fill, which is the one place on this
            // screen a fixed white is right: it is over a color this app
            // chose, not over a ground the theme did.
            .foregroundStyle(filled ? AnyShapeStyle(Color.white) : AnyShapeStyle(.primary))
            // Quick enough to read as the key going down rather than as the
            // row animating. A press that fades in over a keystroke is worse
            // than none, because it arrives after the character does.
            .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
    }

    /// Pressed is one step UP the hierarchy, not a lower opacity: a key that
    /// fades under a finger looks like a key that did not take the press.
    private func fill(pressed: Bool) -> AnyShapeStyle {
        if filled { return AnyShapeStyle(Color.accentColor.opacity(pressed ? 0.75 : 1)) }
        return pressed ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.quaternary)
    }
}

/// Attaches the software keyboard to a view with no text of its own.
///
/// `UIKeyInput` rather than the full `UITextInput` a `UITextField` needs: it
/// is the smaller protocol that still gets a keyboard attached, and this view
/// has no text to hold, no cursor to place, and no selection to track — the
/// terminal grid is the only place any of those are ever drawn.
private struct KeystrokeField: UIViewRepresentable {
    var focusRequest: Int
    /// Bumped to put the keyboard away. A counter rather than a boolean for
    /// the same reason `focusRequest` is one: what matters is that a fresh
    /// request happened, not what state anything is in.
    var dismissRequest: Int
    /// The height, in points, one terminal row is actually drawn at right
    /// now — what a drag on this view is converted to lines against. Passed
    /// in rather than measured here, because only `TerminalView` knows the
    /// current font, size and scale that produced it.
    var cellHeight: CGFloat
    /// Where the emulator is in its own scrollback, and how much of one it
    /// has — the bounds the physics rubberbands against, and the anchor the
    /// drawn offset is measured from. Passed in on every update rather than
    /// read from the session here, because this type is a `UIViewRepresentable`
    /// and the session is the view's.
    var coreOffset: Int
    var historyLines: Int
    /// See `TerminalScrollPosition.alternate`: a pane with no scrollback behind
    /// it keeps the whole-line scroll it always had.
    var alternateScreen: Bool
    var onInsertText: (String) -> Void
    var onDeleteBackward: () -> Void
    var onScroll: (Int, CGPoint) -> Int
    var onReadout: (TerminalScrollReadout) -> Void
    /// Where a long press landed, for the link actions.
    var onHold: (CGPoint) -> Void
    /// The key row, handed to the system as the keyboard's accessory so it is
    /// drawn as part of the keyboard rather than as a strip above one.
    var accessory: AnyView

    func makeCoordinator() -> Coordinator { Coordinator() }

    /// The accessory's exact height. Stated rather than self-sized, because a
    /// `UIInputView` that sizes itself ends up taller than its content and the
    /// difference is transparent — which is what put a strip of window between
    /// the row and the keyboard and made the two look unrelated.
    private static let accessoryHeight: CGFloat = 52

    func makeUIView(context: Context) -> KeystrokeSink {
        let view = KeystrokeSink()
        view.onInsertText = onInsertText
        view.onDeleteBackward = onDeleteBackward
        view.onScroll = onScroll
        view.onHold = onHold
        view.onReadout = onReadout
        view.syncCore(
            offset: coreOffset, history: historyLines, alternate: alternateScreen,
            cellHeight: cellHeight)
        view.backgroundColor = .clear

        let host = UIHostingController(rootView: accessory)
        host.view.backgroundColor = .clear
        // The accessory sits ON the keyboard, so the home indicator is the
        // keyboard's problem and not this row's. Left alone, the hosting
        // controller adds the bottom inset itself and the row floats.
        host.safeAreaRegions = []
        host.view.translatesAutoresizingMaskIntoConstraints = false
        context.coordinator.accessoryHost = host

        // `.keyboard` style draws the system keyboard's own background, which
        // is what makes the row belong to the keyboard rather than resemble it.
        let bar = UIInputView(
            frame: CGRect(x: 0, y: 0, width: 0, height: Self.accessoryHeight),
            inputViewStyle: .keyboard)
        bar.autoresizingMask = .flexibleWidth
        bar.addSubview(host.view)
        NSLayoutConstraint.activate([
            host.view.leadingAnchor.constraint(equalTo: bar.leadingAnchor),
            host.view.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            host.view.topAnchor.constraint(equalTo: bar.topAnchor),
            host.view.bottomAnchor.constraint(equalTo: bar.bottomAnchor),
        ])
        view.accessory = bar
        return view
    }

    func updateUIView(_ uiView: KeystrokeSink, context: Context) {
        uiView.onInsertText = onInsertText
        uiView.onDeleteBackward = onDeleteBackward
        uiView.onScroll = onScroll
        uiView.onHold = onHold
        uiView.onReadout = onReadout
        uiView.syncCore(
            offset: coreOffset, history: historyLines, alternate: alternateScreen,
            cellHeight: cellHeight)
        context.coordinator.accessoryHost?.rootView = accessory

        // `updateUIView` runs on every poll, not just on a tap — the grid it
        // sits beside changes every second. Only actually re-focus when
        // `focusRequest` itself moved, or an on-screen keyboard the user
        // deliberately dismissed would be pulled back up on the next tick.
        if context.coordinator.lastDismissRequest != dismissRequest {
            context.coordinator.lastDismissRequest = dismissRequest
            DispatchQueue.main.async { uiView.resignFirstResponder() }
            return
        }

        guard context.coordinator.lastFocusRequest != focusRequest else { return }
        context.coordinator.lastFocusRequest = focusRequest
        DispatchQueue.main.async { uiView.becomeFirstResponder() }
    }

    final class Coordinator {
        var lastFocusRequest = -1
        var lastDismissRequest = 0
        /// Retained because nothing else owns it: the input view holds the
        /// hosting controller's VIEW, and a controller referenced only through
        /// its own view is deallocated along with its SwiftUI state.
        var accessoryHost: UIHostingController<AnyView>?
    }
}

private final class KeystrokeSink: UIView, UIKeyInput, UIGestureRecognizerDelegate {
    var onInsertText: ((String) -> Void)?
    var onDeleteBackward: (() -> Void)?
    /// Move the view by whole lines, and answer with the offset the emulator
    /// ACTUALLY landed on.
    ///
    /// The return value is the whole of how this class stays honest. It asks
    /// for a delta and is told where that put things, so a clamp at either end
    /// — or a history that grew under it — corrects the driver on the same
    /// call rather than accumulating. It can be synchronous because
    /// `TerminalSession.scroll` is; see the note there.
    var onScroll: ((Int, CGPoint) -> Int)?
    /// Where a long press landed, for the link actions.
    var onHold: ((CGPoint) -> Void)?
    /// How far the grid should be drawn from the row the core is on, and the
    /// peaks a test reads to tell tracking from quantising.
    var onReadout: ((TerminalScrollReadout) -> Void)?
    var cellHeight: CGFloat = 16
    /// How many lines of scrollback sit above the live screen.
    private(set) var historyLines = 0
    /// Whether the pane is on the alternate screen, where none of the physics
    /// below applies. See `TerminalScrollPosition.alternate`.
    private(set) var alternateScreen = false
    /// The key row, shown by the system as part of the keyboard.
    var accessory: UIView?

    override var inputAccessoryView: UIView? { accessory }


    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// Both gesture recognizers this view carries, wired together here so the
    /// relationship between them is set up in exactly one place.
    ///
    /// The tap is what asks for the keyboard; the pan is what scrolls. Both
    /// live on this view — the one thing on screen that already owns every
    /// touch landing on the terminal — rather than the pan being a SwiftUI
    /// `DragGesture` layered above it, which would be racing this view's own
    /// recognizer for the same touches instead of cooperating with it.
    /// `require(toFail:)` is what keeps a drag that scrolls from also being
    /// read as the tap that raises the keyboard: the tap does not fire until
    /// the pan has definitively not started.
    private func setup() {
        let tap = UITapGestureRecognizer(target: self, action: #selector(focus))
        let pan = UIPanGestureRecognizer(target: self, action: #selector(handlePan))
        tap.require(toFail: pan)
        tap.delegate = self
        pan.delegate = self
        addGestureRecognizer(tap)
        addGestureRecognizer(pan)

        // A long press offers the link under it, if there is one.
        //
        // Tap already means "give me the keyboard" and cannot be made ambiguous,
        // so the link actions go on the one gesture this view had nothing on.
        // Android long-presses to paste and keeps doing so; there is no terminal
        // paste on this platform to preserve, so a press away from a link simply
        // does nothing rather than being given a new meaning nobody asked for.
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(handleHold))
        hold.delegate = self
        addGestureRecognizer(hold)
    }

    /// Every touch that lands on this view, the instant it lands — the hook a
    /// coasting scroll has to be stopped from, and the one place that reliably
    /// fires.
    ///
    /// `UIView.touchesBegan` looks like the obvious place for that and it is
    /// where this used to live: override it, check whether something is
    /// moving, stop it. It never ran. Measured on 2026-09-02 with `NSLog` in
    /// `touchesBegan`/`touchesEnded`/`touchesCancelled` and in every gesture
    /// recognizer's action, streamed live off the simulator with `log
    /// stream`: a tap synthesized onto a coasting pane fired the PAN
    /// recognizer's `.began`/`.ended` for the flick and then the TAP
    /// recognizer's action for the interrupting touch — and not one of this
    /// view's own `touchesXXX` overrides, for either touch. This view sits
    /// inside a `UIViewRepresentable`, and SwiftUI's own hosting does not
    /// forward raw touch delivery down into it the way a plain `UIView`
    /// hierarchy would — only the gesture recognizers attached to it, which
    /// are asked about a touch independently of who UIKit hit-tested. Which
    /// is this delegate method: called once per touch per recognizer, before
    /// any of them have decided anything, and it is what a tap gesture
    /// recognizer's `require(toFail:)` neighbour is normally used FOR.
    ///
    /// One touch reaches every recognizer here (tap, pan, hold all sit on
    /// this same view), so this fires two or three times for a single
    /// finger going down — `lastTouchDownHandled` is what keeps the second
    /// and third calls from re-deriving "was something moving" from a
    /// `displayLink` the first call already tore down.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard touch !== lastTouchDownHandled else { return true }
        lastTouchDownHandled = touch
        stopWhateverIsMoving()
        return true
    }

    /// A touch just landed. If a coast or a settle was under way, it is
    /// over — content stays exactly where it was caught, and the touch that
    /// caught it is marked spent (`tapStoppedTheCoast`) so `focus()` knows
    /// not to also raise the keyboard for it.
    private func stopWhateverIsMoving() {
        guard displayLink != nil else {
            // Per touch sequence, never left armed. A press that stopped a
            // coast and then became something other than a tap — a long
            // press, a gesture the shell took — must not spend the NEXT
            // tap's keyboard; see `handlePan` and `handleHold` for the two
            // ways a touch that stopped a coast is *not* the one that
            // consumes this.
            tapStoppedTheCoast = false
            return
        }
        stopMotion()
        velocity = 0
        motion = .rest
        tapStoppedTheCoast = true
    }

    /// **There is no `gestureRecognizerShouldBegin` here any more, and there
    /// never really was one.**
    ///
    /// A rule lived on this class refusing a pan whose first ten points leaned
    /// sideways, so that a horizontal drag would fall through to the shell's
    /// page turn. It was measured on 2026-08-30 and it NEVER RAN:
    /// `UIView.gestureRecognizerShouldBegin(_:)` is consulted for a view's own
    /// recognizers only when the view is their DELEGATE, and nothing ever set
    /// one. With the body replaced by a bare `return false` — refuse every pan,
    /// as broken as that rule can be made — the pan still began, the terminal
    /// still scrolled, and both of the tests it claimed to be load-bearing for
    /// still passed.
    ///
    /// It was left standing then, with a note saying so, on the grounds that
    /// wiring it up would make it real for the first time and a rule that can
    /// silently kill a scroll should not be switched on off the back of an
    /// argument. That is still true, so it is deleted rather than wired: a rule
    /// nobody enforces reads exactly like a rule somebody does, and this one had
    /// already convinced a comment in `ShellDrag.swift` that the terminal
    /// refuses sideways drags. It does not, and never did.
    ///
    /// What actually arbitrates is `.simultaneousGesture` on the track
    /// (`ShellRootView.paneTrack`): the shell's page turn runs alongside this
    /// pan rather than instead of it, and a sideways drag moves the grid by the
    /// zero rows its `translation.y` is worth.
    /// `testTheShellStillTurnsThePageOverALiveTerminal` and
    /// `testTheShellDoesNotStealTheTerminalsScroll` are the two opposite
    /// failures that pin it, and both pass with this gone.

    override var canBecomeFirstResponder: Bool { true }
    var hasText: Bool { true }

    /// Raise the keyboard — unless this touch was the one that stopped a coast.
    ///
    /// The talk's imperceptible stop is only half a promise: *"while content is
    /// scrolling, it makes it feel like you can just put your finger down
    /// again, and continue scrolling. You don't have to wait for anything to
    /// finish."* A finger put down on moving content is asking it to stop, and
    /// on this view a finger put down and lifted again is also the gesture that
    /// asks for the keyboard. Every scroll view on the platform resolves that
    /// the same way, so this does too: the tap that stops the motion is spent
    /// stopping it.
    ///
    /// Spent stopping it, not spent doing nothing: `require(toFail: pan)`
    /// guarantees this only fires for a touch that did not drag, and a touch
    /// that stopped a coast and then lifted without dragging is the exact
    /// case this class quantises for — the content may be sitting half a row
    /// from where the emulator can put it, and this is where that gets
    /// landed. See `settleAfterATouchThatDidNotDrag`.
    @objc func focus() {
        if tapStoppedTheCoast {
            tapStoppedTheCoast = false
            settleAfterATouchThatDidNotDrag()
            return
        }
        becomeFirstResponder()
    }

    // MARK: - Scroll physics

    /// Where the content is, in points back from the live screen, and what it
    /// is doing to get somewhere else.
    ///
    /// One continuous number rather than a line count, because the whole point
    /// is that it is allowed to sit between two lines and outside both ends.
    /// `shown` is what that number looks like after the edges resist it; the
    /// core is asked for `round(shown / cellHeight)` and the leftover is drawn.
    private enum Motion: Equatable {
        case rest
        case tracking
        case coasting
        /// Elastic return: from an over-drag to the edge, or from the tail of
        /// a coast onto the nearest row.
        case settling(target: CGFloat)
    }

    private var motion: Motion = .rest
    /// Points back from the live screen. May leave `0...span` — that is what
    /// there is to rubberband.
    private var position: CGFloat = 0
    /// Points per second, positive back into history.
    private var velocity: CGFloat = 0
    /// `position` when the current drag began.
    private var panAnchor: CGFloat = 0
    /// The line the emulator is on, as the emulator last reported it.
    private var committed = 0
    /// Where to say a scroll happened, for the wheel events the alternate
    /// screen sends. Kept from the last touch so a coast has a column and row.
    private var lastTouch: CGPoint = .zero
    private var displayLink: CADisplayLink?
    private var lastFrame: CFTimeInterval = 0
    private var tapStoppedTheCoast = false
    /// The touch `gestureRecognizer(_:shouldReceive:)` last acted on, so the
    /// two or three recognizers on this view asking about the same finger
    /// going down cost one stop rather than three.
    private weak var lastTouchDownHandled: UITouch?
    private var readout = TerminalScrollReadout()
    /// The last readout SwiftUI was actually given, so a frame that changed
    /// nothing worth drawing does not cost a redraw. See `commit`.
    private var published = TerminalScrollReadout()

    /// Below this, a coast has nothing left to say and hands over to the spring
    /// that lands it on a row. 40 points a second is under three points a
    /// frame — slow enough that the handover cannot be seen, fast enough that
    /// the tail of the exponential is not left crawling for another second.
    private static let coastFloor: CGFloat = 40
    /// The spring that returns content from an over-drag and lands a coast on a
    /// row. One constant for both because they are the same behaviour: content
    /// relaxing into a resting position once the strain is off it.
    private static let settleResponse: CGFloat = 0.3

    /// How far back the content can go before there is nothing above it.
    private var span: CGFloat { CGFloat(historyLines) * cellHeight }

    /// `position` with the edges' resistance applied — what is actually drawn.
    private var shown: CGFloat {
        let dimension = max(bounds.height, cellHeight * 4)
        if position < 0 {
            return TerminalScrollPhysics.rubberband(position, dimension: dimension)
        }
        if position > span {
            return span + TerminalScrollPhysics.rubberband(position - span, dimension: dimension)
        }
        return position
    }

    /// Tell the view what the core is on, so a scroll that came from anywhere
    /// else — typing jumps to the bottom, a pane rebuilt by a poll — does not
    /// leave this class believing something the emulator does not.
    ///
    /// Only while nothing is moving. Mid-gesture the finger is the authority
    /// and the core is following it; the reconciliation that matters there is
    /// the offset `onScroll` hands back on every single commit.
    func syncCore(offset: Int, history: Int, alternate: Bool, cellHeight height: CGFloat) {
        historyLines = history
        alternateScreen = alternate
        let resized = height > 0 && height != cellHeight
        if height > 0 { cellHeight = height }
        guard motion == .rest else { return }
        // A font change, a rotation, or a pane that reflowed: the line the core
        // is on has not moved but the points it is worth have.
        if resized || offset != committed {
            committed = offset
            position = CGFloat(offset) * cellHeight
            velocity = 0
        }
    }

    /// One drag: track it one-to-one, then hand its momentum to the content.
    ///
    /// Sub-line, deliberately. This used to convert `translation.y` into whole
    /// lines and put the remainder back with `setTranslation`, which meant the
    /// content moved a row at a time under a thumb that was moving smoothly —
    /// the talk's one-to-one tracking broken by up to half a row, on the one
    /// surface in this app people look at longest. The whole-line part still
    /// goes to the emulator, because whole lines are the only thing it can
    /// show; the remainder is drawn as an offset instead of being rounded away.
    @objc private func handlePan(_ recognizer: UIPanGestureRecognizer) {
        guard cellHeight > 0 else { return }
        lastTouch = recognizer.location(in: self)

        // The alternate screen keeps exactly the behaviour it had. There is no
        // scrollback behind it, so there is no local offset to track between
        // rows, nothing to bound and no edge to rubberband against — the wheel
        // belongs to the program, and `TerminalSession.scroll` is the one place
        // that decides so. Sliding the grid by fractional points here would
        // only slide a picture the program is about to repaint from scratch.
        guard !alternateScreen else {
            quantisedPan(recognizer)
            return
        }

        switch recognizer.state {
        case .began:
            // This touch already went through `gestureRecognizer(_:shouldReceive:)`
            // on the way here, so anything it was going to stop is already
            // stopped. What only becoming a drag settles is `tapStoppedTheCoast`:
            // a touch that stopped a coast and THEN dragged must not spend the
            // NEXT tap's keyboard — `tap.require(toFail:)` already keeps this
            // touch itself from reaching `focus()`, this is for the one after it.
            tapStoppedTheCoast = false
            stopMotion()
            motion = .tracking
            panAnchor = position
            // The peaks describe the gesture in progress, not the pane's whole
            // life. A running maximum that never resets can only ever answer
            // "did this pane rubberband at some point", and the question worth
            // asking is about the drag you just made.
            readout.grainPeak = 0
            readout.bandPeak = 0
            // The ten points of hysteresis a pan spends proving it is a pan are
            // not content the finger asked to move — dropping them is what
            // keeps the first frame from jumping.
            recognizer.setTranslation(.zero, in: self)
        case .changed:
            guard motion == .tracking else { return }
            position = panAnchor + recognizer.translation(in: self).y
            commit()
        case .ended, .cancelled, .failed:
            guard motion == .tracking else { return }
            release(velocity: recognizer.velocity(in: self).y)
        default:
            break
        }
    }

    /// The drag-to-lines conversion this class used for every pane, kept for
    /// the one kind of pane it is still right for.
    ///
    /// `setTranslation` puts back only the fractional remainder after each
    /// callback, so a slow drag accumulates towards the next line instead of
    /// being rounded away — the touchscreen equivalent of the Mac's
    /// `wheelTicks`, using the recognizer's own accumulated translation as
    /// the running total instead of a separately stored property.
    private func quantisedPan(_ recognizer: UIPanGestureRecognizer) {
        let translation = recognizer.translation(in: self)
        let lines = Int((translation.y / cellHeight).rounded(.towardZero))
        guard lines != 0 else { return }
        recognizer.setTranslation(
            CGPoint(x: translation.x, y: translation.y - CGFloat(lines) * cellHeight), in: self)
        _ = onScroll?(lines, recognizer.location(in: self))
    }

    /// The finger left, and its momentum did not.
    ///
    /// Released past an edge, the content springs back to it. Released inside,
    /// it coasts on `UIScrollView.DecelerationRate.normal` — the same curve as
    /// every other scroll view on the phone, which is the entire argument for
    /// using it.
    private func release(velocity released: CGFloat) {
        velocity = released
        if position < 0 || position > span {
            motion = .settling(target: position < 0 ? 0 : span)
        } else {
            motion = .coasting
        }
        startMotion()
    }

    @objc private func step(_ link: CADisplayLink) {
        // A link that has not fired yet reports `timestamp == 0`, so the first
        // frame has to be seeded here rather than in `startMotion` — seeded
        // from zero it would integrate the whole clamp of 50ms on frame one,
        // which at flick speed is a dozen rows arriving in a single jump right
        // where the motion is supposed to be handing over from the finger.
        if lastFrame == 0 { lastFrame = link.timestamp }
        let now = link.targetTimestamp
        // Clamped, so one dropped frame costs a slightly short step rather than
        // a lurch — the talk's point about what is IN the frames, not how many
        // of them there are.
        let dt = min(max(CGFloat(now - lastFrame), 0), 1.0 / 20)
        lastFrame = now
        guard dt > 0 else { return }

        switch motion {
        case .coasting:
            let next = TerminalScrollPhysics.decelerate(velocity: velocity, dt: dt)
            position += next.travelled
            velocity = next.velocity
            if position < 0 || position > span {
                // Ran off the end still moving. The spring takes the velocity
                // it had, so there is no moment where one behaviour stopped and
                // another started — the talk's seamless handoff, as arithmetic.
                motion = .settling(target: position < 0 ? 0 : span)
            } else if abs(velocity) < Self.coastFloor {
                motion = .settling(target: nearestRow(to: position))
            }
        case .settling(let target):
            let next = TerminalScrollPhysics.settle(
                value: position, velocity: velocity, toward: target,
                response: Self.settleResponse, dt: dt)
            position = next.value
            velocity = next.velocity
            // Half a point and eight points a second: under one frame's worth
            // of movement, which is the definition of "no longer visible".
            if abs(position - target) < 0.5, abs(velocity) < 8 {
                position = target
                velocity = 0
                motion = .rest
            }
        case .rest, .tracking:
            motion = .rest
        }

        commit()
        if motion == .rest { stopMotion() }
    }

    /// Ask the emulator for whatever whole rows the content has crossed, and
    /// publish the leftover for the grid to be drawn at.
    ///
    /// **One call per frame at most, and only when the row changed.** A finger
    /// crosses a row every few frames; a throw crosses fifty in the first
    /// quarter second, and asking once per row would be fifty separate hops in
    /// the time it takes to draw fifteen frames. So the row is computed from
    /// where the content IS and the difference is sent as one delta — which is
    /// coalescing in the only place it is safe to coalesce, because the drawn
    /// offset is measured from the row that came back, not from the row that
    /// was asked for.
    private func commit() {
        guard cellHeight > 0 else { return }
        var visible = shown
        let want = min(max(Int((visible / cellHeight).rounded()), 0), max(historyLines, 0))
        if want != committed {
            let actual = onScroll?(want - committed, lastTouch) ?? want
            // The emulator refused to go as far as it was asked, which means
            // the content ends before this class thought it did. Wherever it
            // stopped IS the end, so the over-drag has to start from there —
            // otherwise the grid would be drawn a row further along than the
            // rows it is actually showing, and the two would stay apart.
            if actual != want {
                position -= CGFloat(want - actual) * cellHeight
                visible = shown
            }
            committed = actual
        }
        let offset = visible - CGFloat(committed) * cellHeight
        let over: CGFloat
        if position < 0 {
            over = visible
        } else if position > span {
            over = visible - span
        } else {
            over = 0
        }
        readout.offset = offset
        readout.over = over
        if over == 0 {
            readout.grainPeak = max(readout.grainPeak, abs(offset))
        }
        readout.bandPeak = max(readout.bandPeak, abs(over))

        // Publishing crosses into SwiftUI and redraws the grid, so it happens
        // when there is something to see and not on every frame regardless. The
        // tail of a coast can spend half a second moving less than a point per
        // frame, and a redraw of several thousand cells to move the glyphs a
        // quarter of a point is work nobody can perceive the result of. A
        // quarter point is the threshold because that is under one device pixel
        // at 3× — below it there is literally nothing different to draw.
        //
        // `motion == .rest` always publishes: the last frame is the one that
        // sets the grid down on a row boundary, and it is the one every reader
        // of the peaks is looking at.
        let moved = abs(readout.offset - published.offset) >= 0.25
        guard moved || motion == .rest else { return }
        published = readout
        onReadout?(readout)
    }

    private func nearestRow(to points: CGFloat) -> CGFloat {
        guard cellHeight > 0 else { return 0 }
        let row = min(max((points / cellHeight).rounded(), 0), CGFloat(max(historyLines, 0)))
        return row * cellHeight
    }

    private func startMotion() {
        guard displayLink == nil else { return }
        let proxy = DisplayLinkProxy()
        proxy.sink = self
        let link = CADisplayLink(target: proxy, selector: #selector(DisplayLinkProxy.tick(_:)))
        // `.common`, so the coast does not stall while anything else on this
        // screen is tracking a touch.
        link.add(to: .main, forMode: .common)
        lastFrame = 0
        displayLink = link
    }

    private func stopMotion() {
        displayLink?.invalidate()
        displayLink = nil
        lastFrame = 0
    }

    /// A touch that stopped a coast leaves the grid wherever it caught it,
    /// which can be half a row down. Land it — called once that touch
    /// resolves as a lift with no drag, from `focus()` for the ordinary case
    /// and from `handleHold` for a press held long enough to win the
    /// platform's default exclusivity over the tap.
    ///
    /// Deliberately not called from `.changed`, and not called at all while
    /// the touch is still down: the content stays frozen exactly where the
    /// touch caught it for as long as that touch is on the glass — landing it
    /// on a row is the LIFT's business, not the touch-down's, or a finger
    /// resting on the pane while deciding what to do next would see the grid
    /// creep to the nearest row out from under it.
    private func settleAfterATouchThatDidNotDrag() {
        guard motion == .rest, displayLink == nil else { return }
        let target = position < 0 || position > span
            ? min(max(position, 0), span) : nearestRow(to: position)
        guard abs(position - target) > 0.5 else { return }
        motion = .settling(target: target)
        startMotion()
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        super.willMove(toWindow: newWindow)
        if newWindow == nil { stopMotion() }
    }

    /// A `CADisplayLink` retains its target, and this view owns the link. Held
    /// weakly through here so that a pane torn down mid-coast is not kept alive
    /// by its own animation.
    private final class DisplayLinkProxy: NSObject {
        weak var sink: KeystrokeSink?
        @objc func tick(_ link: CADisplayLink) { sink?.step(link) }
    }

    /// Report where a long press landed, once, when it begins — and, if this
    /// press is the one that stopped a coast, land the content on a row when
    /// it finally lifts.
    ///
    /// The second half exists because a long press wins the platform's
    /// default exclusivity over a tap once it reaches its own `.began` (0.5s
    /// by default): the tap on this same view never fires for that touch, so
    /// `focus()` — the usual place a stopped, undragged touch gets settled —
    /// never runs for it either. Without this, a press held long enough to
    /// open a link's menu, or just held and released over plain text, would
    /// leave the grid stranded mid-row until something else nudged it.
    ///
    /// `onHold?` fires once, on `.began` only — a press that is held reports
    /// repeatedly otherwise, and a dialog presented several times over is a
    /// dialog you cannot dismiss. The settle fires once too, on lift, and
    /// only when this press actually was the one that caught a moving pane.
    @objc private func handleHold(_ recognizer: UILongPressGestureRecognizer) {
        switch recognizer.state {
        case .began:
            onHold?(recognizer.location(in: self))
        case .ended, .cancelled:
            guard tapStoppedTheCoast else { return }
            tapStoppedTheCoast = false
            settleAfterATouchThatDidNotDrag()
        default:
            break
        }
    }

    func insertText(_ text: String) { onInsertText?(text) }
    func deleteBackward() { onDeleteBackward?() }

    // Every trait below exists to stop iOS from "helping": autocorrect
    // rewriting a flag, smart quotes producing a character no shell
    // recognizes, autocapitalization upper-casing the first letter of a
    // command nobody typed that way.
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var spellCheckingType: UITextSpellCheckingType = .no
    var keyboardType: UIKeyboardType = .asciiCapable
    var returnKeyType: UIReturnKeyType = .send
}

#if DEBUG

/// What one `draw` costs, and how many of them a second actually gets.
///
/// Both numbers, because either alone can lie. Time inside the closure misses
/// anything a `GraphicsContext` defers to its own rasterisation pass; frames a
/// second misses nothing but says nothing about where the time went. When the
/// two agree — when `ms` is about `1000 / draws` — the closure is the frame.
final class TerminalDrawMeter {
    private(set) var latest = "measuring…"
    private var samples: [Double] = []
    private var windowStart = CFAbsoluteTimeGetCurrent()
    private let label: String

    init(label: String) {
        self.label = label
    }

    func record(_ seconds: Double) {
        samples.append(seconds)
        let now = CFAbsoluteTimeGetCurrent()
        let elapsed = now - windowStart
        guard elapsed >= 1 else { return }
        let sorted = samples.sorted()
        let mean = samples.reduce(0, +) / Double(samples.count)
        let median = sorted[sorted.count / 2]
        let worst = sorted[sorted.count * 9 / 10]
        latest = String(
            format: "%@  %.0f draws/s  mean %.2fms  median %.2fms  p90 %.2fms",
            label, Double(samples.count) / elapsed, mean * 1000, median * 1000,
            worst * 1000)
        // The unified log rather than stdout, so `simctl spawn <device> log
        // stream` is the whole harness — an app launched by `simctl launch`
        // has no console attached and a `print` from it reaches nobody. A
        // number that has to be read off a screenshot is a number nobody
        // takes twice.
        NSLog("terminal-bench %@", latest)
        samples.removeAll(keepingCapacity: true)
        windowStart = now
    }
}

/// The terminal renderer under a stopwatch, and under a magnifying glass.
///
/// `-terminal-bench` draws a 55×38 grid — the size the owner's panes actually
/// are — at the display's refresh rate and reports what each draw costs.
/// Nothing about it touches the network or a host: the grid is a fixture, so
/// the number is the renderer's and not a poll's, and it is the same number on
/// a simulator and on a phone.
///
/// The flags exist so the answer can be attributed rather than asserted:
///
///   -terminal-bench                 the grid as it ships
///   -terminal-bench -bench-blank    the same grid with no characters in it,
///                                   which is every cost except the glyphs
///   -terminal-bench -bench-cell-text   one `Text` per cell, what it used to do
///   -terminal-bench -bench-row-text    one `Text` per row, the wrong batching
///
/// `-terminal-ligature` is not a benchmark: it is the fixture
/// `TerminalLigatureTests` photographs. See `TerminalBenchHarness.ligature`.
struct TerminalBenchHarness: View {
    static var isRequested: Bool {
        CommandLine.arguments.contains("-terminal-bench")
            || CommandLine.arguments.contains("-terminal-ligature")
    }

    private static var isLigatureFixture: Bool {
        CommandLine.arguments.contains("-terminal-ligature")
    }

    private static var path: TerminalDrawPath {
        if CommandLine.arguments.contains("-bench-cell-text") { return .cellText }
        if CommandLine.arguments.contains("-bench-row-text") { return .rowText }
        return .glyphs
    }

    /// Static, so that a body re-evaluation cannot quietly hand the benchmark
    /// a fresh stopwatch and start the window again.
    private static let meter = TerminalDrawMeter(
        label: "\(path.rawValue)\(CommandLine.arguments.contains("-bench-blank") ? "+blank" : "")")

    var body: some View {
        if Self.isLigatureFixture {
            ligature
        } else {
            benchmark
        }
    }

    // MARK: - The benchmark

    private static let columns = 55
    private static let rows = 38
    private static let fontSize: Double = 13

    private var renderer: TerminalRenderer {
        TerminalRenderer(fontChoice: .iosevka, fontSize: Self.fontSize, path: Self.path)
    }

    private var benchmark: some View {
        let grid = Self.benchGrid
        return VStack(spacing: 0) {
            // A live schedule, so the canvas is asked for a frame as often as
            // the screen can show one. Anything slower and the benchmark would
            // be measuring the schedule.
            TimelineView(.animation) { timeline in
                // The date is USED, and it has to be: a `Canvas` whose closure
                // captures nothing that changed is a `Canvas` SwiftUI can
                // legitimately decline to re-run, and the first version of this
                // benchmark measured exactly one draw and then sat there
                // reporting "measuring…". Feeding it through `scrolledBy` also
                // makes the benchmark the case that matters — a grid nudged
                // between two rows, sixty times a second, which is what a
                // finger on the scroll and the shell animating over the pane
                // both ask for.
                let phase = timeline.date.timeIntervalSinceReferenceDate
                // `-bench-still` pins the nudge at zero so two paths can be
                // photographed and compared pixel for pixel. It stops being a
                // benchmark at that point — a canvas whose inputs never change
                // is one SwiftUI need never redraw — and that is the point.
                let nudge: CGFloat =
                    CommandLine.arguments.contains("-bench-still")
                    ? 0 : CGFloat(sin(phase * 3) * 3)
                Canvas { context, size in
                    let start = CFAbsoluteTimeGetCurrent()
                    renderer.draw(
                        grid: grid, into: context, size: size, scrolledBy: nudge)
                    Self.meter.record(CFAbsoluteTimeGetCurrent() - start)
                }
                .overlay(alignment: .bottom) {
                    Text(Self.meter.latest)
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                        .foregroundStyle(.white)
                        .padding(6)
                        .background(.black.opacity(0.7))
                        .accessibilityIdentifier("terminal-bench-readout")
                }
            }
        }
        .background(TerminalPalette.background)
        .ignoresSafeArea()
        .preferredColorScheme(.dark)
    }

    /// A screenful that looks like work: code, punctuation a coding face has
    /// opinions about, a coloured prompt, a diff, and a selected line with a
    /// background of its own.
    ///
    /// Deterministic — the same screen every run, on every machine — because a
    /// benchmark whose input changes is two measurements of two things.
    private static let benchGrid: TerminalGrid = {
        let blank = CommandLine.arguments.contains("-bench-blank")
        let ground: UInt32 = 0x1E1E1E
        let lines = [
            "\u{1}~/overnight \u{4}main\u{0} $ cargo test -p farcooler-vt",
            "   Compiling farcooler-vt v0.1.0 (/Users/e/overnight/crates/vt)",
            "\u{2}    Finished\u{0} test profile [unoptimized] in 4.21s",
            "     Running unittests src/lib.rs (target/debug/deps/vt-9f2)",
            // The row that is not ASCII, and is here on purpose: a wide
            // ideograph that must take two columns, box drawing and arrows
            // Iosevka does have, an accented letter, and an emoji it does not
            // — which is the fallback face, and then the character no single
            // glyph stands for at all. Every one of those is a different
            // branch of the glyph path, and the pixel comparison against the
            // old renderer covers all of them at once.
            "\u{5}日本語\u{0} ┌─┤√≈✓ café 🚀 e\u{301}",
            "running 41 tests",
            "test grid::tests::a_wide_cell_takes_two_columns ... \u{2}ok\u{0}",
            "test grid::tests::inverse_swaps_fg_and_bg ... \u{2}ok\u{0}",
            "test parse::tests::csi_with_no_params_is_a_default ... \u{2}ok\u{0}",
            "test parse::tests::osc_52_round_trips ... \u{2}ok\u{0}",
            "test scroll::tests::display_offset_never_exceeds_history ... \u{2}ok\u{0}",
            "",
            "  if cursor.column != grid.columns - 1 {",
            "      let next = grid.cell(row, column + 1);",
            "      if next.width == 0 { return None; }   // wide spacer",
            "  }",
            "  let advance = if cell.is_wide() { 2 } else { 1 };",
            "  debug_assert!(column + advance <= self.columns);",
            "",
            "\u{3}diff --git a/crates/vt/src/grid.rs b/crates/vt/src/grid.rs\u{0}",
            "\u{5}@@ -211,7 +211,7 @@ impl Grid {\u{0}",
            "\u{2}+    pub fn cell(&self, row: usize, column: usize) -> Cell {\u{0}",
            "\u{6}-    pub fn cell(&self, row: usize, col: usize) -> Cell {\u{0}",
            "       self.rows[row].cells[column]",
            "   }",
            "",
            "\u{1}~/overnight \u{4}main\u{0} $ git status --short",
            " M crates/vt/src/grid.rs",
            " M apps/ios/FarCooler/TerminalView.swift",
            "?? docs/notes/frame-budget.md",
            "",
            "\u{1}~/overnight \u{4}main\u{0} $ rg -n 'CTFontDrawGlyphs' apps/",
            "apps/ios/FarCooler/TerminalView.swift:412:  CTFontDrawGlyphs(font,",
            "apps/macos/Sources/FarCooler/Terminal.swift:88:  CTFontDrawGlyphs(",
            "",
            "\u{1}~/overnight \u{4}main\u{0} $ ",
        ]
        // The palette, by the escape-like markers above: 0 default, 1 green,
        // 2 bright green, 3 white bold, 4 blue, 5 cyan, 6 red.
        let colors: [UInt32] = [
            0xD4D4D4, 0x6A9955, 0x4EC9B0, 0xE0E0E0, 0x569CD6, 0x9CDCFE, 0xF14C4C,
        ]
        var cells: [TerminalCell] = []
        cells.reserveCapacity(columns * rows)
        for row in 0..<rows {
            let line = row < lines.count ? lines[row] : ""
            var color = colors[0]
            var bold = false
            var background = ground
            // One row given a selection band, so the background-run batching
            // has something to batch.
            if row == 21 { background = 0x14301C }
            if row == 22 { background = 0x3A1418 }
            var written = 0
            for character in line {
                if let marker = character.asciiValue, marker < 8 {
                    color = colors[Int(marker)]
                    bold = marker == 3
                    continue
                }
                guard written < columns else { break }
                let wide = Self.isWide(character)
                cells.append(
                    TerminalCell(
                        character: blank ? nil : character, bold: bold, wide: wide,
                        foregroundPacked: color, backgroundPacked: background))
                written += 1
                // A wide character owns the column after it. The emulator emits
                // that spacer for real; the fixture has to as well, or every
                // column after an ideograph is off by one.
                if wide, written < columns {
                    cells.append(
                        TerminalCell(
                            character: nil, foregroundPacked: color,
                            backgroundPacked: background))
                    written += 1
                }
            }
            while written < columns {
                cells.append(
                    TerminalCell(
                        character: nil, foregroundPacked: colors[0],
                        backgroundPacked: background))
                written += 1
            }
        }
        return TerminalGrid(
            columns: columns, rows: rows, cells: cells, cursorRow: 36, cursorColumn: 14)
    }()

    /// The two-column characters, roughly: enough of East Asian Wide and of
    /// emoji for a fixture. The real answer lives in the core, which is where
    /// a pane's own `isWide` comes from.
    private static func isWide(_ character: Character) -> Bool {
        guard let scalar = character.unicodeScalars.first else { return false }
        switch scalar.value {
        case 0x1100...0x115F, 0x2E80...0x303E, 0x3041...0x33FF,
            0x3400...0x4DBF, 0x4E00...0x9FFF, 0xA000...0xA4CF,
            0xAC00...0xD7A3, 0xF900...0xFAFF, 0xFE30...0xFE6F,
            0xFF00...0xFF60, 0xFFE0...0xFFE6,
            0x1F300...0x1F64F, 0x1F900...0x1F9FF, 0x20000...0x3FFFD:
            return true
        default:
            return false
        }
    }

    // MARK: - The ligature fixture

    /// Two characters a coding face wants to join, and the same two characters
    /// on their own, drawn by the renderer the app ships.
    ///
    /// The whole assertion is that the pair looks like the singletons. Iosevka
    /// carries `!=` and `->` in `calt`, so a renderer that ever hands CoreText
    /// a run to shape draws half of `≠` in the first cell — a different glyph
    /// from `!`, at the same place — and the comparison fails. See
    /// `TerminalLigatureTests`, and `-bench-row-text`, which is a renderer
    /// that does exactly that and is here to be failed against.
    private static let ligatureFontSize: Double = 30
    private static let ligatureColumns = 16

    private var ligatureRenderer: TerminalRenderer {
        TerminalRenderer(
            fontChoice: .iosevka, fontSize: Self.ligatureFontSize, path: Self.path)
    }

    private var ligature: some View {
        let renderer = ligatureRenderer
        let cell = renderer.cellSize
        // Sized to exactly the content, so `layout` cannot scale it: the test
        // reads cell width off this view's own frame, and a scale it did not
        // know about would move every column it looks at.
        let width = 12 + CGFloat(Self.ligatureColumns) * cell.width
        let height = 12 + 2 * cell.height
        return VStack {
            // Clear of the status bar, which draws over anything that reaches
            // the top of the screen and would put the system clock through the
            // middle of the very cells the test measures.
            Spacer().frame(height: 80)
            Canvas { context, size in
                renderer.draw(grid: Self.ligatureGrid, into: context, size: size)
            }
            .frame(width: width, height: height)
            .accessibilityElement()
            .accessibilityIdentifier("terminal-ligature-fixture")
            // By name, like `terminal-surface`'s: the test reads `cell` and
            // `pad` out of it rather than recomputing the font metrics in the
            // test process, where the font is not even registered.
            .accessibilityValue(
                "cell=\(cell.width) line=\(cell.height) pad=6 "
                    + "columns=\(Self.ligatureColumns)")
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(TerminalPalette.background)
        .preferredColorScheme(.dark)
    }

    /// `!=` at columns 0–1, `!` alone at 4, `=` alone at 6; `->` at 8–9, `-`
    /// alone at 12, `>` alone at 14. The cursor is parked on the second row,
    /// out of the way of everything the test looks at.
    static let ligatureGrid: TerminalGrid = {
        var row: [Character?] = Array(repeating: nil, count: ligatureColumns)
        row[0] = "!"
        row[1] = "="
        row[4] = "!"
        row[6] = "="
        row[8] = "-"
        row[9] = ">"
        row[12] = "-"
        row[14] = ">"
        var cells: [TerminalCell] = []
        for character in row {
            cells.append(
                TerminalCell(
                    character: character, foregroundPacked: 0xFFFFFF,
                    backgroundPacked: 0x000000))
        }
        for _ in 0..<ligatureColumns {
            cells.append(
                TerminalCell(
                    character: nil, foregroundPacked: 0xFFFFFF, backgroundPacked: 0x000000))
        }
        return TerminalGrid(
            columns: ligatureColumns, rows: 2, cells: cells, cursorRow: 1,
            cursorColumn: ligatureColumns - 1)
    }()
}

#endif
