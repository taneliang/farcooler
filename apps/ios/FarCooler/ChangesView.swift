import SwiftUI

/// What the branch header knows about this branch's pull request.
///
/// Three facts rather than a bare `PullRequest?`, because the ABSENCE of one is
/// two different situations and only one of them may be acted on: `gh` said
/// there is no pull request, or `gh` could not be asked at all. See
/// `StackResponse.prKnown`, which is the only thing on the wire that separates
/// them, and `ChangesView.pullRequestRow`, which is where it decides what is
/// drawn.
///
/// A plain value, assembled by `WorkspaceView` from one `stack.get`. Everything
/// this row needs and nothing that would pull a `Connection` into a lazy stack.
struct BranchPullRequest: Equatable {
    /// GitHub's answer for this branch, or nil when there is none to have.
    var pr: PullRequest?
    /// Whether `gh` answered at all — the gate on offering to create one.
    var known: Bool
    /// The repository's page on GitHub, for the compare link. Nil when `gh`
    /// has not said what this repository is, and then there is no offer to
    /// make: a button that opens a broken page is worse than no button.
    var repoURL: String?
}

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
///
/// ## What this screen is shaped around
///
/// Between sets. One hand. Ninety seconds at a time, a dozen times an hour,
/// with iOS very likely terminating the app in between — and the thing being
/// read is a branch an agent wrote overnight, forty files across a dozen
/// commits. Every decision below follows from that: the controls that move you
/// through a diff are at the BOTTOM, because that is where a thumb is; exactly
/// one file is open at a time, because that is what makes "next" mean
/// something; where you were is written to disk on every move, because the
/// process will not be alive to be asked; and the output of reading — a note
/// for the agent — is collected here rather than requiring a trip to another
/// screen to type.
///
/// ## Where the accent goes on this screen
///
/// Reading a diff is almost entirely movement — next file, next commit, back to
/// the whole branch, open the body, open the index — and every one of those was
/// a `.bordered` button, which paints its label in the tint. So a commit card
/// carried four accent controls at once (two chevrons, Whole Branch, History)
/// with a fifth, More, right under them, none of which is what anybody came
/// here to do. That is the "blue text buttons on dark backgrounds" report, and
/// it is the same failure `FleetList` names: color spent on everything says
/// nothing about anything.
///
/// The rule here is narrower than that file's, because this screen's shape
/// allows it. **Moving through a diff is grey; producing something is blue.**
/// Movement is found by position — the chevrons live in fixed corners, the bar
/// is always at the bottom — and it repeats on every commit and every file.
/// What does not repeat is the one thing reading a diff is FOR: a note for the
/// agent. Exactly one file is open at a time, so "Comment on This File" is on
/// screen once, and it keeps the accent. Continue, on the resume card, keeps a
/// filled accent for the same reason `FleetList` gives about a control that
/// appears once — it is a one-time offer with one obvious answer.
///
/// Grey here means `.tint(.secondary)` with `.foregroundStyle(.primary)` over
/// it: the tint is the part that reliably reaches a system button style, and
/// the foreground style is what keeps the label at full label contrast instead
/// of the 60%-of-grey a greyed tint would leave. Losing the second one fails
/// quiet; losing the first fails back to blue, which is why both are said.
///
/// What this screen never records is a JUDGMENT. There is no "reviewed" tick on
/// a file, and that is deliberate: an agent is still editing these files, so a
/// tick on a file that has changed twice since would be a claim the app is in
/// no position to make. The daemon's workspace-level `changed_since_reviewed`
/// watermark is the piece of review state that survives an edit, because an
/// edit is what invalidates it. See `ReviewPosition`.
@MainActor
struct ChangesView: View {
    /// Owned by `Connection`, not by this view — the view is destroyed on every
    /// tab switch and the scroll, the folds and the fetched diffs must not be.
    @ObservedObject var store: ChangesStore
    let workspaceName: String

    /// The agent panes in this worktree a review note can be sent to.
    ///
    /// A plain array of values rather than the `Connection` it is derived from.
    /// Holding the connection here would re-evaluate this view's body on every
    /// three-second fleet poll, and this is the one screen in the app whose
    /// body is a forty-card lazy stack somebody is mid-scroll through.
    var agents: [ReviewAgentTarget] = []

    /// What GitHub says about this branch, and the way in to act on it.
    ///
    /// Resolved by `WorkspaceView` — which holds `currentWorkspace` and
    /// therefore the repository id `stack.get` takes — and handed down as a
    /// plain value for exactly the reason `agents` is one, above.
    ///
    /// Nil until the first read answers, and nil draws nothing. A row that
    /// appears a moment later is better than a placeholder somebody has to
    /// read and then read again.
    var pullRequest: BranchPullRequest?

    @State private var showingIndex = false
    @State private var showingComments = false
    @State private var composing: ComposeRequest?

    /// A composer that has been asked for, and what it is about.
    ///
    /// A wrapper with its own identity rather than presenting on the anchor
    /// itself: two notes written about the same hunk are two separate sheets,
    /// and `sheet(item:)` keyed on the anchor would refuse the second one
    /// because it compares equal to the first.
    private struct ComposeRequest: Identifiable {
        let id = UUID()
        let anchor: ReviewAnchor
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                    // Everything above the files, laid out the way the whole
                    // stack used to be.
                    //
                    // The stack itself now says `spacing: 0`, because a file is
                    // a `Section` — a pinned heading and its hunks are two
                    // elements of this stack, and any spacing between them
                    // would open a seam across the middle of every card. So the
                    // gap between cards moved into the heading, where it can be
                    // given a ground of its own; see `ChangesFileHeading`. This
                    // block keeps the spacing it always had.
                    VStack(alignment: .leading, spacing: 10) {
                        summary
                            .id(Self.topAnchor)

                        if let saved = store.resume {
                            resumeCard(saved)
                        }
                        if let note = store.resumeNote {
                            // Tap to dismiss. It has said its piece the moment it
                            // is read, and a sentence about a file that moved has
                            // no business still being on the screen three commits
                            // later.
                            ChangesNotice(symbol: "bookmark", tint: .secondary, text: note)
                                .onTapGesture { store.resumeNote = nil }
                        }

                        if let error = store.error {
                            ChangesNotice(
                                symbol: "exclamationmark.triangle", tint: .orange,
                                text: error.sentence, detail: error.transcript)
                        } else if store.commitUnreadable {
                            // Ahead of the empty case on purpose: a commit that could
                            // not be read also has no files, and "nothing changed here"
                            // is the one sentence that must not be said about it.
                            //
                            // The Mac's words for this exact failure, from
                            // `ChangesPane.nothingTitle` and `nothingDetail`, rather
                            // than a second spelling of them — and both controls it
                            // names are on this screen, in the row under a commit's
                            // subject. No transcript here, and that is the Mac's
                            // reasoning too: what `changes.commit_files` returns is
                            // about a subprocess, and this is about a commit.
                            ChangesNotice(
                                symbol: "exclamationmark.triangle",
                                tint: .orange,
                                text: "Couldn’t read this commit. It might not be on this branch "
                                    + "anymore. Choose another, or go back to the whole branch.")
                        } else if store.files.isEmpty && !store.loading {
                            ChangesNotice(
                                symbol: "checkmark.circle", tint: .secondary, text: nothingHere)
                        }
                    }

                    fileCards
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
            // No scroll restore WITHIN a run of the app, and that is still the
            // point.
            //
            // There used to be one here: an offset kept in the store, put back
            // on appear, asked for repeatedly because a lazy stack clamps the
            // first request to the one screenful that exists. It half-worked —
            // the offset was recorded before the restore had had its turn, so a
            // second or third visit jumped — and all of it existed only because
            // the view was destroyed on every pane switch. `WorkspaceView` keeps the
            // pane mounted, so the scroll never moves and there is nothing to
            // put back.
            //
            // What DOES get put back is a position across a process death,
            // which is a different problem with a different answer: a file
            // path, offered rather than applied. See `ReviewPosition` for why
            // an anchor and not an offset, and `resumeCard` for why it asks.
            .background(TerminalPalette.background)
            // Pull to refresh asks the daemon to recompute rather than answer from
            // its cache: it is the affordance that exists because no watcher is
            // perfect, and the user must always have a way to be certain.
            .refreshable { await store.load(fresh: true) }
            .task { await store.loadIfNeeded() }
            // Every jump on this screen goes through one channel, so there is
            // one place that can be wrong about scrolling: the index sheet, the
            // Next and Previous buttons, and the resumed bookmark all set
            // `store.jump` and this moves the scroll. Cleared immediately after,
            // so nothing re-scrolls under somebody who has since scrolled away.
            .onChange(of: store.jump) { _, jump in
                guard let jump else { return }
                withAnimation(.easeInOut(duration: 0.2)) {
                    proxy.scrollTo(jump.path, anchor: .top)
                }
                store.jump = nil
            }
            // At the bottom, in thumb reach, and never in the navigation bar.
            //
            // The bar is the whole of "moving through a large diff" and it has
            // to be reachable by the hand already holding the phone — the top
            // of a modern iPhone is not, one-handed, and a control you have to
            // shuffle your grip to press is a control that does not get pressed
            // between sets. It also stays put while the diff scrolls behind it,
            // so "how far in am I" is answerable without scrolling anywhere.
            .safeAreaInset(edge: .bottom, spacing: 0) {
                ReviewBar(
                    store: store,
                    comments: store.comments,
                    onIndex: { showingIndex = true },
                    onComments: { showingComments = true })
            }
            // A sheet, not a push. Choosing a commit is a detour off the thing on
            // screen and it ends by coming straight back to it — the same shape as
            // the worktree switcher and `BranchPicker`, which is what this app
            // already uses for a choice of this weight. A push would put the diff
            // behind a Back button and make the branch feel like somewhere you had
            // left.
            //
            // It is no longer the ONLY way through the commits, which is the
            // change that matters: a sheet is right for "which one of these" and
            // wrong for "start at the beginning and keep going". The second
            // reading now has controls of its own — see `commitHeader` and
            // `ChangesStore.showNextCommit` — and this stayed as what it always
            // was, a picker.
            //
            // Presented from here rather than from either control that opens it:
            // one of those controls lives in `WorkspaceView`'s toolbar, and a sheet
            // hung off a `Menu`'s content is hung off something that is not a live
            // view hierarchy to present from. The flag lives on the store so both
            // can reach it.
            .sheet(isPresented: $store.showingHistory) {
                CommitHistorySheet(store: store)
            }
            .sheet(isPresented: $showingIndex) {
                FileIndexSheet(store: store)
            }
            .sheet(item: $composing) { request in
                CommentComposer(anchor: request.anchor, comments: store.comments)
            }
            .sheet(isPresented: $showingComments) {
                CommentOutboxSheet(
                    comments: store.comments,
                    agents: agents,
                    branch: store.changeSet.branch)
            }
        }
    }

    /// The id of the summary card, so landing "at the top" has somewhere to go.
    private static let topAnchor = ChangesStore.topAnchor

    /// The files, with whatever a tool generated held to the end.
    ///
    /// Two loops rather than one, because the generated files get a heading of
    /// their own — see `ChangedFile.isGenerated` for why they are separated at
    /// all. The order here is `ChangesStore.reviewOrder`, which is also the
    /// order Next and Previous walk, so `7 of 23` counts the same list the
    /// reader is looking at.
    @ViewBuilder
    private var fileCards: some View {
        ForEach(store.handWrittenFiles) { file in
            card(file)
        }
        if !store.generatedFiles.isEmpty {
            generatedHeading
            ForEach(store.generatedFiles) { file in
                card(file)
            }
        }
    }

    /// One file: its heading, which stays, and its hunks, which scroll.
    ///
    /// A `Section` written out here rather than returned from a view of its
    /// own, because that is what makes the heading stick: `pinnedViews` is
    /// resolved from the sections the lazy stack's own content produces, and a
    /// custom `View` that happens to return a `Section` is not documented to be
    /// looked inside. A heading that quietly stopped pinning would look exactly
    /// like this having never been built, so the section stays where the stack
    /// can see it.
    private func card(_ file: ChangedFile) -> some View {
        Section {
            ChangesFileBody(
                file: file,
                store: store,
                onComment: { composing = ComposeRequest(anchor: $0) })
        } header: {
            ChangesFileHeading(
                file: file,
                store: store,
                onVisible: { store.noteVisible(file.path, isVisible: $0) }
            )
            // The scroll target. Every jump names a path, and the heading is
            // where a jump belongs now: it is the part the pin holds at the top
            // of the screen, so scrolling it to the top lands the file exactly
            // where it is about to stay.
            .id(file.path)
        }
    }

    private var generatedHeading: some View {
        HStack(spacing: 8) {
            Image(systemName: "wrench.and.screwdriver")
                .font(.caption2)
                .foregroundStyle(.tertiary)
            Text(
                store.generatedFiles.count == 1
                    ? "1 Generated File" : "\(store.generatedFiles.count) Generated Files"
            )
            .font(.footnote.weight(.medium))
            .foregroundStyle(.secondary)
            Spacer(minLength: 4)
            Text("\(store.generatedLineCount) lines")
                .font(.caption2.monospaced())
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 4)
        // Eighteen rather than eight: the stack no longer adds ten of its own
        // between this and the card above it. The ten below it comes from the
        // next heading, which carries its own gap.
        .padding(.top, 18)
    }

    /// Which nothing this is, when the list is empty.
    private var nothingHere: String {
        switch store.scope {
        case .branch: return "This branch hasn’t committed anything yet."
        case .local: return "Nothing uncommitted. The workspace is clean."
        case .commit:
            // Not "this commit is empty". A commit is compared against its
            // FIRST parent here — `Selector::Commit` in the daemon's
            // file_diff.rs — and a merge that only joined two branches
            // genuinely changed nothing against that side while changing
            // plenty against the other. Naming the comparison is the
            // difference between a fact and a claim this screen cannot make.
            return "Nothing changed against this commit’s first parent, "
                + "which is also what a clean merge looks like."
        }
    }

    // MARK: - Coming back

    /// The offer to go back where the last window ended.
    ///
    /// An offer and not a jump. The app was almost certainly terminated between
    /// the two windows, and in the meantime the agent has probably kept
    /// working: restoring silently would drop somebody who opened this pane to
    /// glance at one thing into the middle of a patch, and — worse — into a
    /// diff that has changed shape underneath the position being restored. So
    /// it says where it thinks they were, in the words of the branch rather
    /// than in path-and-sha, and waits to be asked.
    private func resumeCard(_ saved: ReviewPosition) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Continue where you stopped", systemImage: "bookmark")
                .font(.footnote.weight(.semibold))
            Text(resumeDescription(saved))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                Button("Continue") {
                    Task { await store.applyResume() }
                }
                .buttonStyle(.borderedProminent)
                Spacer(minLength: 4)
                // Grey beside a filled Continue. `.bordered` paints its label
                // in the tint, so declining an offer was accent text an inch
                // from an accent fill — two blues for a yes/no. See the accent
                // rule in `ChangesView`'s own doc comment.
                Button("Not Now") { store.dismissResume() }
                    .buttonStyle(.bordered)
                    .tint(.secondary)
                    .foregroundStyle(.primary)
            }
            .font(.footnote)
            .controlSize(.small)
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 12))
    }

    /// Where the bookmark says you were, said the way the branch says it.
    ///
    /// A subject rather than a sha wherever one is known: "you were reading
    /// `push.ts` in *handle retries on 429*" is a place somebody recognizes,
    /// and `local/a1b2c3d4` is a place they have to decode.
    private func resumeDescription(_ saved: ReviewPosition) -> String {
        var place: String
        if let sha = ReviewPosition.sha(in: saved.scope) {
            let known = store.changeSet.commits.first { $0.sha == sha }
            if let known, !known.subject.isEmpty {
                place = "in “\(known.subject)”"
            } else {
                place = "in commit \(sha.prefix(8))"
            }
        } else if saved.scope == "local" {
            place = "in the uncommitted work"
        } else {
            place = "on the whole branch"
        }
        if let file = saved.file ?? saved.topFile {
            return "You were at \((file as NSString).lastPathComponent), \(place)."
        }
        return "You were \(place)."
    }

    // MARK: - The summary card

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
                comparisonHeader
            }
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 14))
    }

    /// What is being compared, its counts, the control, and the ways into the
    /// commits.
    ///
    /// The top line follows the segment, and that is the whole of this. It used
    /// to be the branch's line whichever segment was on: `vs main` and the
    /// branch's committed totals sat above a list of uncommitted files, which
    /// is precisely the failure the card's own comment above warns of for a
    /// commit — a header describing something nobody is looking at. And "vs
    /// main" was not merely stale under Uncommitted, it was the wrong
    /// comparison: uncommitted work is what the worktree has that HEAD does
    /// not, and the base does not come into it.
    ///
    /// The Mac branches on scope for the counts, in `TileView.changeCount`.
    /// This does the same, and — on a screen that draws the base where the
    /// Mac's strip does not — for the words as well.
    @ViewBuilder
    private var comparisonHeader: some View {
        // Committed work against the base, or everything uncommitted against
        // HEAD. `uncommittedInsertions` reads the working tree the daemon sent
        // rather than the diffs on screen; see its own comment for why that
        // distinction is the difference between a total and a running one.
        let local = store.scope == .local
        let total =
            local
            ? (store.uncommittedInsertions, store.uncommittedDeletions)
            : (store.changeSet.insertions, store.changeSet.deletions)
        // The comparison's own totals while nothing was generated, and the
        // hand-written subtotal once something was. See `generatedNote`.
        let shown =
            store.generatedFiles.isEmpty
            ? total
            : (store.writtenInsertions, store.writtenDeletions)

        HStack(spacing: 10) {
            if local {
                // HEAD rather than the branch name, because that is the
                // comparison: `git diff HEAD`, staged and unstaged together,
                // plus what git is not tracking yet. Git's own word, because
                // it is the exact one and the reader of a diff screen knows
                // it — and because the alternative reads as a base branch,
                // which is the thing this line stopped saying.
                Text("vs HEAD")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            } else if !store.changeSet.baseRef.isEmpty {
                Text("vs \(store.changeSet.baseRef)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            Text("+\(shown.0)")
                .font(.caption.monospaced()).foregroundStyle(.green)
            Text("−\(shown.1)")
                .font(.caption.monospaced()).foregroundStyle(.red)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(spokenComparison(insertions: shown.0, deletions: shown.1))

        generatedNote

        // Below the comparison and above the warnings, because it is the same
        // KIND of fact as the line above it — something true of this branch as
        // a whole — and because the first thing anyone wants to know about a
        // branch an agent wrote overnight is whether it is already a pull
        // request and whether that pull request is in trouble.
        //
        // Under Uncommitted too, unlike the guessed-base warning below. That
        // warning is about the COMPARISON, which uncommitted work does not
        // make; this is about the branch, which is the same branch whichever
        // segment is showing.
        pullRequestRow

        // Only a GUESSED base is called out. The others are recorded facts;
        // a guess is the one that can silently produce a wrong diff that
        // looks exactly like a right one.
        //
        // Under Uncommitted the guess cannot have produced anything: that
        // comparison never reaches for the base. Left on, the warning would be
        // telling a reader that a diff against HEAD may be wrong, which is the
        // one thing about it that cannot be.
        if store.changeSet.baseIsGuessed && !local {
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

        commitEntry
    }

    /// The pull request on this branch in one line, or the way to open one.
    ///
    /// Three outcomes, and the third is the one this row is careful about:
    ///
    /// - There is a pull request, so it is drawn and the whole line opens it.
    /// - `gh` answered and there is none, so the line offers to create one.
    /// - `gh` could not be asked, so the line says NOTHING. "There is no pull
    ///   request" and "we could not find out" arrive as the same absent value
    ///   on every link, and offering to create one while a pull request exists
    ///   behind a logged-out `gh` is this app confidently proposing the wrong
    ///   action. `StackResponse.prAnswered` is the whole difference and it is
    ///   read the safe way — a runner too old to answer counts as a no.
    @ViewBuilder
    private var pullRequestRow: some View {
        if let info = pullRequest {
            if let pr = info.pr {
                pullRequestState(pr)
            } else if info.known,
                let url = PullRequestLink.compare(
                    repoURL: info.repoURL,
                    baseRef: store.changeSet.baseRef,
                    head: store.changeSet.branch)
            {
                createPullRequest(url)
            }
        }
    }

    /// What GitHub says, and a tap anywhere on it to go there.
    ///
    /// The whole row is the link rather than a chevron at the end of it, since
    /// the row has nothing else it could do.
    ///
    /// **Quiet when healthy.** An open, approved, green pull request is drawn
    /// entirely in `.secondary` with no glyph at all: it is the ordinary case,
    /// it wants nothing, and a color spent on it would be a color that says
    /// nothing anywhere. That is this codebase's existing position — `71934f8`
    /// turned the last permanent green dot neutral for it — and it is what
    /// makes the red one glanceable at arm's length. Green appears nowhere on
    /// this row, including on a passing check.
    @ViewBuilder
    private func pullRequestState(_ pr: PullRequest) -> some View {
        if let url = URL(string: pr.url) {
            Link(destination: url) { pullRequestLine(pr, linked: true) }
                // The row's own colors are set on every element inside it, so
                // the accent never reaches the label. This is the same pairing
                // the file header describes for greyed controls, and losing it
                // fails back to blue.
                .tint(.secondary)
                .accessibilityIdentifier("changes-pr-row")
                .accessibilityLabel(Self.spokenPullRequest(pr))
                .accessibilityHint("Opens this pull request on GitHub")
        } else {
            // A pull request whose URL did not parse is still a pull request,
            // and its state is the half of this row that matters. Drawn without
            // the arrow, because there is nowhere to go.
            pullRequestLine(pr, linked: false)
                .accessibilityElement(children: .ignore)
                .accessibilityIdentifier("changes-pr-row")
                .accessibilityLabel(Self.spokenPullRequest(pr))
        }
    }

    private func pullRequestLine(_ pr: PullRequest, linked: Bool) -> some View {
        let tint = Self.tint(for: pr.emphasis)
        return HStack(spacing: 6) {
            // The number first, because it is the identifier — the thing you
            // say out loud and the thing you match against a browser tab.
            Text("#\(pr.number)")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
            if let symbol = pr.headerSymbol {
                Image(systemName: symbol)
                    .font(.caption2)
                    .foregroundStyle(tint)
            }
            Text(pr.headerSentence)
                .font(.caption)
                .foregroundStyle(tint)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 4)
            if linked {
                Image(systemName: "arrow.up.right")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
        // The Spacer is part of the target, not a hole in it.
        .contentShape(Rectangle())
    }

    /// The one thing to offer when there is no pull request yet.
    ///
    /// Blue, and the only blue on this header. This screen's rule is that
    /// moving through a diff is grey and PRODUCING something is blue, and the
    /// accent is affordable here for the reason the resume card's Continue
    /// gives: it appears once, and it is a one-time offer with one obvious
    /// answer.
    ///
    /// A link and not a button, deliberately. `gh pr create --web` would open
    /// a browser on the RUNNER, which is a machine nobody is looking at.
    private func createPullRequest(_ url: URL) -> some View {
        Link(destination: url) {
            HStack(spacing: 6) {
                Image(systemName: "plus.circle")
                    .font(.caption2)
                Text("Create Pull Request")
                    .font(.caption.weight(.medium))
                Spacer(minLength: 4)
                Image(systemName: "arrow.up.right")
                    .font(.caption2)
            }
            .contentShape(Rectangle())
        }
        .accessibilityIdentifier("changes-pr-create")
        .accessibilityLabel("Create Pull Request")
        .accessibilityHint("Opens GitHub with a new pull request for this branch")
    }

    /// Grey, orange, red — and never green. See `PullRequestEmphasis`.
    private static func tint(for emphasis: PullRequestEmphasis) -> Color {
        switch emphasis {
        case .quiet: return .secondary
        case .pending: return .orange
        case .alarm: return .red
        }
    }

    /// The row, said rather than spelled.
    ///
    /// The number reads as "Pull request 412" and not "hash four one two", and
    /// the middle dot that separates the two halves on screen is a pause here
    /// rather than a character VoiceOver has to make something of.
    private static func spokenPullRequest(_ pr: PullRequest) -> String {
        let sentence = pr.headerSentence.replacingOccurrences(of: " · ", with: ", ")
        return sentence.isEmpty
            ? "Pull request \(pr.number)" : "Pull request \(pr.number), \(sentence)"
    }

    /// The card's top line, said rather than spelled.
    ///
    /// Read aloud the parts are "vs main", "plus 82", "minus 13" — three
    /// fragments that never say which comparison they belong to, and the
    /// segmented control that would have said it is a separate element further
    /// down. So the label names the comparison it is the total of — the same
    /// fix the rows that lead here got, in `NeedsYou` and `FleetList`.
    private func spokenComparison(insertions: Int, deletions: Int) -> String {
        let subject: String
        if store.scope == .local {
            subject = "Uncommitted, against HEAD"
        } else if store.changeSet.baseRef.isEmpty {
            subject = "Branch"
        } else {
            subject = "Branch, against \(store.changeSet.baseRef)"
        }
        return "\(subject), \(insertions) added, \(deletions) removed"
    }

    /// What the two numbers at the top are not counting.
    ///
    /// The whole reason `isGenerated` exists. A branch that touched eleven
    /// source files and regenerated a lockfile reads as four thousand lines
    /// changed, and somebody with ninety seconds cannot tell that from a branch
    /// that really did rewrite four thousand lines. Split, the headline is the
    /// work and this line is the lockfile — and the two still add up to what
    /// the daemon counted, which is why this says the numbers rather than
    /// hiding them.
    @ViewBuilder
    private var generatedNote: some View {
        if !store.generatedFiles.isEmpty {
            HStack(spacing: 6) {
                Image(systemName: "wrench.and.screwdriver")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                Text(
                    store.generatedFiles.count == 1
                        ? "plus 1 generated file" : "plus \(store.generatedFiles.count) generated files"
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
                Text("+\(store.generatedInsertions) −\(store.generatedDeletions)")
                    .font(.caption2.monospaced())
                    .foregroundStyle(.tertiary)
            }
        }
    }

    /// The two ways in to one commit at a time.
    ///
    /// Rows rather than toolbar items alone, because a control nobody can see
    /// is a feature nobody has: the toolbar's copy of History exists for the
    /// reader who is already scrolled a thousand lines down, not for the one
    /// arriving.
    ///
    /// Two of them, because "which commit" and "all of them, in order" are
    /// different questions and the second is the one an agent-authored branch
    /// is actually read with. Until this existed the picker was the only door,
    /// which made the reading this screen is FOR — first commit, then the next,
    /// then the next — a trip through a sheet every time.
    @ViewBuilder
    private var commitEntry: some View {
        let count = store.changeSet.commits.count

        if count > 0 {
            Button {
                Task { await store.startAtFirstCommit() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "list.number")
                        .font(.caption)
                    Text("Review Commit by Commit")
                        .font(.footnote.weight(.medium))
                    Spacer(minLength: 4)
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 6)
        }

        Button {
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

    /// Which commit is on screen, what it said it was doing, and the ways on.
    ///
    /// The counts come from `ChangeCommit` when the daemon sent them and from
    /// the commit's own file list otherwise — see `ChangesStore.commitCounts`.
    /// They are the same number either way; `--shortstat` is the sum of the
    /// same commit's `--numstat`.
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
            if let counts = store.commitCounts {
                Text("+\(counts.insertions)")
                    .font(.caption.monospaced()).foregroundStyle(.green)
                Text("−\(counts.deletions)")
                    .font(.caption.monospaced()).foregroundStyle(.red)
            }
        }

        generatedNote

        // The rationale, which for an agent's commit is usually the only one
        // written down anywhere.
        //
        // Here rather than only in the picker, and at length rather than in a
        // preview: this is the screen that is ABOUT this commit, and the body
        // is the cheapest context there is before a line of diff is read —
        // which between two sets is very often the only context there is time
        // for. Keyed on the sha so moving to the next commit starts it folded
        // again rather than inheriting however far the last one was opened.
        if let body = known?.bodyText {
            CommitBodyText(text: body)
                .id(sha)
        }

        // The change set no longer lists this sha, so there is no subject and no
        // author to show — an amend or a rebase during the read does exactly
        // that. The patch below is usually still right, because the object it
        // names is still in the repository, so this warns rather than blanks
        // the pane.
        if known == nil {
            Label(
                "This commit isn’t on the branch anymore. It was probably amended "
                    + "or rebased while you were reading.",
                systemImage: "exclamationmark.triangle")
                .font(.caption2)
                .foregroundStyle(.orange)
        }

        // Where this commit sits in the branch, and the way to the ones either
        // side of it without opening a picker. This is what makes commit-by-
        // commit a path: the reader who has finished one commit's files takes
        // one tap to the next intention, and the position says how many are
        // left — which a sheet, opened and dismissed, never could.
        if let position = store.commitPositionLabel {
            HStack(spacing: 8) {
                Button {
                    Task { await store.showPreviousCommit() }
                } label: {
                    Image(systemName: "chevron.left")
                }
                .disabled(store.previousCommit == nil)
                .accessibilityLabel("Previous commit")

                Text(position)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Button {
                    Task { await store.showNextCommit() }
                } label: {
                    Image(systemName: "chevron.right")
                }
                .disabled(store.nextCommit == nil)
                .accessibilityLabel("Next commit")

                Spacer(minLength: 4)
            }
            .font(.footnote)
            .buttonStyle(.bordered)
            // Grey. Two accent chevrons and an accent position label is a
            // traffic sign where a pair of arrows will do — and these two are
            // the most-repeated controls on the screen, one pair per commit
            // for as long as the branch is. See the accent rule above.
            .tint(.secondary)
            .foregroundStyle(.primary)
            .controlSize(.small)
            .padding(.top, 2)
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
        // Grey, with the chevrons above them. Both are ways back to a list you
        // have already seen — the definition of movement on this screen — and
        // they are also the two widest controls in the card, so they were most
        // of what made a commit header read as a row of links.
        .tint(.secondary)
        .foregroundStyle(.primary)
        .controlSize(.small)
        .padding(.top, 2)
    }
}

/// A commit body, folded to four lines until it is asked for.
///
/// Folded because an agent's body runs to paragraphs and this card sits above
/// the file list — left open, a good commit message would push the diff off the
/// screen. Four lines is enough for the first sentence of the rationale, which
/// is the part that decides whether the rest is worth reading.
private struct CommitBodyText: View {
    let text: String
    @State private var expanded = false

    /// Whether folding it would actually hide anything.
    ///
    /// Measured crudely — a line count and a length — rather than with a text
    /// layout pass, because the cost of being wrong is a "More" button that
    /// reveals nothing, and the cost of measuring properly on every redraw of a
    /// scrolling list is real.
    private var isLong: Bool {
        text.contains("\n\n") || text.count > 200
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(expanded ? nil : 4)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
            if isLong {
                // `.bordered`, like Whole Branch and History a few points below
                // it in the same card. This was accent text at `.caption2` —
                // eleven points of blue with nothing behind it, on a card that
                // sits on the terminal's theme-chosen ground rather than on a
                // system background, which is the worst of the three ways this
                // screen was spending the accent: the smallest type, the
                // thinnest contrast, and no ground of its own. A real button
                // style brings a ground with it, and the size goes up one step
                // to `.caption` because a control is not a footnote.
                //
                // What that fix did NOT do is stop it being blue: a bordered
                // button paints its label in the tint too, so the smallest
                // control in the card simply gained a ground under its accent.
                // Unfolding a paragraph you can already see four lines of is
                // the most incidental thing on this screen. Grey.
                Button(expanded ? "Less" : "More") {
                    withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
                }
                .font(.caption)
                .buttonStyle(.bordered)
                .tint(.secondary)
                .foregroundStyle(.primary)
                .controlSize(.small)
            }
        }
        .padding(.top, 2)
    }
}

// MARK: - The bar at the bottom

/// Where you are in the diff, and the controls that move you through it.
///
/// Everything here is one-handed. The two chevrons are 44 points square at the
/// trailing edge, where a right thumb rests; the index is a single tap at the
/// leading edge; and the position between them is the only place on this screen
/// that answers "how much of this is left", because a phone's scrollbar is a
/// hairline that appears while you drag and vanishes while you read.
///
/// The Next button is the same control for two different moves, and changes its
/// face when it changes its meaning: while a commit has files left it goes to
/// the next file, and on the last file of a commit it becomes Next Commit. That
/// is the join that turns a stack of commits into something you can walk from
/// end to end without ever opening a picker.
private struct ReviewBar: View {
    @ObservedObject var store: ChangesStore
    /// Observed separately from the store, so adding a note redraws this bar
    /// and not the forty-card diff behind it.
    @ObservedObject var comments: ReviewCommentQueue
    let onIndex: () -> Void
    let onComments: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            // Only when there is something in it. A permanent empty outbox
            // would be a row of chrome charging rent on a screen that has none
            // to spare.
            if !comments.pending.isEmpty {
                Button(action: onComments) {
                    HStack(spacing: 8) {
                        Image(systemName: "text.bubble")
                            .font(.caption)
                        Text(
                            comments.pending.count == 1
                                ? "1 note for the agent" : "\(comments.pending.count) notes for the agent"
                        )
                        .font(.footnote.weight(.medium))
                        Spacer(minLength: 4)
                        // Secondary, not accent. The whole row is the button —
                        // these three words are not a target of their own, and
                        // painting them accent on `.bar`, which is a dark
                        // material under a dark theme, was the app spending its
                        // one "this is tappable" signal on the part of the row
                        // that is not tappable. The chevron already says the
                        // row goes somewhere, which is exactly how the History
                        // row in the summary card above draws the same shape.
                        Text("Review and Send")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Image(systemName: "chevron.right")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                Divider()
            }

            HStack(spacing: 10) {
                Button(action: onIndex) {
                    Label("Files", systemImage: "list.bullet.indent")
                        .font(.footnote.weight(.medium))
                        .padding(.vertical, 10)
                        .padding(.trailing, 8)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(store.files.isEmpty)

                Spacer(minLength: 4)

                Text(store.positionLabel)
                    .font(.footnote.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                Spacer(minLength: 4)

                Button {
                    store.showPreviousFile()
                } label: {
                    Image(systemName: "chevron.up")
                        .font(.body.weight(.medium))
                        // A glyph is not a tap target; this is.
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!store.hasPreviousFile)
                .accessibilityLabel("Previous file")

                Button {
                    if store.nextIsCommit {
                        Task { await store.showNextCommit() }
                    } else {
                        store.showNextFile()
                    }
                } label: {
                    Group {
                        if store.nextIsCommit {
                            HStack(spacing: 4) {
                                Text("Next Commit").font(.footnote.weight(.medium))
                                Image(systemName: "chevron.right.2").font(.caption)
                            }
                            .padding(.horizontal, 8)
                        } else {
                            Image(systemName: "chevron.down")
                                .font(.body.weight(.medium))
                                .frame(width: 44)
                        }
                    }
                    .frame(height: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!store.hasNextFile && !store.nextIsCommit)
                .accessibilityLabel(store.nextIsCommit ? "Next commit" : "Next file")
            }
            .padding(.leading, 16)
            .padding(.trailing, 6)
        }
        .background(.bar)
    }
}

// MARK: - The file index

/// Every file in this comparison, on one screen, without scrolling the diff.
///
/// The difference between reviewing four files and reviewing forty. A lazy
/// stack of forty patches has no table of contents — the only way to learn what
/// a branch touched is to scroll past all of it — and on a phone that is a
/// minute of dragging before the first decision about where to look.
///
/// The counts are here for the same reason: a forty-file branch is not forty
/// equal things, and `+412 −6` beside one name is usually enough to say where
/// to start.
private struct FileIndexSheet: View {
    @ObservedObject var store: ChangesStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""

    private func matching(_ files: [ChangedFile]) -> [ChangedFile] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return files }
        // The whole path, not the leaf: "daemon" is how somebody asks for
        // everything under `crates/daemon/`, and matching only the filename
        // would answer that with nothing.
        return files.filter { $0.path.lowercased().contains(query) }
    }

    var body: some View {
        NavigationStack {
            List {
                let written = matching(store.handWrittenFiles)
                let generated = matching(store.generatedFiles)

                if written.isEmpty && generated.isEmpty {
                    Text(
                        store.files.isEmpty
                            ? "Nothing changed in this comparison."
                            : "No files match that."
                    )
                    .foregroundStyle(.secondary)
                }

                if !written.isEmpty {
                    Section {
                        ForEach(written) { file in
                            row(file)
                        }
                    } header: {
                        Text("Files")
                    }
                }

                if !generated.isEmpty {
                    Section {
                        ForEach(generated) { file in
                            row(file)
                        }
                    } header: {
                        Text("Generated")
                    } footer: {
                        // Says what the split is FOR, at the one place somebody
                        // is looking at both halves at once.
                        Text(
                            "Counted apart from the branch’s totals, so a lockfile "
                                + "doesn’t make a branch look bigger than it is.")
                    }
                }
            }
            .searchable(text: $search, prompt: "Filter files")
            .navigationTitle("Files")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }

    private func row(_ file: ChangedFile) -> some View {
        Button {
            // Expanding is what selects: one file is open at a time, so opening
            // this one is the same act as making it the place Next counts from.
            store.expand(file.path)
            dismiss()
        } label: {
            HStack(spacing: 10) {
                Text(file.status?.mark ?? "•")
                    .font(.caption2.monospaced().weight(.bold))
                    .foregroundStyle(file.status?.tint ?? .secondary)
                    .frame(width: 14)
                    .accessibilityLabel(file.status?.label ?? "Changed")

                VStack(alignment: .leading, spacing: 2) {
                    Text(file.name)
                        .font(.subheadline)
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if !file.directory.isEmpty {
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

                if store.isExpanded(file.path) {
                    Image(systemName: "checkmark").font(.footnote).foregroundStyle(.tint)
                }
            }
            .contentShape(Rectangle())
        }
        // A file name and its directory are content, not an action — without
        // this they inherit the button's accent tint. See `CommitHistorySheet`.
        .buttonStyle(.plain)
    }
}

/// The changes pane's contextual toolbar control.
///
/// Owned by `WorkspaceView`, not `ChangesView`: the host owns every navigation-bar
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
///
/// A flat FILL is not the same claim as a flat surface, and one card on this
/// screen makes the distinction. A file card's heading pins, so both of its
/// halves put this wash on `.regularMaterial` and let a blur do the thing a
/// surface floating over moving content has to do. A material is an ordinary
/// `ShapeStyle` — no container to satisfy, no live sampling to drop out of —
/// which is exactly what makes it available where Liquid Glass is not. Every
/// other card here floats over nothing and stays only this.
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

/// One file's heading — and the one part of a card that does not scroll away.
///
/// A `Section` header inside a `LazyVStack(pinnedViews: [.sectionHeaders])`, so
/// while a file's hunks are on screen its name is held at the top of the scroll
/// view, and scrolling into the next file pushes this one out with that file's
/// name in its place. That displacement is the whole reason this is a section
/// header rather than a bar drawn over the scroll by hand: an overlay would
/// have to be told which file the scroll is currently inside, which is the same
/// "which file am I looking at" question the pinning answers for nothing.
///
/// **It carries what the heading always carried**, and that is a decision
/// rather than an omission. A pinned header is the same view pinned or not, so
/// anything cut from it here is also cut from the forty headings somebody
/// scrolls past — and the status letter and the two counts are most of what
/// makes a heading worth scrolling past. It is also still the fold control,
/// which is the small thing the pinning hands over for free: the way to close a
/// file you are done with is now always in reach at the top of the screen
/// rather than a thousand lines back up.
private struct ChangesFileHeading: View {
    let file: ChangedFile
    @ObservedObject var store: ChangesStore
    /// Whether this file is on screen, so the store can remember roughly where
    /// the reader had got to. See `ChangesStore.noteVisible`.
    let onVisible: (Bool) -> Void

    private var expanded: Bool { store.isExpanded(file.path) }

    /// All four corners while the file is folded, top two while its hunks are
    /// under it.
    ///
    /// Named rather than written at the call site because TWO layers are
    /// clipped to it now — the wash, and the material under the wash — and a
    /// card whose blur and whose fill disagreed about a corner would show the
    /// disagreement precisely at the fold, which is the one moment the corner
    /// changes.
    private var cardShape: UnevenRoundedRectangle {
        .rect(
            topLeadingRadius: 12,
            bottomLeadingRadius: expanded ? 0 : 12,
            bottomTrailingRadius: expanded ? 0 : 12,
            topTrailingRadius: 12)
    }

    var body: some View {
        Button {
            store.toggle(file.path)
        } label: {
            heading
        }
        .buttonStyle(.plain)
        // Square along the bottom while there are hunks under it, so the
        // heading and the patch read as one card and not two stacked ones.
        //
        // The card's own wash, on a MATERIAL — and the material is the
        // correction to what this heading shipped as, which was the same wash
        // on a flat opaque fill. The constraint that fill was answering is real
        // and has not moved: a pinned heading floats over its own diff, and
        // `ChangesSurface.card` is a six-percent wash that the lines sliding
        // under it would show straight through. Opacity was the wrong
        // instrument for a right constraint. What ought to happen to content
        // behind a floating surface is that it BLURS, and a material is what
        // the platform hands you to do it — the workspace tab strip floats a
        // few points above this heading over the same scroll and has been
        // saying so the whole time.
        //
        // `.regularMaterial` in particular. It is already the material this app
        // holds over moving content: `AgentView`'s plan overlay, held over a
        // streaming transcript, for this reason and with this note attached.
        // `.thickMaterial` here is the slash-command popup — a dense panel that
        // stops the screen behind it, which for a file heading is the flat slab
        // again by a longer road. `.bar` belongs to the review bar, which sits
        // on the window's edge rather than inside the content. The thin
        // materials pass enough luminance that a diff moving under a heading
        // shows as bands travelling behind its own file's name.
        //
        // Not `GlassSurface`, for two reasons and neither of them taste. It
        // applies `.glassEffect` per view, and a heading is realized and thrown
        // away by the `LazyVStack` on every scroll — the exact case
        // `ChangesSurface` documents the effect dropping out of. And
        // `GlassSurface(radius:)` draws one uniform `.rect(cornerRadius:)`,
        // which cannot say "square along the bottom while the file is open",
        // which is the shape the fold turns on. The tab strip has neither
        // problem: one surface, one radius, nothing lazy underneath it.
        .background(ChangesSurface.card, in: cardShape)
        .background(.regularMaterial, in: cardShape)
        // The gap between cards lives here rather than in the stack's spacing,
        // and it alone is still drawn opaque — which is not the old mistake
        // left in place, but the one strip where an opaque ground is the
        // answer. While this heading is pinned, these ten points are the band
        // at the very top of the scroll, and what they are painted with is the
        // theme's own ground, which is what the scroll view is drawn on anyway:
        // above a pinned heading that reads as the page continuing rather than
        // as a bar of anything, and between two cards it is invisible. Carrying
        // the material up here instead would tint all forty gaps, turning the
        // ground between cards into a stripe; leaving the strip clear would run
        // a ten-point slot of moving diff across the top of the screen.
        .padding(.top, 10)
        .background(TerminalPalette.background)
        // The scroll decides what gets read: a file is fetched when its heading
        // comes into view, not when the change set loads.
        .task(id: taskKey) {
            guard expanded else { return }
            await store.ensure(file.path)
        }
        // On the heading rather than on the whole card, because the heading is
        // the part that exists whether the file is open or folded — and because
        // a pinned heading is on screen exactly while its own hunks are, which
        // is the question `noteVisible` is asking.
        .onScrollVisibilityChange(threshold: 0.05) { onVisible($0) }
        // A header as well as a button, so VoiceOver's rotor can walk the files
        // the way the eye walks the pinned names.
        .accessibilityAddTraits(.isHeader)
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
    private var taskKey: String { "\(file.path)#\(store.generation)#\(expanded)" }

    private var heading: some View {
        HStack(spacing: 10) {
            // The host's letter, whichever scope this row came from.
            //
            // `A`, `D`, `R` and `T` are real here. A commit's files come from
            // `changes.commit_files`, which merges `git diff --name-status`
            // onto the counts (crates/daemon/src/file_diff.rs); the branch's
            // come from `change_set::numstat`, which has always done the same;
            // Local reads the working tree's own porcelain codes. So a file a
            // commit created is badged `A`, not `M`.
            //
            // The bullet is the no-status case only — a daemon so old it omits
            // the field, which decodes as nil rather than as a wrong letter.
            // "Changed" is what it can honestly say, and the spoken label below
            // says the same.
            Text(file.status?.mark ?? "•")
                .font(.caption2.monospaced().weight(.bold))
                .foregroundStyle(file.status?.tint ?? .secondary)
                .frame(width: 14)

            VStack(alignment: .leading, spacing: 1) {
                // The leaf, then its directory underneath. A phone cannot fit
                // `crates/daemon/src/review_ops.rs` at a readable size, and the
                // leaf is the half that identifies the file.
                //
                // Truncated in the MIDDLE, which matters more now that this is
                // what stays on screen: a long leaf is `ChangesStore+Resume` or
                // `WorkspaceDetailViewController`, and both ends of it say more
                // than either end alone.
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
                    //
                    // Truncated from the HEAD, so what survives a narrow screen
                    // is the directory the file is actually in rather than the
                    // `crates/` every path on the branch begins with.
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

            Image(systemName: expanded ? "chevron.down" : "chevron.right")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(spokenHeading)
        .accessibilityHint(expanded ? "Folds this file" : "Opens this file")
    }

    /// The heading as one phrase, and without the directory.
    ///
    /// Pinned, this is read on the way into every file's section, and the path
    /// in it would be `crates/daemon/src` spelled out once per file on a forty
    /// file branch. `NeedsYou`'s workspace header leaves a branch out of its
    /// label for exactly that reason. What is left is the phrase that answers
    /// "which file is this" — the leaf, what happened to it, and how much
    /// moved. The directory stays on screen for the eye, and `FileIndexSheet`,
    /// which is where a VoiceOver reader CHOOSES a file rather than passes one,
    /// still says the whole of it.
    private var spokenHeading: String {
        var parts = [file.name, file.status?.label ?? "Changed"]
        if file.binary {
            parts.append("binary")
        } else {
            if file.insertions > 0 { parts.append("\(file.insertions) added") }
            if file.deletions > 0 { parts.append("\(file.deletions) removed") }
        }
        return parts.joined(separator: ", ")
    }
}

/// The patch under a heading, and nothing at all while the file is folded.
///
/// A `Section`'s content, so what scrolls is separated from what sticks. Empty
/// rather than hidden when the file is folded: an empty section body draws
/// nothing and takes no space, which is what a folded card was already.
private struct ChangesFileBody: View {
    let file: ChangedFile
    @ObservedObject var store: ChangesStore
    /// Ask for a note about some part of this file.
    let onComment: (ReviewAnchor) -> Void

    private var expanded: Bool { store.isExpanded(file.path) }
    private var lines: [DiffComputation.Line] { store.fileDiffs[file.path] ?? [] }

    /// Square along the top, where the heading is, and rounded at the bottom,
    /// where the card ends. Named for the reason `ChangesFileHeading.cardShape`
    /// is: the wash and the material beneath it are clipped to one shape.
    private var bodyShape: UnevenRoundedRectangle {
        .rect(
            topLeadingRadius: 0,
            bottomLeadingRadius: 12,
            bottomTrailingRadius: 12,
            topTrailingRadius: 0)
    }

    var body: some View {
        if expanded {
            VStack(alignment: .leading, spacing: 0) {
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
                    DiffHunks(
                        lines: lines,
                        file: file.path,
                        commit: store.scope.commitSha,
                        onComment: onComment)
                }

                // At the END of the file rather than in the heading, because
                // that is where the reader is when they have something to say
                // about it — and because a second button inside the heading's
                // button is a tap target overlapping the one that folds the
                // card.
                //
                // `.bordered`, which is what the summary card's own buttons say
                // two screens up. It used to be `.plain` with `.foregroundStyle`
                // forced to `.tint`: accent text with nothing behind it, on a
                // card that sits on the terminal's theme-chosen ground rather
                // than on a system background — so how well it could be read
                // depended on which theme was in force. A real button style
                // brings its own ground with it and clears whatever is behind.
                //
                // **The one control on this screen that keeps the accent**,
                // and the sweep that greyed the rest of them left it
                // deliberately. It is the only thing here that produces
                // something rather than moving you somewhere, and it is on
                // screen exactly once: one file is open at a time, so this
                // does not repeat down the scroll the way a per-card control
                // would. If everything on a screen is grey, grey has stopped
                // meaning anything either.
                Button {
                    onComment(
                        ReviewAnchor(
                            file: file.path, commit: store.scope.commitSha,
                            firstLine: nil, lastLine: nil, quote: nil))
                } label: {
                    Label("Comment on This File", systemImage: "text.bubble")
                        .font(.footnote)
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // On the same material as the heading, which is the part of the
            // heading's fix that is not about the heading.
            //
            // Nothing ever moves behind a body: it scrolls with its own hunks
            // and has only the ground behind it, so the blur here has nothing
            // to blur. What it buys is the join. A material has a tone of its
            // own, so a material under the heading and none under the body
            // would put a step in the color straight across the middle of every
            // card — and that step is on screen constantly, because at most ONE
            // heading is pinned while the three or four others in view are
            // ordinary card tops. A blur per realized card is what the two
            // halves reading as one card costs, and cards are realized a few at
            // a time; this is not the per-row glass `ChangesSurface` rules out.
            .background(ChangesSurface.card, in: bodyShape)
            .background(.regularMaterial, in: bodyShape)
        }
    }
}

// MARK: - The patch

/// How a file's lines are cut up for a phone.
///
/// Pure functions over the lines the daemon sent. Nothing here fetches, widens
/// or recovers anything: the daemon decided what this patch contains — its
/// context, its truncation, its redactions — and this only decides how much of
/// what arrived is on screen at once.
private enum DiffLayout {
    /// One run of lines with no gap in it.
    ///
    /// Derived from the line NUMBERS rather than from a `@@` header, because
    /// the phone's `FileDiff` is already structured hunks and the flattening
    /// into `DiffComputation.Line` drops the boundaries — a jump in the new
    /// file's numbering is what is left of them, and it is enough.
    struct Hunk: Identifiable {
        let id: Int
        let lines: [DiffComputation.Line]

        var firstLine: Int? { lines.compactMap(\.newNumber).first }
        var lastLine: Int? { lines.compactMap(\.newNumber).last }

        /// `Lines 120-148`, or nothing at all for a hunk with no new-side
        /// numbering — every line of a deleted file, for one.
        var rangeLabel: String? {
            guard let firstLine else { return nil }
            guard let lastLine, lastLine > firstLine else { return "Line \(firstLine)" }
            return "Lines \(firstLine)-\(lastLine)"
        }
    }

    static func hunks(_ lines: [DiffComputation.Line]) -> [Hunk] {
        var out: [Hunk] = []
        var current: [DiffComputation.Line] = []
        for (index, line) in lines.enumerated() {
            if index > 0, gap(lines[index - 1], line), !current.isEmpty {
                out.append(Hunk(id: out.count, lines: current))
                current = []
            }
            current.append(line)
        }
        if !current.isEmpty { out.append(Hunk(id: out.count, lines: current)) }
        return out
    }

    private static func gap(_ previous: DiffComputation.Line, _ current: DiffComputation.Line)
        -> Bool
    {
        guard let before = previous.newNumber, let after = current.newNumber else { return false }
        return after > before + 1
    }

    /// A stretch of a hunk, either drawn or folded away.
    struct Segment: Identifiable {
        let id: Int
        let lines: [DiffComputation.Line]
        /// How many unchanged lines this stands in for, when it is a fold.
        let folded: Int?
    }

    /// Lines kept either side of a folded run.
    private static let keep = 2
    /// The shortest run of unchanged lines worth folding at all.
    ///
    /// Five, which given the daemon's default three lines of context means this
    /// fires on hunks git MERGED — a run of six unchanged lines between two
    /// changes is what two nearby edits look like after `-U3` joins them. That
    /// is exactly the shape an LLM refactor produces, twenty small edits
    /// scattered down one file, and folding each gap to a tappable line saves
    /// several screens of dragging over the length of the file. It never fires
    /// on a new file, which has no unchanged lines at all.
    private static let foldFrom = 5

    static func segments(of hunk: Hunk) -> [Segment] {
        var out: [Segment] = []
        var index = 0
        let lines = hunk.lines

        func append(_ slice: ArraySlice<DiffComputation.Line>, folded: Int?) {
            guard !slice.isEmpty || folded != nil else { return }
            out.append(Segment(id: out.count, lines: Array(slice), folded: folded))
        }

        while index < lines.count {
            guard lines[index].kind == .context else {
                let start = index
                while index < lines.count, lines[index].kind != .context { index += 1 }
                append(lines[start..<index], folded: nil)
                continue
            }
            let start = index
            while index < lines.count, lines[index].kind == .context { index += 1 }
            let run = lines[start..<index]
            let atStart = start == 0
            let atEnd = index == lines.count
            // Head and tail are what stays: the lines actually touching a
            // change are the ones that give it its place, and the ones further
            // out are the ones nobody reads.
            let head = atStart ? 0 : keep
            let tail = atEnd ? 0 : keep
            let hidden = run.count - head - tail
            guard run.count >= foldFrom, hidden >= 2 else {
                append(run, folded: nil)
                continue
            }
            if head > 0 { append(run.prefix(head), folded: nil) }
            append(ArraySlice(run.dropFirst(head).dropLast(tail)), folded: hidden)
            if tail > 0 { append(run.suffix(tail), folded: nil) }
        }
        return out
    }
}

/// The patch itself, one hunk at a time.
///
/// Each hunk scrolls sideways in its OWN scroll view, and that is the fix for
/// the thing that made long lines miserable on a phone: with one scroll view
/// around the whole file, dragging to see the end of a 200-character line in
/// the third hunk dragged the first two hunks off the screen with it, and
/// coming back meant dragging all of them back. Per hunk, a long line moves
/// only its own neighborhood — and every other hunk stays where it was left.
///
/// Wrapping instead is not an option: a wrapped diff line breaks the one
/// property a diff has, that a line is a line, and on a phone almost every line
/// of real code would wrap.
private struct DiffHunks: View {
    let lines: [DiffComputation.Line]
    let file: String
    let commit: String?
    let onComment: (ReviewAnchor) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(DiffLayout.hunks(lines)) { hunk in
                HunkView(hunk: hunk, file: file, commit: commit, onComment: onComment)
            }
        }
        .padding(.bottom, 4)
    }
}

private struct HunkView: View {
    let hunk: DiffLayout.Hunk
    let file: String
    let commit: String?
    let onComment: (ReviewAnchor) -> Void

    /// Folds the reader has opened, by segment. Local to the hunk and lost when
    /// the card is folded, which is right: the reason to open a fold is the
    /// question being asked right now.
    @State private var revealed: Set<Int> = []
    /// The widest row in this hunk, so every row can be drawn that wide. See
    /// the note on `onPreferenceChange` below.
    @State private var rowWidth: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ScrollView(.horizontal, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(DiffLayout.segments(of: hunk)) { segment in
                        if let folded = segment.folded, !revealed.contains(segment.id) {
                            foldRow(segment, count: folded)
                        } else {
                            ForEach(segment.lines) { line in
                                DiffLineRow(line: line, width: rowWidth)
                            }
                        }
                    }
                }
                // Every row as wide as the widest, so the tint behind them is
                // one shape.
                //
                // A row is an `HStack` of two `Text`s inside a horizontal
                // `ScrollView`, so it sizes to ITS OWN line and the colored
                // background sizes with it. Nine added lines of nine different
                // lengths were therefore nine differently-sized green blocks
                // stacked on each other, and the steps between them read as
                // stray rounded corners down the edge of the diff — which is
                // exactly what they were reported as.
                //
                // Measured rather than guessed at, because the width that
                // matters is the widest LAID OUT row and only the layout knows
                // it: the font is the user's terminal face at the user's
                // terminal size, and a tab is not a character wide. The
                // feedback settles in one pass — each row is asked for at
                // least the running maximum, and a row already at the maximum
                // reports the maximum.
                .onPreferenceChange(DiffRowWidth.self) { rowWidth = $0 }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
        }
    }

    /// Where in the file this is, and the way to say something about it.
    ///
    /// The line range is what makes a note on a hunk into an instruction an
    /// agent can act on: `push.ts` alone leaves it to search a 300-line file
    /// for whatever was meant, and `push.ts`, around lines 120-148 does not.
    private var header: some View {
        HStack(spacing: 8) {
            if let range = hunk.rangeLabel {
                Text(range)
                    .font(.caption2.monospaced())
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 4)
            Button {
                onComment(
                    ReviewAnchor(
                        file: file,
                        commit: commit,
                        firstLine: hunk.firstLine,
                        lastLine: hunk.lastLine,
                        quote: ReviewAnchor.quoting(changedLine ?? "")))
            } label: {
                Image(systemName: "text.bubble")
                    .font(.caption)
                    .frame(width: 40, height: 32)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Comment on this hunk")
        }
        .padding(.leading, 12)
        .padding(.trailing, 4)
        .padding(.top, 4)
    }

    /// The first line this hunk actually changed, for the note to quote.
    ///
    /// The first CHANGED line rather than the first line: a hunk opens with
    /// context, and quoting an untouched line back at an agent points it at the
    /// line before the thing being talked about.
    private var changedLine: String? {
        hunk.lines.first { $0.kind != .context }?.text
    }

    private func foldRow(_ segment: DiffLayout.Segment, count: Int) -> some View {
        Button {
            withAnimation(.easeInOut(duration: 0.15)) { _ = revealed.insert(segment.id) }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.down.circle")
                    .font(.caption2)
                Text(count == 1 ? "1 unchanged line" : "\(count) unchanged lines")
                    .font(.caption2)
            }
            .foregroundStyle(.tertiary)
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The widest row a hunk has laid out, reduced by `max`.
///
/// Not private: this app draws diffs in two places — here, and the tool cards
/// in `AgentView.diffBody` — and both are a horizontally scrolling stack of
/// rows that size to their own line. One key so the two cannot answer the
/// question differently.
struct DiffRowWidth: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

private struct DiffLineRow: View {
    let line: DiffComputation.Line
    /// The widest row in this hunk. Zero on the first pass, before anything
    /// has been measured, which draws exactly what this drew before.
    var width: CGFloat = 0

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
        // Report this row's natural width, then take at least the widest.
        //
        // `minWidth`, not `width`: a row longer than the running maximum must
        // still be allowed to be itself, or the measurement it is about to
        // report would be the clamped value and the maximum could never grow.
        .background(
            GeometryReader { geometry in
                Color.clear.preference(key: DiffRowWidth.self, value: geometry.size.width)
            }
        )
        .frame(minWidth: width, alignment: .leading)
        // `in: .rect` stated, not left to the default.
        //
        // `.background(_ style:)` resolves its shape against the container it
        // finds itself in, and the container here is a card clipped to an
        // `UnevenRoundedRectangle` with 12pt bottom corners — so every row was
        // inheriting the CARD's bottom corners and drawing them at the bottom
        // of each line. That is the scalloped left edge: eight-odd points of
        // curve on every row of every diff, which reads as rounded corners
        // scattered down the gutter because that is exactly what it was.
        .background(background, in: .rect)
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
    /// What came back from the runner, for the one notice this app has no
    /// account of. Inside the card rather than beneath it, because the words
    /// and the sentence are one report; and in a `DetailBox` rather than a
    /// second `Text`, because a box is what marks output as somebody else's.
    /// Every other caller leaves it nil and draws exactly what it drew before.
    var detail: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Image(systemName: symbol).foregroundStyle(tint)
                Text(text).font(.footnote).foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            if let detail, !detail.isEmpty {
                DetailBox(text: detail)
            }
        }
        .padding(12)
        .background(ChangesSurface.card, in: .rect(cornerRadius: 12))
    }
}

// MARK: - Saying something back

/// Writing one note about one part of the diff.
///
/// **Dictation first**, which on iOS means the system keyboard's microphone
/// rather than a recognizer of this app's own: the field comes up focused with
/// the keyboard already open, so dictating is one tap on a key that is always
/// in the same place, and it needs no microphone permission, no speech
/// entitlement, and no second transcription engine to disagree with the one the
/// user already has. Typing with a thumb is the fallback rather than the
/// expectation.
///
/// The anchor is shown, not implied. What separates a comment from a prompt is
/// that it is ABOUT something, and the reader has to be able to see which
/// something before deciding what to say about it.
private struct CommentComposer: View {
    let anchor: ReviewAnchor
    @ObservedObject var comments: ReviewCommentQueue
    @Environment(\.dismiss) private var dismiss

    @State private var text = ""
    @FocusState private var writing: Bool

    private var canAdd: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        "What should the agent do about this?", text: $text, axis: .vertical)
                        .lineLimit(3...10)
                        .focused($writing)
                } header: {
                    Text("Note")
                } footer: {
                    Text("Tap the microphone on the keyboard to dictate.")
                }

                Section {
                    Text((anchor.file as NSString).lastPathComponent)
                        .font(.subheadline)
                    Text(anchor.file)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if anchor.firstLine != nil {
                        Text(anchor.placeDescription.capitalizedFirst)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    if let quote = anchor.quote {
                        Text(quote)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                } header: {
                    Text("About")
                } footer: {
                    // Says the thing that makes collecting worth the wait.
                    Text(
                        "Notes are collected and sent to the agent together, so it "
                            + "gets one turn instead of one per note.")
                }
            }
            .navigationTitle("Add a Note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        comments.add(
                            ReviewComment(
                                anchor: anchor,
                                text: text.trimmingCharacters(in: .whitespacesAndNewlines)))
                        dismiss()
                    }
                    .disabled(!canAdd)
                }
            }
            .task {
                // Focused on arrival, because the whole point is to say the
                // thing while it is in mind — and because a keyboard that has
                // to be summoned is a tap the free hand is not available for.
                //
                // After the presentation animation rather than during it:
                // focus asked for while a sheet is still sliding up is
                // routinely dropped on the floor, and a composer that comes up
                // with no keyboard costs the tap this was meant to save.
                try? await Task.sleep(for: .milliseconds(250))
                writing = true
            }
        }
    }
}

extension String {
    /// `lines 12-40` → `Lines 12-40`, for the one place a phrase written for
    /// mid-sentence use has to start one.
    fileprivate var capitalizedFirst: String {
        guard let first = self.first else { return self }
        return first.uppercased() + dropFirst()
    }
}

/// Everything written but not yet said, and the one button that says it.
///
/// **Collect, then send**, which is a decision about the agent rather than
/// about the phone: five notes fired off as five prompts are five turns, each
/// re-reading the files the last one just touched, and the fifth arrives while
/// the agent is still acting on the first. The same five delivered together are
/// one turn against one branch.
///
/// **Nothing here is ever resent on its own.** `session/prompt` goes out with
/// `request_no_wait` and its response signals end-of-turn rather than receipt,
/// so there is no acknowledgment anywhere on this path that an agent received a
/// prompt. A failed send therefore means "this client did not get an answer",
/// which is not the same as "the agent did not get the prompt" — and an
/// automatic retry would be the app deciding, with no evidence, that a message
/// which may already have arrived should arrive twice. So the notes stay, the
/// failure is stated, and the reader is the one who decides.
private struct CommentOutboxSheet: View {
    @ObservedObject var comments: ReviewCommentQueue
    let agents: [ReviewAgentTarget]
    let branch: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                if let failure = comments.failure {
                    Section {
                        Label(failure.sentence, systemImage: "exclamationmark.triangle")
                            .font(.footnote)
                            .foregroundStyle(.orange)
                        // Only where the queue has no account of what went
                        // wrong. Orange and not red, and a `Label` and not
                        // `SheetFailureSection`: nothing was lost here — the
                        // notes are still in the list below — and the button
                        // beneath already reads Try Again.
                        if let transcript = failure.transcript, !transcript.isEmpty {
                            DetailBox(text: transcript)
                        }
                    }
                }

                Section {
                    if comments.pending.isEmpty {
                        Text("Nothing written yet. Tap the speech bubble beside a hunk.")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(comments.pending) { comment in
                        row(comment)
                    }
                    .onDelete { offsets in
                        // Resolved to comments BEFORE anything is removed:
                        // deleting by index while the indices are shifting
                        // under the loop is how a two-row swipe deletes the
                        // wrong second row.
                        for comment in offsets.map({ comments.pending[$0] }) {
                            comments.remove(comment)
                        }
                    }
                } header: {
                    Text("To Send")
                }

                if !comments.pending.isEmpty {
                    Section {
                        sendControl
                    } footer: {
                        Text(
                            "Far Cooler can’t tell whether an agent received a prompt, "
                                + "so nothing is ever resent on its own.")
                    }
                }

                if !comments.sent.isEmpty {
                    Section {
                        ForEach(comments.sent) { batch in
                            sentRow(batch)
                        }
                    } header: {
                        Text("Sent")
                    } footer: {
                        // The receipt, and why it is the only one there can be.
                        Text("What went, and when. There’s no delivery receipt to show.")
                    }
                }
            }
            .navigationTitle("Notes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    /// One button when there is one agent, a menu when there are several, and a
    /// sentence when there are none.
    ///
    /// A picker with one entry would be a choice nobody has, and a disabled
    /// button with no explanation is the app refusing without saying why: a
    /// workspace whose agent has exited has nowhere to send to, and that is a
    /// fact about the workspace rather than a fault in the notes.
    @ViewBuilder
    private var sendControl: some View {
        if comments.sending {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Sending…").foregroundStyle(.secondary)
            }
        } else if agents.isEmpty {
            Text("No agent is running in this workspace, so there’s nowhere to send these yet.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        } else if agents.count == 1, let only = agents.first {
            Button {
                Task { await comments.send(to: only, branch: branch) }
            } label: {
                Label(
                    comments.failure == nil ? "Send to \(only.name)" : "Try Again",
                    systemImage: "paperplane")
            }
        } else {
            Menu {
                ForEach(agents) { agent in
                    Button(agent.name) {
                        Task { await comments.send(to: agent, branch: branch) }
                    }
                }
            } label: {
                Label(
                    comments.failure == nil ? "Send to an Agent" : "Try Again",
                    systemImage: "paperplane")
            }
        }
    }

    private func row(_ comment: ReviewComment) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(comment.text)
                .font(.subheadline)
            Text(place(comment.anchor))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.head)
        }
    }

    private func place(_ anchor: ReviewAnchor) -> String {
        let leaf = (anchor.file as NSString).lastPathComponent
        guard anchor.firstLine != nil else { return leaf }
        return "\(leaf) · \(anchor.placeDescription)"
    }

    private func sentRow(_ batch: SentReviewBatch) -> some View {
        DisclosureGroup {
            Text(batch.text)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
        } label: {
            VStack(alignment: .leading, spacing: 3) {
                Text(batch.count == 1 ? "1 note" : "\(batch.count) notes")
                    .font(.subheadline)
                Text(
                    "to \(batch.agentName) · "
                        + Date(timeIntervalSince1970: batch.sentAt)
                        .formatted(date: .omitted, time: .shortened)
                )
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
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
///
/// A picker, and now ONLY a picker: reading a branch commit by commit no longer
/// comes through here, because it never should have — see `commitEntry` and
/// `ChangesStore.showNextCommit`. This is for the commit somebody has in mind.
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
        // Subject, body, author and sha, because all four are things people
        // half-remember about a commit they are looking for. The body is in
        // there now that it is carried at all, and it is often where the word
        // somebody remembers actually appears — an agent puts the file it
        // touched in the rationale far more often than in the subject. The sha
        // is matched as a prefix: nobody searches for the middle of a hash, and
        // a substring match on hex turns every two-character query into noise.
        return store.commitsNewestFirst.filter {
            $0.subject.lowercased().contains(query)
                || ($0.bodyText?.lowercased().contains(query) ?? false)
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
                    // `.plain`, or the row comes out blue. A button's label
                    // inherits the accent tint, and `.primary`, `.secondary`
                    // and `.tertiary` are hierarchical — they resolve against
                    // whatever the current foreground style is, so inside a
                    // tinted label they render as shades of the accent rather
                    // than shades of gray. Every row in this sheet is content
                    // that happens to be tappable; the Mac's commit popover has
                    // said `.plain` here all along.
                    .buttonStyle(.plain)
                }

                Section {
                    if store.changeSet.commits.isEmpty {
                        // The branch itself is empty, which is the ordinary
                        // state of a worktree an agent has only just started in
                        // — not a failure, and not the same as a filter that
                        // matched nothing.
                        Text("This branch hasn’t committed anything yet.")
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
                        .buttonStyle(.plain)
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
        // A plain label hit-tests where its content is, and the gap a thumb
        // lands in is between the words and the checkmark. The commit rows
        // below already say this; `FleetList`'s rows say it too.
        .contentShape(Rectangle())
    }

    /// Sha, subject, the top of the rationale, author, age and what it changed.
    ///
    /// The two lines of body are the addition that changes what this list is
    /// for. A subject is a label; the body is the closest thing an agent writes
    /// to an explanation, and two lines of it is usually the difference between
    /// "some commit about retries" and knowing whether this is the commit worth
    /// opening — which, with ninety seconds, decides the whole window.
    ///
    /// The `+N −M` are the daemon's own, from `--shortstat` on the same
    /// `git log` that produced the row, and they are absent rather than zero
    /// when it could not count them. See `ChangeCommit.counts`.
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
                if let preview = commit.bodyPreview {
                    Text(preview)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
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
            // Down the trailing edge rather than along the meta line, which had
            // no room left: sha, author and age already fill a phone's width,
            // and a fourth and fifth item on that line truncated the author to
            // an initial.
            VStack(alignment: .trailing, spacing: 2) {
                if let counts = commit.counts {
                    HStack(spacing: 4) {
                        Text("+\(counts.insertions)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.green)
                        Text("−\(counts.deletions)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.red)
                    }
                }
                // How WIDE the commit is, which the two line counts do not say:
                // `+300 −40` across one file is a rewrite and across thirty is
                // a rename sweep, and on a branch being read one commit at a
                // time that is the difference between opening it now and
                // leaving it for the next window.
                if let touched = commit.filesChanged, touched > 0 {
                    Text(touched == 1 ? "1 file" : "\(touched) files")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            if store.scope.commitSha == commit.sha {
                Image(systemName: "checkmark").font(.footnote).foregroundStyle(.tint)
            }
        }
        .contentShape(Rectangle())
    }
}

#if DEBUG

/// The review pane over a canned change set, so it can be looked at.
///
/// `-changes-layout-harness`, with `-commit` for the commit-by-commit reading.
/// The sibling of `AgentLayoutHarness`, added for the same reason and after
/// the same failure: this screen's controls had twice been argued about from
/// the code — accent text became `.bordered`, which is still accent text — and
/// nobody had put the result on a screen. The commit arm is the one that
/// matters most, because it is the only place four of these controls are drawn
/// at once.
///
/// Deliberately NOT a full stand-in. `resume` and `commitFiles` are
/// `private(set)` for good reasons and the harness does not reach around them,
/// so the commit arm shows its header over "nothing changed here". The header
/// is the part being looked at.
struct ChangesLayoutHarness: View {
    static var isRequested: Bool {
        CommandLine.arguments.contains("-changes-layout-harness")
    }

    @StateObject private var connection = Connection()

    var body: some View {
        let store = connection.changesStores.store(for: "harness")
        let commit = Self.changeSet.commits[1]
        store.standIn(
            on: Self.changeSet,
            scope: CommandLine.arguments.contains("-commit") ? .commit(commit.sha) : .branch,
            diffs: Self.diffs,
            expanded: CommandLine.arguments.contains("-commit") ? nil : "crates/daemon/src/file_diff.rs")
        return NavigationStack {
            ChangesView(
                store: store, workspaceName: "add-retries", pullRequest: Self.pullRequest)
                .navigationTitle("add-retries")
                .navigationBarTitleDisplayMode(.inline)
        }
    }

    /// The pull request row's states, one launch argument each.
    ///
    /// The DEFAULT is the case the row exists to be careful about: `gh` could
    /// not be asked, so the header says nothing about a pull request and offers
    /// nothing. Making that the argument-free case is deliberate — it is the
    /// one a screenshot has to prove, and the one that is wrong if anybody
    /// reaches for a `PullRequest?` alone.
    ///
    /// | argument | what the row shows |
    /// | --- | --- |
    /// | none | nothing at all — `gh` could not answer |
    /// | `-pr-none` | Create Pull Request |
    /// | `-pr-healthy` | #335 Approved · Checks passed, all grey |
    /// | `-pr-failing` | #335 Changes requested · Checks failed, red |
    /// | `-pr-pending` | #335 Draft · Checks running, orange |
    /// | `-pr-merged` | #335 Merged, grey, with the merge glyph |
    private static var pullRequest: BranchPullRequest? {
        let args = CommandLine.arguments
        // `gh` answered and there is no pull request on this branch.
        if args.contains("-pr-none") {
            return BranchPullRequest(
                pr: nil, known: true, repoURL: "https://github.com/o/overnight")
        }
        func known(_ pr: PullRequest) -> BranchPullRequest {
            BranchPullRequest(pr: pr, known: true, repoURL: "https://github.com/o/overnight")
        }
        func canned(state: String, checks: String, review: String) -> PullRequest {
            PullRequest(
                number: 335,
                url: "https://github.com/o/overnight/pull/335",
                state: state,
                checks: checks,
                review: review,
                stale: false,
                headOid: "a1b2c3d4e5f6",
                mergedAt: state == "merged" ? 1_785_925_777_000 : nil,
                fetchedAt: 1_785_925_800_000)
        }
        if args.contains("-pr-healthy") {
            return known(canned(state: "open", checks: "passing", review: "approved"))
        }
        if args.contains("-pr-failing") {
            return known(canned(state: "open", checks: "failing", review: "changes_requested"))
        }
        if args.contains("-pr-pending") {
            return known(canned(state: "draft", checks: "pending", review: "review_required"))
        }
        if args.contains("-pr-merged") {
            return known(canned(state: "merged", checks: "passing", review: "approved"))
        }
        // Nothing was said about `gh` at all, so nothing is said on the row.
        return nil
    }

    private static let changeSet = ChangeSet(
        branch: "feat/handle-retries-on-429",
        baseRef: "origin/main",
        baseSource: "tracked",
        baseCommit: "9f21c0d4",
        headCommit: "a1b2c3d4",
        insertions: 182,
        deletions: 47,
        commits: [
            ChangeCommit(
                sha: "a1b2c3d4e5f6", subject: "Back off on 429 instead of retrying immediately",
                body: nil, author: "claude", timestamp: 1_755_800_000,
                filesChanged: 2, insertions: 96, deletions: 12),
            ChangeCommit(
                sha: "b2c3d4e5f607",
                subject: "Read the Retry-After header when the server sends one",
                body: """
                    The server is allowed to say how long to wait and usually does. \
                    Honoring it is both politer and faster than a fixed ladder, which \
                    on a 429 storm is either far too eager or far too patient.

                    What this does not do is trust the header blindly: a runner that \
                    answers with an hour would otherwise park the queue for an hour.
                    """,
                author: "claude", timestamp: 1_755_790_000,
                filesChanged: 1, insertions: 86, deletions: 35),
        ],
        files: [
            ChangedFile(
                path: "crates/daemon/src/file_diff.rs", status: nil, oldPath: nil,
                insertions: 96, deletions: 12, binary: false),
            ChangedFile(
                path: "crates/client/src/changes_json.rs", status: nil, oldPath: nil,
                insertions: 86, deletions: 35, binary: false),
        ],
        workingTree: nil)

    private static let diffs: [String: [DiffComputation.Line]] = [
        "crates/daemon/src/file_diff.rs": DiffComputation.compute(
            old: """
                fn backoff(attempt: u32) -> Duration {
                    Duration::from_millis(200)
                }
                """,
            new: """
                fn backoff(attempt: u32, retry_after: Option<Duration>) -> Duration {
                    if let Some(after) = retry_after {
                        return after.min(Duration::from_secs(30));
                    }
                    Duration::from_millis(200 << attempt.min(6))
                }
                """)
    ]
}

#endif
