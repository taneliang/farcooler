import Foundation

/// A destination as a notification carries it, and as every notification
/// that already shipped spells it (ov-183).
///
/// New posts carry the encoding whole under `destination`: a string, because
/// Firebase's data values are strings only. Every older spelling still reads:
///
/// - a task notice (`kind: "task"`, `task`, `runner`, `noticeId`, `event`,
///   ov-94), whose runner can also be read off its id, `t:<runner>:<key>`;
/// - a decision (`kind: "decision"`, with or without `event`), a legacy
///   runner's or relay's;
/// - an agent's push (`terminal`, and `runner` from a runner that says it),
///   and a banner this app posted itself, filed under the terminal's id as
///   its thread;
/// - a local Mac post's `target` (`DaemonClient.target`) and `repository`;
/// - Android's own agent banner, `com.farcooler.terminal`;
/// - the Live Activity and widget link, `<scheme>://terminal/<id>`.
///
/// A task is read before a terminal, because the relay sends `terminal: ""`
/// beside a task, and Firebase copies that into the launch intent.
extension Destination {
    /// The `userInfo` (or Firebase data, or intent extra) key the encoding
    /// rides under.
    public static let payloadKey = "destination"

    /// Android's `Notifier.EXTRA_TERMINAL`, read as `terminal` is.
    public static let androidTerminalKey = "com.farcooler.terminal"

    /// What a notification's tap asks for, or nil for one about nothing this
    /// build can open. `thread` is the notification's thread identifier.
    public init?(userInfo: [AnyHashable: Any], thread: String = "") {
        if let text = userInfo[Self.payloadKey] as? String, let value = Destination(encoded: text) {
            self = value
            return
        }
        let host = userInfo["target"] as? String
        let kind = userInfo["kind"] as? String
        if kind == "task" || kind == "decision", let key = Self.nonEmpty(userInfo["task"]) {
            let noticeId = Self.nonEmpty(userInfo["noticeId"]) ?? (thread.hasPrefix("t:") ? thread : nil)
            let runner = Self.nonEmpty(userInfo["runner"]) ?? noticeId.flatMap(Self.parse(noticeId:))?.runner
            let event = userInfo["event"] as? String
            self.init(
                runner: Runner(host: host, id: runner?.lowercased()),
                place: .task(workspace: nil, task: TaskRef(key: key, repository: Self.nonEmpty(userInfo["repository"]))),
                // A legacy decision with no event is a decision all the same.
                question: event == "decision" || (kind == "decision" && event == nil))
            return
        }
        let threaded = thread.hasPrefix("t:") || thread.hasPrefix("a:") ? nil : Self.nonEmpty(thread)
        // `com.farcooler.terminal` is the extra Android's own agent banner
        // puts its pane under; it has no thread to fall back on.
        let named = Self.nonEmpty(userInfo["terminal"]) ?? Self.nonEmpty(userInfo[Self.androidTerminalKey])
        guard let terminal = named ?? threaded else { return nil }
        self.init(
            runner: Runner(host: host, id: Self.nonEmpty(userInfo["runner"])?.lowercased()),
            place: .terminal(terminal))
    }

    /// What a local post files: the encoding under `payloadKey`, and the
    /// older spelling beside it, so `TaskNotice(userInfo:)` still reads a
    /// task notice's answer buttons and an older reader still opens it.
    public var userInfo: [String: Any] {
        var info: [String: Any] = [Self.payloadKey: encoded]
        if let host = runner.host { info["target"] = host }
        if let id = runner.id { info["runner"] = id }
        switch place {
        case .task(_, let task):
            if let key = task.key {
                info["kind"] = "task"
                info["task"] = key
            }
            if let repository = task.repository { info["repository"] = repository }
        case .terminal(let id):
            info["terminal"] = id
        default:
            break
        }
        return info
    }

    /// `t:<runner id>:<task key>`, a task notice id's long form, as its
    /// parts. Nil for anything else, including the hashed form
    /// (`t:<16 hex>`) a runner uses when the long one would pass 64 bytes.
    public static func parse(noticeId: String) -> (runner: String, key: String)? {
        let parts = noticeId.split(separator: ":", maxSplits: 2, omittingEmptySubsequences: false)
        guard parts.count == 3, parts[0] == "t", !parts[1].isEmpty, !parts[2].isEmpty else { return nil }
        return (String(parts[1]), String(parts[2]))
    }

    /// A link's destination: `<scheme>://open?d=<encoding>`, or the Live
    /// Activity's and widget's `<scheme>://terminal/<id>[?runner=<id>]`.
    /// Any scheme: the system only hands over the ones this build registered.
    public init?(url: URL) {
        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        let query = components?.queryItems ?? []
        func item(_ name: String) -> String? { query.first(where: { $0.name == name })?.value }
        switch url.host() {
        case "open":
            guard let text = item("d"), let value = Destination(encoded: text) else { return nil }
            self = value
        case "terminal":
            let id = url.lastPathComponent
            guard !id.isEmpty, id != "/" else { return nil }
            self.init(runner: Runner(id: Self.nonEmpty(item("runner"))?.lowercased()), place: .terminal(id))
        default:
            return nil
        }
    }

    /// The link `init(url:)` reads: the terminal form for a bare pane, which
    /// every build already opens, and the `open` form for anything else.
    public func url(scheme: String) -> URL? {
        var components = URLComponents()
        components.scheme = scheme
        if case .terminal(let id) = place, runner.host == nil, tab == nil, segment == nil, pane == nil,
            agent == nil, !question
        {
            components.host = "terminal"
            components.percentEncodedPath = "/" + Self.escaped(id)
            if let runnerId = runner.id { components.percentEncodedQuery = "runner=" + Self.escaped(runnerId) }
        } else {
            components.host = "open"
            components.percentEncodedQuery = "d=" + Self.escaped(encoded)
        }
        return components.url
    }

    /// Percent-encoded down to the unreserved characters: `URLComponents`
    /// leaves `&`, `=` and `+` alone in a query value, and any of them in an
    /// id would split or change it on the way back.
    private static func escaped(_ text: String) -> String {
        var allowed = CharacterSet.alphanumerics.intersection(CharacterSet(charactersIn: Unicode.Scalar(0)..<Unicode.Scalar(128)))
        allowed.insert(charactersIn: "-._~")
        return text.addingPercentEncoding(withAllowedCharacters: allowed) ?? text
    }
}
