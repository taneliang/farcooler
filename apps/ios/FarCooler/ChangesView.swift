import SwiftUI

/// Reviewing what a worktree changed, on a phone.
///
/// The Mac shows this in a pane beside the agent that did the changing. A phone
/// has one pane, so this is what a `changes` pane draws — the same pane mode,
/// the same daemon-derived change set, laid out for a screen that can hold one
/// column of it.
///
/// Before this existed, a `changes` pane fell through `TerminalView`'s
/// `isAgentPane` check to the VT renderer and was drawn as a raw terminal: a
/// grid of whatever bytes happened to be on a pane that is not a tty. The pane
/// mode has been in the protocol and in the fleet all along; the phone simply
/// had no surface to put it on.
@MainActor
struct ChangesView: View {
    /// Owned by `Connection`, not by this view — the view is destroyed on every
    /// tab switch and the scroll, the folds and the fetched diffs must not be.
    @ObservedObject var store: ChangesStore
    let workspaceName: String


    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                summary

                if let error = store.error {
                    ChangesNotice(
                        symbol: "exclamationmark.triangle", tint: .orange, text: error)
                } else if store.files.isEmpty && !store.loading {
                    ChangesNotice(
                        symbol: "checkmark.circle",
                        tint: .secondary,
                        text: store.scope == .branch
                            ? "This branch hasn't committed anything yet."
                            : "Nothing uncommitted. The worktree is clean.")
                }

                ForEach(store.files) { file in
                    ChangesFileCard(file: file, store: store)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
        }
        // No scroll restore, and that is the point.
        //
        // There used to be one here: an offset kept in the store, put back on
        // appear, asked for repeatedly because a lazy stack clamps the first
        // request to the one screenful that exists. It half-worked — the offset
        // was recorded before the restore had had its turn, so a second or
        // third visit jumped — and all of it existed only because the view was
        // destroyed on every pane switch. `PaneHost` keeps the pane mounted, so
        // the scroll never moves and there is nothing to put back.
        .background(TerminalPalette.background)
        // Pull to refresh asks the daemon to recompute rather than answer from
        // its cache: it is the affordance that exists because no watcher is
        // perfect, and the user must always have a way to be certain.
        .refreshable { await store.load(fresh: true) }
        .task { await store.loadIfNeeded() }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Picker("Showing", selection: $store.scope) {
                        ForEach(DiffScope.allCases) { scope in
                            Text(scope.label).tag(scope)
                        }
                    }
                    Divider()
                    // What clears the badge on this worktree everywhere — the
                    // fleet row here, and the Mac's sidebar.
                    Button {
                        Task { await store.markRead() }
                    } label: {
                        Label("Mark as Reviewed", systemImage: "checkmark.circle")
                    }
                    Button {
                        Task { await store.load(fresh: true) }
                    } label: {
                        Label("Recompute", systemImage: "arrow.clockwise")
                    }
                } label: {
                    Image(systemName: "line.3.horizontal.decrease.circle")
                }
                .accessibilityLabel("Review options")
            }
        }
    }

    /// Branch, base, and the two numbers — the whole worktree in one card.
    private var summary: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(store.changeSet.branch.isEmpty ? workspaceName : store.changeSet.branch)
                    .font(.subheadline.weight(.semibold).monospaced())
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 4)
                if store.loading { ProgressView().controlSize(.small) }
            }

            HStack(spacing: 10) {
                if !store.changeSet.baseRef.isEmpty {
                    Text("vs \(store.changeSet.baseRef)")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Text("+\(store.changeSet.insertions)")
                    .font(.caption.monospaced()).foregroundStyle(.green)
                Text("−\(store.changeSet.deletions)")
                    .font(.caption.monospaced()).foregroundStyle(.red)
            }

            // Only a GUESSED base is called out. The others are recorded facts;
            // a guess is the one that can silently produce a wrong diff that
            // looks exactly like a right one.
            if store.changeSet.baseIsGuessed {
                Label(
                    "Base branch was guessed, so this diff may be wrong.",
                    systemImage: "questionmark.circle")
                    .font(.caption2)
                    .foregroundStyle(.orange)
            }

            // Which question the list below is answering. A segmented control
            // rather than only the menu item, because the two scopes are read
            // one after the other and a menu makes that two taps each way.
            Picker("Showing", selection: $store.scope) {
                ForEach(DiffScope.allCases) { scope in
                    Text(scope.label).tag(scope)
                }
            }
            .pickerStyle(.segmented)
            .padding(.top, 2)
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 14))
    }
}

/// What a card sits on.
///
/// A flat fill, deliberately, where the rest of the app's chrome uses
/// `GlassSurface`. Liquid Glass is a real-time sampling effect and wants a
/// `GlassEffectContainer` around a known, small set of shapes; applied per row
/// inside a `LazyVStack` it is re-created as each row is realized on scroll, and
/// the effect drops out — which is the background vanishing "as they come into
/// view". Dozens of independent glass surfaces would also be the expensive way
/// to draw a list even if it did work.
///
/// Derived from the theme's own ground rather than a system grey, so a card
/// reads as sitting ON the terminal palette instead of next to it.
@MainActor
enum ChangesSurface {
    static var card: Color {
        // Lifted off the background rather than tinted: the palette can be
        // light or dark, and only "slightly lighter than whatever is behind"
        // holds in both.
        Themes.shared.current.colorScheme == .dark
            ? Color.white.opacity(0.06)
            : Color.black.opacity(0.04)
    }
}

/// One file, folded to its heading until it is read.
private struct ChangesFileCard: View {
    let file: ChangedFile
    @ObservedObject var store: ChangesStore

    private var collapsed: Bool { store.collapsedFiles.contains(file.path) }
    private var lines: [DiffComputation.Line] { store.fileDiffs[file.path] ?? [] }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                if collapsed {
                    store.collapsedFiles.remove(file.path)
                } else {
                    store.collapsedFiles.insert(file.path)
                }
            } label: {
                heading
            }
            .buttonStyle(.plain)

            if !collapsed {
                if store.loadingFiles.contains(file.path) {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Reading…").font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(12)
                } else if let why = store.unsupported[file.path] {
                    Text(why)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(12)
                } else if store.isUntracked(file.path) {
                    // A file git has never seen has no diff and cannot be given
                    // one. Saying which kind of nothing this is beats an empty
                    // card that reads as a bug.
                    Text("New file — git has no earlier version to compare against.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(12)
                } else {
                    DiffLines(lines: lines)
                }
            }
        }
        .background(ChangesSurface.card, in: .rect(cornerRadius: 12))
        // The scroll decides what gets read: a file is fetched when its card
        // comes into view, not when the change set loads.
        .task(id: taskKey) {
            guard !collapsed else { return }
            await store.ensure(file.path)
        }
    }

    /// Re-runs when the file, the scope, or the fold state changes — each of
    /// which genuinely changes whether and what to fetch.
    private var taskKey: String { "\(file.path)#\(store.scope.rawValue)#\(collapsed)" }

    private var heading: some View {
        HStack(spacing: 10) {
            Text(file.status?.mark ?? "M")
                .font(.caption2.monospaced().weight(.bold))
                .foregroundStyle(file.status?.tint ?? .secondary)
                .frame(width: 14)

            VStack(alignment: .leading, spacing: 1) {
                // The leaf, then its directory underneath. A phone cannot fit
                // `crates/daemon/src/review_ops.rs` at a readable size, and the
                // leaf is the half that identifies the file.
                Text(file.name)
                    .font(.footnote.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                if !file.directory.isEmpty {
                    // Not monospaced. The Mac's file list draws the parent path
                    // in the plain system face and reserves monospace for the
                    // counts, and a directory set in Iosevka next to a
                    // proportional filename read as two different kinds of
                    // thing on the same row.
                    Text(file.directory)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }

            Spacer(minLength: 4)

            if file.binary {
                Text("binary").font(.caption2).foregroundStyle(.secondary)
            } else {
                if file.insertions > 0 {
                    Text("+\(file.insertions)")
                        .font(.caption2.monospaced()).foregroundStyle(.green)
                }
                if file.deletions > 0 {
                    Text("−\(file.deletions)")
                        .font(.caption2.monospaced()).foregroundStyle(.red)
                }
            }

            Image(systemName: collapsed ? "chevron.right" : "chevron.down")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
    }
}

/// The patch itself.
///
/// Scrolls sideways inside its own card rather than wrapping: a wrapped diff
/// line breaks the one property a diff has — that a line is a line — and on a
/// phone almost every line of real code would wrap.
private struct DiffLines: View {
    let lines: [DiffComputation.Line]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                    // Where the hunks were joined. The line numbers jumping is
                    // what says lines were left out, so the divider is drawn
                    // from the numbers rather than from a `@@` header a phone
                    // has no room to show.
                    if gapBefore(index) {
                        Rectangle()
                            .fill(Color.secondary.opacity(0.25))
                            .frame(height: 1)
                            .padding(.vertical, 3)
                    }
                    DiffLineRow(line: line)
                }
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 10)
        }
    }

    private func gapBefore(_ index: Int) -> Bool {
        guard index > 0 else { return false }
        guard let previous = lines[index - 1].newNumber, let current = lines[index].newNumber
        else { return false }
        return current > previous + 1
    }
}

private struct DiffLineRow: View {
    let line: DiffComputation.Line

    // The terminal's own face and size, from the same `@AppStorage` keys the
    // grid reads.
    //
    // A diff is code, and the whole app already has an answer to "what does
    // code look like here" — one the user chose in Settings. Drawing it in the
    // system monospace instead meant the review pane and the terminal one tab
    // away disagreed about the width of a tab stop and the shape of a zero,
    // which is exactly the comparison a diff exists to support.
    @AppStorage(TerminalSettings.fontKey) private var fontChoice: TerminalFontChoice = .iosevka
    @AppStorage(TerminalSettings.fontSizeKey) private var fontSize: Double =
        TerminalSettings.defaultFontSize

    var body: some View {
        HStack(spacing: 8) {
            Text(line.newNumber.map(String.init) ?? line.oldNumber.map(String.init) ?? "")
                // Two points down and dimmed: the gutter is for orientation, not
                // for reading, and at the body size it competes with the code.
                .font(.terminal(fontChoice, size: fontSize - 2))
                .foregroundStyle(.tertiary)
                .frame(width: 30, alignment: .trailing)

            Text(marker + line.text)
                .font(.terminal(fontChoice, size: fontSize))
                .foregroundStyle(color)
                .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.vertical, 0.5)
        .background(background)
    }

    private var marker: String {
        switch line.kind {
        case .added: return "+"
        case .removed: return "-"
        case .context: return " "
        }
    }

    private var color: Color {
        switch line.kind {
        case .added: return .green
        case .removed: return .red
        case .context: return .primary.opacity(0.75)
        }
    }

    private var background: Color {
        switch line.kind {
        case .added: return .green.opacity(0.10)
        case .removed: return .red.opacity(0.10)
        case .context: return .clear
        }
    }
}

/// One sentence in a card, for the states that are not a list of files.
private struct ChangesNotice: View {
    let symbol: String
    let tint: Color
    let text: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).foregroundStyle(tint)
            Text(text).font(.footnote).foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 12))
    }
}
