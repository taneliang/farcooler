import AgentKit
import Foundation

// One record per Mac window (ov-248, ov-233): where it is, where it has been,
// how it's laid out, and where it sits on screen.
//
// Kept by the app itself, in `UserDefaults`, because nothing the system keeps
// can be leaned on: `@SceneStorage` and window restoration both come back
// only when "Close windows when quitting an application" is off, which it is
// not by default, and both lose a force quit or a crash mid-session. A
// window's record is written as it changes, so there's no last word to miss.

/// A window's record, as it's stored.
struct WindowSession: Equatable {
    /// The record's own version, apart from `Destination`'s. A record of any
    /// other version is ignored whole, and its window opens fresh.
    static let version = 1
    /// How many stops of history a record keeps on each side.
    static let historyLimit = 50

    /// One stop of history. `title` is what the place was called when the
    /// stop was recorded, for naming it in the menu before its runner has
    /// answered on a relaunch; the window's live names win when it has them.
    struct Entry: Equatable {
        var place: Destination
        var trail: Destination?
        var title: String?
    }

    /// What the window shows around its place.
    struct Layout: Equatable {
        var focus = false
        var navigatorHidden = false
        /// The navigator's pane heights (`navigator.split`, ov-244).
        var split = ""
    }

    /// Stable for the window's life and across relaunches.
    var id: UUID
    /// Where the window is, as `lastDestination` keeps it: its tab, agent and
    /// pane included.
    var place: Destination?
    /// Oldest first.
    var back: [Entry] = []
    /// Nearest first.
    var forward: [Entry] = []
    var layout = Layout()
    /// `NSWindow.frameDescriptor`, or nil before the window has been placed.
    var frame: String?
    var fullScreen = false
    /// When it last changed; stamped by the store.
    var savedAt = Date(timeIntervalSince1970: 0)

    // MARK: - The wire form

    var json: [String: Any] {
        var out: [String: Any] = [
            "v": Self.version, "id": id.uuidString, "savedAt": savedAt.timeIntervalSince1970,
            "back": back.suffix(Self.historyLimit).map(Self.json), "forward": forward.prefix(Self.historyLimit).map(Self.json),
            "layout": ["focus": layout.focus, "navigatorHidden": layout.navigatorHidden, "split": layout.split],
            "fullScreen": fullScreen,
        ]
        if let place { out["place"] = place.json }
        if let frame { out["frame"] = frame }
        return out
    }

    private static func json(_ entry: Entry) -> [String: Any] {
        var out: [String: Any] = ["place": entry.place.json]
        if let trail = entry.trail { out["trail"] = trail.json }
        if let title = entry.title { out["title"] = title }
        return out
    }

    /// A record from its JSON, or nil for one that isn't this version's or
    /// has no id. A stop that won't decode (a place kind this build doesn't
    /// know, a later `Destination` version) is dropped alone, so its
    /// neighbors survive.
    init?(json object: [String: Any]) {
        guard (object["v"] as? NSNumber)?.intValue == Self.version,
            let id = (object["id"] as? String).flatMap(UUID.init(uuidString:))
        else { return nil }
        self.id = id
        place = (object["place"] as? [String: Any]).flatMap(Destination.init(json:))
        back = Self.entries(object["back"])
        forward = Self.entries(object["forward"])
        let layout = object["layout"] as? [String: Any] ?? [:]
        self.layout = Layout(
            focus: layout["focus"] as? Bool ?? false, navigatorHidden: layout["navigatorHidden"] as? Bool ?? false,
            split: layout["split"] as? String ?? "")
        frame = (object["frame"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        fullScreen = object["fullScreen"] as? Bool ?? false
        savedAt = Date(timeIntervalSince1970: (object["savedAt"] as? NSNumber)?.doubleValue ?? 0)
    }

    init(id: UUID) { self.id = id }

    private static func entries(_ value: Any?) -> [Entry] {
        (value as? [[String: Any]] ?? []).compactMap { object in
            guard let place = (object["place"] as? [String: Any]).flatMap(Destination.init(json:)) else { return nil }
            return Entry(
                place: place, trail: (object["trail"] as? [String: Any]).flatMap(Destination.init(json:)),
                title: (object["title"] as? String).flatMap { $0.isEmpty ? nil : $0 })
        }
    }

    /// Every window's record, as one string.
    static func encode(_ sessions: [WindowSession]) -> String {
        let data = try? JSONSerialization.data(
            withJSONObject: sessions.map(\.json), options: [.sortedKeys, .withoutEscapingSlashes])
        return data.flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    }

    /// The records `encode` wrote: each that reads, in order. A record that
    /// doesn't is left out alone; unreadable text is no records.
    static func decode(_ text: String) -> [WindowSession] {
        guard let objects = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [[String: Any]] else { return [] }
        return objects.compactMap(WindowSession.init(json:))
    }
}

/// Every window's record, and which of them a live window holds.
@MainActor
final class WindowSessions {
    static let shared = WindowSessions()

    /// Where the records are kept.
    static let key = "window.sessions.v1"
    /// How many windows are kept. Recording another drops the oldest.
    nonisolated static let limit = 8

    private let defaults: UserDefaults
    private(set) var sessions: [WindowSession]
    /// The records a window in this run holds.
    private var claimed: Set<UUID> = []
    private var restoredAtLaunch = false
    private var writePending = false
    /// The app is quitting: every window's record is kept.
    var quitting = false

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        sessions = Self.capped(WindowSession.decode(defaults.string(forKey: Self.key) ?? ""))
    }

    /// What a window gets when it opens.
    struct Adoption: Equatable {
        /// Its record: one left by an earlier run or window, or a new one
        /// with nowhere kept yet.
        var session: WindowSession
        /// More windows to open for the records still unclaimed: only for the
        /// first window of the run, which opens every window there was.
        var open: Int
    }

    /// A window opening takes the most recent record nobody holds: the first
    /// of a launch takes the newest and says how many windows more to open;
    /// one opened later (the Dock after the last was closed) takes the one
    /// that was kept. With none to take, it's new.
    func adopt() -> Adoption {
        let unclaimed = sessions.filter { !claimed.contains($0.id) }.sorted { $0.savedAt > $1.savedAt }
        let first = !restoredAtLaunch
        restoredAtLaunch = true
        let session = unclaimed.first ?? WindowSession(id: UUID())
        claimed.insert(session.id)
        return Adoption(session: session, open: first ? max(0, unclaimed.count - 1) : 0)
    }

    /// `session` as it is now. Written once per run-loop turn, however many
    /// changes land in it.
    func update(_ session: WindowSession) {
        var stamped = session
        stamped.savedAt = Date()
        if let at = sessions.firstIndex(where: { $0.id == session.id }) {
            guard sessions[at].withoutStamp != stamped.withoutStamp else { return }
            sessions[at] = stamped
        } else {
            sessions.append(stamped)
        }
        sessions = Self.capped(sessions)
        scheduleWrite()
    }

    /// The window holding `id` closed: its record goes, unless the app is
    /// quitting or it was the last window, which the Dock reopens where it
    /// was (`afterClose`).
    func closed(_ id: UUID) {
        claimed.remove(id)
        sessions = Self.afterClose(sessions, closing: id, remaining: claimed.count, quitting: quitting)
        scheduleWrite()
    }

    /// `sessions` once the window holding `id` has closed with `remaining`
    /// others open. Closing one of several forgets it, as Safari and VS Code
    /// do; quitting keeps every window's record, and so does closing the last.
    nonisolated static func afterClose(_ sessions: [WindowSession], closing id: UUID, remaining: Int, quitting: Bool)
        -> [WindowSession]
    {
        quitting || remaining == 0 ? sessions : sessions.filter { $0.id != id }
    }

    /// The newest `limit` records, in the order they were first kept.
    nonisolated static func capped(_ sessions: [WindowSession]) -> [WindowSession] {
        guard sessions.count > limit else { return sessions }
        let keep = Set(sessions.sorted { $0.savedAt > $1.savedAt }.prefix(limit).map(\.id))
        return sessions.filter { keep.contains($0.id) }
    }

    /// Write now, whatever is pending: a test's, and a quit's.
    func flush() {
        writePending = false
        defaults.set(WindowSession.encode(sessions), forKey: Self.key)
    }

    private func scheduleWrite() {
        guard !writePending else { return }
        writePending = true
        DispatchQueue.main.async { [self] in if writePending { flush() } }
    }
}

private extension WindowSession {
    /// The record without its stamp, to tell a change from a repeat.
    var withoutStamp: WindowSession {
        var copy = self
        copy.savedAt = Date(timeIntervalSince1970: 0)
        return copy
    }
}
