import AppKit
import SwiftUI

/// The last few lines of each terminal on screen, kept fresh while a panel is
/// looking at them.
///
/// Not a second terminal emulator. The switcher's tiles are 224 points wide and
/// show eight lines: attaching a real `TerminalSurface` to each would mean a
/// dozen live tmux streams and a dozen VT cores for a picture nobody reads word
/// by word, and would fight the one surface that is actually being typed into.
/// A snapshot of text is the whole of what a tile needs.
///
/// Lifetime is the view's, not the app's. There is no timer here and nothing
/// starts on its own — the panel drives `refresh` from a `.task`, so closing it
/// cancels the loop and a closed palette costs exactly nothing. That is the
/// property worth protecting: this shells out once per tile, and a background
/// poll of seventeen terminals nobody is looking at would be indefensible.
@MainActor
final class ScreenPreviews: ObservableObject {
    /// Keyed by a terminal's short id, which is what the CLI takes.
    @Published private(set) var tails: [String: [String]] = [:]

    /// Fetch the terminals given, in order, and forget the rest.
    ///
    /// Sequential rather than concurrent. Each fetch is a subprocess that
    /// returns in single-digit milliseconds, so a dozen in a row is well under a
    /// frame's worth of wall clock — and doing them one at a time means the
    /// panel can never have twelve processes outstanding if the daemon is slow,
    /// which is exactly when it would matter.
    func refresh(_ shorts: [String], lines: Int, using screen: (String) async -> String) async {
        for short in shorts {
            let raw = await screen(short)
            // Cancellation is checked between fetches rather than only at the
            // top: the panel closes mid-sweep constantly, and writing to
            // `@Published` after that is a view update for a view that has gone.
            if Task.isCancelled { return }
            tails[short] = ScreenPreviews.tail(of: raw, lines: lines)
        }

        // Dropped rather than kept, because the cheapest cache to reason about
        // is one that holds exactly what is on screen. Nothing here is expensive
        // enough to be worth keeping for a revisit.
        let showing = Set(shorts)
        tails = tails.filter { showing.contains($0.key) }
    }

    /// The bottom of a rendered screen, as plain lines.
    ///
    /// The bottom, because that is where a terminal's present tense is — a
    /// question waiting for an answer, the command that is running, the error
    /// that stopped everything. The top of the screen is history.
    static func tail(of raw: String, lines: Int) -> [String] {
        var rows = plain(raw)
            .split(separator: "\n", omittingEmptySubsequences: false)
            // Truncated well beyond any tile's width. The point is to stop a
            // pathological line — a base64 blob, a minified bundle — from being
            // laid out at full length only to be clipped.
            .map { String($0.prefix(400)).trimmingTrailingWhitespace() }

        // Trailing blank lines are what a program leaves behind when it clears
        // to the bottom of the pane, and a tile filled with them would show
        // nothing while claiming the terminal is empty.
        while let last = rows.last, last.isEmpty { rows.removeLast() }
        return Array(rows.suffix(lines))
    }

    /// Escape sequences out, text in.
    ///
    /// `terminal screen` returns the rendered screen with its color escapes
    /// intact, because that is what a real renderer wants. A tile is not one:
    /// it draws a paragraph of `Text`, so the escapes have to go, and color
    /// goes with them. Losing the color costs less than it sounds — at nine
    /// points, syntax highlighting is noise, and the shape of the text is what
    /// makes a screen recognizable.
    static func plain(_ raw: String) -> String {
        var out = String.UnicodeScalarView()
        let scalars = Array(raw.unicodeScalars)
        var index = 0

        while index < scalars.count {
            let scalar = scalars[index]
            guard scalar.value == 0x1B else {
                switch scalar.value {
                case 0x0A, 0x09: out.append(scalar)
                // Carriage returns and every other C0 control are structure the
                // daemon has already applied; what arrives here is a laid-out
                // screen, so they can only corrupt it.
                case 0..<0x20, 0x7F: break
                default: out.append(scalar)
                }
                index += 1
                continue
            }

            index += 1
            guard index < scalars.count else { break }
            let kind = scalars[index]
            index += 1

            switch kind {
            case "[":
                // CSI: parameters, then a final byte in @ … ~.
                while index < scalars.count, !(0x40...0x7E).contains(scalars[index].value) {
                    index += 1
                }
                index += 1
            case "]", "P", "X", "^", "_":
                // A string-terminated sequence — OSC and friends. Ends at BEL or
                // at ST, and the terminator is two scalars when it is ST.
                while index < scalars.count {
                    if scalars[index].value == 0x07 { index += 1; break }
                    if scalars[index].value == 0x1B, index + 1 < scalars.count,
                        scalars[index + 1] == "\\"
                    {
                        index += 2
                        break
                    }
                    index += 1
                }
            default:
                // A two-character escape; already consumed.
                break
            }
        }
        return String(out)
    }

    /// The terminal's own face, at tile size.
    ///
    /// Not `Preferences.terminalFont()`. That carries the user's chosen SIZE,
    /// which is set for text you read for hours and is roughly twice what fits
    /// in a tile. The FACE is the part worth honouring: a preview typeset in a
    /// different typeface from the terminal it is a preview of reads as a
    /// picture of some other program.
    static func font(size: CGFloat) -> NSFont {
        let name = Preferences.shared.fontName
        if name != Preferences.defaultFontName, let font = NSFont(name: name, size: size) {
            return font
        }
        return .monospacedSystemFont(ofSize: size, weight: .regular)
    }
}

/// A terminal's last few lines, on a terminal's background.
///
/// The background is `Palette.background` and not a material or a system
/// color, and that is deliberate in an app that follows the system appearance
/// everywhere else: a terminal is always dark here, `Palette` is fixed for that
/// reason, and a preview that turned white at noon would be a picture of a
/// different terminal than the one behind it.
struct ScreenPreviewText: View {
    let lines: [String]
    /// The tile's inner width, used to work out where to cut each line.
    let width: CGFloat
    var size: CGFloat = 9

    /// Cut to the column count that fits, rather than left to truncate.
    ///
    /// SwiftUI would end each over-long line with an ellipsis, which is right
    /// for a title and wrong here — a terminal clips at its right edge, and a
    /// column of ellipses reads as a list of names rather than as a screen.
    private var columns: Int {
        let font = ScreenPreviews.font(size: size)
        let advance = ("M" as NSString).size(withAttributes: [.font: font]).width
        return max(Int(width / max(advance, 1)), 4)
    }

    /// How tall a box has to be to hold this many whole lines.
    ///
    /// Measured off the font, not chosen. A box a point short clips the top line
    /// through its middle, which does not read as "there is more above" — it
    /// reads as a rendering fault, and it is the first thing the eye finds in a
    /// grid of twelve. The half point per line is slack for the difference
    /// between the font's own line height and what SwiftUI lays out.
    static func height(lines: Int, size: CGFloat = 9) -> CGFloat {
        let font = ScreenPreviews.font(size: size)
        let line = font.ascender - font.descender + font.leading + 0.5
        return ceil(line * CGFloat(lines))
    }

    var body: some View {
        let font = ScreenPreviews.font(size: size)
        let fitted = columns

        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                // An empty line still has to occupy its row, or the last eight
                // lines of a sparse screen would collapse into three and the
                // tile would jump every time the terminal blanked one.
                Text(line.isEmpty ? " " : String(line.prefix(fitted)))
                    .font(Font(font))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        // Dimmed a little. The text is a hint at what is on the screen, not
        // something to read at this size, and full-strength white in twelve
        // tiles at once turns the panel into a wall.
        .foregroundStyle(Color.white.opacity(0.74))
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension String {
    /// Trailing spaces are how a full-width screen pads its short lines; they
    /// are not content and they defeat "is this line blank?".
    func trimmingTrailingWhitespace() -> String {
        var copy = self
        while let last = copy.last, last.isWhitespace { copy.removeLast() }
        return copy
    }
}
