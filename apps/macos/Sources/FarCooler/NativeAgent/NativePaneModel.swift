import AgentKit
import AppKit
import Foundation
import SwiftUI

/// Where the native composer's message goes (ov-372): `terminal.compose`,
/// which types it into the TUI's box and presses Enter past the same gate as
/// `terminal tell`, or refuses with a word and types nothing. With the
/// runner's `compose` (ov-367), its line breaks, images and slash command
/// too; the client core uploads the images first where the runner takes
/// that (ov-393). True when the agent was working and claude's own queue
/// took it (R-29).
protocol ComposeSink: Sendable {
    /// With `files` (ov-454): any kind, written on the runner and their paths
    /// typed first; sent only where the runner offers `compose_files`.
    func compose(terminal: String, text: String, images: [ComposeImage], files: [ComposeFile]) async throws -> Bool
}

extension ComposeSink {
    func compose(terminal: String, text: String, images: [ComposeImage] = []) async throws -> Bool {
        try await compose(terminal: terminal, text: text, images: images, files: [])
    }
}

extension RunnerCore: ComposeSink {
    func compose(terminal: String, text: String, images: [ComposeImage], files: [ComposeFile]) async throws -> Bool {
        var args: [String: any Sendable] = ["terminal": terminal, "text": text]
        if !images.isEmpty {
            args["images"] = images.map { ["mime": $0.mime, "base64": $0.data.base64EncodedString()] }
        }
        if !files.isEmpty {
            args["files"] = files.map { ["name": $0.name, "base64": $0.data.base64EncodedString()] }
        }
        let data = try await call("terminal.compose", args)
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        return object["queued"] as? Bool ?? false
    }
}

/// One terminal-mode claude or codex pane's native side: its rows, the composer's
/// draft, and which of the two views is showing (ov-372).
///
/// Held by `NativeAgents` for the life of the app rather than by a view, so
/// a layout change, a tile move or switching views never loses the draft or
/// starts the rows over. Nothing here touches the pane's process: the
/// terminal under the native view is the same tmux pane, never respawned.
@MainActor
final class NativePaneModel: ObservableObject {
    let terminal: String
    let store: AgentRowStore
    /// The agent the pane runs, as its program says: `claude` or `codex`
    /// (ov-416).
    @Published var program = "claude"
    /// The agent's name, as the view's words say it.
    var agent: String { AgentConversation.agentName(preset: program) }

    /// The composer's text. Against a runner without `compose`, line breaks
    /// become spaces as they arrive: it takes one line, so what you see is
    /// what's sent.
    @Published var draft = "" {
        didSet {
            draftKeeper?.changed(draft)
            if !rich, draft.contains(where: \.isNewline) {
                draft = draft.replacingOccurrences(of: "\r\n", with: " ").replacingOccurrences(of: "\n", with: " ")
                    .replacingOccurrences(of: "\r", with: " ")
            }
        }
    }
    /// Whether the runner takes line breaks, images and slash commands
    /// (`compose`, ov-367), as its hello said. Without it, one line.
    @Published var rich = false {
        didSet {
            guard !rich else { return }
            images = []
            files = []
            draft = draft
        }
    }
    /// Images to send with the text, in order (ov-400). Only with `rich`.
    @Published private(set) var images: [ComposeImage] = []
    /// Whether the runner takes files of any kind (`compose_files`, ov-454).
    /// Without it, a file that isn't an image drops into the text as its
    /// path, as it always did.
    @Published var takesFiles = false {
        didSet { if !takesFiles { files = [] } }
    }
    /// Files to send with the text, in order (ov-454). Only with `takesFiles`.
    @Published private(set) var files: [ComposeFile] = []
    /// Each image's chip picture, made once as it's added.
    private(set) var thumbnails: [UUID: NSImage] = [:]
    @Published private(set) var sending = false
    /// What stopped the last send, until the next one or a dismissal.
    @Published var issue: SendIssue?
    /// Messages claude's queue took that its transcript hasn't shown yet,
    /// drawn as Queued rows below the list.
    @Published private(set) var queued: [String] = []
    /// Whether this pane shows the native view; remembered per pane.
    @Published var showsNative: Bool {
        didSet {
            NativePaneModel.remember(showsNative, for: terminal)
            followIfShown()
        }
    }

    /// Where sends go: the runner's connection, replaced on a reconnect.
    var sink: (any ComposeSink)?
    /// The prompts' images, fetched from the runner where it serves them
    /// (ov-454); its source set with the connection.
    let promptImages: PromptImageStore
    /// Where Stop and Send Now go (ov-368): the runner's connection, where
    /// it serves `terminal_interrupt`; nil, and neither is offered, where not.
    @Published var keys: (any InterruptSink)?
    /// A Stop or a Send Now on its way, until the runner answers.
    @Published var pressing: PaneKey?
    /// Where Bring Here reads and clears claude's box (ov-369): the
    /// runner's connection, where it serves `bring_draft`; nil, and only
    /// Show Terminal is offered, where not.
    @Published var drafts: (any DraftSink)?
    /// A Bring Here on its way, until the runner answers the clear.
    @Published var bringing = false
    /// Where a held ask's answer goes (ov-370): the runner's connection.
    @Published var answers: (any AgentAnswerSink)?
    /// The held ask whose answer is on its way, by its id.
    @Published var answering: String?
    /// Why an ask's answer didn't land, by the ask's id.
    @Published var answerIssues: [String: String] = [:]
    /// Where rows come from, once the runner is connected.
    var source: (any AgentRowSource)? {
        didSet { if source != nil { following = false } }
    }
    /// Whether `store` follows `source` now.
    private var following = false

    /// The views that say they show this pane, one token per mount.
    private var screens: Set<UUID> = []

    /// Whether this pane is on screen: mounted, not behind a zoomed pane or
    /// another workspace's, in a window somebody can see
    /// (`NativeSwitch.onScreen`). Off until its view says so, and off again
    /// when the view goes. A pane whose view is only remembered as the
    /// conversation holds no follow: each is a held call on the runner, and
    /// the runner takes 32 connections.
    ///
    /// A terminal can be mounted twice (a hidden layer and a tile): it's on
    /// screen while any mount says so, and one going away can't turn off
    /// another's.
    var onScreen: Bool { !screens.isEmpty }

    /// `view` says whether it shows this pane.
    func setOnScreen(_ shown: Bool, by view: UUID) {
        let before = onScreen
        if shown { screens.insert(view) } else { screens.remove(view) }
        if onScreen != before { followIfShown() }
    }

    /// Follow while the conversation is on screen, and stop when it isn't:
    /// one held follow per pane the person can see, not per claude pane in
    /// the app. The rows stay in the store, and a return follows from where
    /// it left off.
    ///
    /// Also what brings back a follow that ended `.unavailable` (the runner's
    /// projector was off, or the pane was gone) once the pane is shown again:
    /// that loop returned, and nothing else would start it.
    func followIfShown() {
        guard let source else { return }
        let wanted = showsNative && onScreen
        if wanted, following, store.phase == .unavailable {
            store.start(source)
        } else if wanted, !following {
            following = true
            store.start(source)
        } else if !wanted, following {
            following = false
            store.stop()
        }
    }

    /// Keeps the draft on disk per terminal (ov-369 F4, R-38); nil in tests
    /// that don't care.
    let draftKeeper: DraftKeeper?

    init(terminal: String, store: AgentRowStore, sink: (any ComposeSink)?, draftKeeper: DraftKeeper? = nil) {
        self.terminal = terminal
        self.store = store
        self.sink = sink
        self.draftKeeper = draftKeeper
        promptImages = PromptImageStore(terminal: terminal, source: nil)
        showsNative = NativePaneModel.remembered(for: terminal)
        if let draftKeeper { draft = draftKeeper.restored }
    }

    /// The longest message the box takes from here (`tell.rs`'s
    /// `LONGEST_MESSAGE`).
    static let longest = 500
    /// The longest with `compose` (`compose.rs`'s `LONGEST_TEXT`).
    static let longestComposed = 100_000
    /// The most images in one message (`compose.rs`'s `MOST_IMAGES`).
    static let mostImages = 10

    /// The longest message the box takes now.
    var longestNow: Int { rich ? Self.longestComposed : Self.longest }

    /// Add `new` after the images already waiting, up to `mostImages`. False
    /// when the runner takes no images, so a paste or a drop goes to the
    /// text instead.
    @discardableResult
    func attach(_ new: [ComposeImage]) -> Bool {
        guard rich else { return false }
        let room = max(0, Self.mostImages - images.count)
        for image in new.prefix(room) {
            thumbnails[image.id] = image.thumbnail()
            images.append(image)
        }
        if new.count > room { issue = .said(Self.tooManyImages) }
        return true
    }

    /// Take the image or the file `id` out of the message.
    func detach(_ id: UUID) {
        images.removeAll { $0.id == id }
        files.removeAll { $0.id == id }
        thumbnails[id] = nil
    }

    /// Add the files at `urls` after those already waiting, up to
    /// `mostImages`; one too large is left out, with its sentence. False when
    /// the runner takes no files, so a drop goes to the text instead.
    @discardableResult
    func attach(fileURLs urls: [URL]) -> Bool {
        guard rich, takesFiles else { return false }
        let room = max(0, Self.mostImages - files.count)
        var tooLarge = false
        for url in urls.prefix(room) {
            let (file, large) = ComposeFile.read(url)
            tooLarge = tooLarge || large
            if let file { files.append(file) }
        }
        if tooLarge { issue = .said(Self.fileTooLarge) } else if urls.count > room { issue = .said(Self.tooManyFiles) }
        return true
    }

    /// Why a message wasn't sent, as the composer says it.
    enum SendIssue: Equatable {
        /// Claude is showing a question, a menu or a panel only the terminal
        /// can draw: the Handoff row, with Show Terminal.
        case handoff
        /// The message is one of claude's own commands that opens a panel or
        /// acts at once (`handoff`): the Handoff row, with Show Terminal.
        case panel
        /// The terminal's box holds text of its own (R-28): Bring Here where
        /// it's offered, and Show Terminal.
        case draftInTerminal
        /// Bring Here put the box's text in the composer but couldn't clear
        /// the box: the text is in both, as this says.
        case draftLeftInTerminal(String)
        /// Something only words can say.
        case said(String)
    }

    /// The draft as it's sent: trimmed of spaces at the ends on one line;
    /// with `compose`, as typed, the runner trimming the ends but keeping an
    /// indent.
    private var outgoing: String {
        rich ? draft : draft.trimmingCharacters(in: .whitespaces)
    }

    /// Whether the draft has anything but white space in it.
    private var hasText: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var canSend: Bool {
        !sending && (hasText || !images.isEmpty || !files.isEmpty) && outgoing.count <= longestNow && sink != nil && !store.isStale
    }

    /// Send the draft and its images. Return's action.
    func send() async {
        let text = outgoing
        let images = self.images
        let files = self.files
        guard canSend, let sink else {
            if outgoing.count > longestNow { issue = .said(tooLongNow) }
            return
        }
        // Without `compose`, a slash or a bang would open claude's command
        // picker or its shell, which Enter would then run. With it, the
        // runner drives the picker, and refuses what it can't.
        if !rich, let first = text.first, "/!#@&$?\\".contains(first) {
            issue = .said(Self.command)
            return
        }
        sending = true
        issue = nil
        defer { sending = false }
        do {
            let wasQueued = try await sink.compose(terminal: terminal, text: text, images: images, files: files)
            if outgoing == text { draft = "" }
            for image in images { detach(image.id) }
            for file in files { detach(file.id) }
            if wasQueued { queued.append(Self.echo(text, images: images.count)) }
        } catch {
            issue = Self.issue(for: error, command: text.trimmingCharacters(in: .whitespaces).hasPrefix("/"), agent: agent)
        }
    }

    /// A Queued row's words until the transcript shows the message: each
    /// image as `[Image]`, then the text.
    static func echo(_ text: String, images: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return (Array(repeating: "[Image]", count: images) + (trimmed.isEmpty ? [] : [trimmed])).joined(separator: " ")
    }

    /// What an echo and the transcript's row for it share: how many images
    /// the message has (claude's `[Image #N]`, the echo's `[Image]`), and its
    /// words without them or the white space around them. So an echo of two
    /// images is never taken for a row of one.
    static func words(_ text: String) -> String {
        let placeholder = #"\[Image( #\d+)?\]"#
        let images = (try? Regex(placeholder)).map { text.ranges(of: $0).count } ?? 0
        let words = text.replacingOccurrences(of: placeholder, with: "", options: .regularExpression)
            .replacingOccurrences(of: composedFile, with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(images) \(words)"
    }

    /// The page above the oldest row held.
    func loadOlder() {
        if let source { store.loadOlder(source) }
    }

    /// Drop a local Queued echo once the transcript shows the message, as a
    /// Queued row or as the turn it became.
    func settleQueued() {
        guard !queued.isEmpty else { return }
        let shown = Set(store.ids.suffix(40).compactMap { store.box($0)?.row }.compactMap { row -> String? in
            switch row.kind {
            case .queued(let q): q.text
            case .turn(let t): t.prompt
            default: nil
            }
        })
        let words = Set(shown.map(Self.words))
        queued.removeAll { words.contains(Self.words($0)) }
    }

    /// `command` when the message was a slash command, which says which
    /// limit `images` means.
    static func issue(for error: Error, command: Bool = false, agent: String = "Claude") -> SendIssue {
        let failure = error as? RunnerCore.Failure
        switch failure?.what {
        case "prompt", "dialog": return .handoff
        case "handoff": return .panel
        case "draft": return .draftInTerminal
        case "typing": return .said("Someone typed in the terminal in the last 3 seconds, so the message wasn’t sent. Try again once they stop.")
        case "busy": return .said("\(agent) is working and can’t take a message from here right now.")
        case "too_long": return .said(tooLong)
        case "command": return .said(AgentConversation.commandRefused(agent))
        case "paste_left": return .said("The message didn’t land in the box as typed, so it was left there and not sent.")
        case "left_at_shell": return .said("\(agent) quit as the message was typed. It wasn’t run.")
        case "unconfirmed": return .said(unconfirmed(agent))
        case "not_running", "not_an_agent": return .said("\(agent) isn’t running in this pane.")
        case "unfamiliar", "unproven": return .said("Far Cooler can’t read this terminal’s box, so nothing was typed.")
        case "images_too_large": return .said(imagesTooLarge)
        case "images": return .said(command ? commandWithImages : tooManyImages)
        case "image_too_large": return .said(imageTooLarge)
        case "backslash": return .said(backslash)
        case "image": return .said("One of the images couldn’t be read, so nothing was sent.")
        case "files": return .said(command ? commandWithFiles : tooManyFiles)
        case "file": return .said("One of the files didn’t reach the runner, so nothing was sent.")
        case "file_too_large": return .said(fileTooLarge)
        case "files_too_large": return .said(filesTooLarge)
        case "unconfirmable": return .said("Far Cooler can’t find \(agent)’s session to confirm a send, so nothing was typed.")
        case "unsupported": return .said("\(agent) can’t take a message from here. Use the terminal.")
        case "picker": return .said(AgentConversation.picker(agent))
        case "too_tall": return .said(AgentConversation.tooTall(agent))
        default:
            switch failure {
            // Never "wasn't sent" for a call that may have arrived: the
            // runner may type it yet, and a second send would go in twice.
            case .timedOut?, .lost(_, notSent: false)?:
                return .said(mayHaveBeenSent)
            case .lost(_, notSent: true)?, .notConnected?:
                return .said("The runner isn’t connected, so the message wasn’t sent.")
            default:
                return .said("The message wasn’t sent.")
            }
        }
    }

    static let tooLong = "That message is over \(longest) characters. Shorten it, or paste it in the terminal."
    static let tooLongComposed = "That message is over 100,000 characters. Shorten it, or paste it in the terminal."
    var tooLongNow: String { rich ? Self.tooLongComposed : Self.tooLong }
    static let command = "A message can’t start with a symbol Claude reads as a command, such as / or !. Use the terminal for commands."
    /// The runner's `command`: a `!`, which claude's box runs in a shell, or
    /// a `/` before something that isn't a command's name.
    static let commandRefused = "Claude would run that as a shell command or doesn’t have that command, so it wasn’t sent. Use the terminal for it."
    static let imagesTooLarge = "These images are too large to send together. Send fewer or smaller ones."
    static let tooManyImages = "A message takes at most \(mostImages) images."
    static let commandWithImages = "A slash command can’t carry images. Send it without them."
    static let commandWithFiles = "A slash command can’t carry files. Send it without them."
    static let imageTooLarge = "That image is too large to send. Use a smaller one."
    static let fileTooLarge = "That file is too large to send. Files up to 16 MB work."
    static let filesTooLarge = "These files are too large to send together. Send fewer or smaller ones."
    static let tooManyFiles = "A message takes at most \(mostImages) files."
    /// A file's path as the runner types it before the text (ov-454): its
    /// copy in the paste directory, quoted where it has a space. An echo
    /// carries no paths, so they're left out of what it's matched by.
    static let composedFile = #"(?:"[^"]*/compose-[^"]*"|\S*/compose-\S*)\s*"#
    static let backslash = "Claude reads a backslash at the end as a new line, so the message wasn’t sent. Remove it, or add a word after it."
    static let unconfirmed = unconfirmed("Claude")
    static func unconfirmed(_ agent: String) -> String {
        "\(agent) didn’t confirm it took the message. Check the pane before sending it again."
    }
    static let mayHaveBeenSent = "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again."

    // MARK: - The view each pane remembers (R-27)

    /// One key per pane, a Bool, so a capture can set it like any other
    /// (`FARCOOLER_CAPTURE_DEFAULTS`).
    static func key(for terminal: String) -> String { "nativeAgent.view.\(terminal)" }

    /// Terminal on the Mac until a pane was switched (R-27).
    static func remembered(for terminal: String, defaults: UserDefaults = .standard) -> Bool {
        defaults.bool(forKey: key(for: terminal))
    }

    static func remember(_ native: Bool, for terminal: String, defaults: UserDefaults = .standard) {
        if native { defaults.set(true, forKey: key(for: terminal)) } else { defaults.removeObject(forKey: key(for: terminal)) }
    }
}
