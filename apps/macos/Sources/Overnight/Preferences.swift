import AppKit
import SwiftUI

/// App preferences.
///
/// Deliberately small. Every setting is a decision the product failed to make
/// for you, so each one has to earn its place — but a terminal's font is not
/// that: monospaced type is something people have real, long-held preferences
/// about, and one that renders badly on your display makes the whole app
/// unpleasant regardless of what it does.
@MainActor
final class Preferences: ObservableObject {
    static let shared = Preferences()

    /// Bumped whenever anything a terminal renders from changes, so views can
    /// react to one thing instead of observing every property.
    @Published private(set) var revision = 0

    @AppStorage("terminal.fontName") var fontName: String = Preferences.defaultFontName {
        didSet { revision += 1 }
    }
    @AppStorage("terminal.fontSize") var fontSize: Double = 12.5 {
        didSet { revision += 1 }
    }

    /// Remove a terminal's record when its command exits cleanly.
    ///
    /// On by default: you typed `exit`, and a dead row you have to dismiss is
    /// clutter. A NON-zero exit is never removed automatically, whatever this
    /// is set to — an agent that crashed is the one thing in the list you
    /// needed to see.
    @AppStorage("terminals.autoRemoveExited") var autoRemoveExited = true

    /// Notify when an agent needs you, or finishes.
    @AppStorage("notifications.enabled") var notifyOnAttention = true
    /// Also notify when an agent finishes, not only when it is blocked.
    @AppStorage("notifications.onDone") var notifyOnDone = true

    static let defaultFontName = "SF Mono"

    /// The monospaced fonts on this machine.
    ///
    /// Filtered to fixed-pitch, because a proportional font in a terminal does
    /// not look wrong so much as become unreadable — every column misaligns.
    static var monospacedFamilies: [String] {
        let manager = NSFontManager.shared
        var names = manager.availableFontFamilies.filter { family in
            guard let members = manager.availableMembers(ofFontFamily: family) else { return false }
            return members.contains { member in
                guard let traits = member[3] as? NSNumber else { return false }
                return NSFontTraitMask(rawValue: traits.uintValue).contains(.fixedPitchFontMask)
            }
        }
        // SF Mono is not reported by availableFontFamilies on every system even
        // though NSFont can make one, so it is added rather than discovered.
        if !names.contains(defaultFontName) {
            names.insert(defaultFontName, at: 0)
        }
        return names.sorted()
    }

    /// The terminal font, falling back rather than failing.
    ///
    /// A font can be uninstalled between launches. Falling back to the system
    /// monospaced face keeps the terminal readable instead of leaving it blank
    /// while someone works out what happened.
    func terminalFont(weight: NSFont.Weight = .regular) -> NSFont {
        let size = CGFloat(fontSize)
        if fontName != Preferences.defaultFontName,
            let font = NSFont(name: fontName, size: size)
        {
            guard weight == .regular else {
                return NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask)
            }
            return font
        }
        return NSFont.monospacedSystemFont(ofSize: size, weight: weight)
    }
}

struct SettingsView: View {
    @ObservedObject private var preferences = Preferences.shared

    var body: some View {
        TabView {
            terminal.tabItem { Label("Terminal", systemImage: "terminal") }
            behaviour.tabItem { Label("Behaviour", systemImage: "gearshape") }
        }
        .frame(width: 460, height: 280)
    }

    private var terminal: some View {
        Form {
            Picker("Font", selection: $preferences.fontName) {
                ForEach(Preferences.monospacedFamilies, id: \.self) { Text($0).tag($0) }
            }

            HStack {
                Slider(value: $preferences.fontSize, in: 9...24, step: 0.5) {
                    Text("Size")
                }
                Text(String(format: "%.1f", preferences.fontSize))
                    .font(.callout.monospacedDigit())
                    .frame(width: 40, alignment: .trailing)
            }

            // A preview, because a font name tells you nothing and this is the
            // whole reason the setting exists.
            Text("overnight ~/project % claude --resume  1234567890")
                .font(Font(preferences.terminalFont() as CTFont))
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color(nsColor: .textBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .formStyle(.grouped)
        .padding()
    }

    private var behaviour: some View {
        Form {
            Section {
                Toggle("Remove terminals that exit cleanly", isOn: $preferences.autoRemoveExited)
                Text(
                    "A terminal you closed leaves nothing behind. One that exited with a "
                    + "failure is always kept, so a crashed agent is never hidden."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Notify when an agent needs you", isOn: $preferences.notifyOnAttention)
                Toggle("Notify when an agent finishes", isOn: $preferences.notifyOnDone)
                    .disabled(!preferences.notifyOnAttention)
                Text("Working agents are never notified about. That is the normal case.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}
