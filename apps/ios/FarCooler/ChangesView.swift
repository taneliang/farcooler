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
                } else if store.commitUnreadable {
                    // Ahead of the empty case on purpose: a commit that could
                    // not be read also has no files, and "nothing changed here"
                    // is the one sentence that must not be said about it.
                    ChangesNotice(
                        symbol: "exclamationmark.triangle",
                        tint: .orange,
                        text: "Couldn't read this commit. An amend or a rebase may have "
                            + "replaced it while you were reading.")
                } else if store.files.isEmpty && !store.loading {
                    ChangesNotice(
                        symbol: "checkmark.circle", tint: .secondary, text: nothingHere)
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
        // A sheet, not a push. Choosing a commit is a detour off the thing on
        // screen and it ends by coming straight back to it — the same shape as
        // the worktree switcher and `BranchPicker`, which is what this app
        // already uses for a choice of this weight. A push would put the diff
        // behind a Back button and make the branch feel like somewhere you had
        // left.
        //
        // Presented from here rather than from either control that opens it:
        // one of those controls lives in `PaneHost`'s toolbar, and a sheet
        // hung off a `Menu`'s content is hung off something that is not a live
        // view hierarchy to present from. The flag lives on the store so both
        // can reach it.
        .sheet(isPresented: $store.showingHistory) {
            CommitHistorySheet(store: store)
        }
    }

    /// Which nothing this is, when the list is empty.
    private var nothingHere: String {
        switch store.scope {
        case .branch: return "This branch hasn't committed anything yet."
        case .local: return "Nothing uncommitted. The worktree is clean."
        case .commit:
            // Not "this commit is empty". A commit is compared against its
            // FIRST parent here — `Selector::Commit` in the daemon's
            // file_diff.rs — and a merge that only joined two branches
            // genuinely changed nothing against that side while changing
            // plenty against the other. Naming the comparison is the
            // difference between a fact and a claim this screen cannot make.
            return "Nothing changed against this commit's first parent, "
                + "which is also what a clean merge looks like."
        }
    }

    /// Branch, what is being compared, and the two numbers — the whole worktree
    /// in one card.
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

            // The card's lower half is the one thing on this screen that says
            // WHAT is being compared, so a commit replaces it outright rather
            // than being squeezed in beside the branch's base and counts, which
            // would then be describing something nobody is looking at.
            if let sha = store.scope.commitSha {
                commitHeader(sha)
            } else {
                branchHeader
            }
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 14))
    }

    /// Base, counts, the comparison control, and the way into the history.
    @ViewBuilder
    private var branchHeader: some View {
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
        //
        // Two segments and not three: a `Commit` segment cannot answer its own
        // question — tapping it says nothing about WHICH commit — so it would
        // have to open the row directly beneath it. See `DiffScope.allCases`.
        Picker("Showing", selection: $store.scope) {
            ForEach(DiffScope.allCases) { scope in
                Text(scope.label).tag(scope)
            }
        }
        .pickerStyle(.segmented)
        .padding(.top, 2)

        historyRow
    }

    /// The way in to one commit at a time.
    ///
    /// A row rather than a toolbar item alone, because a control nobody can see
    /// is a feature nobody has: the toolbar's copy of this exists for the reader
    /// who is already scrolled a thousand lines down, not for the one arriving.
    private var historyRow: some View {
        let count = store.changeSet.commits.count
        return Button {
            store.showingHistory = true
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "clock")
                    .font(.caption)
                    .foregroundStyle(count == 0 ? .tertiary : .secondary)
                Text("History")
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(count == 0 ? .tertiary : .primary)
                Spacer(minLength: 4)
                // Said rather than left blank. A branch with nothing on it yet
                // is the ordinary state of a worktree an agent has only just
                // started in, and a control that does nothing when tapped is
                // worse than one that says why it is quiet.
                Text(count == 0 ? "No commits yet" : count == 1 ? "1 commit" : "\(count) commits")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if count > 0 {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(count == 0)
        .padding(.top, 4)
    }

    /// Which commit is on screen, what it did, and the ways back out.
    ///
    /// The counts are summed from the commit's own file list rather than read
    /// off `ChangeCommit`, whose three count fields the daemon hardcodes to
    /// zero — see `ChangesStore.commitInsertions`. They appear only once that
    /// list has landed, because until then the honest number is no number.
    @ViewBuilder
    private func commitHeader(_ sha: String) -> some View {
        let known = store.selectedCommitInfo

        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(String(sha.prefix(8)))
                .font(.caption.monospaced().weight(.semibold))
                .foregroundStyle(.secondary)
            // Up to two lines: a subject is one line of prose written for a
            // terminal, and truncating it to a phone's width regularly cuts it
            // before the verb.
            if let known, !known.subject.isEmpty {
                Text(known.subject)
                    .font(.footnote.weight(.medium))
                    .lineLimit(2)
            }
        }

        HStack(spacing: 10) {
            if let known {
                Text("\(known.author) · \(known.made)")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            Spacer(minLength: 4)
            if !store.commitFiles.isEmpty {
                Text("+\(store.commitInsertions)")
                    .font(.caption.monospaced()).foregroundStyle(.green)
                Text("−\(store.commitDeletions)")
                    .font(.caption.monospaced()).foregroundStyle(.red)
            }
        }

        // The change set no longer lists this sha, so there is no subject and no
        // author to show — an amend or a rebase during the read does exactly
        // that. The patch below is usually still right, because the object it
        // names is still in the repository, so this warns rather than blanks
        // the pane.
        if known == nil {
            Label(
                "This commit isn't on the branch anymore. It was probably amended "
                    + "or rebased while you were reading.",
                systemImage: "exclamationmark.triangle")
                .font(.caption2)
                .foregroundStyle(.orange)
        }

        HStack(spacing: 8) {
            // The obvious way back, said in words. On the Mac this is a segment
            // of a control that is still on screen; here that control is not
            // drawn while a commit is, both because a segmented picker whose
            // selection matches no segment is a control with nothing selected
            // and because the space it wanted is what the subject is using.
            Button("Whole Branch") { store.showWholeBranch() }
            Spacer(minLength: 4)
            Button("History") { store.showingHistory = true }
        }
        .font(.footnote)
        .buttonStyle(.bordered)
        .controlSize(.small)
        .padding(.top, 2)
    }
}

/// The changes pane's contextual toolbar control.
///
/// Owned by `PaneHost`, not `ChangesView`: the host owns every navigation-bar
/// item and can therefore keep their order stable as panes change. A mounted
/// child contributing its own toolbar item made SwiftUI merge two independent
/// toolbar trees, which moved the worktree switcher whenever this menu appeared.
struct ChangesToolbarMenu: View {
    @ObservedObject var store: ChangesStore

    var body: some View {
        Menu {
            // While a commit is showing, this picker has nothing checked —
            // truthfully, since neither the whole branch nor the uncommitted
            // work is what is on screen — and choosing either entry is one of
            // the ways back to the branch.
            Picker("Showing", selection: $store.scope) {
                ForEach(DiffScope.allCases) { scope in
                    Text(scope.label).tag(scope)
                }
            }
            Divider()
            // The second way in to the history, for the reader who is already
            // a long way down a diff and would have to scroll back to the
            // summary card to find the first one.
            Button {
                store.showingHistory = true
            } label: {
                Label("History", systemImage: "clock")
            }
            .disabled(store.changeSet.commits.isEmpty)
            Divider()
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

    /// Re-runs when the file, the fold state, or the generation changes — each
    /// of which genuinely changes whether and what to fetch.
    ///
    /// The generation rather than the scope, and that is the load-bearing part:
    /// nothing else in this view asks for a file's diff a second time, so a
    /// heading that stays realized while the cache is emptied underneath it
    /// would sit at "Reading…" forever. Every emptying bumps the generation,
    /// including the ones a scope change causes, so this covers strictly more
    /// than keying on the scope did — a pull to refresh included.
    private var taskKey: String { "\(file.path)#\(store.generation)#\(collapsed)" }

    private var heading: some View {
        HStack(spacing: 10) {
            // A bullet, not `M`, when nothing determined the status.
            //
            // Every file in a commit arrives that way: `changes.commit_files`
            // is built from `git diff --numstat`, which counts lines and never
            // says added or deleted, and `CommitFiles.asDetermined` drops the
            // "modified" the parser invents so that nothing here has to guess.
            // Defaulting to `M` would put "Modified" beside a file the commit
            // created — the accessibility label below says "Changed" for the
            // same reason.
            Text(file.status?.mark ?? "•")
                .font(.caption2.monospaced().weight(.bold))
                .foregroundStyle(file.status?.tint ?? .secondary)
                .frame(width: 14)
                .accessibilityLabel(file.status?.label ?? "Changed")

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

/// The branch, one commit at a time.
///
/// A branch is a sequence of decisions, and until this existed the pane could
/// show the whole sequence or the uncommitted work and nothing in between.
///
/// A `List` inside a `NavigationStack` with Cancel in the leading slot, which is
/// this app's shape for "pick one of these and come straight back" — the same as
/// `BranchPicker` and the worktree switcher. Lazy by construction, so a branch
/// with six hundred commits on it costs what a screenful costs; `.searchable`
/// is what makes that branch usable rather than merely survivable.
private struct CommitHistorySheet: View {
    @ObservedObject var store: ChangesStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""

    /// Read once, when the sheet is built, and passed to every row.
    ///
    /// `ChangeCommit.age(at:)` takes a clock rather than reading one for the
    /// reason `Terminal.displayDuration(at:)` gives — a `Date()` read inside a
    /// property is an input SwiftUI cannot observe. Nothing here needs to tick:
    /// a sheet is open for seconds, and `3h` does not become `4h` inside one.
    @State private var now = Date()

    private var shown: [ChangeCommit] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return store.commitsNewestFirst }
        // Subject, author and sha, because all three are things people
        // half-remember about a commit they are looking for. The sha is matched
        // as a prefix: nobody searches for the middle of a hash, and a
        // substring match on hex turns every two-character query into noise.
        return store.commitsNewestFirst.filter {
            $0.subject.lowercased().contains(query)
                || $0.author.lowercased().contains(query)
                || $0.sha.lowercased().hasPrefix(query)
        }
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Button {
                        store.showWholeBranch()
                        dismiss()
                    } label: {
                        wholeBranch
                    }
                }

                Section {
                    if store.changeSet.commits.isEmpty {
                        // The branch itself is empty, which is the ordinary
                        // state of a worktree an agent has only just started in
                        // — not a failure, and not the same as a filter that
                        // matched nothing.
                        Text("This branch hasn't committed anything yet.")
                            .foregroundStyle(.secondary)
                    } else if shown.isEmpty {
                        Text("No commits match that.")
                            .foregroundStyle(.secondary)
                    }

                    ForEach(shown) { commit in
                        Button {
                            // Dismissed without waiting. The read is a round
                            // trip over somebody's cellular link, and holding
                            // the sheet up for it would hide the pane that has
                            // the spinner in it; the task outlives this view
                            // because the store, not the sheet, owns it.
                            Task { await store.select(commit: commit.sha) }
                            dismiss()
                        } label: {
                            row(commit)
                        }
                    }
                } header: {
                    Text("Commits")
                }
            }
            .searchable(text: $search, prompt: "Filter commits")
            .navigationTitle("History")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }

    /// The way back to everything, at the top, where the thing you undo a
    /// choice with belongs. Two more exist in the card the sheet covers, since
    /// somebody who has already chosen a commit should not have to open a
    /// picker to stop looking at one.
    private var wholeBranch: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.triangle.branch")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Whole Branch")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.primary)
                // The base is named when it is known. It is empty until the
                // first read lands, and "Every commit since , at once" is how
                // an unloaded pane would read it out.
                Text(
                    store.changeSet.baseRef.isEmpty
                        ? "Every commit on this branch, at once"
                        : "Every commit since \(store.changeSet.baseRef), at once"
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            Spacer(minLength: 4)
            if store.scope.commitSha == nil {
                Image(systemName: "checkmark").font(.footnote).foregroundStyle(.tint)
            }
        }
    }

    /// Sha, subject, author and age — everything that is actually known.
    ///
    /// No `+N -M`, and that is not an omission this file can fix: the daemon
    /// hardcodes all three of `ChangeCommit`'s count fields to zero and the wire
    /// drops them entirely. See `ChangeCommit`'s own comment. The counts appear
    /// in the summary card once a commit is chosen, summed from the file list
    /// that choosing it fetches, where they are real numbers.
    private func row(_ commit: ChangeCommit) -> some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                // Two lines, because a subject is one line of prose written for
                // a terminal and a phone's width regularly cuts it before the
                // verb.
                Text(commit.subject.isEmpty ? "(no subject)" : commit.subject)
                    .font(.subheadline)
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                HStack(spacing: 6) {
                    Text(commit.short)
                        .font(.caption2.monospaced())
                        .foregroundStyle(.tertiary)
                    Text(commit.author)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Text("·")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                    Text(commit.age(at: now))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 4)
            if store.scope.commitSha == commit.sha {
                Image(systemName: "checkmark").font(.footnote).foregroundStyle(.tint)
            }
        }
        .contentShape(Rectangle())
    }
}
