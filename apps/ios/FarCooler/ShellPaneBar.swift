import PhotosUI
import SwiftUI

// The bar that belongs to a PANE, which is the platform's own navigation bar.
//
// ## The only bar
//
// A worktree is a cover over the phone's stack, and this bar is all of its
// chrome: Back, which takes the cover away; the pane's name; and its More
// menu, which lists the worktree's other tabs (ov-617: the bottom bar that
// was the worktree, its swipes and its column are gone) and what the pane can
// do.
//
// - **The keyboard cannot take it away.** `DockedBar.swift:34-41`: an input
//   accessory lives in the KEYBOARD's window, so a bar at the bottom is
//   covered for as long as somebody types. One pinned to the top keeps its
//   material, and the pane's controls stay reachable with the keyboard up.
//
// - **It is glass, and the pane under it is matte.** Glass belongs to the
//   functional layer; the diff's cards and the terminal's grid are content and
//   stay flat.
//
// ## Why it is a `NavigationStack` and not a surface this file draws
//
// A navigation bar cannot be mistaken for anything else, and every property a
// hand-built one had to state it gets for nothing: the material and its
// scroll-edge behavior, the 44-point row, the title's type and truncation, the
// safe area above it, and content passing underneath it rather than stopping
// at it. One stack per PANE, inside the pane, so it travels with the pane and
// the shell itself is inside nobody's navigation. See
// `ShellScreen.ShellPaneRealView.body` for where it is mounted.
//
// ## What it costs the pane, said out loud
//
// A navigation bar reduces the safe area of what it is over, so a scroll view
// treats it as a CONTENT inset. For the terminal the same inset is a real
// loss: a VT grid is not scrollable content, so rows that ran under the bar
// would be rows you cannot read, and the honest thing is for the grid to be a
// bar shorter: 44 points.

/// One pane's own chrome: what this pane is, and what it can do.
///
/// A wrapper around the pane rather than a bar drawn beside it, because
/// `.toolbar` is a modifier on a navigation stack's ROOT — so the thing that
/// owns the buttons has to be the thing that owns the content. That shape pays
/// for itself twice over: the picker and the sheets below hang off the pane,
/// which is a live view hierarchy that can present, rather than off a view
/// hosted inside a navigation bar, which is the same trap the old toolbar
/// carried a note about one level down (a `PhotosPicker` inside a `Menu`).
///
/// ## Which capabilities live here, and why each one
///
/// Five capabilities lost their door when the shell replaced the pane host.
/// Three of them are here, and each is here rather than somewhere else for a
/// reason worth writing down:
///
/// **Review options** (`ChangesToolbarMenu`) — the `DiffScope` picker is the
/// ONLY way to change what a diff is compared against, and one of two ways
/// into the commit history. It was the pane host's toolbar item and has been
/// unmounted since. The reason it was moved out of `ChangesView` in the first
/// place — two toolbar trees merging in a different order per pane — is not a
/// risk here: `ChangesView`'s body declares no toolbar of its own, and its
/// stacks are all inside sheets it presents.
///
/// **Send an image** — types a path into a tty, so it is offered only on a
/// PLAIN terminal pane. A conversation view has its own picker in its
/// composer, and a second one up here would be two doors to one action with
/// different behavior behind them.
///
/// **Remove worktree** — worktree-scoped, and it lives on this bar because the
/// moment you decide a worktree is finished with is the moment you are looking
/// at it.
///
/// **The card's menu has it too now**, which is the Mac's own arrangement —
/// the sidebar row and the worktree detail both carry it — and it is not two
/// behaviors: the ceremony moved into `RemoveWorktreeFlow` so both doors ask
/// for exactly the same confirmation. See `RemoveWorktreeConfirmSheet` for the
/// typed name itself, recovered rather than rewritten.
///
struct ShellPaneChromeModifier: ViewModifier {
    /// The tab's own title, straight off the shell's model.
    ///
    /// Not looked up again from the fleet, and that matters: the ribbon, the
    /// column and this bar are then three renderings of ONE string, so a
    /// terminal cannot be "claude 2" in the column and something else here.
    let title: String
    /// The runner this pane is on, for the task chip's route.
    let runner: UUID
    @ObservedObject var connection: Connection
    /// Images on their way into a terminal. Owned by `ShellScreen`, so a
    /// transfer started here keeps running when you swipe to another pane.
    @ObservedObject var pastes: ImagePasteQueue
    /// The worktree this pane belongs to, read live off the fleet.
    let worktree: Worktree?
    /// The terminal this pane IS, read live off the fleet — nil on a
    /// worktree's Diff tab, which has no pane on the runner behind it.
    ///
    /// Live rather than the snapshot `ShellPaneRealView` latched, and that
    /// distinction is `TerminalView.live`'s: the latched value is the pane's
    /// IDENTITY and is right to freeze, while the pane's MODE is exactly what
    /// the switch below changes. A frozen copy left the old button asking for
    /// the same switch every time.
    let live: Terminal?
    /// The review this pane shows, on the Diff tab and nowhere else.
    let changes: ChangesStore?
    /// Whether this pane is the one on screen. See `dismissEverything`.
    let isVisible: Bool
    /// A terminal this bar just made, handed up so the shell can land on it.
    ///
    /// The pane cannot move the shell itself — nothing inside a pane knows
    /// where it sits on the track — so `ShellScreen` owns the arrival. See
    /// `newTerminalItem` for why landing on it is part of the action rather
    /// than a nicety on top of it.
    let onCreated: (String) -> Void

    @Environment(\.phoneNavigator) private var navigator
    /// The worktree's tabs, to switch between and close (ov-617).
    @Environment(\.shellTabs) private var tabs
    @Environment(\.displayScale) private var displayScale
    @Environment(\.colorScheme) private var scheme
    @ObservedObject private var nativePanes = NativePanes.shared

    /// Who this phone is on the runner, as `member.list` said (ov-597): kept
    /// here so a menu built before it arrived reads "your" once it does.
    @State private var viewer: String?
    @State private var showPhotoPicker = false
    @State private var pickedImage: PhotosPickerItem?
    /// Where a removal started here has got to. The ceremony itself is
    /// `RemoveWorktreeFlow`.
    @State private var removing: RemoveWorktreeRequest?
    @State private var newTerminalFailure: NewTerminalFailure?
    /// The read-only Files browser, over this worktree (ov-259).
    @State private var browsingFiles = false

    /// Which sentence a refused New Terminal shows.
    ///
    /// The app's OWN two, chosen from `Connection.NewTerminalResult` — never a
    /// string from the runner. `Identifiable` so `.alert`'s `presenting:` form
    /// can carry it, which is what keeps the title and the body from being
    /// derived twice out of the same optional.
    private enum NewTerminalFailure: String, Identifiable {
        case disconnected
        case refused
        var id: String { rawValue }
    }

    func body(content: Content) -> some View {
        content
            .modifier(PhoneBackItem())
            .onAppear { viewer = connection.talk.people?.you }
            .onReceive(connection.talk.$people) { viewer = $0?.you }
            .navigationTitle(title)
            // Inline, and not a choice worth agonising over: a large title
            // belongs to a screen you scroll from the top of, and a pane is a
            // terminal or a diff you are already somewhere inside. It is also
            // what keeps the cost of this bar to one 44-point row, measured
            // above.
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if let chip = taskChip {
                    ToolbarItem(placement: .principal) { chipTitle(chip) }
                }
                // One group rather than an item each, so the system spaces
                // them the way it spaces every other trailing group on this
                // platform instead of this file inventing a gap.
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if let changes {
                        ChangesToolbarMenu(store: changes)
                    }
                    // Not while a conversation covers the terminal (ov-373):
                    // the path would land in a box nobody can see.
                    if let live, !live.isClientDrawn, !nativePanes.covered.contains(live.id) {
                        imageMenu(live)
                    }
                    if hasOverflow {
                        overflowMenu
                    }
                }
            }
            // The picker hangs off the PANE rather than off the menu that
            // opens it, and now rather than off the bar either. A
            // `PhotosPicker` placed directly in a `Menu` renders as a row and
            // can never present, because menu content is not a live view
            // hierarchy to present FROM — the same note the pane host's
            // toolbar carried, and the reason this type wraps the pane instead
            // of being handed to `.toolbar` as a view.
            .photosPicker(isPresented: $showPhotoPicker, selection: $pickedImage, matching: .images)
            .onChange(of: pickedImage) { _, item in
                guard let item else { return }
                // No terminal to type a path into — only reachable if the pane
                // changed while the picker was up. Cleared rather than left
                // set, or picking the same photo again would not read as a
                // change and nothing would happen twice.
                guard let target = live?.id else {
                    pickedImage = nil
                    return
                }
                let core = connection.core
                Task {
                    // Loaded as data rather than as an `Image`: the picker
                    // hands back the original file, and re-encoding a
                    // screenshot through SwiftUI would smear the small text
                    // that is usually the whole reason someone is sending one.
                    guard let data = try? await item.loadTransferable(type: Data.self),
                        let image = UIImage(data: data)
                    else {
                        pickedImage = nil
                        return
                    }
                    pastes.send(image, terminal: target, core: core)
                    pickedImage = nil
                }
            }
            .removeWorktreeFlow($removing)
            .sheet(isPresented: $browsingFiles) {
                if let worktree {
                    FilesSheet(connection: connection, root: FilesLocation(place: .worktree(worktree.id)))
                }
            }
            // Far Cooler's own two sentences, and nothing the runner wrote.
            //
            // An alert rather than the `SheetFailureSection` the remove flow
            // uses, because there is no sheet: a menu item acts immediately,
            // so the only surface a refusal has is one the platform puts up
            // over the pane. What that costs is the `DetailBox` the sheet has
            // for a transcript, and nothing is lost by it — see
            // `Connection.NewTerminalResult`, which never carries one.
            //
            // The disconnected sentence does NOT claim nothing was made.
            // `WatchLinkHost.reason` is careful about exactly this and it is
            // the same situation: losing the link is losing the ANSWER, not
            // stopping the call, and a create that landed a moment later into
            // a dropped connection is a real tab that this side would be
            // telling somebody does not exist.
            .alert(
                newTerminalFailure == .disconnected
                    ? "Lost Connection to This Runner"
                    : "The runner didn’t open a terminal",
                isPresented: Binding(
                    get: { newTerminalFailure != nil },
                    set: { if !$0 { newTerminalFailure = nil } }),
                presenting: newTerminalFailure
            ) { _ in
                Button("OK", role: .cancel) {}
            } message: { failure in
                switch failure {
                case .disconnected:
                    Text(
                        "The connection dropped before it answered, so the terminal may or "
                            + "may not have been made. Its tabs will say which, once the "
                            + "runner’s back.")
                case .refused:
                    Text("It answered, and the answer was no. Nothing was created.")
                }
            }
            .onChange(of: isVisible) { _, visible in
                if !visible { dismissEverything() }
            }
    }

    /// The task this pane is shown under, by `TaskLink`'s rule, with the
    /// workspace its screen is on. On the Changes tab, which has no pane, the
    /// worktree's one open task. Nil for an orchestrator, and for a pane with
    /// no task or two.
    private var taskChip: (task: NeedsYouTask, workspace: String?)? {
        guard let worktree else { return nil }
        let id: String?
        if let live {
            id = TaskLink.task(of: live, in: worktree)
        } else {
            let open = worktree.openTaskIDs
            id = open.count == 1 ? open[0] : nil
        }
        guard let id, let task = worktree.openTasks?.first(where: { $0.id == id }) else {
            return nil
        }
        let workspace = live.flatMap { connection.fleet.workspace(of: $0, in: worktree) }
            ?? worktree.workspace
            ?? (connection.fleet.workspaces == nil ? worktree.repository : nil)
        return (task, workspace)
    }

    /// The pane's title with its task under it: "bil-9 Invoice PDF export",
    /// which opens the task (spec §3.2).
    private func chipTitle(_ chip: (task: NeedsYouTask, workspace: String?)) -> some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.headline)
                .lineLimit(1)
            Button {
                guard let workspace = chip.workspace else { return }
                navigator?.open(
                    .task(
                        PhoneWorkspace(runner: runner.uuidString, workspace: workspace),
                        task: chip.task.id))
            } label: {
                HStack(spacing: 4) {
                    Text(chip.task.key).font(.caption.monospaced())
                    Text(chip.task.title).font(.caption).lineLimit(1)
                }
                .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .disabled(navigator == nil || chip.workspace == nil)
            .accessibilityLabel("Task \(chip.task.key), \(chip.task.title)")
            .accessibilityIdentifier("pane-task-chip")
        }
    }

    /// Every pane in the track stays MOUNTED when it is not on screen — that is
    /// the whole point of `ShellPaneTrack` — and a presentation is not part of
    /// the pane's own view, it is on the window. So a picker or a sheet left up
    /// by a pane that has gone would be a sheet belonging to a worktree nobody
    /// is looking at, and in the destructive case a typed-name confirmation for
    /// a worktree that is no longer the one on screen.
    ///
    /// In practice a swipe cannot happen while a sheet is up — the sheet has
    /// the touches. This exists for the paths that do not go through a finger:
    /// a deep link, a Live Activity tap, a worktree disappearing underneath
    /// the pane.
    private func dismissEverything() {
        showPhotoPicker = false
        pickedImage = nil
        removing = nil
        newTerminalFailure = nil
        browsingFiles = false
    }

    /// An image, sent by typing its path into the tty.
    ///
    /// Plain-terminal only. `AgentView`'s composer has the agent-pane version
    /// and always has; this is the case the shell left with no door at all,
    /// even though `ImagePasteQueue` is still owned above the panes and
    /// `ImagePasteChips` is still drawn over them — every part of the transfer
    /// survived except the way to start one.
    ///
    /// No hand-set 44-point frame on the glyph, unlike the capsule this
    /// replaced: a toolbar item already has the platform's hit target, and
    /// forcing a frame on top of it is how a toolbar ends up with one button
    /// visibly larger than the one beside it.
    private func imageMenu(_ terminal: Terminal) -> some View {
        Menu {
            Button {
                showPhotoPicker = true
            } label: {
                Label("Choose Photo", systemImage: "photo")
            }
            if UIPasteboard.general.hasImages {
                Button {
                    if let image = UIPasteboard.general.image {
                        pastes.send(image, terminal: terminal.id, core: connection.core)
                    }
                } label: {
                    Label("Paste Image", systemImage: "doc.on.clipboard")
                }
            }
        } label: {
            Image(systemName: "photo.badge.plus")
        }
        .accessibilityLabel("Send an image")
    }

    /// Whether the overflow has anything in it. A `Menu` that opens onto
    /// nothing is worse than one that is not there.
    private var hasOverflow: Bool { canCreateTerminal || canRemove || canBrowseFiles || switchesTabs || closable != nil }

    /// A tab's mark as a picture: a menu draws an image beside a row's
    /// words, not a view.
    private func markImage(_ mark: GlanceMark) -> Image {
        let renderer = ImageRenderer(content: GlanceMarkView(mark, inAppDiameter: 9).padding(2).environment(\.colorScheme, scheme))
        renderer.scale = displayScale
        guard let picture = renderer.uiImage else { return Image(systemName: "circle") }
        return Image(uiImage: picture).renderingMode(.original)
    }

    /// Whether the worktree has another tab to go to.
    private var switchesTabs: Bool { (tabs?.tabs.count ?? 0) > 1 }

    /// The tab on screen, when it's a terminal that can be closed.
    private var closable: ShellTab? {
        tabs.flatMap { tabs in tabs.tabs.first { $0.id == tabs.current && $0.closable } }
    }

    /// Whether this worktree's files can be read from here: the runner serves
    /// them and this phone's grant may read them (`DaemonBuild.offersFiles`).
    /// Hidden otherwise, and until the runner has said what it can do.
    private var canBrowseFiles: Bool { worktree != nil && (connection.knownBuild?.offersFiles ?? false) }

    /// Every worktree can take another terminal — including this one when the
    /// pane in front of you is the Diff, which has no terminal behind it and is
    /// a perfectly ordinary place to decide you want one.
    private var canCreateTerminal: Bool { worktree != nil }

    private var canRemove: Bool {
        guard let worktree else { return false }
        // Offering to remove the repository's own checkout would offer to
        // delete the directory the repository itself lives in.
        return !worktree.isPrimaryCheckout
    }

    /// Whose Claude login this pane's agent runs on, in plain words (ov-597);
    /// nil on a runner where only the owner has one. Read as who this phone
    /// is once the runner has said, by name until then.
    private var loginSentence: String? {
        live?.claudeLogin?.sentence(viewer: viewer)
    }

    private var overflowMenu: some View {
        Menu {
            // The login rides as the first section's title, so it reads as
            // information about the pane rather than as something to tap, and
            // the system wraps or truncates a long name at any width.
            if let loginSentence {
                Section(loginSentence) { overflowItems }
            } else {
                overflowItems
            }
        } label: {
            Image(systemName: "ellipsis")
        }
        .accessibilityLabel("More")
        // The identifier the UI suite finds it by, unchanged: an identifier
        // isn't spoken, and VoiceOver hears "More".
        .accessibilityIdentifier("Pane options")
    }

    @ViewBuilder
    private var overflowItems: some View {
        // The worktree's tabs, where the bottom bar's column was (ov-617).
        if switchesTabs, let tabs {
            // Each tab with its status mark, so one that needs you shows, and
            // the one up checked.
            Picker("Tabs", selection: Binding(get: { tabs.current ?? "" }, set: { tabs.choose($0) })) {
                ForEach(tabs.tabs) { tab in
                    Label { Text(tab.title) } icon: { markImage(tab.mark) }
                        .tag(tab.id)
                }
            }
            .pickerStyle(.inline)
        }
        if canCreateTerminal, let worktree { newTerminalItem(worktree) }
        if canBrowseFiles {
            Button {
                browsingFiles = true
            } label: {
                Label("Files", systemImage: "folder")
            }
            .accessibilityIdentifier("pane-files")
        }
        if let tab = closable, let tabs {
            Button(role: .destructive) {
                tabs.close(tab)
            } label: {
                Label("Close Terminal", systemImage: "xmark")
            }
            .accessibilityIdentifier("pane-close-terminal")
        }
        if canRemove, let worktree {
            Divider()  // style-exempt: menu
            Button(role: .destructive) {
                removing = .confirming(worktree, on: connection)
            } label: {
                Label("Remove Worktree…", systemImage: "trash")
            }
        }
    }

    /// Another terminal in this worktree.
    ///
    /// **The capability existed everywhere except here.** `terminal.create` has
    /// been a wire method since the protocol had one (`proto/farcooler.proto`,
    /// tag 24); the CLI, the Mac (⌘T, the sidebar, the palette, ⌃B c) and
    /// Android all call it, and `Connection.createTerminal` was already written
    /// on this side — with no caller. So a worktree on the phone could show
    /// its terminals, switch between them, scroll them and type into them, and
    /// the one thing you want first in a fresh worktree was the one thing there
    /// was no way to ask for. Nothing new crosses the wire for this; it is a
    /// door on a room that was already built.
    ///
    /// A menu item is the right shape: it is tapped on purpose, it can be
    /// titled, and it can put an alert up when the runner says no.
    ///
    /// ## Landing on it is part of the action
    ///
    /// `onCreated` hands the id up so the shell moves to the new tab, which is
    /// the Mac's own argument in `openTerminalInNewLayout`: you made a terminal
    /// because you want to type in it, and leaving the selection where it was
    /// means going and finding it.
    private func newTerminalItem(_ worktree: Worktree) -> some View {
        Button {
            Task {
                switch await connection.createTerminal(in: worktree) {
                case .created(let id): onCreated(id)
                case .disconnected: newTerminalFailure = .disconnected
                case .refused: newTerminalFailure = .refused
                }
            }
        } label: {
            // "plus", the symbol the Mac's own New terminal carries in the
            // sidebar and in the palette: one action, one glyph.
            Label("New Terminal", systemImage: "plus")
        }
    }
}

/// Back, on a pane's own bar, when the worktree it's in was pushed onto the
/// phone's stack (`WorktreeScreen`). That screen hides the stack's bar, since
/// every pane has a bar of its own, so the way back has to be on this one.
struct PhoneBackItem: ViewModifier {
    @Environment(\.phoneBack) private var back

    func body(content: Content) -> some View {
        content.toolbar {
            if let back {
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: back) {
                        Image(systemName: "chevron.backward")
                            .fontWeight(.semibold)
                    }
                    .accessibilityLabel("Back")
                    .accessibilityIdentifier("worktree-back")
                }
            }
        }
    }
}
