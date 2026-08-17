import AgentKit
import SwiftUI

/// What this worktree changed: the whole branch, in one scroll.
///
/// Every changed file is here, one after another, the way `git diff` reads and
/// the way a review actually goes — you start at the top and go down. Showing
/// one file at a time was the first design and it turned a diff into a filing
/// cabinet: to find out what a branch did you opened forty drawers, and nothing
/// on screen ever told you how much was left.
///
/// So the file list does not FILTER, it JUMPS. Picking a file scrolls to it and
/// leaves everything else where it was, which means you can jump to the file you
/// came for and still fall into the one after it.
///
/// One concept at two widths rather than two modes to learn: the jump control is
/// a combo box when the pane is narrow and the same list promoted to a column
/// when it is wide.
///
/// This is the contents of a tmux pane, not a panel the app arranges for itself.
/// It was the latter first, and the app carried a whole second layout engine to
/// do it — a tree of tiles with its own splits, fractions, dividers and
/// persistence — so that one view could sit beside the terminals. The two trees
/// could not agree: the diff could not be zoomed, dragged onto another pane,
/// broken out into its own window or reached with `⌃B` arrows, because every one
/// of those belongs to tmux and the diff was not in tmux. It is now, and all of
/// that arrived for free.
struct ChangesPane: View {
    @ObservedObject var changes: ChangesStore

    /// The terminal's own font, because this is the same code, read the same
    /// way, a few inches from a pane rendering it. Two monospaced faces side by
    /// side is a difference that means nothing, and someone who has set their
    /// terminal to 14pt Berkeley Mono did not do it to read diffs in 10.5pt SF.
    @ObservedObject private var preferences = Preferences.shared

    /// Below this the file list is a combo box; above it, a column.
    private static let wideEnough: CGFloat = 620

    /// The scroll's own coordinate space, so a heading can say where it sits.
    private static let scrollSpace = "diff.scroll"

    @State private var picking = false
    @State private var query = ""
    @FocusState private var filtering: Bool
    @State private var lastHunkJump: String?

    private var codeFont: Font { Font(preferences.terminalFont() as CTFont) }

    var body: some View {
        GeometryReader { geo in
            VStack(spacing: 0) {
                if changes.error != nil {
                    problem
                }
                if geo.size.width >= Self.wideEnough {
                    HStack(spacing: 0) {
                        fileColumn.frame(width: fileColumnWidth(for: geo.size.width))
                        Divider()
                        VStack(spacing: 0) {
                            diffNavigator(compact: false)
                            Divider()
                            diffBody
                        }
                    }
                } else {
                    VStack(spacing: 0) {
                        diffNavigator(compact: true)
                        Divider()
                        diffBody
                    }
                }
            }
        }
        .background(WorkspaceStyle.document)
        .task(id: changes.workspace.id) { await changes.loadIfNeeded() }
        // Cancelled with the view, which is what keeps this honest: the poll
        // exists only while somebody is reading the diff.
        .task(id: changes.workspace.id) { await changes.follow() }
    }

    /// More room when the pane has it, without letting navigation consume the
    /// document. At the compact breakpoint the diff still gets 400pt; in a
    /// full-window review, long paths can breathe instead of becoming a column
    /// of identical leading ellipses.
    private func fileColumnWidth(for paneWidth: CGFloat) -> CGFloat {
        min(280, max(220, paneWidth * 0.20))
    }

    /// An older runner cannot do this at all, and that is worth saying rather
    /// than rendering as a worktree with no changes.
    private var problem: some View {
        let old = changes.client.changesSupported == false
        return VStack(alignment: .leading, spacing: 3) {
            Text(old ? "This runner can't show changes yet" : "Couldn't read this worktree")
                .font(.system(size: 11.5, weight: .medium))
            Text(
                old
                    ? "Its copy of Far Cooler is older than this. Update it in Settings › Runners."
                    : (changes.error ?? "")
            )
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(9)
        .background(.orange.opacity(0.12))
    }

    // MARK: - Navigation

    /// Where to jump next, cleared as soon as the scroll has taken it.
    ///
    /// A path rather than a call into a scroll proxy, because both jump controls
    /// live OUTSIDE the `ScrollViewReader` that owns the proxy — the combo box
    /// sits above the scroll and the column sits beside it.
    @State private var jumpTo: String?

    /// Which row Enter will take, so the highlight is a promise rather than a
    /// reminder of what is already open.
    @State private var highlighted = 0

    /// Where the scroll actually is, reported by the scroll view itself.
    ///
    /// `ScrollPosition.point` is not this: it answers only for a position that
    /// was SET as a point, and is nil after a scroll to an id or a drag by
    /// hand — which is every case the restore below has to add its offset to.
    @State private var scrolledX: CGFloat = 0
    @State private var scrolledY: CGFloat = 0

    /// True from the moment a restore starts until it has landed, so the scroll
    /// it performs is not mistaken for the reader scrolling and recorded as a
    /// new target — which would overwrite the destination with the journey.
    @State private var restoring = false

    /// False until the view has taken up the position it is supposed to be in.
    /// Until then nothing the scroll view reports about itself is news, and
    /// nothing it is showing is worth looking at — see `body`, which keeps the
    /// diff invisible until this turns true.
    @State private var settled = false

    /// A jump the reader asked for, still travelling.
    ///
    /// Set while the scroll animates towards a file somebody picked, so the
    /// tracking that names the file you are INSIDE does not narrate the trip.
    /// Without it, picking a file lit that row in the list and then handed the
    /// highlight to every file the scroll passed over on its way there — the
    /// selection appearing to run down the list on its own and land somewhere
    /// you did not click.
    @State private var jumping = false

    /// Put the scroll back where it was before this pane's layout was switched
    /// away from, before anybody sees it anywhere else.
    ///
    /// Runs when the pane comes into view — which is when its tmux layout is
    /// activated, whether or not the pane has focus. Focus is a separate
    /// question and not one the scroll should wait on: a layout you switch to
    /// in order to READ the diff usually has the keyboard somewhere else.
    private func restore() {
        let target = changes.scrollTarget
        guard target != .zero else {
            // Nothing to go back to, so say so explicitly rather than leaving
            // it to a default. A two-axis scroll view handed a position binding
            // it has never been given a value for does not reliably start at
            // the top LEFT — the first diff drawn came up scrolled a gutter and
            // a half to the right, line numbers off the edge, which reads as a
            // diff someone had already been dragging around.
            changes.scrollPosition.scrollTo(point: .zero)
            settled = true
            return
        }
        restoring = true
        Task { @MainActor in
            // Asked repeatedly, because once is not enough and cannot be. The
            // stack is lazy: the first request is clamped to the bottom of the
            // one screenful that exists, which builds the rows just past it,
            // which lets the next request go further. Three or four rounds
            // covers a nine-thousand-line diff; the loop stops early the moment
            // it arrives, and stops anyway if a round makes no progress — which
            // is what a diff that has genuinely become shorter looks like.
            var previous = -1.0
            for _ in 0..<16 {
                changes.scrollPosition.scrollTo(point: target)
                try? await Task.sleep(for: .milliseconds(25))
                if abs(scrolledY - target.y) < 2 { break }
                if scrolledY == previous { break }
                previous = scrolledY
            }
            restoring = false
            settled = true
        }
    }

    private func jump(to f: ChangedFile) {
        picking = false
        changes.selectedFile = f.path
        lastHunkJump = nil
        jumping = true
        jumpTo = f.path
        Task { await changes.ensure(f.path) }
    }

    /// Always-visible orientation and movement. At narrow widths the current
    /// file opens the searchable picker; at wide widths the file column already
    /// provides that choice, so the same space becomes a sticky current-file
    /// label. Hunk movement remains available at either width.
    private func diffNavigator(compact: Bool) -> some View {
        HStack(spacing: 4) {
            if compact {
                navButton("chevron.up", help: "Previous file") { moveFile(-1) }
                filePicker
                navButton("chevron.down", help: "Next file") { moveFile(1) }
            } else if let file = chosen {
                FileStatusBadge(status: file.status, binary: file.binary)
                Text(pathLabel(file))
                    .font(.system(size: 11, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.head)
                Text(counts(file))
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(.secondary)
            } else {
                Text(chosenLabel)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 4)

            Divider().frame(height: 14).padding(.horizontal, 2)
            let hunks = hunkTargets.count
            if hunks > 0 {
                Text(hunks == 1 ? "1 hunk" : "\(hunks) hunks")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .fixedSize()
            }
            navButton("arrow.up.to.line", help: "Previous hunk or file") { moveHunk(-1) }
            navButton("arrow.down.to.line", help: "Next hunk or file") { moveHunk(1) }
        }
        .padding(.horizontal, 6)
        .frame(height: 28)
        .background(WorkspaceStyle.paneChrome.opacity(0.60))
    }

    private func navButton(_ symbol: String, help: String, action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 9.5, weight: .medium))
                .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .disabled(changes.files.isEmpty)
        .help(help)
    }

    private func moveFile(_ direction: Int) {
        let files = changes.files
        guard !files.isEmpty else { return }
        let current = files.firstIndex { $0.path == changes.selectedFile }
            ?? (direction > 0 ? -1 : 0)
        let next = (current + direction + files.count) % files.count
        jump(to: files[next])
    }

    private func moveHunk(_ direction: Int) {
        let targets = hunkTargets
        guard !targets.isEmpty else {
            moveFile(direction)
            return
        }

        if let lastHunkJump,
            let current = targets.firstIndex(of: lastHunkJump)
        {
            let next = current + direction
            guard targets.indices.contains(next) else {
                moveFile(direction)
                return
            }
            jump(toHunk: targets[next])
            return
        }

        guard let target = direction > 0 ? targets.first : targets.last else {
            moveFile(direction)
            return
        }
        jump(toHunk: target)
    }

    private func jump(toHunk target: String) {
        lastHunkJump = target
        jumping = false
        jumpTo = target
    }

    private var hunkTargets: [String] {
        let current = changes.selectedFile
        return rows.compactMap { row in
            guard row.path == current, case .hunk = row.kind else { return nil }
            return row.id
        }
    }

    /// Which file, when there is no room for a list.
    ///
    /// A combo box: it opens, and you type. Two versions came before it and both
    /// failed the same test, a branch with forty changed files in it. A strip of
    /// tabs became a bar you scrolled sideways through, hunting, with the one you
    /// wanted off-screen in a direction you could not guess. A plain menu fixed
    /// the sideways scrolling and kept the hunting: you still read forty rows
    /// looking for the one you already knew the name of.
    ///
    /// Filtering runs on the whole path, not the file name, so `store/rev` finds
    /// `crates/store/src/review.rs` — which is how you remember a file when
    /// three of them are called `mod.rs`.
    private var filePicker: some View {
        Button { picking = true } label: {
            HStack(spacing: 5) {
                if let file = chosen {
                    FileStatusBadge(status: file.status, binary: file.binary)
                }
                Text(chosenLabel)
                    .font(.system(size: 10.5))
                    .lineLimit(1)
                    .truncationMode(.head)
                Spacer(minLength: 0)
                if let f = chosen {
                    Text(counts(f))
                        .font(.system(size: 10.5, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 8))
                    .foregroundStyle(.secondary)
            }
            // Inside the label, so the padding is part of the target rather
            // than a margin around it.
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .leading)
        .help("Jump to a file")
        .popover(isPresented: $picking, arrowEdge: .bottom) { filterList }
        .onChange(of: picking) { _, open in
            // Cleared on DISMISS, not on pick: clearing it as the popover
            // closes repopulates the list mid-animation, and clearing it only
            // on a successful pick means Escape leaves a filter behind that
            // silently hides files the next time it opens.
            if !open {
                query = ""
                highlighted = 0
            }
        }
    }

    private var filterList: some View {
        VStack(spacing: 0) {
            TextField("Filter files", text: $query)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11))
                .padding(7)
                .focused($filtering)
                // A popover does not focus a field on macOS, and a combo box
                // you have to click into before typing is just a menu.
                .onAppear { filtering = true }
                .onChange(of: query) { _, _ in highlighted = 0 }
                .onSubmit { openHighlighted() }
            Divider()
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        if matches.isEmpty {
                            Text(
                                query.isEmpty
                                    ? "Nothing changed here"
                                    : "No file matches “\(query)”"
                            )
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .padding(8)
                        }
                        ForEach(Array(matches.enumerated()), id: \.element.id) { i, f in
                            Button { jump(to: f) } label: {
                                HStack(spacing: 6) {
                                    Text(pathLabel(f))
                                        .font(.system(size: 11))
                                        .lineLimit(1)
                                        .truncationMode(.head)
                                    Spacer(minLength: 4)
                                    Text(counts(f))
                                        .font(.system(size: 11, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                }
                                .padding(.horizontal, 7)
                                .padding(.vertical, 3)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                                .background(
                                    i == highlighted
                                        ? Color.accentColor.opacity(0.18) : .clear,
                                    in: RoundedRectangle(cornerRadius: 4))
                            }
                            .buttonStyle(.plain)
                            .id(f.path)
                        }
                    }
                    .padding(5)
                }
                .frame(maxHeight: 260)
                .onChange(of: highlighted) { _, i in
                    guard matches.indices.contains(i) else { return }
                    proxy.scrollTo(matches[i].path, anchor: .center)
                }
            }
        }
        .frame(width: 380)
        // Arrows move the highlight; the field keeps the keystrokes it wants.
        .onMoveCommand { direction in
            switch direction {
            case .up: highlighted = max(0, highlighted - 1)
            case .down: highlighted = min(max(0, matches.count - 1), highlighted + 1)
            default: break
            }
        }
    }

    private func openHighlighted() {
        guard matches.indices.contains(highlighted) else { return }
        jump(to: matches[highlighted])
    }

    /// Every file whose path contains what was typed, case-insensitively.
    private var matches: [ChangedFile] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return changes.files }
        return changes.files.filter {
            $0.path.range(of: q, options: .caseInsensitive) != nil
        }
    }

    private var chosen: ChangedFile? {
        changes.files.first { $0.path == changes.selectedFile }
    }

    private var chosenLabel: String {
        if let c = chosen { return c.path }
        let n = changes.files.count
        return n == 1 ? "1 changed file" : "\(n) changed files"
    }

    private func counts(_ f: ChangedFile) -> String {
        if f.binary { return "binary" }
        var parts: [String] = []
        if f.insertions > 0 { parts.append("+\(f.insertions)") }
        if f.deletions > 0 { parts.append("-\(f.deletions)") }
        // A rename or a mode change touches no lines, and a blank cell would
        // read as a file that failed to load.
        return parts.isEmpty ? "no lines" : parts.joined(separator: " ")
    }

    private func pathLabel(_ file: ChangedFile) -> String {
        guard file.status == .renamed, let old = file.oldPath, old != file.path else {
            return file.path
        }
        return "\(old) → \(file.path)"
    }

    /// File name first, because that is how a list of source files is scanned.
    /// The parent path remains just below it for the three `mod.rs` case.
    private func fileNameLabel(_ file: ChangedFile) -> String {
        let current = (file.path as NSString).lastPathComponent
        guard file.status == .renamed, let old = file.oldPath, old != file.path else {
            return current
        }
        return "\((old as NSString).lastPathComponent) → \(current)"
    }

    private func parentPathLabel(_ file: ChangedFile) -> String? {
        let current = (file.path as NSString).deletingLastPathComponent
        let currentLabel = current.isEmpty ? nil : current
        guard file.status == .renamed, let old = file.oldPath, old != file.path else {
            return currentLabel
        }
        let previous = (old as NSString).deletingLastPathComponent
        if previous == current { return currentLabel }
        let oldLabel = previous.isEmpty ? "." : previous
        let newLabel = current.isEmpty ? "." : current
        return "\(oldLabel) → \(newLabel)"
    }

    /// The same list as the combo box, with room to show it.
    private var fileColumn: some View {
        VStack(spacing: 0) {
            HStack(spacing: 5) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 9.5))
                    .foregroundStyle(.tertiary)
                TextField("Filter files", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 11))
                Text("\(matches.count)")
                    .font(.system(size: 9.5, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
            }
            .padding(.horizontal, 7)
            .frame(height: 22)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(Color.primary.opacity(0.045)))
            .padding(.horizontal, 6)
            .frame(height: 28)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(matches) { f in
                            Button { jump(to: f) } label: {
                            HStack(alignment: .center, spacing: 6) {
                                FileStatusBadge(status: f.status, binary: f.binary)
                                VStack(alignment: .leading, spacing: 1) {
                                    HStack(spacing: 5) {
                                        Text(fileNameLabel(f))
                                            .font(.system(size: 11, weight: .medium))
                                            .lineLimit(1)
                                            .truncationMode(.middle)
                                        Spacer(minLength: 3)
                                        Text(counts(f))
                                            .font(.system(size: 9.5, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                            .fixedSize()
                                    }
                                    Text(parentPathLabel(f) ?? " ")
                                        .font(.system(size: 9.5))
                                        .foregroundStyle(.tertiary)
                                        .lineLimit(1)
                                        .truncationMode(.head)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            // The whole row, not just where the glyphs are.
                            // Without this a click landed only on the text, so
                            // hitting a file's `+12 -3` — the part you aim at,
                            // because it is what you are deciding on — did
                            // nothing at all.
                            .contentShape(Rectangle())
                            .background(
                                changes.selectedFile == f.path
                                    ? WorkspaceStyle.navigatorSelection : .clear,
                                in: RoundedRectangle(cornerRadius: 4))
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(6)
            }
            .scrollContentBackground(.hidden)
        }
    }

    // MARK: - The diff

    /// Empty means two different things, and saying the wrong one is a wrong
    /// answer: a branch that matches its base is finished work, and a worktree
    /// with nothing uncommitted is work that has been committed.
    private var nothingDetail: String {
        if changes.scope == .local { return "Everything here is committed." }
        return changes.changeSet.baseRef.isEmpty
            ? "There's nothing to compare this branch against yet."
            : "This branch matches \(changes.changeSet.baseRef)."
    }

    @ViewBuilder
    private var diffBody: some View {
        if changes.error != nil {
            // The banner above already said what went wrong, and "Nothing
            // changed here" underneath it would contradict it.
            Color.clear
        } else if changes.files.isEmpty {
            PaneNotice(title: "Nothing changed here", detail: nothingDetail)
        } else {
            ScrollViewReader { proxy in
                // ONE scroll axis here, and the horizontal one lives inside each
                // file. Wrapping this whole list in a horizontal `ScrollView`
                // was the obvious way to write it and it hung the app: a
                // cross-axis scroll view has to measure its content's full
                // width, which forces the `LazyVStack` to realize every child to
                // ask how wide it is. Every file fetched its diff at once and
                // every line was built before anything drew.
                GeometryReader { geo in
                    ScrollView([.vertical, .horizontal]) {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(rows) { r in row(r) }
                        }
                        // Stated, never measured. This is what keeps the lazy
                        // stack lazy in a scroll view that also scrolls
                        // sideways: with a definite width it has nothing to ask
                        // its children, so it builds only what is on screen.
                        //
                        // `minHeight` is what pins a short diff to the top. A
                        // scroll view that scrolls in BOTH directions centres
                        // content smaller than its viewport — in both axes — so
                        // a branch that changed one file drew it floating in
                        // the middle of the pane with empty space above it,
                        // reading as a diff that had been scrolled to rather
                        // than one that starts here. Asking for at least the
                        // viewport's height, top-aligned, costs nothing once
                        // the content is taller: `minHeight` stops constraining
                        // the moment it is exceeded.
                        //
                        // Two frames rather than one, because the modifier that
                        // takes a definite `width` has no `minHeight` to give.
                        .frame(width: max(geo.size.width, contentWidth), alignment: .leading)
                        .frame(minHeight: geo.size.height, alignment: .topLeading)
                    }
                    // A diff starts at its top-left corner. Both words matter,
                    // and neither is the default.
                    //
                    // A scroll view given two axes centres content that does not
                    // fill them — so a short diff floated in the middle of the
                    // pane, and, worse, that centre was KEPT as the content grew:
                    // every file read made the content wider, the view held its
                    // horizontal middle, and the diff drifted sideways until the
                    // line numbers were off the left edge. It looked like a view
                    // somebody had been dragging around, on a first draw nobody
                    // had touched.
                    .defaultScrollAnchor(.topLeading)
                    // Where the scroll is, kept in the STORE rather than here,
                    // so switching tmux layouts and coming back lands where you
                    // left off. See `ChangesStore.scrollTarget`.
                    .scrollPosition($changes.scrollPosition)
                    .onScrollGeometryChange(for: CGPoint.self) { $0.contentOffset } action: {
                        _, offset in
                        scrolledX = offset.x
                        scrolledY = offset.y
                        // Not before the restore has had its turn, and not
                        // during it.
                        //
                        // A scroll view reports its geometry as it is built —
                        // at zero, since that is where a new one starts — and
                        // that report arrives BEFORE `onAppear`. Recorded, it
                        // overwrote the very position `onAppear` was about to
                        // put back, so every return landed at the top and the
                        // restore looked like it was doing nothing at all.
                        if settled, !restoring { changes.scrollTarget = offset }
                    }
                    // Not shown until it is where it belongs.
                    //
                    // The restore takes a few frames — it has to, since the
                    // rows are lazy and each attempt builds the ones the next
                    // needs — and every one of those frames was on screen. So
                    // coming back to a layout drew the diff at the top and then
                    // walked it down to where you had been, which reads as the
                    // view scrolling itself while you watch. Laid out but
                    // invisible, it does the same work and arrives finished.
                    .opacity(settled ? 1 : 0)
                    .animation(.easeIn(duration: 0.1), value: settled)
                    .onAppear { restore() }
                    // A diff nobody can see is a worse bug than a diff that
                    // arrives in the wrong place, so the invisibility above has
                    // a deadline it cannot miss.
                    .task {
                        try? await Task.sleep(for: .milliseconds(700))
                        settled = true
                    }
                }
                .coordinateSpace(name: Self.scrollSpace)
                // The file you are inside is the last heading at or above the
                // top of the view. Falls back to the topmost one still below it,
                // which is the case on the very first screen.
                .onPreferenceChange(HeadingTops.self) { tops in
                    guard !tops.isEmpty else { return }
                    let above = tops.filter { $0.value <= 1 }
                    let current =
                        above.max(by: { $0.value < $1.value })?.key
                        ?? tops.min(by: { $0.value < $1.value })?.key
                    guard let current else { return }
                    // A jump in progress is not a reader scrolling, and neither
                    // is a restore. Both move the scroll past files nobody
                    // asked about, and reporting each one in turn made the
                    // selection in the list run away from the row that had just
                    // been clicked. The jump ends when it arrives.
                    if jumping {
                        if current == changes.selectedFile { jumping = false }
                        return
                    }
                    guard !restoring else { return }
                    if current != changes.selectedFile { changes.selectedFile = current }
                }
                .onChange(of: jumpTo) { _, target in
                    guard let target else { return }
                    // `.top` is `UnitPoint(x: 0.5, y: 0)` — its x centres the
                    // content horizontally, which scrolled every jump into the
                    // middle of a line. Only the vertical position is wanted.
                    withAnimation(.easeInOut(duration: 0.18)) {
                        proxy.scrollTo(target, anchor: UnitPoint(x: 0, y: 0))
                    }
                    jumpTo = nil
                }
            }
        }
    }

    /// The whole diff as ONE flat list of rows, headings and lines together.
    ///
    /// Flat because the lazy stack can only virtualize what it holds directly.
    /// Nesting each file's lines inside a section made the stack lazy per FILE:
    /// realizing one heading built every line under it, so scrolling into a
    /// four-thousand-line file built four thousand views in a frame and the app
    /// went to 100% CPU. Flattened, only the rows on screen exist, and a file's
    /// size stops mattering.
    private var rows: [DiffRow] {
        var out: [DiffRow] = []
        out.reserveCapacity(changes.files.count * 8)
        for f in changes.files {
            out.append(DiffRow(id: f.path, kind: .heading(f), path: f.path))
            if changes.collapsedFiles.contains(f.path) { continue }
            if f.binary {
                out.append(note(f, "Binary file — nothing to show"))
            } else if changes.isUntracked(f.path) {
                // Listed but not diffed, and it says so. git has nothing to
                // compare a brand new file against, and "No textual changes"
                // under the name of a file somebody just wrote is the most
                // wrong thing this view could say.
                out.append(note(f, "New file — git isn't tracking it yet"))
            } else if let lines = changes.fileDiffs[f.path] {
                if lines.isEmpty {
                    out.append(note(f, "No textual changes"))
                } else {
                    out.append(contentsOf: body(of: f, lines: lines))
                }
            } else {
                out.append(note(f, changes.loadingFiles.contains(f.path) ? "Reading…" : "…"))
            }
        }
        return out
    }

    private func note(_ f: ChangedFile, _ text: String) -> DiffRow {
        DiffRow(id: "\(f.path)!note", kind: .note(text), path: f.path)
    }

    /// One file's lines, with the gaps between its hunks marked.
    ///
    /// A diff is not the file: git prints three lines either side of each change
    /// and leaves out everything between. Those omissions are exactly where "is
    /// this safe?" gets decided, and until now the only way to see them was to
    /// open the file somewhere else.
    ///
    /// A gap is found rather than parsed: two consecutive lines whose new line
    /// numbers are not consecutive have the missing ones between them. That
    /// works without keeping hunk headers around, which this view deliberately
    /// does not — the numbers are already on every row.
    private func body(of f: ChangedFile, lines: [DiffComputation.Line]) -> [DiffRow] {
        var out: [DiffRow] = []
        var gap = 0
        var hunk = 0
        var previous: Int?
        var beginsHunk = true

        for (i, line) in lines.enumerated() {
            if let above = previous, let below = line.newNumber, below > above + 1 {
                let key = "\(f.path)#\(gap)"
                if changes.openGaps.contains(key) {
                    for filler in changes.context(in: f.path, after: above, before: below) {
                        out.append(
                            DiffRow(
                                id: "\(f.path)~\(filler.newNumber ?? 0)",
                                kind: .line(filler), path: f.path))
                    }
                } else {
                    out.append(
                        DiffRow(
                            id: "\(f.path)!gap\(gap)",
                            kind: .gap(f.path, gap, below - above - 1),
                            path: f.path))
                }
                gap += 1
                beginsHunk = true
            }
            if beginsHunk {
                out.append(
                    DiffRow(
                        id: "\(f.path)!hunk\(hunk)",
                        kind: .hunk(hunk + 1, line.oldNumber, line.newNumber),
                        path: f.path))
                hunk += 1
                beginsHunk = false
            }
            // Only a line with a new-side number can bound a gap. A removed line
            // has none, and treating its absence as a jump would open a gap
            // inside a hunk that has none.
            if let n = line.newNumber { previous = n }
            out.append(DiffRow(id: "\(f.path)#\(i)", kind: .line(line), path: f.path))
        }
        return out
    }

    @ViewBuilder
    private func row(_ r: DiffRow) -> some View {
        switch r.kind {
        case .heading(let f):
            fileHeading(f)
                .id(r.id)
                // Read when the heading comes into view, not when the branch
                // loads: forty files would otherwise be forty round trips
                // before anything drew.
                .task(id: f.path) {
                    guard !changes.collapsedFiles.contains(f.path) else { return }
                    await changes.ensure(f.path)
                }
                // Where this heading sits relative to the top of the scroll, so
                // the jump bar can name the file you are INSIDE.
                //
                // Reported by geometry rather than by `onAppear`, which fires in
                // realization order and not reading order: on the first draw
                // every visible heading appeared at once and the last one to do
                // so won, so the bar named a file three screens further down.
                .background(
                    GeometryReader { g in
                        Color.clear.preference(
                            key: HeadingTops.self,
                            value: [f.path: g.frame(in: .named(Self.scrollSpace)).minY])
                    })
        case .note(let text):
            Text(text)
                .font(codeFont)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
        case .line(let line):
            lineRow(line)
        case .gap(let path, let index, let count):
            gapRow(path, index, count)
        case .hunk(let index, let old, let new):
            hunkRow(index: index, old: old, new: new)
        }
    }

    private func hunkRow(index: Int, old: Int?, new: Int?) -> some View {
        HStack(spacing: 0) {
            Color.clear.frame(width: gutter * 2 + 12)
            Text("@@")
                .foregroundStyle(Color.accentColor.opacity(0.72))
            Text("  \(old ?? 0) → \(new ?? 0)")
                .foregroundStyle(.secondary)
            Rectangle()
                .fill(WorkspaceStyle.hairline.opacity(0.72))
                .frame(height: 1)
                .padding(.leading, 10)
                .padding(.trailing, 8)
        }
        .font(.system(size: max(10.5, preferences.fontSize - 1), design: .monospaced))
        .padding(.vertical, 2)
        .accessibilityLabel("Hunk \(index), old line \(old ?? 0), new line \(new ?? 0)")
    }

    /// The control that opens a gap.
    ///
    /// Sized and gutter-aligned like a line of the diff rather than centred,
    /// because it sits in a column of code and anything centred in that column
    /// reads as a separator between files instead of a seam inside one.
    private func gapRow(_ path: String, _ index: Int, _ count: Int) -> some View {
        let refused = changes.tooWide.contains("\(path)#\(index)")
        return DiffGapControl(
            count: count,
            gutter: gutter,
            font: codeFont,
            refused: refused
        ) {
            Task { await changes.open(gap: index, of: count, in: path) }
        }
    }

    /// The line-number gutter, sized from the font rather than pinned, so it
    /// still fits its digits when the terminal is set to 18pt.
    private var gutter: CGFloat { max(26, CGFloat(preferences.fontSize) * 2.2) }

    /// How wide the widest line read so far needs the diff to be.
    ///
    /// Computed, not measured — see `ChangesStore.widestLine`. One advance per
    /// character is exact in a monospaced face, and asking the layout engine
    /// instead is what made a lazy stack build every row and hang the app.
    private var contentWidth: CGFloat {
        let advance = preferences.terminalFont().maximumAdvancement.width
        return gutter * 2 + 12 + CGFloat(changes.widestLine + 2) * advance
    }

    private func fileHeading(_ f: ChangedFile) -> some View {
        HStack(spacing: 8) {
            Button {
                if changes.collapsedFiles.contains(f.path) {
                    changes.collapsedFiles.remove(f.path)
                    Task { await changes.ensure(f.path) }
                } else {
                    changes.collapsedFiles.insert(f.path)
                }
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .rotationEffect(.degrees(changes.collapsedFiles.contains(f.path) ? -90 : 0))
                    .frame(width: 16, height: 18)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help(changes.collapsedFiles.contains(f.path) ? "Expand file" : "Collapse file")

            FileStatusBadge(status: f.status, binary: f.binary)
            Text(pathLabel(f))
                .font(.system(size: 11, weight: .semibold))
                .lineLimit(1)
                .truncationMode(.head)
            Text(counts(f))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            WorkspaceStyle.fileHeader
        )
        .overlay(alignment: .bottom) {
            Rectangle().fill(WorkspaceStyle.hairline).frame(height: 1)
        }
    }

    private func lineRow(_ line: DiffComputation.Line) -> some View {
        HStack(spacing: 0) {
            Rectangle()
                .fill(lineAccent(line.kind))
                .frame(width: 2)
            HStack(spacing: 0) {
                Text(line.oldNumber.map(String.init) ?? "")
                    .frame(width: gutter, alignment: .trailing)
                Text(line.newNumber.map(String.init) ?? "")
                    .frame(width: gutter, alignment: .trailing)
            }
            .foregroundStyle(.tertiary)
            .background(WorkspaceStyle.diffGutter)
            .overlay(alignment: .trailing) {
                Rectangle().fill(WorkspaceStyle.hairline.opacity(0.65)).frame(width: 1)
            }
            Text(marker(line.kind))
                .foregroundStyle(lineAccent(line.kind))
                .frame(width: 12)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 0)
        }
        .font(codeFont)
        .padding(.vertical, 0.5)
        .background(background(line.kind))
    }

    // No syntax highlighting, still. What earns the pixels is which lines
    // changed, and coloring keywords on top of an add/remove background fights
    // the one signal that matters.
    private func marker(_ kind: DiffComputation.Kind) -> String {
        switch kind {
        case .added: return "+"
        case .removed: return "−"
        case .context: return " "
        }
    }

    private func background(_ kind: DiffComputation.Kind) -> Color {
        switch kind {
        case .added: return .green.opacity(0.07)
        case .removed: return .red.opacity(0.07)
        case .context: return .clear
        }
    }

    private func lineAccent(_ kind: DiffComputation.Kind) -> Color {
        switch kind {
        case .added: return .green.opacity(0.62)
        case .removed: return .red.opacity(0.62)
        case .context: return .clear
        }
    }
}

/// A quiet seam inside a file rather than a full-width toolbar stripe.
///
/// The entire row remains clickable, but only the small disclosure gets visual
/// weight until hover. Repeated a dozen times in a lockfile, that difference is
/// the difference between seeing code and seeing chrome.
private struct DiffGapControl: View {
    let count: Int
    let gutter: CGFloat
    let font: Font
    let refused: Bool
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 0) {
                Color.clear.frame(width: gutter * 2 + 12)
                HStack(spacing: 6) {
                    Image(systemName: refused ? "exclamationmark.triangle" : "ellipsis")
                        .font(.system(size: 9, weight: .medium))
                    Text(
                        refused
                            ? "\(count) unchanged lines — too many to show"
                            : (count == 1 ? "1 unchanged line" : "\(count) unchanged lines")
                    )
                    .font(font)
                    if !refused {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 8, weight: .semibold))
                    }
                }
                .foregroundStyle(hovering && !refused ? .secondary : .tertiary)
                .padding(.horizontal, 6)
                .frame(height: 20)
                .background(
                    hovering && !refused ? WorkspaceStyle.disclosureHover : .clear,
                    in: RoundedRectangle(cornerRadius: 4))

                Rectangle()
                    .fill(WorkspaceStyle.hairline.opacity(0.72))
                    .frame(height: 1)
                    .padding(.leading, 8)
                    .padding(.trailing, 8)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(refused)
        .help(refused ? "This gap is too large to show" : "Show unchanged lines")
        .onHover { hovering = $0 }
    }
}

private struct FileStatusBadge: View {
    let status: ChangedFileStatus?
    let binary: Bool

    private var color: Color {
        switch status {
        case .added, .untracked: return .green
        case .deleted, .conflicted: return .red
        case .renamed, .copied: return .orange
        case .modified, .typeChanged, .none: return .secondary
        }
    }

    var body: some View {
        Text(binary ? "B" : (status?.mark ?? "M"))
            .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
            .foregroundStyle(color)
            .frame(width: 12, alignment: .center)
            .help(binary ? "Binary file" : (status?.label ?? "Modified"))
    }
}

/// A pane with nothing to show, that says which nothing it is.
struct PaneNotice: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 4) {
            Text(title).font(.system(size: 12, weight: .medium))
            Text(detail)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(16)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// One line of the diff view: a file's heading, a note about it, or a line of
/// its patch.
///
/// Flat on purpose. A lazy stack only virtualizes what it holds DIRECTLY, so
/// headings and lines have to be siblings — nested, realizing a heading built
/// every line beneath it, and one four-thousand-line file was enough to peg a
/// core and stall the window.
struct DiffRow: Identifiable {
    enum Kind {
        case heading(ChangedFile)
        case note(String)
        case line(DiffComputation.Line)
        case hunk(Int, Int?, Int?)
        /// The unchanged lines a diff left out: which file, which gap, and how
        /// many lines are hiding in it.
        case gap(String, Int, Int)
    }

    let id: String
    let kind: Kind
    /// Which file this row belongs to, so scrolling can say where you are.
    let path: String
}

/// Where each realized file heading sits relative to the top of the diff.
///
/// Only headings currently built report, which is a handful — the lazy stack
/// makes sure of that — so this stays cheap while scrolling.
struct HeadingTops: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]

    static func reduce(
        value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]
    ) {
        value.merge(nextValue()) { _, newer in newer }
    }
}
