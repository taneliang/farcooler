import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Each window's own record (ov-248, ov-233): what it keeps, what a damaged
/// or later one costs, which window takes which on a relaunch, and what
/// closing one does to it.
@MainActor
struct WindowSessionTests {
    private typealias Selection = ContentView.Selection

    private static let ws = "billing"
    private static func task(_ id: String) -> Destination {
        Destination(runner: .init(host: ""), place: .task(workspace: ws, task: .init(id: id)))
    }

    private static func session(_ place: String, at seconds: Double, back: [String] = []) -> WindowSession {
        var session = WindowSession(id: UUID())
        session.place = task(place)
        session.back = back.map { WindowSession.Entry(place: task($0), trail: nil, title: "bil-\($0)") }
        session.savedAt = Date(timeIntervalSince1970: seconds)
        return session
    }

    /// A store on its own suite, so a run never touches the app's defaults.
    private static func store(_ name: String = #function) -> (WindowSessions, UserDefaults) {
        let suite = "farcooler.test.sessions.\(name)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        return (WindowSessions(defaults: defaults), defaults)
    }

    @Test("A record keeps its place, both sides of history, its layout and its frame, and reads back as itself")
    func roundTrips() throws {
        var kept = Self.session("t1", at: 100, back: ["a", "b"])
        kept.forward = [WindowSession.Entry(place: Self.task("f"), trail: Self.task("t"), title: nil)]
        kept.layout = .init(focus: true, navigatorHidden: true, split: "0.4,0.6")
        kept.frame = "100 200 900 700 0 0 1440 900"
        kept.fullScreen = true
        let back = try #require(WindowSession.decode(WindowSession.encode([kept])).first)
        #expect(back == kept)
    }

    @Test("Two windows' records come back as two places")
    func twoWindows() {
        let one = Self.session("t1", at: 1), two = Self.session("t2", at: 2)
        let read = WindowSession.decode(WindowSession.encode([one, two]))
        #expect(read.map(\.place) == [Self.task("t1"), Self.task("t2")])
        #expect(Set(read.map(\.id)).count == 2)
    }

    @Test("A record of another version is ignored whole; one stop that won't read is dropped alone")
    func damage() throws {
        var later = Self.session("t1", at: 1).json
        later["v"] = 2
        var damaged = Self.session("t2", at: 2, back: ["a", "b", "c"]).json
        var back = try #require(damaged["back"] as? [[String: Any]])
        back[1]["place"] = ["v": 9, "place": ["kind": "unheard-of"]]
        damaged["back"] = back
        let text = String(
            data: try JSONSerialization.data(withJSONObject: [later, damaged, ["v": 1]]), encoding: .utf8)!
        let read = WindowSession.decode(text)
        #expect(read.count == 1, "the later version and the one with no id are ignored")
        #expect(read.first?.back.map(\.place) == [Self.task("a"), Self.task("c")])
        #expect(WindowSession.decode("not json").isEmpty)
    }

    @Test("A record keeps fifty stops a side: the nearest")
    func historyCapped() throws {
        var long = Self.session("t", at: 1, back: (0..<80).map { "b\($0)" })
        long.forward = (0..<80).map { WindowSession.Entry(place: Self.task("f\($0)"), trail: nil, title: nil) }
        let back = try #require(WindowSession.decode(WindowSession.encode([long])).first)
        #expect(back.back.count == 50 && back.back.first?.place == Self.task("b30") && back.back.last?.place == Self.task("b79"))
        #expect(back.forward.count == 50 && back.forward.first?.place == Self.task("f0"))
    }

    @Test("Closing one of several windows forgets it; quitting keeps every one; closing the last keeps it")
    func afterClose() {
        let one = Self.session("t1", at: 1), two = Self.session("t2", at: 2)
        let all = [one, two]
        #expect(WindowSessions.afterClose(all, closing: one.id, remaining: 1, quitting: false) == [two])
        #expect(WindowSessions.afterClose(all, closing: one.id, remaining: 1, quitting: true) == all)
        #expect(WindowSessions.afterClose(all, closing: two.id, remaining: 0, quitting: false) == all)
    }

    @Test("Nine windows keep the eight most recent")
    func capped() {
        let nine = (0..<9).map { Self.session("t\($0)", at: Double($0 + 1)) }
        let kept = WindowSessions.capped(nine)
        #expect(kept.count == WindowSessions.limit)
        #expect(!kept.contains { $0.id == nine[0].id })
    }

    @Test("A launch's first window takes the newest record and opens the rest; later ones take what's left")
    func adopting() {
        let (store, defaults) = Self.store()
        let old = Self.session("t1", at: 1), mid = Self.session("t2", at: 2), new = Self.session("t3", at: 3)
        defaults.set(WindowSession.encode([old, new, mid]), forKey: WindowSessions.key)
        let reloaded = WindowSessions(defaults: defaults)
        let first = reloaded.adopt()
        #expect(first.session.id == new.id && first.open == 2)
        #expect(reloaded.adopt() == .init(session: mid, open: 0))
        #expect(reloaded.adopt() == .init(session: old, open: 0))
        // Nothing left: a window of its own, with nowhere kept.
        let fresh = reloaded.adopt()
        #expect(fresh.session.place == nil && fresh.open == 0)
        _ = store
    }

    @Test("The Dock reopening after the last window closed takes the record that was kept")
    func dockReopen() {
        let (_, defaults) = Self.store()
        let last = Self.session("t1", at: 5)
        defaults.set(WindowSession.encode([last]), forKey: WindowSessions.key)
        let sessions = WindowSessions(defaults: defaults)
        let first = sessions.adopt()
        #expect(first.session.id == last.id && first.open == 0)
        sessions.closed(last.id)
        #expect(sessions.sessions.map(\.id) == [last.id], "the last window's record stays")
        let reopened = sessions.adopt()
        #expect(reopened.session.id == last.id && reopened.open == 0)
    }

    @Test("Closing a window while another is open forgets it, and quitting doesn't")
    func closing() {
        for quitting in [false, true] {
            let (_, defaults) = Self.store("closing\(quitting)")
            let a = Self.session("t1", at: 1), b = Self.session("t2", at: 2)
            defaults.set(WindowSession.encode([a, b]), forKey: WindowSessions.key)
            let sessions = WindowSessions(defaults: defaults)
            _ = sessions.adopt()
            _ = sessions.adopt()
            sessions.quitting = quitting
            sessions.closed(a.id)
            sessions.flush()
            let stored = WindowSession.decode(defaults.string(forKey: WindowSessions.key) ?? "")
            #expect(stored.count == (quitting ? 2 : 1), "quitting \(quitting)")
        }
    }

    @Test("A change is written; a repeat of it is not")
    func updates() {
        let (sessions, defaults) = Self.store()
        var record = WindowSession(id: UUID())
        record.place = Self.task("t1")
        sessions.update(record)
        sessions.flush()
        let written = defaults.string(forKey: WindowSessions.key)
        #expect(WindowSession.decode(written ?? "").first?.place == Self.task("t1"))
        let stamp = sessions.sessions[0].savedAt
        sessions.update(record)
        #expect(sessions.sessions[0].savedAt == stamp, "an unchanged record keeps its stamp")
        record.layout.split = "0.5,0.5"
        sessions.update(record)
        #expect(sessions.sessions[0].layout.split == "0.5,0.5")
    }

    // MARK: - History, kept

    private static let names = HistoryMenu.Names(
        workspace: { _, _ in nil }, task: { _, _, id in "live \(id)" }, worktree: { _, _ in nil })

    @Test("A window's history is kept as stops and read back as the same history, with the names it had")
    func historyRoundTrips() {
        func at(_ id: String) -> Selection { .workspace(host: "", workspace: Self.ws, focus: .task(id)) }
        var history = NavigationHistory()
        history.record(from: at("a"), to: at("b"))
        history.record(from: at("b"), to: at("c"), trail: at("z"))
        _ = history.goBack(from: at("c"), resolves: { _ in true })
        history.record(from: at("c"), to: at("b"))
        var record = WindowSession(id: UUID())
        record.savedAt = Date(timeIntervalSince1970: 1)
        record.back = ContentView.entries(history.back, names: Self.names)
        record.forward = ContentView.entries(history.forward.reversed(), names: Self.names)
        let read = WindowSession.decode(WindowSession.encode([record]))[0]
        #expect(ContentView.history(of: read) == history)
        #expect(ContentView.titles(of: read)[at("a")] == "live a")
        #expect(history.forward.count == 1 && read.forward.count == 1)
    }

    @Test("History read back drops a repeat, and a terminal that no window keeps as a place")
    func historyReadsCleanly() {
        var record = WindowSession(id: UUID())
        record.back = [Self.task("a"), Self.task("a"), Destination(runner: .init(host: ""), place: .terminal("t")), Self.task("b")]
            .map { WindowSession.Entry(place: $0, trail: nil, title: nil) }
        let history = ContentView.history(of: record)
        #expect(history.back.count == 2)
    }
}
