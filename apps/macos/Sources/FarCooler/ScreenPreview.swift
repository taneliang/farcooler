import AppKit
import SwiftUI

/// A terminal's last few lines, on a terminal's background.
///
/// Both colors come from `Palette` and not from a material or a system color,
/// and that is deliberate in an app that follows the system appearance
/// everywhere else: a preview drawn in the window's colors rather than the
/// terminal's would be a picture of a different terminal than the one behind
/// it.
///
/// This comment used to say that a terminal is "always dark here" and that
/// `Palette` is fixed for that reason. Both halves stopped being true when
/// `Palette` became live theme reads — see its note — and the text kept a
/// hardcoded `Color.white`, which is invisible on the three shipped themes
/// whose ground is `#FFFFFF` and barely there on the fourth light one. The
/// pairing has to come from the same theme or it cannot be reasoned about at
/// all, so the text now reads `Palette.foreground` beside the ground's
/// `Palette.background`.
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
        let font = Self.font(size: size)
        let advance = ("M" as NSString).size(withAttributes: [.font: font]).width
        return max(Int(width / max(advance, 1)), 4)
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

    /// How tall a box has to be to hold this many whole lines.
    ///
    /// Measured off the font, not chosen. A box a point short clips the top line
    /// through its middle, which does not read as "there is more above" — it
    /// reads as a rendering fault, and it is the first thing the eye finds in a
    /// grid of twelve. The half point per line is slack for the difference
    /// between the font's own line height and what SwiftUI lays out.
    static func height(lines: Int, size: CGFloat = 9) -> CGFloat {
        let font = Self.font(size: size)
        let line = font.ascender - font.descender + font.leading + 0.5
        return ceil(line * CGFloat(lines))
    }

    var body: some View {
        let font = Self.font(size: size)
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
        // something to read at this size, and twelve tiles at full strength
        // turn the panel into a wall. The alpha is over the theme's own
        // foreground, so it dims TOWARD the ground in either polarity rather
        // than washing out to white on a light one.
        .foregroundStyle(Color(nsColor: Palette.foreground).opacity(0.74))
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
