import SwiftUI

/// One runner's `config.toml`, as a screen instead of an ssh session.
///
/// A sheet rather than a disclosure inside the runner row it opens from. The
/// Settings window is 520 points wide, and the theme editor below is nineteen
/// color wells over a live terminal preview — inside a form row that is a
/// column of swatches two wide. A sheet can be the size the content needs.
struct RunnerSettingsSheet: View {
    /// The runner, as it reads in the list that opened this.
    let name: String
    @StateObject private var store: RunnerSettingsStore
    @Environment(\.dismiss) private var dismiss

    /// The theme being edited, or nil.
    ///
    /// One value rather than a flag plus a draft: a sheet that can be presented
    /// with nothing to edit is a sheet that eventually will be.
    @State private var editingTheme: Theme?
    @State private var editingAdapter: AdapterInfo?
    /// What is in the branch-prefix field, which is not what the runner says
    /// until it is committed — otherwise every keystroke would be a write.
    @State private var prefixDraft = ""

    init(name: String, target: String) {
        self.name = name
        _store = StateObject(wrappedValue: RunnerSettingsStore(target: target))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            Form {
                branchSection
                themesSection
                adaptersSection
            }
            .formStyle(.grouped)
            Divider()
            footer
        }
        .frame(width: 620, height: 560)
        .task {
            await store.load()
            prefixDraft = store.branchPrefix
        }
        .sheet(item: $editingTheme) { theme in
            ThemeEditor(theme: theme) { edited in
                Task { await store.save(theme: edited) }
            }
        }
        .sheet(item: $editingAdapter) { adapter in
            AdapterEditor(
                adapter: adapter, store: store,
                // New only when nothing on the runner has this name yet.
                isNew: !store.adapters.contains { $0.preset == adapter.preset }
            ) { edited in
                Task { await store.save(adapter: edited) }
            }
        }
    }

    private var header: some View {
        HStack {
            Image(systemName: name == "This Mac" ? "laptopcomputer" : "server.rack")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 1) {
                Text(name).font(.headline)
                Text("Settings on this runner, in its own config file.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if store.loading { ProgressView().controlSize(.small) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    private var footer: some View {
        HStack {
            // One banner for every failure here, because they are all the same
            // kind: the runner could not be reached, or it refused.
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(2)
            }
            Spacer()
            Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    // MARK: - Branch prefix

    private var branchSection: some View {
        Section {
            HStack {
                TextField("Branch prefix", text: $prefixDraft, prompt: Text("feat/"))
                    .autocorrectionDisabled()
                    // Committed on return or on losing focus, not per keystroke:
                    // this is a file write and an ssh round trip.
                    .onSubmit { commitPrefix() }
                if prefixDraft != store.branchPrefix {
                    Button("Save") { commitPrefix() }
                }
            }
        } header: {
            Text("Branches")
        } footer: {
            Text(
                "Goes in front of a branch name made from a task description, so "
                + "\u{201c}add authentication\u{201d} becomes \u{201c}\(effectivePrefix)add-authentication\u{201d}. "
                + "Leave it empty for no prefix.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var effectivePrefix: String {
        prefixDraft.isEmpty ? "" : prefixDraft
    }

    private func commitPrefix() {
        guard prefixDraft != store.branchPrefix else { return }
        Task { await store.setBranchPrefix(prefixDraft) }
    }

    // MARK: - Themes

    /// Built-ins and this runner's own, merged by name.
    ///
    /// The built-ins are compiled into every client, so this needs nothing from
    /// the runner to show them — and a runner theme sharing a built-in's name is
    /// an override, which is a fact a name comparison already knows.
    private var allThemes: [(theme: Theme, isRunnerDefined: Bool, shadowsBuiltIn: Bool)] {
        let runnerNames = Set(store.themes.map(\.name))
        var rows = store.themes.map {
            (theme: $0, isRunnerDefined: true, shadowsBuiltIn: store.shadowsBuiltIn.contains($0.name))
        }
        // `Themes.shared.available` is the MERGED list every picker in the app
        // reads, so subtracting the runner's own leaves exactly the shipped ones.
        rows += Themes.shared.available
            .filter { !runnerNames.contains($0.name) }
            .map { (theme: $0, isRunnerDefined: false, shadowsBuiltIn: false) }
        return rows.sorted {
            $0.theme.name.localizedStandardCompare($1.theme.name) == .orderedAscending
        }
    }

    private var themesSection: some View {
        Section {
            ForEach(allThemes, id: \.theme.name) { row in
                HStack(spacing: 10) {
                    ThemeSwatchRow(theme: row.theme)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(row.theme.name)
                        Text(themeOrigin(row))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Edit") { editingTheme = row.theme }
                        .buttonStyle(.link)
                    // Only what the file owns can be removed. A built-in has no
                    // table to delete, so offering it would be a button that
                    // does nothing.
                    if row.isRunnerDefined {
                        Button(row.shadowsBuiltIn ? "Revert to Default" : "Delete") {
                            Task { await store.delete(themeNamed: row.theme.name) }
                        }
                        .buttonStyle(.link)
                        .foregroundStyle(.red)
                    }
                }
            }
            Button {
                // Duplicating is how a new theme starts: nineteen colors from
                // nothing is not a thing anyone wants to type, and every good
                // theme is a tweak to one that already works.
                var copy = Themes.shared.current
                copy.name = Self.unusedName(like: copy.name, taken: Set(allThemes.map(\.theme.name)))
                editingTheme = copy
            } label: {
                Label("Duplicate the Current Theme", systemImage: "plus")
            }
            .buttonStyle(.link)
        } header: {
            Text("Themes")
        } footer: {
            Text("Editing a shipped theme writes an override on this runner. Reverting removes it.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func themeOrigin(_ row: (theme: Theme, isRunnerDefined: Bool, shadowsBuiltIn: Bool))
        -> String
    {
        if row.shadowsBuiltIn { return "Overriding a shipped theme" }
        return row.isRunnerDefined ? "Defined on this runner" : "Shipped with Far Cooler"
    }

    /// "Nord" → "Nord Copy", then "Nord Copy 2", and so on.
    static func unusedName(like base: String, taken: Set<String>) -> String {
        let first = "\(base) Copy"
        if !taken.contains(first) { return first }
        for n in 2... {
            let candidate = "\(first) \(n)"
            if !taken.contains(candidate) { return candidate }
        }
        return first
    }

    // MARK: - Adapters

    private var adaptersSection: some View {
        Section {
            ForEach(store.adapters) { adapter in
                HStack(spacing: 10) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(adapter.preset)
                        Text(adapterSubtitle(adapter))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer()
                    Button("Edit") { editingAdapter = adapter }
                        .buttonStyle(.link)
                    if adapter.origin == .override || adapter.origin == .user {
                        Button(adapter.origin == .override ? "Revert to Default" : "Delete") {
                            Task { await store.delete(adapterNamed: adapter.preset) }
                        }
                        .buttonStyle(.link)
                        .foregroundStyle(.red)
                    }
                }
            }
            Button {
                let taken = Set(store.adapters.map(\.preset))
                var name = "my-agent"
                var n = 2
                while taken.contains(name) {
                    name = "my-agent-\(n)"
                    n += 1
                }
                editingAdapter = AdapterInfo(preset: name)
            } label: {
                Label("Add an Agent", systemImage: "plus")
            }
            .buttonStyle(.link)
        } header: {
            Text("Agents")
        } footer: {
            Text(
                "An adapter lets Far Cooler show an agent as a chat instead of its terminal. "
                + "One with no launch command stays a terminal, which is a supported state.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func adapterSubtitle(_ adapter: AdapterInfo) -> String {
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
}

/// A theme's colors as one small strip, for a list row.
private struct ThemeSwatchRow: View {
    let theme: Theme

    var body: some View {
        // The background, the text, and the eight normal ANSI colors. Enough to
        // tell two themes apart at a glance, which is all a row has to do.
        HStack(spacing: 1) {
            ForEach(Array(preview.enumerated()), id: \.offset) { _, packed in
                Rectangle().fill(Color(nsColor: Theme.color(packed))).frame(width: 5, height: 18)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(RoundedRectangle(cornerRadius: 3).strokeBorder(Color.primary.opacity(0.12)))
    }

    private var preview: [UInt32] {
        [theme.background, theme.foreground] + theme.ansi.prefix(8)
    }
}
