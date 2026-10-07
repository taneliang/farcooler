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
    @AppStorage("agents.preferChatMode") var preferChatMode = AgentOpening.preferChatDefault

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
    @AppStorage("settings.tab") var settingsTab = SettingsTab.general

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

    /// Notify when an agent needs you, or its turn ends.
    @AppStorage("notifications.enabled") var notifyOnAttention = true
    /// Also notify when an agent's turn ends — whether it finished or failed —
    /// not only when it is blocked. Both endings arrive as `done`, so this one
    /// toggle governs the pair.
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

/// The tabs Settings can open on, as stored values.
enum SettingsTab {
    static let general = "general"

    /// What `settings.tab` held before General existed: Behavior and Startup
    /// were tabs of their own, and both are sections of General now. Someone who
    /// last left Settings on either lands on General rather than on no tab.
    static func normalized(_ stored: String) -> String {
        switch stored {
        case "behavior", "startup": return general
        default: return stored
        }
    }

    /// A binding that reads the stored value through `normalized`.
    static func binding(_ stored: Binding<String>) -> Binding<String> {
        Binding(get: { normalized(stored.wrappedValue) }, set: { stored.wrappedValue = $0 })
    }
}

/// How big Settings and the sheets it opens are allowed to be.
///
/// A sheet is never larger than the window it opens from: it either forces the
/// window to grow or hangs off its edge. The two editors open over the runner
/// sheet, which is itself a sheet over Settings, so each step is smaller than
/// the one under it.
enum SettingsSheetSize {
    /// Settings itself. Tall enough for General, the longest pane.
    static let window = CGSize(width: 560, height: 560)
    /// One runner's themes and agents.
    static let runner = CGSize(width: 520, height: 500)
    /// The theme and agent editors, which open over `runner`.
    static let editor = CGSize(width: 500, height: 480)
}

struct SettingsView: View {
    @ObservedObject private var preferences = Preferences.shared
    @ObservedObject private var themes = Themes.shared
    @StateObject private var service = ServiceRegistration()
    @StateObject private var cliTools = CommandLineTools()
    @ObservedObject private var native = NativeAgents.shared

    var body: some View {
        TabView(selection: SettingsTab.binding($preferences.settingsTab)) {
            // General first, as on every Mac settings window. Startup's two
            // switches live in it rather than in a tab of their own: a tab with
            // two rows is a window that is mostly empty.
            general.tabItem { Label("General", systemImage: "gearshape") }.tag(SettingsTab.general)
            terminal.tabItem { Label("Terminal", systemImage: "terminal") }.tag("terminal")
            RunnersSettings().tabItem { Label("Runners", systemImage: "server.rack") }
                // The tag is a stored value, not a word anyone reads: it is what
                // `settings.tab` already holds on disk, and changing it would
                // drop people onto a different tab once, for nothing.
                .tag("machines")
            EditorsSettings()
                .tabItem { Label("Editors", systemImage: "chevron.left.forwardslash.chevron.right") }
                .tag("editors")
            // Devices next to Runners, because the two are one subject read from
            // opposite ends: a runner is a machine that runs things, a device is
            // a thing that may ask it to.
            DevicesSettings()
                .tabItem { Label("Devices", systemImage: "iphone.gen3") }
                .tag("devices")
            account.tabItem { Label("Account", systemImage: "person.crop.circle") }.tag("account")
        }
        // One size for every tab, and the ceiling for every sheet Settings
        // opens: a sheet larger than the window it comes from hangs off it
        // (`SettingsSheetSize`). Tall enough for General, which is the longest
        // pane; every other pane is a grouped form that scrolls.
        .frame(width: SettingsSheetSize.window.width, height: SettingsSheetSize.window.height)
    }

    /// Signing in, which buys notifications and nothing else.
    ///
    /// Its own tab rather than a row under Runners: an account is about this
    /// person, and a runner list is about runners. Pairing — which runner may
    /// notify you — stays in Runners, where the runners are.
    private var account: some View {
        // One Form for the sign-in row and the two lists it makes meaningful:
        // two grouped Forms in a ScrollView nested scroll views and doubled the
        // spacing at the seam between them.
        Form {
            AccountSection()
            // Under the account rather than in its own tab: which relay
            // this build talks to is the answer to "why is nothing
            // notifying me", and that question starts here.
            RelaySection()
            // And under that, the tunnel's own rendezvous. Same shape
            // of question, rarer day: this is the one for when the
            // service tunneled runners meet at stops answering.
            RendezvousSection()
            AccountDevicesSections()
        }
        .formStyle(.grouped)
    }

    /// Whether this Mac stays reachable when nobody is at it.
    ///
    /// A preference you set once, not a status you watch. It used to sit in the
    /// sidebar's status bar, where a piece of configuration read as live
    /// information about the fleet.
    private var startup: some View {
        Section("Startup") {
            Setting("Keeps this Mac available to your other devices while Far Cooler is closed.") {
                switch service.state {
                case .registered, .notRegistered:
                    // One switch for both, through a binding that actually
                    // moves. It used to be two `.constant` toggles with
                    // `.onTapGesture`, which a `Toggle` never delivers.
                    Toggle(
                        "Start the daemon at login",
                        isOn: SystemSwitch.binding(
                            isOn: service.state.isOn,
                            turnOn: { service.register() },
                            turnOff: { service.unregister() }))
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
                "Adds \(CommandLineTools.tools.map(\.link).joined(separator: " and ")) to your PATH for Terminal and SSH sessions."
            ) {
                switch cliTools.state {
                case .installed, .notInstalled:
                    // The same decorative switch as the one above, and fixed
                    // the same way.
                    Toggle(
                        "Command-line tools",
                        isOn: SystemSwitch.binding(
                            isOn: cliTools.state == .installed,
                            turnOn: { cliTools.install() },
                            turnOff: { cliTools.uninstall() }))
                case .conflict(let why), .unavailable(let why):
                    Text(why).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
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
                // The theme's own colors: a font previewed on plain white or
                // black says nothing about how it will look in the terminal.
                .foregroundStyle(Color(nsColor: themes.current.foregroundColor))
                .background(Color(nsColor: themes.current.backgroundColor))
                .clipShape(.control)
        }
        .formStyle(.grouped)
    }

    private var general: some View {
        Form {
            Section {
                Setting("⌘T opens a plain shell.") {
                    Picker("New worktrees start with", selection: $preferences.defaultAgent) {
                        Text("Claude Code").tag("claude")
                        Text("Codex").tag("codex")
                        Text("Cursor").tag("cursor")
                    }
                }
            }

            Section {
                Setting("Applies to the terminal and, when Appearance is Theme, the app.") {
                    Picker("Theme", selection: themes.selectedNameBinding) {
                        ForEach(themes.available) { theme in
                            Text(theme.name).tag(theme.name)
                        }
                    }
                }

                Setting("Choose Theme, System, Light, or Dark for the app.") {
                    Picker("Appearance", selection: $preferences.appearance) {
                        ForEach(Appearance.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }

                Setting("Lost terminals are kept until you dismiss them.") {
                    Toggle("Remove terminals when they exit", isOn: $preferences.autoRemoveExited)
                }

                Setting("Available for recognized coding agents. Press \(PrefixKey.current) A to switch a pane.") {
                    Toggle("Open coding agents as a chat", isOn: $preferences.preferChatMode)
                }

                // Off by default until it matches the terminal (ov-372).
                Setting(LocalizedStringKey(NativeAgents.settingNote)) {
                    Toggle(
                        "Conversation view for Claude panes",
                        isOn: Binding(get: { native.enabled }, set: { on in Task { await native.setEnabled(on) } }))
                        .disabled(native.changing)
                    if let trouble = native.settingTrouble {
                        Text(trouble).font(.callout).foregroundStyle(.secondary)
                    }
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

            // The task classes (ov-94): one switch each, in the order the
            // phones list them, all under the master switch below.
            Section {
                ForEach(TaskNoticeEvent.allCases, id: \.self) { event in
                    TaskNoticeToggle(event: event)
                        .disabled(!preferences.notifyOnAttention)
                }
            } header: {
                Text("Tasks")
            } footer: {
                Text("An agent working on a task notifies through its task.")
            }

            // A section footer, not a row: this one line covers both toggles,
            // and a `Text` written beside them would become a third setting.
            Section {
                Toggle("Notify when an agent needs you", isOn: $preferences.notifyOnAttention)
                    // The relay hears the master switch as "no task classes".
                    .onChange(of: preferences.notifyOnAttention) { _, _ in
                        Task { await PushRegistration.shared.sendIfPossible() }
                    }
                // "or fails", because a turn that failed IS a turn that ended:
                // the daemon reports both as `done` with failure carried beside
                // it, and one toggle has always governed the pair. The label
                // said only half of that, which made a silenced app look broken
                // the first time a failure did not arrive.
                Toggle("Notify when an agent finishes or fails", isOn: $preferences.notifyOnDone)
                    .disabled(!preferences.notifyOnAttention)
                    // Registration runs when a push token arrives, which is at
                    // launch — so without this the relay keeps whatever it was
                    // told last time and the toggle appears to do nothing until
                    // the app happens to re-register. That is exactly the bug
                    // this setting had, in a new place.
                    .onChange(of: preferences.notifyOnDone) { _, _ in
                        Task { await PushRegistration.shared.sendIfPossible() }
                    }
            } header: {
                Text("Agents Without a Task")
            } footer: {
                Text("Far Cooler doesn’t send notifications while an agent is working.")
            }

            startup
        }
        .formStyle(.grouped)
    }
}


/// One task class's switch, kept under its own key (`TaskNoticeEvent`), and
/// sent to the relay when it changes so pushes honor it too.
private struct TaskNoticeToggle: View {
    let event: TaskNoticeEvent
    @AppStorage private var on: Bool

    init(event: TaskNoticeEvent) {
        self.event = event
        _on = AppStorage(wrappedValue: event.onByDefault, event.defaultsKey)
    }

    var body: some View {
        Toggle(event.title, isOn: $on)
            .onChange(of: on) { _, _ in
                Task { await PushRegistration.shared.sendIfPossible() }
            }
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
