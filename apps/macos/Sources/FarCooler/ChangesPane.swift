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

    /// Whether this is the pane the keyboard is aimed at.
    ///
    /// The Diff menu's shortcuts are pane-scoped, exactly as ⌘W and ⌃B z
    /// already are: a window can hold a diff and three terminals, and a key
    /// that moved a diff nobody was looking at would be the layout's most
    /// surprising control. Clicking anywhere in a pane focuses it — see
    /// `TileView.pane(_:rect:group:size:)` — so aiming this is the gesture the
    /// reader already made to start reading.
    let isFocused: Bool

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
    @State private var pickingCommit = false
    @State private var query = ""
    @FocusState private var filtering: Bool
    @State private var commitQuery = ""
    @FocusState private var filteringCommits: Bool
    /// Which commit row Enter will open. See `highlighted`, which is the same
    /// idea for files.
    @State private var highlightedCommit = 0
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
        .onCommand { command in
            guard isFocused else { return }
            switch command {
            case .diffNextHunk: moveHunk(1)
            case .diffPreviousHunk: moveHunk(-1)
            case .diffNextFile: moveFile(1)
            case .diffPreviousFile: moveFile(-1)
            case .diffNextCommit: moveCommit(1)
            case .diffPreviousCommit: moveCommit(-1)
            case .diffFirstCommit: Task { await readCommitByCommit() }
            default: break
            }
        }
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
    ///
    /// The other case says a sentence of its own and puts the CLI’s words in a
    /// `DetailBox` underneath — the shape `DaemonUpdateCard` and
    /// `RunnersSettings` already use for exactly this. `changes.error` is
    /// `farcooler`’s stderr, and stderr set as body text under a heading the
    /// app wrote is the app appearing to say it. Not dropped: an unreachable
    /// runner is diagnosed from those words and from nothing else. Just put
    /// where output goes rather than where prose does.
    private var problem: some View {
        let old = changes.client.changesSupported == false
        return VStack(alignment: .leading, spacing: 3) {
            Text(old ? "This runner can’t show changes yet" : "Couldn’t read this workspace")
                .font(WorkspaceStyle.paneTitle)
            Text(
                old
                    ? "Its copy of Far Cooler is older than this. Update it in Settings › Runners."
                    : "The command that reads it didn’t finish."
            )
            .font(.system(size: WorkspaceStyle.PaneText.body))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            // Only where the app has no diagnosis of its own. The old-runner
            // case names both the cause and the fix, and a transcript under a
            // sentence that already answers the question is noise.
            if !old, let words = changes.error, !words.isEmpty {
                DetailBox(text: words)
            }
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
            // First, because it is the outermost question: which comparison,
            // then which file in it, then which hunk in that. The header's
            // segmented control asks the same question one level up — whole
            // branch or uncommitted — and this is the third answer it has no
            // room for.
            commitPicker(compact: compact)
            commitWalk(compact: compact)
            Divider().frame(height: 14).padding(.horizontal, 2)

            if compact {
                navButton("chevron.up", help: "Previous file") { moveFile(-1) }
                filePicker
                navButton("chevron.down", help: "Next file") { moveFile(1) }
            } else if let file = chosen {
                FileStatusBadge(status: file.status, binary: file.binary)
                Text(pathLabel(file))
                    .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.head)
                Text(counts(file))
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(.secondary)
            } else {
                Text(chosenLabel)
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 4)

            Divider().frame(height: 14).padding(.horizontal, 2)
            let hunks = hunkTargets.count
            if hunks > 0 {
                Text(hunks == 1 ? "1 hunk" : "\(hunks) hunks")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .fixedSize()
            }
            navButton("arrow.up.to.line", help: "Previous hunk or file") { moveHunk(-1) }
            navButton("arrow.down.to.line", help: "Next hunk or file") { moveHunk(1) }
        }
        .padding(.horizontal, 6)
        .frame(height: WorkspaceStyle.paneHeaderHeight)
        .background(WorkspaceStyle.paneChrome.opacity(0.60))
    }

    /// `enabled` overrides the default rule, which is that moving is
    /// meaningless in a pane with no files. Commit movement is the exception it
    /// exists for: a commit that changed nothing is one you walk THROUGH, and a
    /// chevron greyed out on it would be the walk ending at the one commit with
    /// nothing in it to read.
    private func navButton(
        _ symbol: String, help: String, enabled: Bool? = nil,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 9.5, weight: .medium))
                .frame(width: WorkspaceStyle.controlTarget, height: WorkspaceStyle.controlTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .disabled(!(enabled ?? !changes.files.isEmpty))
        .help(help)
    }

    /// Along the branch, one commit at a time.
    ///
    /// Only while a commit is on screen, because that is the only time it means
    /// anything: from Branch or Uncommitted there is no position to move from.
    /// The position label goes when the pane is narrow and the chevrons stay —
    /// a control you can still press is worth more in that space than a number
    /// you can still read, and the tooltips carry the number anyway.
    @ViewBuilder
    private func commitWalk(compact: Bool) -> some View {
        if changes.scope == .commit {
            navButton(
                "chevron.left", help: walkHelp(-1), enabled: changes.previousCommit != nil
            ) { moveCommit(-1) }
            if !compact, let position = changes.commitPositionLabel {
                Text(position)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .fixedSize()
                    .help("This commit’s place in the branch, base forward")
            }
            navButton(
                "chevron.right", help: walkHelp(1), enabled: changes.nextCommit != nil
            ) { moveCommit(1) }
        }
    }

    /// The subject of the commit being moved to, because a chevron that says
    /// "Next commit" answers a question nobody has — the reader knows which
    /// direction they pressed, and wants to know what is over there.
    private func walkHelp(_ direction: Int) -> String {
        let neighbor = direction > 0 ? changes.nextCommit : changes.previousCommit
        guard let neighbor else {
            return direction > 0
                ? "You’re at the newest commit on this branch"
                : "You’re at the first commit on this branch"
        }
        let lead = direction > 0 ? "Next" : "Previous"
        // A commit with an empty subject is possible and the sha is all there
        // is left to name it by.
        let name = neighbor.subject.isEmpty ? neighbor.short : neighbor.subject
        return "\(lead) commit — \(name)"
    }

    /// Start at the base of the branch and go forward.
    ///
    /// The way in the history list is not: a list is the right shape for "which
    /// one of these" and the wrong shape for "start at the beginning and keep
    /// going", which is the reading that makes an agent-authored branch legible
    /// — each commit is one intention, and the third one only makes sense after
    /// the second.
    private func readCommitByCommit() async {
        await changes.startAtFirstCommit()
        guard let file = changes.reviewOrder.first else { return }
        jump(to: file)
    }

    private func moveCommit(_ direction: Int) {
        let neighbor = direction > 0 ? changes.nextCommit : changes.previousCommit
        guard let neighbor else { return }
        // From the top of it, in both directions. A chevron pressed on purpose
        // means "show me that commit", which starts where the commit starts —
        // unlike the hand-off at the end of a file list, which is a journey
        // already in progress.
        Task { await show(commit: neighbor.sha, landingOnLast: false) }
    }

    private func moveFile(_ direction: Int) {
        perform(
            DiffWalk.step(
                direction, hunks: [], after: nil,
                files: changes.reviewOrder.count, at: currentFileIndex,
                boundary: boundary(direction)),
            direction: direction)
    }

    private func moveHunk(_ direction: Int) {
        perform(
            DiffWalk.step(
                direction, hunks: hunkTargets, after: lastHunkJump,
                files: changes.reviewOrder.count, at: currentFileIndex,
                boundary: boundary(direction)),
            direction: direction)
    }

    private var currentFileIndex: Int? {
        changes.reviewOrder.firstIndex { $0.path == changes.selectedFile }
    }

    /// What running out of files in this direction means.
    ///
    /// Branch and Uncommitted wrap, as they always have: each is the whole of
    /// what it can show, so there is nothing past the last file to hand off to.
    /// A commit is a position in a sequence, so the end of its file list is
    /// where the next intention begins — and the end of the LAST commit is the
    /// end of the branch, which is a place to stop rather than a place to loop.
    /// The rule itself is `DiffWalk.boundary`; this is where the store answers
    /// its three questions.
    private func boundary(_ direction: Int) -> DiffBoundary {
        DiffWalk.boundary(
            direction, scope: changes.scope,
            next: changes.nextCommit, previous: changes.previousCommit)
    }

    /// `direction` only decides where a hand-off LANDS: forward into a commit
    /// means its first file, backward means its last, so a reader walking a
    /// branch in either direction keeps traveling the same way.
    private func perform(_ step: DiffStep, direction: Int) {
        switch step {
        case .stay:
            break
        case .hunk(let target):
            jump(toHunk: target)
        case .file(let index):
            let files = changes.reviewOrder
            guard files.indices.contains(index) else { return }
            jump(to: files[index])
        case .commit(let sha):
            Task { await show(commit: sha, landingOnLast: direction < 0) }
        }
    }

    /// Move to another commit and open a file in it, so the hand-off reads as
    /// one continuous motion rather than as arriving somewhere new.
    ///
    /// The jump is what puts the scroll at the top of that file. Without it the
    /// pane keeps whatever offset the previous commit was scrolled to and draws
    /// the new commit from the middle of it.
    private func show(commit sha: String, landingOnLast: Bool) async {
        await changes.select(commit: sha)
        let files = changes.reviewOrder
        guard let file = landingOnLast ? files.last : files.first else { return }
        jump(to: file)
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
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .lineLimit(1)
                    .truncationMode(.head)
                Spacer(minLength: 0)
                if let f = chosen {
                    Text(counts(f))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
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

    /// Which commit, or all of them.
    ///
    /// A popover rather than a menu, and for the same reason the file jumper is
    /// one: a branch with thirty commits on it is a list you READ, and a menu
    /// makes you read it inside a strip of chrome that is one line tall.
    ///
    /// The label carries the sha because that is what a person copies out of
    /// this view and pastes into a terminal, and the subject because a sha is
    /// not something anybody recognizes. At compact widths the subject goes and
    /// the sha stays: eight monospaced characters still identify the commit,
    /// where the first four words of a subject line usually do not.
    private func commitPicker(compact: Bool) -> some View {
        Button { pickingCommit = true } label: {
            HStack(spacing: 5) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.system(size: 10))
                    .foregroundStyle(changes.scope == .commit ? Color.accentColor : .secondary)
                if let sha = changes.selectedCommit {
                    // The sha never shrinks. It is the part of this label that
                    // identifies anything, and half a sha identifies nothing —
                    // where half a subject line still reads.
                    Text(String(sha.prefix(8)))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                        .fixedSize()
                    if !compact, let subject = changes.selectedCommitInfo?.subject {
                        Text(subject)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            // Capped so a commit whose subject runs to eighty
                            // characters cannot push the file name and the hunk
                            // controls off the end of the strip.
                            .frame(maxWidth: 160, alignment: .leading)
                    }
                } else {
                    Text("History")
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .fixedSize()
                }
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 8))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(commitPickerHelp)
        .popover(isPresented: $pickingCommit, arrowEdge: .bottom) { commitList }
        .onChange(of: pickingCommit) { _, open in
            if open {
                // Opens ON the commit being read, so ↑↓ starts where the reader
                // is rather than at the newest commit they did not ask about.
                highlightedCommit =
                    changes.commitsNewestFirst.firstIndex { $0.sha == changes.selectedCommit } ?? 0
            } else {
                // Cleared on DISMISS for the reason the file filter gives:
                // clearing it on a successful pick leaves Escape with a filter
                // still set, silently hiding commits the next time it opens.
                commitQuery = ""
                highlightedCommit = 0
            }
        }
    }

    /// Three states and three sentences. The middle one is the case a rebase or
    /// an amend creates while somebody is reading: the commit is no longer part
    /// of the branch, its diff is still perfectly readable because the object it
    /// names is still in the repository, and saying nothing about that would
    /// leave the pane quietly describing a commit the branch has forgotten.
    private var commitPickerHelp: String {
        guard changes.scope == .commit else { return "Look at one commit" }
        if changes.selectedCommitInfo == nil {
            return "This commit isn’t on the branch anymore. It’s still readable."
        }
        return "One commit, against its first parent"
    }

    private var commitList: some View {
        // Read once, here, so every row in this list ages against the same
        // instant — and read at all only because the popover is opening. See
        // `ChangeCommit.age(at:)`.
        let now = Date()
        let commits = changes.commitsNewestFirst
        let shown = matchingCommits
        return VStack(alignment: .leading, spacing: 0) {
            Button {
                changes.showWholeBranch()
                pickingCommit = false
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "checkmark")
                        .font(.system(size: 9, weight: .semibold))
                        .opacity(changes.scope == .commit ? 0 : 1)
                        .frame(width: 11)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Whole Branch")
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        Text("Every commit at once")
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.tertiary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(5)

            // The other way to read a branch, and the one that has no row of
            // its own to click: a list is reached for with one commit in mind,
            // and this is for the reader who has none and wants the story.
            // Greyed out rather than hidden on a branch with no commits, so the
            // idea is still visible on the branch where it does not yet apply.
            Button {
                pickingCommit = false
                Task { await readCommitByCommit() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "text.line.first.and.arrowtriangle.forward")
                        .font(.system(size: 9, weight: .semibold))
                        .frame(width: 11)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Read Commit by Commit")
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        Text("Start at the first commit; ⌃⌘] for the next")
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.tertiary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(commits.isEmpty)
            .padding(.horizontal, 5)
            .padding(.bottom, 5)

            Divider()

            if commits.isEmpty {
                // Not a failure, and it must not read as one. A branch cut a
                // minute ago has no commits of its own and can still be full of
                // uncommitted work, which the two segments in the header show.
                VStack(alignment: .leading, spacing: 2) {
                    Text("No commits yet")
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                    Text(noCommitsDetail)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(9)
            } else {
                commitFilterField
                Divider()
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 1) {
                            if shown.isEmpty {
                                Text("No commit matches “\(commitQuery)”")
                                    .font(.system(size: WorkspaceStyle.PaneText.body))
                                    .foregroundStyle(.secondary)
                                    .padding(8)
                            }
                            ForEach(Array(shown.enumerated()), id: \.element.id) { i, c in
                                commitRow(c, now: now, highlighted: i == highlightedCommit)
                                    .id(c.sha)
                            }
                        }
                        .padding(5)
                    }
                    // A branch is allowed to be long. Capped and scrolled, a
                    // hundred commits is a list; uncapped, it is a popover taller
                    // than the display it opened on.
                    .frame(maxHeight: 280)
                    .onChange(of: highlightedCommit) { _, i in
                        guard shown.indices.contains(i) else { return }
                        proxy.scrollTo(shown[i].sha, anchor: .center)
                    }
                }
            }

            Divider()

            // Said once, here, rather than worked out per commit — the client
            // cannot tell a merge from an ordinary commit anyway, since the
            // change set carries no parent counts. It is true of every row: the
            // daemon diffs a commit against its FIRST parent, never with `git
            // show`, whose combined diff for a merge an ordinary parser reads
            // as nonsense. For a merge that means what the merged branch
            // brought, which is not everything the merge contains.
            Text("A commit is shown against its first parent.")
                .font(.system(size: WorkspaceStyle.PaneText.minimum))
                .foregroundStyle(.tertiary)
                .padding(.horizontal, 9)
                .padding(.vertical, 6)
        }
        .frame(width: 380)
        // Arrows move the highlight; the field keeps the keystrokes it wants.
        // The same three-part deal the file filter makes — see `filterList`.
        .onMoveCommand { direction in
            switch direction {
            case .up: highlightedCommit = max(0, highlightedCommit - 1)
            case .down:
                highlightedCommit = min(max(0, shown.count - 1), highlightedCommit + 1)
            default: break
            }
        }
    }

    /// Type the part you remember.
    ///
    /// Over the BODY as well as the subject, which is the half that makes this
    /// worth having: the phone's experience of the same list is that the
    /// remembered word — the approach that was tried, the thing that was
    /// decided against — is usually in the rationale an agent wrote underneath,
    /// not in the seven words it put on the first line. The sha and the author
    /// are in too, because `a3f1` and a name are both things people paste.
    private var commitFilterField: some View {
        TextField("Filter commits", text: $commitQuery)
            .textFieldStyle(.roundedBorder)
            .font(.system(size: WorkspaceStyle.PaneText.body))
            .padding(7)
            .focused($filteringCommits)
            // A popover does not focus a field on macOS, and a list you have to
            // click into before typing is just a menu.
            .onAppear { filteringCommits = true }
            .onChange(of: commitQuery) { _, _ in highlightedCommit = 0 }
            .onSubmit { openHighlightedCommit() }
    }

    /// Every commit whose sha, subject, body or author contains what was typed.
    private var matchingCommits: [ChangeCommit] {
        let q = commitQuery.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return changes.commitsNewestFirst }
        return changes.commitsNewestFirst.filter { c in
            [c.sha, c.subject, c.body ?? "", c.author].contains {
                $0.range(of: q, options: .caseInsensitive) != nil
            }
        }
    }

    private func openHighlightedCommit() {
        let shown = matchingCommits
        guard shown.indices.contains(highlightedCommit) else { return }
        let sha = shown[highlightedCommit].sha
        pickingCommit = false
        Task { await changes.select(commit: sha) }
    }

    private var noCommitsDetail: String {
        let base = changes.changeSet.baseRef
        return base.isEmpty
            ? "There’s nothing to compare this branch against yet."
            : "Nothing has been committed here since \(base)."
    }

    /// One commit: what it is called, why, and how big it was.
    ///
    /// The body preview is the row's second line and the reason this list is
    /// worth reading rather than scanning. An agent's commit body is usually
    /// the closest thing to a written rationale for what it did, and it costs
    /// nothing to fetch — it travels in the change set the pane has already
    /// read.
    ///
    /// Two lines of it, not the whole thing. Four commits with eight lines each
    /// is not a list any more; the tooltip carries the rest, which is the Mac
    /// having somewhere to put it that a phone does not.
    private func commitRow(_ c: ChangeCommit, now: Date, highlighted: Bool) -> some View {
        Button {
            Task { await changes.select(commit: c.sha) }
            pickingCommit = false
        } label: {
            HStack(alignment: .top, spacing: 7) {
                Text(c.short)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, design: .monospaced))
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(c.subject)
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 4)
                        // Absent rather than `+0 −0` when the runner could not
                        // count — see `ChangeCommit.counts`.
                        if let counts = c.counts {
                            Text(
                                DiffCounts.pair(
                                    insertions: counts.insertions, deletions: counts.deletions)
                            )
                            .font(
                                .system(size: WorkspaceStyle.PaneText.minimum, design: .monospaced)
                            )
                            .foregroundStyle(.secondary)
                            .fixedSize()
                        }
                    }
                    if let preview = c.bodyPreview {
                        Text(preview)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(byline(c, now: now))
                        .font(.system(size: WorkspaceStyle.PaneText.minimum))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 7)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            // Two different claims, so two different colors, and the keyboard's
            // wins where they land on the same row: the accent is a promise
            // about what Enter will do, and the navigator tint is a statement
            // about what is already on screen.
            .background(
                highlighted
                    ? AnyShapeStyle(Color.accentColor.opacity(0.18))
                    : (changes.selectedCommit == c.sha
                        ? AnyShapeStyle(WorkspaceStyle.navigatorSelection)
                        : AnyShapeStyle(Color.clear)),
                in: RoundedRectangle(cornerRadius: 4))
        }
        .buttonStyle(.plain)
        .help(rowHelp(c))
    }

    /// When it was made, and everything the author wrote — trailers and all.
    ///
    /// The preview above skips trailer-only paragraphs because it has one line
    /// to spend. This has no such budget, and on the screen that is ABOUT one
    /// commit a `Co-Authored-By:` line is part of what the commit says.
    private func rowHelp(_ c: ChangeCommit) -> String {
        guard let body = c.bodyText else { return c.made }
        return "\(c.made)\n\n\(body)"
    }

    /// Author, age and how many files, on one line. Each part is dropped rather
    /// than left as a gap where a fact should be: a commit with no author name
    /// is not worth a separator, and a file count of zero means the runner
    /// could not tell us — the same rule `ChangeCommit.counts` keeps.
    private func byline(_ c: ChangeCommit, now: Date) -> String {
        var parts: [String] = []
        if !c.author.isEmpty { parts.append(c.author) }
        parts.append(c.age(at: now))
        if let files = c.filesChanged, files > 0 {
            parts.append(files == 1 ? "1 file" : "\(files) files")
        }
        return parts.joined(separator: " · ")
    }

    private var filterList: some View {
        VStack(spacing: 0) {
            TextField("Filter files", text: $query)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: WorkspaceStyle.PaneText.body))
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
                            .font(.system(size: WorkspaceStyle.PaneText.body))
                            .foregroundStyle(.secondary)
                            .padding(8)
                        }
                        ForEach(Array(matches.enumerated()), id: \.element.id) { i, f in
                            Button { jump(to: f) } label: {
                                HStack(spacing: 6) {
                                    Text(pathLabel(f))
                                        .font(.system(size: WorkspaceStyle.PaneText.body))
                                        .lineLimit(1)
                                        .truncationMode(.head)
                                    Spacer(minLength: 4)
                                    Text(counts(f))
                                        .font(.system(size: WorkspaceStyle.PaneText.body, design: .monospaced))
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
        guard !q.isEmpty else { return changes.reviewOrder }
        return changes.reviewOrder.filter {
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
        // Through `DiffCounts`, so the minus is the same character and the
        // separators are the same rule as the sidebar's pair — the two are on
        // screen together and were spelled differently.
        if f.insertions > 0 { parts.append(DiffCounts.added(f.insertions)) }
        if f.deletions > 0 { parts.append(DiffCounts.removed(f.deletions)) }
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
                    .font(.system(size: WorkspaceStyle.PaneText.body))
                Text("\(matches.count)")
                    .font(.system(size: WorkspaceStyle.PaneText.minimum, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
            }
            .padding(.horizontal, 7)
            .frame(height: 22)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(Color.primary.opacity(0.045)))
            .padding(.horizontal, 6)
            .frame(height: WorkspaceStyle.paneHeaderHeight)
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
                                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                                            .lineLimit(1)
                                            .truncationMode(.middle)
                                        Spacer(minLength: 3)
                                        Text(counts(f))
                                            .font(.system(size: WorkspaceStyle.PaneText.minimum, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                            .fixedSize()
                                    }
                                    Text(parentPathLabel(f) ?? " ")
                                        .font(.system(size: WorkspaceStyle.PaneText.minimum))
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

    /// Empty means several different things, and saying the wrong one is a
    /// wrong answer: a branch that matches its base is finished work, a
    /// worktree with nothing uncommitted is work that has been committed, and a
    /// commit that could not be read is not an empty commit at all.
    private var nothingTitle: String {
        changes.commitUnreadable ? "Couldn’t read this commit" : "Nothing changed here"
    }

    private var nothingDetail: String {
        if changes.commitUnreadable {
            // The case that puts this here: an amend or a rebase during a
            // review rewrites the branch, and the sha in hand stops being one
            // this worktree can answer for. Never the daemon's own words —
            // they are about a subprocess, and this is about a commit.
            return "It might not be on this branch anymore. Choose another, or go back to the whole branch."
        }
        switch changes.scope {
        case .local:
            return "Everything here is committed."
        case .commit:
            // Against its FIRST parent, which is the only wording that stays
            // true for a merge: a merge whose branch brought nothing new is
            // empty by this comparison and very far from empty by any other.
            return "This commit changed nothing against its first parent."
        case .branch:
            return changes.changeSet.baseRef.isEmpty
                ? "There’s nothing to compare this branch against yet."
                : "This branch matches \(changes.changeSet.baseRef)."
        }
    }

    @ViewBuilder
    private var diffBody: some View {
        if changes.error != nil {
            // The banner above already said what went wrong, and "Nothing
            // changed here" underneath it would contradict it.
            Color.clear
        } else if changes.files.isEmpty && changes.loading {
            // A read in progress is not an answer. Without this the pane
            // asserted "Nothing changed here" for the length of every round
            // trip — on first arrival, and again each time a commit was
            // chosen — which is a sentence about the worktree, not about the
            // wait.
            Color.clear
        } else if changes.files.isEmpty {
            PaneNotice(title: nothingTitle, detail: nothingDetail)
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
        // The reading order, not the daemon's, and the same one `moveFile`
        // walks — see `ChangesStore.reviewOrder`. Today they are the same list.
        let files = changes.reviewOrder
        out.reserveCapacity(files.count * 8)
        for f in files {
            out.append(DiffRow(id: f.path, kind: .heading(f), path: f.path))
            if changes.collapsedFiles.contains(f.path) { continue }
            if f.binary {
                out.append(note(f, "Binary file — nothing to show"))
            } else if changes.isUntracked(f.path) {
                // Listed but not diffed, and it says so. git has nothing to
                // compare a brand new file against, and "No textual changes"
                // under the name of a file somebody just wrote is the most
                // wrong thing this view could say.
                out.append(note(f, "New file — git isn’t tracking it yet"))
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
                //
                // Keyed on the generation as well as the path, because the path
                // alone does not change when the COMPARISON does. A heading
                // that stays on screen across a switch from one commit to
                // another keeps its identity, so this task never ran again
                // while the diff under it had just been thrown away — and the
                // file sat at `…` until it was scrolled out of view and back.
                .task(id: "\(changes.generation)\u{1}\(f.path)") {
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
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                .lineLimit(1)
                .truncationMode(.head)
            Text(counts(f))
                .font(.system(size: WorkspaceStyle.PaneText.body, design: .monospaced))
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
                .fill(line.kind.accent)
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
            Text(line.kind.marker)
                .foregroundStyle(line.kind.accent)
                .frame(width: 12)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 0)
        }
        .font(codeFont)
        .padding(.vertical, 0.5)
        // No syntax highlighting, still. What earns the pixels is which lines
        // changed, and coloring keywords on top of an add/remove background
        // fights the one signal that matters.
        //
        // The marker, the wash and the stripe were three private functions
        // here and a second, disagreeing copy of two of them in `DiffView`.
        // One table now, on the kind itself — see the `DiffComputation.Kind`
        // extension in `DiffView.swift`.
        .background(line.kind.wash)
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

    /// Inside a diff surface green and red already mean added and removed
    /// lines, so a created or deleted FILE wearing them reads without being
    /// learned. Orange is spoken for: everywhere else in this app it means
    /// something wants you — a blocked agent, an unreachable runner, an adapter
    /// that will not load — and a conflict is exactly that, the one row where
    /// nothing moves until a human resolves it. A rename is not an alarm, so it
    /// is gray like a modification, and the letter is what tells them apart.
    ///
    /// Conflicted was red here, which is the other honest reading: an error
    /// state, and red is what `StatusGlyph` gives a turn that died or a runner
    /// that is lost. It loses inside this pane, where red is the diff's own —
    /// `background` and `lineAccent` a hundred lines up paint removed lines
    /// with it — so a conflict wore the color of a deletion and only the letter
    /// said otherwise. Renamed and copied were orange here until commit rows
    /// started carrying them, which is when a moved file first read as a
    /// warning; the phone had them gray all along.
    private var color: Color {
        switch status {
        case .added, .untracked: return .green
        case .deleted: return .red
        case .conflicted: return .orange
        case .modified, .renamed, .copied, .typeChanged, .none: return .secondary
        }
    }

    /// No status is its own state, and it is now rare rather than usual.
    ///
    /// Every scope carries a real letter: the daemon determines them from git
    /// for all three — `change_set::numstat` for a branch, porcelain codes for
    /// the working tree, and `--name-status` for one commit — and `changes
    /// files --json` carries a commit's letter the last hop to this pane. What
    /// is left is a runner whose `farcooler` predates that flag, for which
    /// `ChangesStore.readCommitFiles` falls back to the human table and gets
    /// counts and a path. A dot says the file changed, which is the whole
    /// of what such a runner said; `M` would be a guess reading as a verdict.
    var body: some View {
        Text(binary ? "B" : (status?.mark ?? "•"))
            .font(.system(size: WorkspaceStyle.PaneText.minimum, weight: .semibold, design: .monospaced))
            .foregroundStyle(color)
            .frame(width: 12, alignment: .center)
            .help(binary ? "Binary file" : (status?.label ?? "Changed"))
    }
}

/// A pane with nothing to show, that says which nothing it is.
struct PaneNotice: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 4) {
            Text(title).font(WorkspaceStyle.paneTitle)
            Text(detail)
                .font(.system(size: WorkspaceStyle.PaneText.body))
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
