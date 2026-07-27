import AppKit
import Foundation

/// Turns tmux's captured screen into styled text.
///
/// Overnight does not emulate a VT. tmux already parsed the program's output and
/// maintains the rendered screen, so `capture-pane -e` hands back the finished
/// picture with SGR colour intact. This only has to understand SGR, not cursor
/// motion, scroll regions, or the alternate screen, because tmux resolved all of
/// that before we ever saw it.
enum ANSIRenderer {

    struct Style {
        var fg: NSColor?
        var bg: NSColor?
        var bold = false
        var italic = false
        var underline = false
        var reverse = false
        var dim = false

        mutating func reset() { self = Style() }
    }

    static func attributed(_ raw: String, font: NSFont, defaultFG: NSColor) -> NSAttributedString {
        let out = NSMutableAttributedString()
        var style = Style()
        let chars = Array(raw)
        var i = 0
        var run = ""

        func flush() {
            guard !run.isEmpty else { return }
            out.append(NSAttributedString(string: run, attributes: attrs(style, font, defaultFG)))
            run = ""
        }

        while i < chars.count {
            let c = chars[i]

            // Only CSI ... m carries styling. Any other escape sequence is
            // consumed and discarded so it never leaks into the visible text.
            if c == "\u{1b}", i + 1 < chars.count, chars[i + 1] == "[" {
                var j = i + 2
                var params = ""
                while j < chars.count, !("@"..."~").contains(chars[j]) {
                    params.append(chars[j])
                    j += 1
                }
                let final = j < chars.count ? chars[j] : "m"
                if final == "m" { flush(); apply(params, to: &style) }
                i = j + 1
                continue
            }

            // OSC: ESC ] ... terminated by BEL or ST (ESC \).
            //
            // These are variable length, so skipping a fixed two characters
            // leaks the body as visible text. Coding agents use OSC 8 for
            // clickable links and OSC 0 for the window title, and both showed up
            // as garbage like "8;id=xxx;https://...".
            if c == "\u{1b}", i + 1 < chars.count, chars[i + 1] == "]" {
                var j = i + 2
                while j < chars.count {
                    if chars[j] == "\u{07}" { j += 1; break }
                    if chars[j] == "\u{1b}", j + 1 < chars.count, chars[j + 1] == "\\" {
                        j += 2
                        break
                    }
                    j += 1
                }
                i = j
                continue
            }

            // Any other escape: skip the introducer and its single final byte.
            if c == "\u{1b}" {
                i += 2
                continue
            }

            // A stray BEL is a control code, not text.
            if c == "\u{07}" {
                i += 1
                continue
            }

            run.append(c)
            i += 1
        }

        flush()
        return out
    }

    private static func attrs(
        _ s: Style, _ font: NSFont, _ defaultFG: NSColor
    ) -> [NSAttributedString.Key: Any] {
        var f = font
        if s.bold, let b = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) as NSFont? {
            f = b
        }
        if s.italic,
            let it = NSFontManager.shared.convert(f, toHaveTrait: .italicFontMask) as NSFont?
        {
            f = it
        }

        var fg = s.fg ?? defaultFG
        var bg = s.bg
        if s.reverse {
            let swapped = bg ?? NSColor.textBackgroundColor
            bg = fg
            fg = swapped
        }
        if s.dim { fg = fg.withAlphaComponent(0.6) }

        var a: [NSAttributedString.Key: Any] = [.font: f, .foregroundColor: fg]
        if let bg { a[.backgroundColor] = bg }
        if s.underline { a[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        return a
    }

    private static func apply(_ params: String, to style: inout Style) {
        let codes = params.split(separator: ";", omittingEmptySubsequences: false)
            .map { Int($0) ?? 0 }
        var k = 0
        if codes.isEmpty { style.reset() }

        while k < codes.count {
            let code = codes[k]
            switch code {
            case 0: style.reset()
            case 1: style.bold = true
            case 2: style.dim = true
            case 3: style.italic = true
            case 4: style.underline = true
            case 7: style.reverse = true
            case 22: style.bold = false; style.dim = false
            case 23: style.italic = false
            case 24: style.underline = false
            case 27: style.reverse = false
            case 30...37: style.fg = palette(code - 30)
            case 39: style.fg = nil
            case 40...47: style.bg = palette(code - 40)
            case 49: style.bg = nil
            case 90...97: style.fg = palette(code - 90 + 8)
            case 100...107: style.bg = palette(code - 100 + 8)
            case 38, 48:
                // Extended colour: 5;N for 256, 2;R;G;B for truecolour.
                let isFG = code == 38
                if k + 1 < codes.count, codes[k + 1] == 5, k + 2 < codes.count {
                    let c = xterm256(codes[k + 2])
                    if isFG { style.fg = c } else { style.bg = c }
                    k += 2
                } else if k + 1 < codes.count, codes[k + 1] == 2, k + 4 < codes.count {
                    let c = NSColor(
                        srgbRed: CGFloat(codes[k + 2]) / 255,
                        green: CGFloat(codes[k + 3]) / 255,
                        blue: CGFloat(codes[k + 4]) / 255, alpha: 1)
                    if isFG { style.fg = c } else { style.bg = c }
                    k += 4
                }
            default: break
            }
            k += 1
        }
    }

    /// The 16 ANSI colours, tuned to stay legible on a dark terminal ground.
    private static func palette(_ i: Int) -> NSColor {
        switch i {
        case 0: return NSColor(srgbRed: 0.26, green: 0.28, blue: 0.33, alpha: 1)
        case 1: return NSColor(srgbRed: 0.94, green: 0.38, blue: 0.40, alpha: 1)
        case 2: return NSColor(srgbRed: 0.42, green: 0.82, blue: 0.51, alpha: 1)
        case 3: return NSColor(srgbRed: 0.90, green: 0.75, blue: 0.36, alpha: 1)
        case 4: return NSColor(srgbRed: 0.44, green: 0.66, blue: 0.95, alpha: 1)
        case 5: return NSColor(srgbRed: 0.79, green: 0.55, blue: 0.94, alpha: 1)
        case 6: return NSColor(srgbRed: 0.36, green: 0.79, blue: 0.82, alpha: 1)
        case 7: return NSColor(srgbRed: 0.83, green: 0.85, blue: 0.88, alpha: 1)
        case 8: return NSColor(srgbRed: 0.42, green: 0.45, blue: 0.51, alpha: 1)
        case 9: return NSColor(srgbRed: 1.00, green: 0.51, blue: 0.52, alpha: 1)
        case 10: return NSColor(srgbRed: 0.56, green: 0.92, blue: 0.63, alpha: 1)
        case 11: return NSColor(srgbRed: 0.98, green: 0.86, blue: 0.49, alpha: 1)
        case 12: return NSColor(srgbRed: 0.58, green: 0.76, blue: 1.00, alpha: 1)
        case 13: return NSColor(srgbRed: 0.88, green: 0.68, blue: 1.00, alpha: 1)
        case 14: return NSColor(srgbRed: 0.51, green: 0.90, blue: 0.92, alpha: 1)
        default: return NSColor(srgbRed: 0.96, green: 0.97, blue: 0.98, alpha: 1)
        }
    }

    private static func xterm256(_ n: Int) -> NSColor {
        if n < 16 { return palette(n) }
        if n < 232 {
            let i = n - 16
            let steps: [CGFloat] = [0, 95, 135, 175, 215, 255]
            return NSColor(
                srgbRed: steps[(i / 36) % 6] / 255,
                green: steps[(i / 6) % 6] / 255,
                blue: steps[i % 6] / 255, alpha: 1)
        }
        let g = CGFloat(8 + (n - 232) * 10) / 255
        return NSColor(srgbRed: g, green: g, blue: g, alpha: 1)
    }
}
