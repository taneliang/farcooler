import CoreText
import SwiftUI
import UIKit

// The terminal renderer's stopwatch and fixtures (`-terminal-bench`,
// `-terminal-ligature`), moved out of TerminalView.swift to give it room under
// its size ceiling (ov-609). Debug builds only.

#if DEBUG

/// What one `draw` costs, and how many of them a second actually gets.
///
/// Both numbers, because either alone can lie. Time inside the closure misses
/// anything a `GraphicsContext` defers to its own rasterization pass; frames a
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
                        .background(.black.opacity(0.7))  // style-exempt: the bench readout, DEBUG only
                        .accessibilityIdentifier("terminal-bench-readout")
                }
            }
        }
        .background(TerminalPalette.background)
        .ignoresSafeArea()
        .preferredColorScheme(.dark)
    }

    /// A screenful that looks like work: code, punctuation a coding face has
    /// opinions about, a colored prompt, a diff, and a selected line with a
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
