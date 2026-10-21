#if DEBUG
import Foundation

/// `-native-live` (ov-626): a busy live conversation for `NativeAgentHarness`
/// (with `-native-live-older`, older rows above, answered when the test lets them through),
/// to hold a reader's place against. The page is long enough to scroll, with
/// older rows above it to load; every follow streams: a new line of reply,
/// the reply written on in place, or a tool call that regroups the rows; a
/// sent message shows as a turn six follows later, so a Queued echo settles
/// while the reader is elsewhere (with `-native-busy`); and every fifth
/// follow is a reset, which makes the pane read its page again, as a resync
/// does (ov-612).
struct NativeLiveRows {
    /// Holds the older page until the test posts `native-older`, so the
    /// reader's place is taken before it comes; it gives up after three
    /// minutes so a test that never posts can't hang the runner.
    actor OlderGate {
        private var isOpen = false

        func open() { isOpen = true }

        func wait() async {
            for _ in 0..<18_000 where !isOpen { try? await Task.sleep(for: .milliseconds(10)) }
        }
    }

    static let olderGate = OlderGate()

    static var isRequested: Bool { CommandLine.arguments.contains("-native-live") }

    /// The streamed lines so far, and the turns still to show, by how many
    /// follows they wait.
    private var streamed: [[String: Any]] = []
    private var waiting: [(text: String, follows: Int)] = []
    private var follows = 0

    private static func row(_ id: String, ord: UInt64, rev: UInt64, kind: [String: Any]) -> [String: Any] {
        ["id": id, "ord": ord, "rev": rev, "provisional": false, "kind": kind]
    }

    private static func prose(_ id: String, ord: UInt64, rev: UInt64, _ text: String) -> [String: Any] {
        row(id, ord: ord, rev: rev, kind: ["Prose": ["text": text, "conclusion": false]])
    }

    /// Forty paragraphs, then what has streamed and been shown since.
    func page(epoch: UInt64, rev: UInt64) -> [String: Any] {
        var rows = (0..<40).map { i in
            Self.prose("prose:live\(i)", ord: 100 + UInt64(i), rev: 1, "Paragraph \(i) of a long session: it reads the parser, notes what changed, and goes on.")
        }
        rows += streamed
        // Older rows above only with `-native-live-older`, so they can't land
        // in the middle of another test's watch.
        return ["epoch": epoch, "rev": rev, "moreBefore": CommandLine.arguments.contains("-native-live-older"), "rows": rows]
    }

    /// The fifteen paragraphs above the page, and nothing above them; with
    /// `-native-live-older-on-scroll`, a full page of a hundred (what a real
    /// runner sends), which pushes the reader's rows several screens down.
    func older(epoch: UInt64, rev: UInt64) -> [String: Any] {
        let big = CommandLine.arguments.contains("-native-live-older-on-scroll")
        let rows = (0..<(big ? 100 : 15)).map { i in
            Self.prose("prose:old\(i)", ord: (big ? 0 : 50) + UInt64(i), rev: 1, "Older paragraph \(i), loaded when the reader reached the top.")
        }
        return ["epoch": epoch, "rev": rev, "moreBefore": false, "rows": rows]
    }

    mutating func sent(_ text: String) { waiting.append((text, 6)) }

    /// One follow's answer: a new streamed line, any turn whose wait is over,
    /// or, every fifth, a reset.
    mutating func follow(epoch: UInt64, rev: inout UInt64, turn: (String) -> [String: Any]) -> [String: Any] {
        follows += 1
        var changes: [[String: Any]] = []
        rev += 1
        if follows % 3 == 0 {
            // A tool call, which joins a run of them and regroups the rows.
            let id = "tool:live\(rev)"
            let tool = Self.row(id, ord: 1_000 + rev, rev: rev, kind: ["Tool": [
                "name": "Bash", "summary": "Run step \(rev)", "status": "Done", "started_ms": 1, "ended_ms": 400, "diff": [Any](),
                "input": "command: true", "result": "",
            ]])
            streamed.append(tool)
            changes.append(["kind": "insert", "id": id, "rev": rev, "row": tool])
        } else if let last = streamed.last, (last["id"] as? String)?.hasPrefix("prose:stream") == true, follows % 3 == 2 {
            // The reply being written: the same row, longer.
            let id = last["id"] as! String
            let grown = Self.prose(id, ord: last["ord"] as! UInt64, rev: rev, "Streamed reply, written on: " + String(repeating: "more words ", count: follows))
            streamed[streamed.count - 1] = grown
            changes.append(["kind": "update", "id": id, "rev": rev, "row": grown])
        } else {
            let line = Self.prose("prose:stream\(rev)", ord: 1_000 + rev, rev: rev, "Streamed line \(rev) of the reply.")
            streamed.append(line)
            changes.append(["kind": "insert", "id": "prose:stream\(rev)", "rev": rev, "row": line])
        }
        waiting = waiting.map { ($0.text, $0.follows - 1) }
        for shown in waiting where shown.follows <= 0 {
            rev += 1
            let id = "turn:live\(rev)"
            let row = Self.row(id, ord: 1_000 + rev, rev: rev, kind: ["Turn": turn(shown.text)])
            streamed.append(row)
            changes.append(["kind": "insert", "id": id, "rev": rev, "row": row])
        }
        waiting.removeAll { $0.follows <= 0 }
        return ["epoch": epoch, "rev": rev, "reset": follows % 5 == 0, "changes": changes]
    }
}
#endif
