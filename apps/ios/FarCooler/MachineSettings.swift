import SwiftUI

/// One machine's `config.toml`, from a phone.
///
/// The whole reason this exists on a phone at all: the machine whose settings
/// these are is frequently a Linux box with no Far Cooler app on it, reached over
/// ssh. Before this, changing its branch prefix meant an ssh session and a text
/// editor — from a phone, which is not a thing anybody does.
///
/// Talks over the same JSON bridge every other screen here uses, so the shapes
/// are the ones `Connection` already decodes.
@MainActor
final class MachineSettingsModel: ObservableObject {
    @Published private(set) var branchPrefix = ""
    @Published private(set) var themes: [Theme] = []
    @Published private(set) var adapters: [AdapterInfo] = []
    @Published var failure: String?
    @Published private(set) var loading = false

    private let connection: Connection

    init(connection: Connection) {
        self.connection = connection
    }

    func load() async {
        loading = true
        defer { loading = false }
        branchPrefix = connection.branchPrefix
        themes = await connection.hostThemes()
        adapters = await connection.adapters()
    }

    func setBranchPrefix(_ prefix: String) async {
        guard let stored = await connection.setBranchPrefix(prefix) else {
            failure = "That machine did not accept the change."
            return
        }
        branchPrefix = stored
    }

    func save(theme: Theme) async {
        themes = await connection.upsertTheme(theme) ?? themes
        // Every client's picker reads the merged list, so a saved theme has to
        // reach it — otherwise the thing you just made is missing from the one
        // place you would go to choose it.
        await connection.reloadThemes()
    }

    func delete(themeNamed name: String) async {
        themes = await connection.deleteTheme(name) ?? themes
        await connection.reloadThemes()
    }

    func save(adapter: AdapterInfo) async {
        adapters = await connection.upsertAdapter(adapter) ?? adapters
    }

    func delete(adapterNamed preset: String) async {
        adapters = await connection.deleteAdapter(preset) ?? adapters
    }

    func test(adapter: AdapterInfo) async -> AdapterTestOutcome {
        await connection.testAdapter(adapter)
    }
}

/// What a handshake said.
enum AdapterTestOutcome: Equatable {
    case worked(String)
    case failed(String)
}

/// One adapter, plus where it came from. Mirrors the Mac's own `AdapterInfo`.
struct AdapterInfo: Identifiable, Equatable {
    enum Origin: String {
        case builtIn, override, user, unknown
    }

    var preset: String
    var program: String = ""
    var args: [String] = []
    var env: [String: String] = [:]
    var commands: [String] = []
    var identity: [String] = []
    var blocked: [String] = []
    var working: [String] = []
    var origin: Origin = .user

    var id: String { preset }

    /// An adapter with no program is a recognized agent that stays a terminal —
    /// a real supported state, not a gap.
    var chatCapable: Bool { !program.trimmingCharacters(in: .whitespaces).isEmpty }

    init?(json: [String: Any]) {
        guard let preset = json["preset"] as? String else { return nil }
        self.preset = preset
        program = json["program"] as? String ?? ""
        args = json["args"] as? [String] ?? []
        env = json["env"] as? [String: String] ?? [:]
        commands = json["commands"] as? [String] ?? []
        identity = json["identity"] as? [String] ?? []
        blocked = json["blocked"] as? [String] ?? []
        working = json["working"] as? [String] ?? []
        origin = Origin(rawValue: json["origin"] as? String ?? "") ?? .unknown
    }

    init(preset: String) {
        self.preset = preset
    }

    var arguments: [String: Any] {
        [
            "preset": preset, "program": program, "args": args, "env": env,
            "commands": commands, "identity": identity, "blocked": blocked, "working": working,
        ]
    }
}

/// The machine's own settings, as a screen.
struct MachineSettingsView: View {
    let name: String
    @StateObject private var model: MachineSettingsModel
    @State private var prefixDraft = ""
    @State private var editingTheme: Theme?
    @State private var editingAdapter: AdapterInfo?

    init(name: String, connection: Connection) {
        self.name = name
        _model = StateObject(wrappedValue: MachineSettingsModel(connection: connection))
    }

    var body: some View {
        Form {
            Section {
                HStack {
                    TextField("Branch prefix", text: $prefixDraft, prompt: Text("feat/"))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit { commitPrefix() }
                    if prefixDraft != model.branchPrefix {
                        Button("Save") { commitPrefix() }
                    }
                }
            } header: {
                Text("Branches")
            } footer: {
                Text(
                    "Goes in front of a branch name made from a task description. "
                    + "Leave it empty for no prefix.")
            }

            Section {
                ForEach(model.themes) { theme in
                    Button {
                        editingTheme = theme
                    } label: {
                        HStack {
                            ThemeStrip(theme: theme)
                            Text(theme.name).foregroundStyle(.primary)
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
                .onDelete { offsets in
                    let names = offsets.map { model.themes[$0].name }
                    Task { for name in names { await model.delete(themeNamed: name) } }
                }
                Button {
                    var copy = Themes.shared.current
                    copy.name = unusedThemeName(like: copy.name)
                    editingTheme = copy
                } label: {
                    Label("Duplicate the Current Theme", systemImage: "plus")
                }
            } header: {
                Text("Themes on this machine")
            } footer: {
                Text(
                    "Only themes this machine defines are listed. Shipped themes have nothing "
                    + "to delete; saving one under a shipped name overrides it here.")
            }

            Section {
                ForEach(model.adapters) { adapter in
                    Button {
                        editingAdapter = adapter
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(adapter.preset).foregroundStyle(.primary)
                                Text(subtitle(adapter))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption)
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
                Button {
                    editingAdapter = AdapterInfo(preset: unusedAdapterName())
                } label: {
                    Label("Add an Agent", systemImage: "plus")
                }
            } header: {
                Text("Agents")
            } footer: {
                Text(
                    "An adapter lets Far Cooler show an agent as a chat instead of its terminal.")
            }

            if let failure = model.failure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.callout)
                }
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if model.loading { ProgressView() }
        }
        .task {
            await model.load()
            prefixDraft = model.branchPrefix
        }
        .sheet(item: $editingTheme) { theme in
            NavigationStack {
                ThemeEditorView(theme: theme) { edited in
                    Task { await model.save(theme: edited) }
                }
            }
        }
        .sheet(item: $editingAdapter) { adapter in
            NavigationStack {
                AdapterEditorView(
                    adapter: adapter,
                    isNew: !model.adapters.contains { $0.preset == adapter.preset },
                    test: { await model.test(adapter: $0) }
                ) { edited in
                    Task { await model.save(adapter: edited) }
                }
            }
        }
    }

    private func commitPrefix() {
        guard prefixDraft != model.branchPrefix else { return }
        Task { await model.setBranchPrefix(prefixDraft) }
    }

    private func subtitle(_ adapter: AdapterInfo) -> String {
        let origin: String
        switch adapter.origin {
        case .builtIn: origin = "Shipped"
        case .override: origin = "Overriding a shipped agent"
        case .user: origin = "Yours"
        case .unknown: origin = "Unknown"
        }
        guard adapter.chatCapable else { return "\(origin) — terminal only" }
        return "\(origin) — \(adapter.program) \(adapter.args.joined(separator: " "))"
    }

    private func unusedThemeName(like base: String) -> String {
        let taken = Set(model.themes.map(\.name))
        let first = "\(base) Copy"
        if !taken.contains(first) { return first }
        for n in 2... where !taken.contains("\(first) \(n)") { return "\(first) \(n)" }
        return first
    }

    private func unusedAdapterName() -> String {
        let taken = Set(model.adapters.map(\.preset))
        if !taken.contains("my-agent") { return "my-agent" }
        for n in 2... where !taken.contains("my-agent-\(n)") { return "my-agent-\(n)" }
        return "my-agent"
    }
}

/// A theme's colours as one strip, for a list row.
struct ThemeStrip: View {
    let theme: Theme

    var body: some View {
        HStack(spacing: 1) {
            ForEach(Array(preview.enumerated()), id: \.offset) { _, packed in
                Rectangle().fill(Color(packed: packed)).frame(width: 5, height: 18)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(RoundedRectangle(cornerRadius: 3).strokeBorder(Color.primary.opacity(0.12)))
    }

    private var preview: [UInt32] {
        [theme.background, theme.foreground] + theme.ansi.prefix(8)
    }
}

extension Color {
    // `Color(packed:)` already exists, in TerminalSession.swift, where the
    // renderer needed it first. Only the reverse direction is new — a colour
    // well hands back a `Color` and the config file wants an integer.

    /// This colour as `0x00RRGGBB`.
    var packed: UInt32 {
        let resolved = UIColor(self)
        var r: CGFloat = 0
        var g: CGFloat = 0
        var b: CGFloat = 0
        var a: CGFloat = 0
        resolved.getRed(&r, green: &g, blue: &b, alpha: &a)
        return (UInt32((r * 255).rounded()) << 16)
            | (UInt32((g * 255).rounded()) << 8)
            | UInt32((b * 255).rounded())
    }
}

extension Theme {
    /// A theme out of the JSON bridge.
    ///
    /// Separate from the synthesized `Decodable` because the bridge hands back
    /// `[String: Any]` rather than `Data`, and going through a second
    /// serialization round trip to reuse `Decodable` would be work with nothing
    /// to show for it.
    init?(bridge json: [String: Any]) {
        guard let name = json["name"] as? String,
            let ansi = json["ansi"] as? [NSNumber], ansi.count == 16
        else { return nil }
        self.init(
            name: name,
            dark: json["dark"] as? Bool ?? true,
            background: (json["background"] as? NSNumber)?.uint32Value ?? 0,
            foreground: (json["foreground"] as? NSNumber)?.uint32Value ?? 0,
            cursor: (json["cursor"] as? NSNumber)?.uint32Value ?? 0,
            ansi: ansi.map(\.uint32Value))
    }

    /// This theme as the bridge's `theme.upsert` arguments.
    var arguments: [String: Any] {
        [
            "name": name, "dark": dark, "background": background,
            "foreground": foreground, "cursor": cursor, "ansi": ansi,
        ]
    }
}
