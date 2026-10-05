#if DEBUG
import Foundation

// The canned runner's plan layer, for the UI suite (`-phone-harness` with
// `-phone-plan`, ov-274).
//
// What it answers with is a real board's plan: `test/fixtures/plan-seeded.json`
// is `farcooler plan --json` and each theme's and lane's `show --json`, read
// from a scratch daemon seeded by `.claude/agent/reports/ov-273/seed.sh`, so the
// bytes the phone decodes are the CLI's. The file's path comes in as
// `-phone-plan-file`: a simulator reads the Mac's disk.
//
//   -phone-plan           the runner advertises `board_plan` and answers `plan.get`
//                         and `plan.events` from the file
//   -phone-plan-fails     it advertises `board_plan` and refuses `plan.get`
//   -phone-plan-hangs     it advertises `board_plan` and never answers `plan.get`
//   a Darwin notification `com.farcooler.harness.plan-news` (or `.task-news`) delivers
//                         a `plan` (or `task`) notice for Billing's board, as the client core
//                         queues one, and the plan the runner answers with from then on is
//                         a new one: its first Next Up lane is called "<name>-v2". Each
//                         `plan.get` and `plan.events` is in the harness's `sent`.
//   -phone-plan-timeout N the phone gives up on a read after N seconds, not 15
//   -phone-plan-outcomes  the first three themes carry an outcome of one line, of two and of
//                         far too many, and no Next or Needs You line, so a test measures
//                         how many lines an outcome gets from the rows' heights
//   -phone-pages          the runner also advertises `board_pages` and answers `page.list`
//                         from the file's `pages`, and Billing's board also holds the file's
//                         `tasks`, so a page's references to them draw live (ov-285). With
//                         `-phone-plan-file test/fixtures/pages-seeded.json`, read from a
//                         scratch daemon seeded by `.claude/agent/reports/ov-284/seed-pages.sh`
//   -phone-pages-fails    it advertises `board_pages` and refuses `page.list`
//   -phone-rulings        the runner also advertises `board_rulings` (ov-304), so the plan's
//                         `rulings` draw as Decided For You. With `-phone-plan-file
//                         test/fixtures/plan-rulings-seeded.json`: `farcooler plan --json` from a
//                         scratch daemon given four rulings with `plan ruling add` and `set`
//
// Without any of these the runner is one from before the plan layer: no
// `board_plan`, so the board has no control.

struct HarnessPlan {
    /// How many notices have moved the plan: the lane the answer renames.
    static var version = 0

    static var advertised: Bool {
        let arguments = CommandLine.arguments
        return arguments.contains("-phone-plan") || arguments.contains("-phone-plan-fails")
            || arguments.contains("-phone-plan-hangs")
    }

    private let capture: [String: Any]

    /// Whether the runner keeps rulings (`-phone-rulings`).
    static var rulingsAdvertised: Bool { CommandLine.arguments.contains("-phone-rulings") }

    /// Whether the runner keeps orchestrator pages (`-phone-pages`).
    static var pagesAdvertised: Bool {
        CommandLine.arguments.contains("-phone-pages") || CommandLine.arguments.contains("-phone-pages-fails")
    }

    /// The file's cards, on `board`: what a page's task and ask references
    /// name. None without `-phone-pages`.
    static func boardTasks(on board: String) -> [[String: Any]] {
        guard CommandLine.arguments.contains("-phone-pages"), let plan = HarnessPlan(),
            let tasks = plan.capture["tasks"] as? [[String: Any]]
        else { return [] }
        return tasks.map { task in
            var task = task
            task["workspace"] = board
            return task
        }
    }

    init?() {
        guard Self.advertised,
            let path = UserDefaults.standard.string(forKey: "phone-plan-file"),
            let data = FileManager.default.contents(atPath: path),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        capture = object
    }

    /// After a notice, the first queued lane is "<name>-v2".
    private static func versioned(_ plan: Any) -> Any {
        guard version > 0, var plan = plan as? [String: Any], var lanes = plan["lanes"] as? [[String: Any]],
            let first = (plan["order"] as? [String])?.first,
            let at = lanes.firstIndex(where: { $0["id"] as? String == first })
        else { return plan }
        lanes[at]["name"] = "\((lanes[at]["name"] as? String) ?? "")-v\(version + 1)"
        plan["lanes"] = lanes
        return plan
    }

    /// After a notice, the first page's title ends " v2".
    private static func versioned(pages: Any) -> Any {
        guard version > 0, var list = pages as? [String: Any], var rows = list["pages"] as? [[String: Any]], !rows.isEmpty
        else { return pages }
        rows[0]["title"] = "\((rows[0]["title"] as? String) ?? "") v\(version + 1)"
        list["pages"] = rows
        return list
    }

    /// The pages as old as they were when the file was captured, not as old
    /// as the file: each `updated_at_ms` moves by the time since the plan's
    /// `now_ms`. Read as written, a page with `stale_after_min` went stale
    /// two hours after the capture, and "Updated" turned into "Not updated
    /// for 2 hours" on every run after that.
    static func current(_ pages: Any, plan: Any?, now: Date = Date()) -> Any {
        guard var list = pages as? [String: Any], var rows = list["pages"] as? [[String: Any]],
            let captured = ((plan as? [String: Any])?["now_ms"] as? NSNumber)?.int64Value
        else { return pages }
        let shift = Int64(now.timeIntervalSince1970 * 1000) - captured
        for index in rows.indices {
            guard let updated = (rows[index]["updated_at_ms"] as? NSNumber)?.int64Value else { continue }
            rows[index]["updated_at_ms"] = updated + shift
        }
        list["pages"] = rows
        return list
    }

    /// `-phone-plan-outcomes`: three themes alike but for the length of their
    /// outcomes.
    private static func measuring(_ plan: Any) -> Any {
        guard CommandLine.arguments.contains("-phone-plan-outcomes"), var plan = plan as? [String: Any],
            var themes = plan["themes"] as? [[String: Any]]
        else { return plan }
        let outcomes = [
            "Short.",
            "Two lines, not one: this sentence runs on past the first line.",
            String(repeating: "A long outcome that keeps going well past three lines of text. ", count: 8),
        ]
        for (index, outcome) in outcomes.enumerated() where index < themes.count {
            themes[index]["outcome"] = outcome
            themes[index]["next"] = ""
            themes[index]["owner_ask"] = ""
        }
        plan["themes"] = themes
        return plan
    }

    /// The runner's answer to `method`, or nil for one that isn't the
    /// plan's. A call for a board the harness doesn't have is refused, as a
    /// runner refuses an id it doesn't know.
    func answer(_ method: String, _ args: [String: Any], boards: [String]) async throws -> Data? {
        switch method {
        case "plan.get":
            if CommandLine.arguments.contains("-phone-plan-hangs") {
                try await Task.sleep(for: .seconds(3600))
            }
            if CommandLine.arguments.contains("-phone-plan-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            guard let board = args["workspace"] as? String, boards.contains(board), let plan = capture["plan"]
            else { throw ClientCore.CoreError.rejected("bad workspace", word: "invalid-argument") }
            return try JSONSerialization.data(withJSONObject: Self.versioned(Self.measuring(plan)))
        case "page.list":
            guard Self.pagesAdvertised else { return nil }
            if CommandLine.arguments.contains("-phone-pages-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            guard let board = args["workspace"] as? String, boards.contains(board), let pages = capture["pages"]
            else { throw ClientCore.CoreError.rejected("bad workspace", word: "invalid-argument") }
            return try JSONSerialization.data(withJSONObject: Self.versioned(pages: Self.current(pages, plan: capture["plan"])))
        case "plan.events":
            let subject = (args["theme"] as? String) ?? (args["lane"] as? String)
            guard let subject, let records = capture["records"] as? [String: Any], let record = records[subject]
            else { throw ClientCore.CoreError.rejected("bad subject", word: "invalid-argument") }
            return try JSONSerialization.data(withJSONObject: record)
        default:
            return nil
        }
    }
}
#endif
