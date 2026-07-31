import SwiftUI
import UIKit

/// The terminal's typeface: bundled Iosevka, or the phone's own monospaced
/// system font.
///
/// Not a free-form font picker. `TerminalView` draws a fixed grid of one
/// glyph per cell — see `draw(grid:into:size:)` — and anything that is not
/// genuinely monospaced would misalign the exact thing a terminal is. Two
/// options is the whole space worth offering: Iosevka for the box-drawing and
/// powerline glyphs coding agents print constantly (`Fonts/README.md`), and
/// System as the face that needs no bundle to have shipped correctly.
enum TerminalFontChoice: String, CaseIterable, Identifiable {
    case iosevka
    case system

    var id: String { rawValue }

    var label: String {
        switch self {
        case .iosevka: return "Iosevka"
        case .system: return "System Monospaced"
        }
    }
}

/// Where the terminal's appearance settings live.
///
/// Bare keys rather than a settings object that owns the values: what makes
/// `TerminalView` redraw the instant one of these changes is `@AppStorage`
/// itself, reading the same `UserDefaults` key `SettingsView`'s controls
/// write to. An intermediary object would need its own way of publishing
/// changes to do what the property wrapper already does for free.
enum TerminalSettings {
    static let fontKey = "terminalFont"
    static let fontSizeKey = "terminalFontSize"

    /// Matches the fixed size the phone rendered at before this screen
    /// existed, so installing this change does not itself resize anyone's
    /// terminal.
    static let defaultFontSize: Double = 13
    static let minFontSize: Double = 9
    static let maxFontSize: Double = 22
}

/// Whether the bundled font actually made it into the running app.
///
/// A font missing from `UIAppFonts`, or listed there under the wrong
/// filename, does not throw or crash anywhere — `UIFont(name:)` just returns
/// nil the first time something asks, and every caller quietly gets the
/// system font back. That reads as "the font picker did nothing" rather than
/// "the font is missing," which is indistinguishable from Iosevka simply
/// being an unremarkable typeface unless something checks on purpose.
enum FontRegistry {
    static let iosevkaFamily = "Iosevka Nerd Font Mono"
    static let iosevkaRegularPostScriptName = "IosevkaNFM"
    static let iosevkaBoldPostScriptName = "IosevkaNFM-Bold"

    static var iosevkaIsRegistered: Bool {
        UIFont.familyNames.contains(iosevkaFamily)
    }
}

extension UIFont {
    /// Resolve the chosen terminal typeface for measuring — `TerminalMetrics`
    /// needs a concrete `UIFont` to lay a glyph out and read its width back.
    ///
    /// Falls back to the system face rather than force-unwrapping, matching
    /// `Font.terminal(_:size:bold:)` below: the two must agree, because one
    /// measures the cell a grid is laid out in and the other draws the glyph
    /// that has to land inside it.
    static func terminal(_ choice: TerminalFontChoice, size: CGFloat, bold: Bool = false) -> UIFont {
        switch choice {
        case .system:
            return .monospacedSystemFont(ofSize: size, weight: bold ? .semibold : .regular)
        case .iosevka:
            let name = bold ? FontRegistry.iosevkaBoldPostScriptName : FontRegistry.iosevkaRegularPostScriptName
            return UIFont(name: name, size: size) ?? .monospacedSystemFont(ofSize: size, weight: bold ? .semibold : .regular)
        }
    }
}

extension Font {
    /// The SwiftUI half of `UIFont.terminal(_:size:bold:)`, kept in sync with
    /// it by construction: both switch on the same `FontRegistry` names and
    /// both fall back to the system face under the same condition, so a
    /// glyph is never measured with one font and drawn with another.
    static func terminal(_ choice: TerminalFontChoice, size: CGFloat, bold: Bool = false) -> Font {
        switch choice {
        case .system:
            return .system(size: size, weight: bold ? .semibold : .regular, design: .monospaced)
        case .iosevka where FontRegistry.iosevkaIsRegistered:
            let name = bold ? FontRegistry.iosevkaBoldPostScriptName : FontRegistry.iosevkaRegularPostScriptName
            return .custom(name, size: size)
        case .iosevka:
            return .system(size: size, weight: bold ? .semibold : .regular, design: .monospaced)
        }
    }
}

/// The app's only settings screen: what the terminal looks like.
///
/// There is nothing else in Overnight to configure — no accounts, no sync,
/// no notification thresholds — so this does not split into a "Fonts"
/// section of a larger screen; it IS the screen. Every control writes
/// straight to `@AppStorage`, so there is nothing to save and nothing to
/// throw away on dismiss.
struct SettingsView: View {
    @AppStorage(TerminalSettings.fontKey) private var fontChoice: TerminalFontChoice = .iosevka
    @AppStorage(TerminalSettings.fontSizeKey) private var fontSize: Double = TerminalSettings.defaultFontSize
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section("Terminal font") {
                Picker("Typeface", selection: $fontChoice) {
                    ForEach(TerminalFontChoice.allCases) { choice in
                        Text(choice.label).tag(choice)
                    }
                }

                // Surfaced rather than trusted silently — see
                // `FontRegistry.iosevkaIsRegistered`'s doc comment for why a
                // failed registration otherwise looks like nothing happened.
                if fontChoice == .iosevka && !FontRegistry.iosevkaIsRegistered {
                    Label(
                        "Iosevka did not register on this build — showing the system font instead.",
                        systemImage: "exclamationmark.triangle"
                    )
                    .font(.footnote)
                    .foregroundStyle(.orange)
                }

                HStack {
                    Text("Size")
                    Slider(
                        value: $fontSize,
                        in: TerminalSettings.minFontSize...TerminalSettings.maxFontSize,
                        step: 1
                    )
                    Text("\(Int(fontSize))")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(width: 22, alignment: .trailing)
                }
            }

            Section("Preview") {
                // The glyphs a coding agent actually prints — box drawing and
                // a couple of Nerd Font icons — not lorem ipsum. Lorem ipsum
                // in a monospaced regular weight looks identical whichever
                // font failed to load; this does not.
                Text("┌─ \u{f126} claude · \u{e0a0} main\n│ 12 files changed")
                    .font(.terminal(fontChoice, size: fontSize))
                    .foregroundStyle(.white)
                    .lineSpacing(4)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(TerminalPalette.background)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            // Presented as a sheet (see `RootView`), which gets no back
            // button of its own — without an explicit dismiss there would be
            // no way off this screen except the system's edge-swipe.
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        // A settings screen over a terminal app is still, in effect, a
        // terminal screen: it exists to preview one. See OvernightApp's
        // reasoning for forcing dark everywhere rather than just over the
        // grid itself.
        .preferredColorScheme(.dark)
    }
}
