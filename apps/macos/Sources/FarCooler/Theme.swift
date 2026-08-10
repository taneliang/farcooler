import AppKit
import SwiftUI

/// A colour scheme: the terminal's palette, and which way the app around it
/// goes.
///
/// Both halves in one value, because a light terminal inside black chrome is
/// two applications sharing a window — the failure the phones' forced dark was
/// introduced to avoid, and the reason this feature could not have been the
/// terminal alone.
///
/// Decoded from `farcooler theme list --json` rather than defined here. The
/// built-in table lives in `farcooler_core::theme` and is merged with the
/// host's own by the CLI, so the Mac, the iPhone and the Android client cannot
/// come to disagree about what "Nord" means.
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

    /// What a client falls back to before it has asked anyone, and if a stored
    /// name no longer resolves.
    ///
    /// Duplicated from `farcooler_core::theme` deliberately and kept small: it
    /// is the one theme that has to exist before the CLI has answered, and the
    /// alternative is a window that renders with no colours at all for the
    /// hundred milliseconds a subprocess takes. Everything else — including
    /// this same theme, once the list arrives — comes from the core.
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

    var backgroundColor: NSColor { Theme.color(background) }
    var foregroundColor: NSColor { Theme.color(foreground) }
    var cursorColor: NSColor { Theme.color(cursor) }

    /// The selection wash.
    ///
    /// Derived rather than carried, because no terminal theme in the world
    /// specifies one and inventing a twentieth colour for every theme file
    /// would be asking authors to answer a question they do not have. ANSI
    /// blue at low alpha reads as a selection on every ground this ships.
    var selectionColor: NSColor { Theme.color(ansi.count > 4 ? ansi[4] : foreground).withAlphaComponent(0.45) }

    static func color(_ packed: UInt32) -> NSColor {
        NSColor(
            srgbRed: CGFloat((packed >> 16) & 0xFF) / 255,
            green: CGFloat((packed >> 8) & 0xFF) / 255,
            blue: CGFloat(packed & 0xFF) / 255,
            alpha: 1)
    }
}

/// The visual vocabulary shared by the native workbench around the terminal.
///
/// Terminal themes still own terminal cells. These surfaces borrow only a small
/// amount of their hue, blended into native macOS colors, so a Nord terminal and
/// a native diff read as neighboring tools instead of unrelated applications.
/// Keeping the values here also stops every pane and sidebar row from inventing
/// another almost-identical gray, font size, and control target.
@MainActor
enum WorkspaceStyle {
    static let paneHeaderHeight: CGFloat = 28
    static let controlTarget: CGFloat = 24

    static let paneTitle = Font.system(size: 11.5, weight: .medium)
    static let sidebarPrimary = Font.system(size: 13, weight: .medium)
    static let sidebarMetadata = Font.system(size: 11)
    static let sectionTitle = Font.system(size: 11.5, weight: .semibold)

    static var canvas: Color {
        Color(nsColor: blend(.windowBackgroundColor, withTheme: 0.05))
    }

    static var sidebar: Color {
        Color(nsColor: blend(.windowBackgroundColor, withTheme: 0.035))
    }

    static var document: Color {
        Color(nsColor: blend(.textBackgroundColor, withTheme: 0.09))
    }

    static var paneChrome: Color {
        Color(nsColor: blend(.controlBackgroundColor, withTheme: 0.14))
    }

    static var hairline: Color { Color.primary.opacity(0.11) }

    /// Resolve dynamic system colors in the app's effective appearance before
    /// mixing. Otherwise Aqua's light value can be captured while Dark Aqua is
    /// on screen, producing a flash when the theme changes.
    private static func blend(_ system: NSColor, withTheme amount: CGFloat) -> NSColor {
        let appearance = NSApp?.effectiveAppearance
            ?? NSAppearance(named: Themes.shared.current.dark ? .darkAqua : .aqua)!
        var native = system
        appearance.performAsCurrentDrawingAppearance {
            native = system.usingColorSpace(.sRGB) ?? system
        }
        let themed = Themes.shared.current.backgroundColor.usingColorSpace(.sRGB)
            ?? Themes.shared.current.backgroundColor

        var nr: CGFloat = 0, ng: CGFloat = 0, nb: CGFloat = 0, na: CGFloat = 0
        var tr: CGFloat = 0, tg: CGFloat = 0, tb: CGFloat = 0, ta: CGFloat = 0
        native.getRed(&nr, green: &ng, blue: &nb, alpha: &na)
        themed.getRed(&tr, green: &tg, blue: &tb, alpha: &ta)

        return NSColor(
            srgbRed: nr + (tr - nr) * amount,
            green: ng + (tg - ng) * amount,
            blue: nb + (tb - nb) * amount,
            alpha: na)
    }
}

/// A pane's chrome is one semantic surface. Focus adds a wash rather than a
/// border, so switching panes is obvious without boxing the content in blue.
struct PaneHeaderBackground: View {
    let focused: Bool

    var body: some View {
        ZStack {
            WorkspaceStyle.paneChrome
            if focused { Color.accentColor.opacity(0.10) }
        }
    }
}

/// Every theme this machine offers, and which one is in force.
///
/// One object rather than a preference holding colours: a theme defined on a
/// host can be EDITED, and a client that had cached its values would go on
/// showing the old ones forever. What is stored is the name; the colours are
/// re-read whenever the list is.
@MainActor
final class Themes: ObservableObject {
    static let shared = Themes()

    @Published private(set) var available: [Theme] = [.fallback]

    /// Bumped whenever the colours in force change, so a live terminal view
    /// repaints. The same mechanism `Preferences.revision` uses for fonts.
    @Published private(set) var revision = 0

    /// The chosen theme's NAME, not its colours — see the type's note.
    var selectedName: String {
        get { UserDefaults.standard.string(forKey: "app.theme") ?? Theme.fallback.name }
        set {
            UserDefaults.standard.set(newValue, forKey: "app.theme")
            revision += 1
            objectWillChange.send()
        }
    }

    /// A binding for the picker.
    ///
    /// `selectedName` reads and writes `UserDefaults` directly rather than
    /// being `@Published`, because what is stored is deliberately a name and
    /// not a value — see the type's note — and `@AppStorage` cannot live on an
    /// `ObservableObject`. This is the adapter, and it re-applies the
    /// appearance so a theme change reaches the chrome in the same turn it
    /// reaches the terminal.
    var selectedNameBinding: Binding<String> {
        Binding(
            get: { self.selectedName },
            set: {
                self.selectedName = $0
                Appearance.apply(Preferences.shared.appearance)
            })
    }

    /// The theme in force. Falls back rather than to nothing when a stored
    /// name no longer resolves — a theme that vanished because a config file
    /// moved should cost you your colours, not your terminal.
    var current: Theme {
        available.first { $0.name == selectedName } ?? .fallback
    }

    /// Re-read the list from the machine the app drives.
    ///
    /// Called at launch and on every reconnection, for the same reason
    /// repositories and roots are: a machine that dropped and came back may
    /// have gained a theme, and staying invisible until relaunch is the
    /// failure `FleetStore.seed` exists to prevent.
    func reload(binary: String?, environment: [String: String], host: [String]) async {
        guard let binary else { return }
        guard let data = await Themes.run(binary: binary, environment: environment, host: host)
        else { return }
        struct Reply: Decodable {
            var themes: [Theme]
        }
        guard let reply = try? JSONDecoder().decode(Reply.self, from: data), !reply.themes.isEmpty
        else { return }
        available = reply.themes
        revision += 1
    }

    private static func run(binary: String, environment: [String: String], host: [String]) async
        -> Data?
    {
        await withCheckedContinuation { continuation in
            let process = Process()
            process.executableURL = URL(fileURLWithPath: binary)
            process.arguments = host + ["--json", "theme", "list"]
            process.environment = environment
            let pipe = Pipe()
            process.standardOutput = pipe
            process.standardError = FileHandle.nullDevice
            do {
                try process.run()
            } catch {
                continuation.resume(returning: nil)
                return
            }
            let data = try? pipe.fileHandleForReading.readToEnd()
            process.waitUntilExit()
            continuation.resume(returning: process.terminationStatus == 0 ? data : nil)
        }
    }
}
