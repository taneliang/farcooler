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

    /// Remove a terminal's record once its process is gone.
    ///
    /// On by default, because a terminal is its process: when that exits there
    /// is nothing left to show and a dead row you have to dismiss is pure
    /// clutter. A `lost` terminal is never removed either way — that is the one
    /// state where Overnight does not know what happened.
    @AppStorage("terminals.autoRemoveExited") var autoRemoveExited = true

    /// The tiling prefix, as a single lowercase letter used with Control.
    ///
    /// Configurable because `⌃B` is not free for everyone — it is `back-char` in
    /// readline and emacs, and someone who lives in either will want `⌃A` or
    /// `⌃Space` instead. `⌃B` is the default because it is tmux's, and tmux is
    /// where most people arriving here have already built the habit.
    @AppStorage("tiling.prefixKey") var prefixKey = "b"

    /// Move between panes with `⌃hjkl`, no prefix.
    ///
    /// A real trade: those four are backspace, newline, kill-line and
    /// clear-screen, and that is exactly why tmux hides its bindings behind a
    /// prefix. The cost is contained by only taking them while more than one pane
    /// is on screen — with a single terminal `⌃L` still clears it — but if you
    /// live in readline inside a tiled worktree, this is the switch.
    @AppStorage("tiling.directTraversal") var directTraversal = true

    /// Which agent a new task starts with.
    ///
    /// Only quick-create uses it. ⌘T still makes a plain shell, because that is
    /// the other thing you want a terminal for and guessing wrong there costs a
    /// process launch.
    @AppStorage("tasks.defaultAgent") var defaultAgent = "claude"

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
    @StateObject private var service = ServiceRegistration()

    var body: some View {
        TabView {
            terminal.tabItem { Label("Terminal", systemImage: "terminal") }
            behaviour.tabItem { Label("Behaviour", systemImage: "gearshape") }
            host.tabItem { Label("Host", systemImage: "bolt") }
        }
        .frame(width: 480, height: 320)
    }

    /// Whether this machine stays reachable when nobody is at it.
    ///
    /// A preference you set once, not a status you watch. It used to sit in the
    /// sidebar's status bar, where a piece of configuration read as live
    /// information about the fleet.
    private var host: some View {
        Form {
            Section {
                switch service.state {
                case .registered:
                    Toggle("Start Overnight's daemon at login", isOn: .constant(true))
                        .onTapGesture { service.unregister() }
                case .notRegistered:
                    Toggle("Start Overnight's daemon at login", isOn: .constant(false))
                        .onTapGesture { service.register() }
                case .awaitingApproval:
                    Button("Approve in System Settings") { service.register() }
                case .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }

                Text(
                    "With this on, agents stay reachable from your phone after you close "
                    + "the app. Terminals keep running either way — they belong to tmux."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding()
        .onAppear { service.refresh() }
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
                Picker("New tasks start with", selection: $preferences.defaultAgent) {
                    Text("Claude Code").tag("claude")
                    Text("Codex").tag("codex")
                    Text("Cursor").tag("cursor")
                }
                Text("⌘T still opens a plain shell.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Remove terminals when they exit", isOn: $preferences.autoRemoveExited)

                Toggle("Move between panes with ⌃H ⌃J ⌃K ⌃L", isOn: $preferences.directTraversal)
                Text(
                    "Only while more than one pane is on screen. With a single "
                    + "terminal these stay backspace, newline, kill-line and clear."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                Picker("Tiling prefix", selection: $preferences.prefixKey) {
                    // ⌃B is tmux's, which is why it is the default. The
                    // alternatives are the two keys people who have already
                    // rebound tmux tend to have rebound it to, and both are
                    // there because ⌃B is `back-char` in readline.
                    Text("⌃B").tag("b")
                    Text("⌃A").tag("a")
                    Text("⌃Space").tag(" ")
                }
                Text(
                    "A terminal is its process. When that exits there is nothing left to "
                    + "show. A lost terminal is always kept — that is the one case Overnight "
                    + "cannot explain."
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
