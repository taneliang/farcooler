import SwiftUI

/// One runner's `config.toml`, from a phone.
///
/// The whole reason this exists on a phone at all: the runner whose settings
/// these are is frequently on a Linux box with no Far Cooler app on it, reached
/// over ssh. Before this, changing its branch prefix meant an ssh session and a
/// text editor — from a phone, which is not a thing anybody does.
///
/// Talks over the same JSON bridge every other screen here uses, so the shapes
/// are the ones `Connection` already decodes.
@MainActor
final class RunnerSettingsModel: ObservableObject {
    @Published private(set) var branchPrefix = ""
    @Published private(set) var themes: [Theme] = []
    @Published private(set) var adapters: [AdapterInfo] = []
    /// What the runner says about itself, once it has been asked. Optional
    /// rather than a blank default, so a daemon too old to answer shows nothing
    /// instead of showing zeroes that look like real readings.
    @Published private(set) var health: HostHealth?
    @Published private(set) var repositories: [Repository] = []
    @Published private(set) var roots: [RepositoryRoot] = []
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
        health = await connection.health()
        repositories = connection.repositories
        roots = await connection.repositoryRoots()
    }

    func removeRoot(_ root: RepositoryRoot) async {
        guard await connection.removeRepositoryRoot(root.id) else {
            failure = "That runner wouldn’t stop watching that folder."
            return
        }
        roots.removeAll { $0.id == root.id }
    }

    func setBranchPrefix(_ prefix: String) async {
        guard let stored = await connection.setBranchPrefix(prefix) else {
            failure = "That runner didn’t accept the change."
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

/// One adapter, plus where it came from. Mirrors the Mac's own `AdapterInfo`.
struct AdapterInfo: Identifiable, Equatable {
    enum Origin: String {
        case builtIn, override, user, unknown
    }

    /// Which protocol Far Cooler speaks to this agent.
    ///
    /// `native` means the agent's own — `codex app-server`, or the Claude
    /// CLI's stream-json control protocol — rather than an ACP adapter
    /// wrapping it.
    ///
    /// Carried without a control to change it, which is deliberate and not an
    /// oversight. The Protocol picker is the Mac's, and this phone editor is
    /// the shorter form. But a value that is not carried is a value that is
    /// lost: with no `backend` here, Test sent none and the runner read the
    /// absent field as ACP — so pressing Test on an adapter saved as native
    /// reported a working adapter for a protocol nothing had spoken to it, and
    /// saving a detection string from this screen rewrote the table to ACP.
    /// Round-tripping it costs a field and fixes both.
    enum Backend: String {
        case acp
        case native
    }

    var preset: String
    var backend: Backend = .acp
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
        // Anything unrecognized, and an object from a runner too old to send
        // the key, is `acp` — the behavior every adapter had before the field
        // existed, and the safe direction to be wrong in.
        backend = Backend(rawValue: json["backend"] as? String ?? "") ?? .acp
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
            "preset": preset, "backend": backend.rawValue,
            "program": program, "args": args, "env": env,
            "commands": commands, "identity": identity, "blocked": blocked, "working": working,
        ]
    }
}

/// The runner's own settings, as a screen.
struct RunnerSettingsView: View {
    let name: String
    @StateObject private var model: RunnerSettingsModel
    @State private var prefixDraft = ""
    @State private var editingTheme: Theme?
    @State private var editingAdapter: AdapterInfo?

    init(name: String, connection: Connection) {
        self.name = name
        _model = StateObject(wrappedValue: RunnerSettingsModel(connection: connection))
    }

    var body: some View {
        Form {
            healthSection
            repositoriesSection

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
                    "Added to branch names created from task descriptions. Leave it empty for "
                    + "no prefix.")
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
                Text("Themes on this runner")
            } footer: {
                Text(
                    "This list includes themes saved on this runner. Saving a theme with a "
                    + "built-in name overrides it.")
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
                Text("Adapters let Far Cooler show supported agents in chat.")
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

    /// What this runner is and whether it is well.
    ///
    /// First, because it is the question you open this screen with when
    /// something is wrong — and because it is what says whether the rest of the
    /// screen can be trusted. Absent entirely on a daemon too old to answer,
    /// rather than shown as zeroes.
    @ViewBuilder
    private var healthSection: some View {
        if let health = model.health {
            Section {
                LabeledContent("Status") {
                    Label(
                        health.healthy ? "Healthy" : "Degraded",
                        systemImage: health.healthy ? "checkmark.circle.fill"
                            : "exclamationmark.triangle.fill")
                        .foregroundStyle(health.healthy ? .green : .orange)
                        .labelStyle(.titleAndIcon)
                }
                // The daemon's own reasons, one per row. Summarizing them would
                // throw away the only part that says what to do about it.
                ForEach(health.reasons, id: \.self) { reason in
                    Text(reason)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                LabeledContent("Far Cooler", value: health.daemonVersion)
                LabeledContent("Platform", value: health.platform)
                LabeledContent("Live panes", value: "\(health.livePanes)")
            } header: {
                Text("This Runner")
            } footer: {
                Text("Protocol \(health.protocolVersion).")
            }
        }
    }

    /// Where work can start, and which folders the runner looks in.
    ///
    /// Two lists rather than one because they answer different questions, and
    /// only the second is something this screen can change: a repository is
    /// registered by starting work in it, a root is a standing permission.
    @ViewBuilder
    private var repositoriesSection: some View {
        if !model.repositories.isEmpty {
            Section("Repositories") {
                ForEach(model.repositories) { repository in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(repository.displayName)
                        if !repository.remote.isEmpty {
                            Text(repository.remote)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                }
            }
        }

        if !model.roots.isEmpty {
            Section {
                ForEach(model.roots) { root in
                    // "Hidden" rather than a blank row: a phone with read scope
                    // is told the root exists and deliberately not told where it
                    // is, and an empty row reads as a bug rather than a rule.
                    Text(root.displayPath ?? "Hidden")
                        .font(root.displayPath == nil ? .body.italic() : .body)
                        .foregroundStyle(root.displayPath == nil ? .secondary : .primary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
                .onDelete { offsets in
                    let targets = offsets.map { model.roots[$0] }
                    Task { for root in targets { await model.removeRoot(root) } }
                }
            } header: {
                Text("Watched Folders")
            } footer: {
                Text(
                    "Far Cooler looks for repositories in these. Removing one stops "
                    + "the search; nothing on the runner is deleted.")
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
