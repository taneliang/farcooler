import Foundation

// When each task starts (ov-212) and who works it (ov-213), as the board's
// own `task list --json` carries them: `wait`, `waiting_on` and `workers`
// on each row. Decoded here, beside the board's read, rather than in
// AgentKit's `TaskRow`: the title bar's queue and activity panel are the
// only readers so far, and a runner older than ov-212 sends none of it, which
// reads as no waits and no workers.

/// One task's start and its recorded subagents.
struct TaskStart: Equatable, Sendable {
    /// Why a task that hasn't started is waiting.
    enum Wait: Equatable, Sendable {
        /// In line for a slot: `line` is `agent` or `build`; `position` is
        /// 1 for next; `ahead` names the tasks before it.
        case inLine(line: String, position: Int, ahead: [String])
        /// Held until a moment, in Unix milliseconds.
        case until(Int64)
        /// Held until an event: `release`, `recurrence`, `clear_board`.
        case after(String)
        case parked
    }

    /// A subagent the orchestrator recorded on the task.
    struct Worker: Equatable, Sendable, Identifiable {
        var id: String
        var harness: String
        var label: String
        /// `running`, `finished`, `stopped`, or `unobserved`: recorded and
        /// not yet seen (ov-213's lane B observes them).
        var state: String
        var doing: String
        /// Unix milliseconds, or nil.
        var startedAt: Int64?
        var endedAt: Int64?

        /// Whether it's still at work as far as anyone knows: running, or
        /// recorded and not seen to end.
        var isActive: Bool { state == "running" || (state == "unobserved" && endedAt == nil) }
    }

    var key: String
    var title: String
    var status: String
    var wait: Wait?
    /// The tasks that block it, by key.
    var waitingOn: [String]
    var workers: [Worker]

    /// Whether it's queued: open, not yet started, and held by a wait or a
    /// blocker.
    var isQueued: Bool {
        ["backlog", "todo"].contains(status) && (wait != nil || !waitingOn.isEmpty)
    }
}

enum TaskStarts {
    /// Every row's start and workers, by key. Empty for data that isn't a
    /// task list; a row without the keys has no wait and no workers.
    static func decode(_ data: Data) -> [String: TaskStart] {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let rows = object["tasks"] as? [[String: Any]]
        else { return [:] }
        var out: [String: TaskStart] = [:]
        for row in rows {
            guard let key = row["key"] as? String else { continue }
            out[key] = TaskStart(
                key: key, title: row["title"] as? String ?? "", status: row["status"] as? String ?? "",
                wait: (row["wait"] as? [String: Any]).flatMap(wait),
                waitingOn: row["waiting_on"] as? [String] ?? [],
                workers: (row["workers"] as? [[String: Any]] ?? []).compactMap(worker))
        }
        return out
    }

    private static func int(_ value: Any?) -> Int64? { (value as? NSNumber)?.int64Value }

    private static func wait(_ json: [String: Any]) -> TaskStart.Wait? {
        switch json["kind"] as? String {
        case "in_line":
            return .inLine(
                line: json["line"] as? String ?? "agent", position: Int(int(json["position"]) ?? 0),
                ahead: json["ahead"] as? [String] ?? [])
        case "until": return int(json["until"]).map(TaskStart.Wait.until)
        case "after": return .after(json["event"] as? String ?? "unknown")
        case "parked": return .parked
        default: return nil
        }
    }

    private static func worker(_ json: [String: Any]) -> TaskStart.Worker? {
        guard let id = json["id"] as? String else { return nil }
        return TaskStart.Worker(
            id: id, harness: json["harness"] as? String ?? "", label: json["label"] as? String ?? "",
            state: json["state"] as? String ?? "unknown", doing: json["doing"] as? String ?? "",
            startedAt: int(json["started_at"]), endedAt: int(json["ended_at"]))
    }

    /// Why a queued task hasn't started, in a few words: "Next in line for
    /// an agent", "3rd in line for a build, after ov-177", "Starts at 3:00
    /// PM", "Starts after the next release", "Parked", "Blocked by ov-191".
    /// A blocker is said first: it's what must change before anything else
    /// matters.
    static func why(
        _ start: TaskStart, now: Date = Date(), calendar: Calendar = .current,
        time: (Date, _ today: Bool) -> String = { date, today in
            today
                ? date.formatted(date: .omitted, time: .shortened)
                : date.formatted(.dateTime.weekday(.abbreviated).hour().minute())
        }
    ) -> String? {
        if !start.waitingOn.isEmpty { return "Blocked by " + list(start.waitingOn) }
        switch start.wait {
        case .inLine(let line, let position, let ahead)?:
            let slot = line == "build" ? "a build" : "an agent"
            let place = position <= 1 ? "Next in line for \(slot)" : "\(ordinal(position)) in line for \(slot)"
            return ahead.isEmpty ? place : "\(place), after \(list(ahead))"
        case .until(let ms)?:
            let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
            return "Starts at \(time(date, calendar.isDate(date, inSameDayAs: now)))"
        case .after(let event)?:
            switch event {
            case "release": return "Starts after the next release"
            case "recurrence": return "Starts when it next comes around"
            case "clear_board": return "Starts once the board is clear"
            default: return "Waiting for something to happen first"
            }
        case .parked?: return "Parked"
        case nil: return nil
        }
    }

    /// "ov-1", "ov-1 and ov-2", "ov-1, ov-2 and 3 more".
    static func list(_ keys: [String]) -> String {
        switch keys.count {
        case 0: return ""
        case 1: return keys[0]
        case 2: return "\(keys[0]) and \(keys[1])"
        case 3: return "\(keys[0]), \(keys[1]) and \(keys[2])"
        default: return "\(keys[0]), \(keys[1]) and \(keys.count - 2) more"
        }
    }

    /// "2nd", "3rd", "11th", "21st".
    static func ordinal(_ n: Int) -> String {
        let tens = n % 100
        let suffix: String
        if (11...13).contains(tens) {
            suffix = "th"
        } else {
            switch n % 10 {
            case 1: suffix = "st"
            case 2: suffix = "nd"
            case 3: suffix = "rd"
            default: suffix = "th"
            }
        }
        return "\(n)\(suffix)"
    }
}
