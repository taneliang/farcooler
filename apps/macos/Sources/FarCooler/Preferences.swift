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
    /// Defaults to system, because that is what the rest of the machine does. It
    /// is offered at all because a great many people who run four agents farcooler
    /// keep every window dark regardless of what the OS is doing at noon.
    ///
    /// It never touches the terminal. Terminal colours come from `Palette`, which
    /// is a fixed dark palette — the emulator's background is the program's
    /// business, not the app chrome's, and a terminal that went white under a
    /// light theme would be a different terminal.
    @AppStorage("app.appearance") var appearance = Appearance.system {
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

    /// The machine this window is driving, as `user@host` or an ssh config
    /// alias. Empty means this one.
    ///
    /// The app drives the `farcooler` CLI, and the CLI already knows how to
    /// operate another machine over ssh — including streaming a terminal,
    /// which it proxies as the byte pipe it is. So making this window a remote
    /// one is a matter of saying which machine, not of building a second
    /// client: everything below this setting is the same code path either way,
    /// which is the only version of remote support worth having. There is no
    /// Far Cooler listener anywhere; a host reachable by ssh is reachable by
    /// Far Cooler, and one that is not, is not.
    /// Which Settings tab to open on.
    ///
    /// Stored rather than passed because Settings is a scene, not a sheet —
    /// nothing that opens it can hand it a parameter. Anything that wants to
    /// send someone to a specific tab sets this first.
    @AppStorage("settings.tab") var settingsTab = "terminal"

    @AppStorage("host.remote") var remoteHost = "" {
        didSet { revision += 1 }
    }

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
/// setting. Six of them made the Behaviour pane read as twelve settings, half
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
    @StateObject private var service = ServiceRegistration()

    var body: some View {
        TabView(selection: $preferences.settingsTab) {
            terminal.tabItem { Label("Terminal", systemImage: "terminal") }.tag("terminal")
            behaviour.tabItem { Label("Behaviour", systemImage: "gearshape") }.tag("behaviour")
            HostsSettings().tabItem { Label("Machines", systemImage: "server.rack") }
                .tag("machines")
            EditorsSettings()
                .tabItem { Label("Editors", systemImage: "chevron.left.forwardslash.chevron.right") }
                .tag("editors")
            account.tabItem { Label("Account", systemImage: "person.crop.circle") }.tag("account")
            host.tabItem { Label("Startup", systemImage: "bolt") }.tag("startup")
        }
        // Tall enough for the longest tab. Behaviour is five settings and a
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
                Form { AccountSection() }
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

    private var behaviour: some View {
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
                Setting("Terminal colours are set separately.") {
                    Picker("Appearance", selection: $preferences.appearance) {
                        ForEach(Appearance.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }

                Setting("Lost terminals are kept until you dismiss them.") {
                    Toggle("Remove terminals when they exit", isOn: $preferences.autoRemoveExited)
                }

                Setting("Available for Claude. Switch any pane with ⌃B A.") {
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
    case system, light, dark

    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    private var named: NSAppearance? {
        switch self {
        // nil hands the decision back to the system, which is what "System"
        // means — not a snapshot of what the system currently is.
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        }
    }

    /// Applied application-wide, so settings and sheets follow too.
    static func apply(_ appearance: Appearance) {
        NSApp?.appearance = appearance.named
    }
}
