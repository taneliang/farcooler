import FarCoolerClient
import SwiftUI

/// A colour scheme: the terminal's palette, and which way the app around it
/// goes.
///
/// The same shape the Mac decodes, from the same source — the built-in table
/// in `farcooler_core::theme`. Neither app defines a colour of its own, which
/// is what stops "Nord" meaning two different things on two screens.
struct Theme: Decodable, Equatable, Identifiable {
    var name: String
    var dark: Bool
    /// Packed `0x00RRGGBB`, the form the VT core's snapshot already uses.
    var background: UInt32
    var foreground: UInt32
    var cursor: UInt32
    /// Sixteen: ANSI 0-7 then 8-15.
    var ansi: [UInt32]

    var id: String { name }

    /// The nineteen values `farcooler_vt_set_palette` expects, in its order.
    var packed: [UInt32] { ansi + [foreground, background, cursor] }

    var backgroundColor: Color { Color(packed: background) }
    var foregroundColor: Color { Color(packed: foreground) }
    var cursorColor: Color { Color(packed: cursor) }

    /// What the whole app is tinted by while a light theme is in force.
    var colorScheme: ColorScheme { dark ? .dark : .light }

    /// The one theme that exists before the core has been asked. See
    /// `Themes.builtIn` — this is only the gap between launch and that call,
    /// which is microseconds, but a `nil` here would be a black screen.
    static let fallback = Theme(
        name: "Nord",
        dark: true,
        background: 0x2E_34_40,
        foreground: 0xD8_DE_E9,
        cursor: 0xD8_DE_E9,
        ansi: [
            0x3B_42_52, 0xBF_61_6A, 0xA3_BE_8C, 0xEB_CB_8B, 0x81_A1_C1, 0xB4_8E_AD, 0x88_C0_D0,
            0xE5_E9_F0, 0x4C_56_6A, 0xBF_61_6A, 0xA3_BE_8C, 0xEB_CB_8B, 0x81_A1_C1, 0xB4_8E_AD,
            0x8F_BC_BB, 0xEC_EF_F4,
        ])
}

/// Every theme this phone offers, and which one is in force.
///
/// Two sources, one list. The built-ins come from the client core with no
/// connection at all — a phone on a train still has to render something — and
/// whatever the connected machine defines is merged on top, host winning a
/// name collision because it is the more specific statement.
@MainActor
final class Themes: ObservableObject {
    static let shared = Themes()

    @Published private(set) var available: [Theme] = []
    /// Bumped whenever the colours in force change, so a live terminal
    /// repaints. The same mechanism the font size already uses.
    @Published private(set) var revision = 0

    @AppStorage("app.theme") private var stored = Theme.fallback.name

    private init() {
        available = Themes.builtIn()
    }

    /// The theme in force.
    ///
    /// Falls back rather than to nothing when a stored name no longer
    /// resolves: a theme that vanished because a host's config file moved
    /// should cost you your colours, not your terminal.
    var current: Theme {
        available.first { $0.name == stored } ?? .fallback
    }

    var selectedName: String {
        get { stored }
        set {
            stored = newValue
            revision += 1
        }
    }

    /// The themes compiled into the client core, with no session required.
    private static func builtIn() -> [Theme] {
        var buffer = [UInt8](repeating: 0, count: 8192)
        let needed = farcooler_client_builtin_themes(&buffer, buffer.count)
        guard needed > 0 else { return [.fallback] }
        if needed > buffer.count {
            buffer = [UInt8](repeating: 0, count: needed)
            guard farcooler_client_builtin_themes(&buffer, buffer.count) == needed else {
                return [.fallback]
            }
        }
        struct Reply: Decodable {
            var themes: [Theme]
        }
        let data = Data(buffer[..<needed])
        let decoded = (try? JSONDecoder().decode(Reply.self, from: data))?.themes
        return (decoded?.isEmpty == false ? decoded! : [.fallback])
    }

    /// Merge in whatever the connected machine defines.
    ///
    /// Additive rather than replacing: the built-ins are this phone's and do
    /// not depend on which machine it happens to be talking to, so switching
    /// hosts must not empty the picker. Only the host's own entries move.
    func merge(hostThemes: [Theme]) {
        var merged = Themes.builtIn()
        for theme in hostThemes {
            // The host wins a name collision — it is the one somebody edited a
            // file on purpose to make.
            if let index = merged.firstIndex(where: { $0.name == theme.name }) {
                merged[index] = theme
            } else {
                merged.append(theme)
            }
        }
        guard merged != available else { return }
        available = merged
        revision += 1
    }
}
