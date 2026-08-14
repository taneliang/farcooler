import AgentKit
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

    /// Bumped when the sidebar's own shape changes.
    ///
    /// Separate from `revision`, which is documented as what a TERMINAL renders
    /// from: collapsing a project in the sidebar is not one of those, and
    /// sharing the counter would repaint every terminal on screen to hide a
    /// couple of rows in a list.
    @Published private(set) var sidebarRevision = 0

    /// Which projects are collapsed in the sidebar, keyed by `groupKey`.
    ///
    /// Persisted, unlike the sidebar's `hiddenExpanded`, and deliberately:
    /// collapsing a repository is a statement about how you want the sidebar to
    /// look, and one that reset on every launch would have to be re-made on
    /// every launch.
    ///
    /// Stored as one newline-joined string because `@AppStorage` cannot hold a
    /// `Set`. Newline rather than anything else because `groupKey` already uses
    /// `\u{1}` as its own host/project separator, and neither a machine name nor
    /// a project display name can contain a newline.
    @AppStorage("sidebar.collapsedProjects") private var collapsedProjects = ""

    func isProjectCollapsed(_ key: String) -> Bool {
        !key.isEmpty && collapsedProjects.split(separator: "\n").contains(Substring(key))
    }

    func toggleProject(_ key: String) {
        guard !key.isEmpty else { return }
        var keys = collapsedProjects.split(separator: "\n").map(String.init)
        if let at = keys.firstIndex(of: key) {
            keys.remove(at: at)
        } else {
            keys.append(key)
        }
        collapsedProjects = keys.joined(separator: "\n")
        sidebarRevision += 1
    }

    /// Remove a terminal's record once its process is gone.
    ///
    /// On by default, because a terminal is its process: when that exits there
    /// is nothing left to show and a dead row you have to dismiss is pure
    /// clutter. A `lost` terminal is never removed either way — that is the one
    /// state where Far Cooler does not know what happened.
    @AppStorage("terminals.autoRemoveExited") var autoRemoveExited = true

    /// Open a detected coding agent as a chat rather than as its terminal.
    ///
    /// Off by default, and that is the product decision rather than caution:
    /// Far Cooler is terminal-first, and chat is an upgrade a user opts into
    /// once they have seen it. Someone who prefers it should not have to ask
    /// for it a second time in every new pane.
    @AppStorage("agents.preferChatMode") var preferChatMode = false

    /// System, light, or dark.
    ///
    /// Defaults to dark rather than to the system.
    ///
    /// It followed the system for a long time, on the reasoning that this is
    /// what the rest of the machine does. What that missed is that most of
    /// this window is a terminal, and a terminal is dark at noon: following
    /// the system produced a light sidebar against a dark grid for half of
    /// every day. Asked for directly, and it is what the phones already did.
    ///
    /// `.theme` is the fourth option and the one that makes theming one
    /// feature rather than two: it takes whichever way the chosen theme says
    /// its chrome should go, so picking Solarized Light lightens the app
    /// around it. The explicit light and dark stay, because someone who has
    /// said "always dark" has said something this must not overrule.
    @AppStorage("app.appearance") var appearance = Appearance.dark {
        didSet { Appearance.apply(appearance) }
    }

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

    /// Which Settings tab to open on.
    ///
    /// Stored rather than passed because Settings is a scene, not a sheet —
    /// nothing that opens it can hand it a parameter. Anything that wants to
    /// send someone to a specific tab sets this first.
    @AppStorage("settings.tab") var settingsTab = "terminal"

    /// The editor the "Open in…" control uses on a click, by `Editor.id`.
    ///
    /// Empty means "whichever is first", which is what a fresh install has and
    /// what someone who has only one editor never has to think about.
    ///
    /// Written only when the user picks an editor from the menu. The fallback
    /// for a remote worktree an editor cannot reach — see `Editors.preferred` —
    /// deliberately does not write here, so working on a box for an afternoon
    /// does not quietly change what this Mac opens.
    @AppStorage("editors.lastUsed") var lastUsedEditor = ""

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

/// A control and the line explaining it, in ONE cell.
///
/// A `Text` written as a sibling of a control inside a `Form` is a row of its
/// own: separator above, full cell height, indistinguishable at a glance from a
/// setting. Six of them made the Behavior pane read as twelve settings, half
/// of them unclickable, and the last one sat under the control it did not
/// describe — a paragraph about lost terminals hanging beneath the tiling
/// prefix.
///
/// Text that explains one control belongs in that control's cell. Text that
/// covers a whole group belongs in the section's `footer`, which is what the
/// notification section uses.
private struct Setting<Control: View>: View {
    private let caption: LocalizedStringKey
    private let control: Control

    init(_ caption: LocalizedStringKey, @ViewBuilder control: () -> Control) {
        self.caption = caption
        self.control = control()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            control
            Text(caption)
                .font(.caption)
                .foregroundStyle(.secondary)
                // Wraps instead of truncating: a form column is narrower than
                // most of these sentences.
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct SettingsView: View {
    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    @StateObject private var service = ServiceRegistration()
    @StateObject private var cliTools = CommandLineTools()

    var body: some View {
        TabView(selection: $preferences.settingsTab) {
            terminal.tabItem { Label("Terminal", systemImage: "terminal") }.tag("terminal")
            behavior.tabItem { Label("Behavior", systemImage: "gearshape") }.tag("behavior")
            HostsSettings().tabItem { Label("Machines", systemImage: "server.rack") }
                .tag("machines")
            EditorsSettings()
                .tabItem { Label("Editors", systemImage: "chevron.left.forwardslash.chevron.right") }
                .tag("editors")
            account.tabItem { Label("Account", systemImage: "person.crop.circle") }.tag("account")
            host.tabItem { Label("Startup", systemImage: "bolt") }.tag("startup")
        }
        // Tall enough for the longest tab. Behavior is five settings and a
        // notification group, and at 400 it clipped the last group mid-row —
        // a settings window that scrolls to reach a checkbox reads as broken.
        .frame(width: 520, height: 520)
    }

    /// Signing in, which buys notifications and nothing else.
    ///
    /// Its own tab rather than a row under Machines: an account is about this
    /// person, and a machine list is about machines. Pairing — which machine may
    /// notify you — stays in Machines, where the machines are.
    private var account: some View {
        // The sign-in row on top of the two lists it makes meaningful. One
        // scroll rather than a tab and a sheet: signing in, being notified, and
        // seeing what can notify you are one subject.
        ScrollView {
            VStack(spacing: 0) {
                Form {
                    AccountSection()
                    // Under the account rather than in its own tab: which relay
                    // this build talks to is the answer to "why is nothing
                    // notifying me", and that question starts here.
                    RelaySection()
                }
                .formStyle(.grouped)
                AccountDevicesView()
            }
        }
        .padding(.vertical, 4)
    }

    /// Whether this machine stays reachable when nobody is at it.
    ///
    /// A preference you set once, not a status you watch. It used to sit in the
    /// sidebar's status bar, where a piece of configuration read as live
    /// information about the fleet.
    private var host: some View {
        Form {
            Setting("Keeps this Mac reachable from your iPhone while Far Cooler is closed.") {
                switch service.state {
                case .registered:
                    Toggle("Start the daemon at login", isOn: .constant(true))
                        .onTapGesture { service.unregister() }
                case .notRegistered:
                    Toggle("Start the daemon at login", isOn: .constant(false))
                        .onTapGesture { service.register() }
                case .awaitingApproval:
                    Button("Approve in System Settings") { service.register() }
                case .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }

            // Named from the same place the symlinks are made, because on
            // anything but a release build they are not called `farcooler` and
            // `farcoolerd` — and a line promising those two names would be
            // telling someone to type a command that will not be there.
            Setting(
                "Puts \(CommandLineTools.tools.map(\.link).joined(separator: " and ")) on your PATH, so a terminal or an SSH session can find them."
            ) {
                switch cliTools.state {
                case .installed:
                    Toggle("Command-line tools", isOn: .constant(true))
                        .onTapGesture { cliTools.uninstall() }
                case .notInstalled:
                    Toggle("Command-line tools", isOn: .constant(false))
                        .onTapGesture { cliTools.install() }
                case .conflict(let why), .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .onAppear {
            service.refresh()
            cliTools.refresh()
        }
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
            Text("farcooler ~/project % claude --resume  1234567890")
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

    private var behavior: some View {
        Form {
            Section {
                Setting("⌘T opens a plain shell.") {
                    Picker("New tasks start with", selection: $preferences.defaultAgent) {
                        Text("Claude Code").tag("claude")
                        Text("Codex").tag("codex")
                        Text("Cursor").tag("cursor")
                    }
                }
            }

            Section {
                Setting("Sets the terminal's colors and, on \"Theme\", the app's too.") {
                    Picker("Theme", selection: themes.selectedNameBinding) {
                        ForEach(themes.available) { theme in
                            Text(theme.name).tag(theme.name)
                        }
                    }
                }

                Setting("Add your own under [themes.name] in ~/.config/farcooler/config.toml.") {
                    Picker("Appearance", selection: $preferences.appearance) {
                        ForEach(Appearance.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }

                Setting("Lost terminals are kept until you dismiss them.") {
                    Toggle("Remove terminals when they exit", isOn: $preferences.autoRemoveExited)
                }

                Setting("Available for agents the registry recognizes. Switch any pane with ⌃B a.") {
                    Toggle("Open coding agents as a chat", isOn: $preferences.preferChatMode)
                }

                Setting("Active only while more than one pane is on screen.") {
                    Toggle(
                        "Move between panes with ⌃H ⌃J ⌃K ⌃L",
                        isOn: $preferences.directTraversal)
                }

                Picker("Tiling prefix", selection: $preferences.prefixKey) {
                    // ⌃B is tmux's, which is why it is the default. The
                    // alternatives are the two keys people who have already
                    // rebound tmux tend to have rebound it to, and both are
                    // there because ⌃B is `back-char` in readline.
                    Text("⌃B").tag("b")
                    Text("⌃A").tag("a")
                    Text("⌃Space").tag(" ")
                }
            }

            // A section footer, not a row: this one line covers both toggles,
            // and a `Text` written beside them would become a third setting.
            Section {
                Toggle("Notify when an agent needs you", isOn: $preferences.notifyOnAttention)
                Toggle("Notify when an agent finishes", isOn: $preferences.notifyOnDone)
                    .disabled(!preferences.notifyOnAttention)
            } footer: {
                Text("You are not notified while an agent is working.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}


/// The app's appearance, independent of the system's.
enum Appearance: String, CaseIterable, Identifiable {
    case theme, system, light, dark

    var id: String { rawValue }

    var label: String {
        switch self {
        case .theme: return "Theme"
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    @MainActor
    private var named: NSAppearance? {
        switch self {
        // nil hands the decision back to the system, which is what "System"
        // means — not a snapshot of what the system currently is.
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        // Whichever way the theme says. Read at apply time rather than stored,
        // so switching to a light theme lightens the chrome without also
        // having to change this setting.
        case .theme:
            return NSAppearance(named: Themes.shared.current.dark ? .darkAqua : .aqua)
        }
    }

    /// Applied application-wide, so settings and sheets follow too.
    @MainActor
    static func apply(_ appearance: Appearance) {
        NSApp?.appearance = appearance.named
    }
}
