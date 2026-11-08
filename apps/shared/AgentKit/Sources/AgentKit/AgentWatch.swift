#if canImport(SwiftUI)
import Foundation
import SwiftUI

/// What watching an engine agent's terminal asks of a runner's connection (ov-976).
///
/// Two verbs and neither can type: start the read-only stream, stop it. There is deliberately no
/// `write`, `resize` or `tell` here, so a model holding only this has no way to put a byte in the
/// agent's pane. The phone's `ClientCore` is the real implementation; a test passes a fake and
/// asserts the calls it saw.
protocol AgentWatchTransport: Sendable {
    /// The stream's name (what `stopWatch` takes), or nil when there is no session to open one on.
    /// `onEnd` gets nil when the agent's pane was reaped, "lost" when the link dropped first, and the
    /// runner's words when it could not be opened.
    func startWatch(workspace: String, item: String, onChunk: @escaping @Sendable ([UInt8]) -> Void,
                    onEnd: @escaping @Sendable (String?) -> Void) async -> String?
    func stopWatch(_ name: String) async
}

/// An emulator, as far as a watch needs one: bytes in. Whatever the program asks to have written back
/// (a cursor-position report) is never read, so it goes nowhere.
@MainActor
protocol WatchScreen: AnyObject {
    func feed(_ bytes: [UInt8])
}

/// The runner's own size marker, `ESC P > farcooler-size;<columns>;<rows> ESC \`, at the head of a watch.
enum WatchSizeMarker {
    static let limit = 64
    private static let head = Array("\u{1b}P>farcooler-size;".utf8)

    /// The pane's size and the bytes after the marker; nil if the marker hasn't fully arrived.
    static func split(_ bytes: [UInt8]) -> (columns: Int, rows: Int, rest: [UInt8])? {
        guard bytes.starts(with: head) else { return nil }
        guard let end = (head.count..<max(head.count, bytes.count - 1)).first(where: { bytes[$0] == 0x1b && bytes[$0 + 1] == 0x5c })
        else { return nil }
        let parts = String(decoding: bytes[head.count..<end], as: UTF8.self).split(separator: ";")
        guard parts.count >= 2, let columns = Int(parts[0]), let rows = Int(parts[1]) else { return nil }
        return (columns, rows, Array(bytes[(end + 2)...]))
    }
}

/// The size a watched pane is drawn at: its grid at the font's own cell size, plus the canvas's padding.
/// Never shrunk to the phone's width: a wider pane scrolls (ov-976).
enum WatchLayout {
    static func contentSize(columns: Int, rows: Int, cellWidth: Double, cellHeight: Double, padding: Double) -> (width: Double, height: Double) {
        (Double(columns) * cellWidth + padding * 2, Double(rows) * cellHeight + padding * 2)
    }
}

/// A running engine agent's terminal, to look at and nothing more (ov-976).
@MainActor
final class AgentWatchModel: ObservableObject {
    enum Phase: Equatable {
        case connecting, live
        /// The agent's pane was reaped: the work moved on.
        case finished
        /// The link dropped while the agent was still running.
        case lost
        /// The runner refused this person: only the owner may watch.
        case denied
        /// The link dropped before any picture arrived.
        case unreachable
        /// The agent isn't running (a review, a merge, one that already ended), or can't be reached.
        case unavailable(gotPicture: Bool)
    }

    @Published private(set) var phase: Phase = .connecting
    /// Moves on every feed, so a view drawing the screen knows to draw again.
    @Published private(set) var revision = 0
    private(set) var screen: (any WatchScreen)?

    private let workspace: String
    private let item: String
    private let transport: any AgentWatchTransport
    private let makeScreen: @MainActor (Int, Int) -> any WatchScreen
    private var name: String?
    private var pending: [UInt8] = []
    private var started = false

    init(workspace: String, item: String, transport: any AgentWatchTransport,
         makeScreen: @escaping @MainActor (Int, Int) -> any WatchScreen) {
        self.workspace = workspace; self.item = item
        self.transport = transport; self.makeScreen = makeScreen
    }

    func start() {
        guard !started else { return }
        started = true
        let (workspace, item) = (self.workspace, self.item)
        Task { [weak self, transport] in
            // Delivered in order: the core's pump calls these on the main queue, one after another.
            let name = await transport.startWatch(workspace: workspace, item: item, onChunk: { bytes in
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.consume(bytes) } }
            }, onEnd: { why in
                DispatchQueue.main.async { MainActor.assumeIsolated { self?.ended(why) } }
            })
            guard let self else { if let name { await transport.stopWatch(name) }; return }
            if let name { self.name = name; if !self.started { await transport.stopWatch(name) } }
            else { self.phase = .unavailable(gotPicture: false) }
        }
    }

    func stop() {
        started = false
        screen = nil
        guard let name else { return }
        self.name = nil
        Task { [transport] in await transport.stopWatch(name) }
    }

    func consume(_ bytes: [UInt8]) {
        guard started else { return }
        if let screen { screen.feed(bytes); revision += 1; return }
        // Until the marker has arrived the screen can't be sized; hold bytes (a marker is under 40).
        pending += bytes
        if let size = WatchSizeMarker.split(pending) {
            let made = makeScreen(min(max(size.columns, 1), 500), min(max(size.rows, 1), 300))
            screen = made; pending = []
            made.feed(size.rest); revision += 1; phase = .live
        } else if pending.count > WatchSizeMarker.limit {
            let made = makeScreen(80, 24)
            screen = made; made.feed(pending); pending = []; revision += 1; phase = .live
        }
    }

    func ended(_ why: String?) {
        guard started else { return }
        let saw = screen != nil
        phase = switch (why, saw) {
        case ("denied", _): .denied
        case ("lost", false): .unreachable
        case ("lost", true): .lost
        case (nil, true): .finished
        default: .unavailable(gotPicture: saw)
        }
    }

    /// What to tell the owner. Nothing while it runs.
    static func status(_ phase: Phase) -> String? {
        switch phase {
        case .connecting, .live: nil
        case .finished: "Finished"
        case .lost: "Lost the connection to this agent."
        case .denied: "Only the owner can watch an agent."
        case .unreachable: "Couldn’t reach this agent."
        case .unavailable(let saw): saw ? "Lost the connection to this agent." : "This agent isn’t running right now."
        }
    }
}
#endif
