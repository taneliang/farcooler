import Foundation
import SwiftUI

// Task keys in text as links (ov-196): "ov-190" in a task's intent, a note, an
// acceptance line or an agent's reply opens that task, where a link to the web
// in the same text already opens.
//
// Which words are keys is decided here and once in Kotlin (`TaskKeyLinks.kt`),
// against one set of cases, `test/fixtures/task-key-links.json`, which both
// suites read. A word is a key only when it reads `<prefix>-<number>` with a
// boundary on each side, its prefix is one of the runner's workspaces' task
// prefixes, and a board the app has read has a task under it. So "utf-8",
// "x-86" and a key nobody filed stay text: a link that opens nothing is worse
// than no link.
//
// The link is `farcooler://task/<runner>/<key>`, and it never leaves the app.
// `Markdown.openGuard(_:)` takes it before the system could: the system would
// hand `farcooler://` to whichever channel's app claimed it last (`AppScheme`),
// a canary's link opening the stable app.

/// Where a key's task is: enough for an app's own open-task path.
public struct TaskKeyTarget: Equatable, Hashable, Sendable {
    /// The app's own id for the runner: the Mac's host target, the phone's
    /// `Host.id`. The link carries it, and the app reads it back.
    public var runner: String
    /// The workspace whose board has the task.
    public var workspace: String
    /// The task's id.
    public var task: String
    /// Its key, "ov-190".
    public var key: String

    public init(runner: String, workspace: String, task: String, key: String) {
        self.runner = runner
        self.workspace = workspace
        self.task = task
        self.key = key
    }

    /// The same place as a `Destination` (ov-182), in this device's own
    /// ids, so a link can open through the destination path once each app
    /// has one: the linker's closure becomes `open(target.destination)`.
    public var destination: Destination {
        Destination(
            runner: .init(host: runner), place: .task(workspace: workspace, task: .init(id: task, key: key)))
    }
}

/// One runner's keys that may become links: its workspaces' prefixes, and
/// the tasks on the boards read so far, by key.
public struct TaskKeyIndex: Equatable, Sendable {
    public var runner: String
    public var prefixes: Set<String>
    public var targets: [String: TaskKeyTarget]

    public static let empty = TaskKeyIndex(runner: "", prefixes: [], targets: [:])

    public init(runner: String, prefixes: Set<String>, targets: [String: TaskKeyTarget]) {
        self.runner = runner
        self.prefixes = prefixes
        self.targets = targets
    }

    /// A runner's index from its workspaces and their boards, by workspace
    /// id. A board under a workspace the runner didn't list still counts:
    /// its keys were read off the runner, so they're its tasks.
    public init(runner: String, workspaces: [WorkspaceSummary], boards: [String: TaskBoardModel]) {
        var targets: [String: TaskKeyTarget] = [:]
        for workspace in boards.keys.sorted() {
            for row in boards[workspace]?.rows ?? [] where targets[row.key] == nil {
                targets[row.key] = TaskKeyTarget(runner: runner, workspace: workspace, task: row.id, key: row.key)
            }
        }
        self.init(
            runner: runner, prefixes: Set(workspaces.map(\.taskPrefix).filter { !$0.isEmpty }), targets: targets)
    }

    public var isEmpty: Bool { prefixes.isEmpty || targets.isEmpty }
}

public enum TaskKeyLinks {
    /// A key found in text: where it starts, in UTF-16 code units (the unit
    /// Kotlin indexes by, so the fixture reads the same to both), and what
    /// it says.
    public struct Match: Equatable, Sendable {
        public var start: Int
        public var key: String
        public var length: Int { key.utf16.count }

        public init(start: Int, key: String) {
            self.start = start
            self.key = key
        }
    }

    /// Every key in `text` under one of `prefixes` that `known` has, in
    /// order.
    ///
    /// A key is `<prefix>-<digits>`, the prefix an ASCII letter and then
    /// letters or digits. Either side of it is the text's edge or anything
    /// but a letter, a digit, `_` or `-`: "nov-190", "ov-190a",
    /// "fix-ov-190" and "ov-190-fix" aren't keys, "(ov-190)." is. The
    /// prefix's case is the workspace's own: "OV-190" isn't "ov-190".
    ///
    /// Two more aren't: a key with a `.` and a digit after it, a version
    /// ("ov-1.2"), and a key in a word that holds `://`, a bare URL's path
    /// (".../tree/ov-190"), which is the URL's. A word here runs between
    /// ASCII whitespace.
    public static func matches(in text: String, prefixes: Set<String>, known: Set<String>) -> [Match] {
        guard !prefixes.isEmpty, !known.isEmpty else { return [] }
        let units = Array(text.utf16)
        var found: [Match] = []
        var i = 0
        while i < units.count {
            guard isASCIILetter(units[i]), i == 0 || !joins(units[i - 1]) else {
                i += 1
                continue
            }
            var j = i + 1
            while j < units.count, isASCIILetter(units[j]) || isASCIIDigit(units[j]) { j += 1 }
            var k = j + 1
            guard j < units.count, units[j] == hyphen, k < units.count, isASCIIDigit(units[k]) else {
                i = j
                continue
            }
            while k < units.count, isASCIIDigit(units[k]) { k += 1 }
            let version = k + 1 < units.count && units[k] == dot && isASCIIDigit(units[k + 1])
            if k == units.count || !joins(units[k]), !version, !inURL(units, from: i, to: k) {
                let prefix = String(decoding: units[i..<j], as: UTF16.self)
                let key = String(decoding: units[i..<k], as: UTF16.self)
                if prefixes.contains(prefix), known.contains(key) {
                    found.append(Match(start: i, key: key))
                }
            }
            i = k
        }
        return found
    }

    /// `matches`, against one runner's index.
    public static func matches(in text: String, index: TaskKeyIndex) -> [Match] {
        matches(in: text, prefixes: index.prefixes, known: Set(index.targets.keys))
    }

    /// An accessibility action's name for opening `key`: "Open ov-190",
    /// with the card's title after it when the card is known.
    public static func openLabel(_ key: String, card: TaskKeyCard?) -> String {
        "Open \(card?.accessibilityLabel ?? key)"
    }

    /// The scheme and host of a task link. Fixed, not the channel's own
    /// scheme: the link is opened in the app that drew it and never handed
    /// to the system (`Markdown.openGuard(_:)`).
    public static let scheme = "farcooler"
    public static let host = "task"

    /// `farcooler://task/<runner>/<key>`, each part escaped.
    public static func url(runner: String, key: String) -> URL? {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove("/")
        guard let runner = runner.isEmpty ? thisMac : runner.addingPercentEncoding(withAllowedCharacters: allowed),
            let key = key.addingPercentEncoding(withAllowedCharacters: allowed), !key.isEmpty
        else { return nil }
        return URL(string: "\(scheme)://\(host)/\(runner)/\(key)")
    }

    /// The link's runner for the Mac's own, whose host target is `""`: an
    /// empty path part would vanish from the URL. No runner is called "-",
    /// which ssh would read as an option.
    static let thisMac = "-"

    /// The runner and key a task link names, or nil for any other URL:
    /// another host under the scheme, a missing part, or one too many.
    public static func parse(_ url: URL) -> (runner: String, key: String)? {
        guard url.scheme?.lowercased() == scheme, url.host()?.lowercased() == host else { return nil }
        let parts = url.pathComponents.filter { $0 != "/" }
        guard parts.count == 2, !parts[0].isEmpty, !parts[1].isEmpty else { return nil }
        return (parts[0] == thisMac ? "" : parts[0], parts[1])
    }

    /// `text` with each key in `index` made a link to its task. A run that
    /// is already a link, or code, is left alone: a key in a URL is the
    /// URL's, and a key in backticks is quoted, not referred to.
    public static func linked(_ text: AttributedString, index: TaskKeyIndex) -> AttributedString {
        guard !index.isEmpty else { return text }
        let plain = String(text.characters)
        let found = matches(in: plain, index: index)
        guard !found.isEmpty else { return text }
        var out = text
        let utf16 = plain.utf16
        for match in found {
            guard let url = url(runner: index.runner, key: match.key) else { continue }
            let lower = utf16.index(utf16.startIndex, offsetBy: match.start)
            let upper = utf16.index(lower, offsetBy: match.length)
            // A key ending inside a character (a combining mark on its last
            // digit) isn't one whole word to underline.
            guard lower.samePosition(in: plain) != nil, upper.samePosition(in: plain) != nil else { continue }
            // Offsets in Characters, which the attributed string's
            // `characters` view shares with `plain`.
            let from = plain.distance(from: plain.startIndex, to: lower)
            let count = plain.distance(from: lower, to: upper)
            let start = out.characters.index(out.startIndex, offsetBy: from)
            let end = out.characters.index(start, offsetBy: count)
            let range = start..<end
            let quoted = out[range].runs.contains {
                $0.link != nil || ($0.inlinePresentationIntent?.contains(.code) ?? false)
            }
            if !quoted { out[range].link = url }
        }
        return out
    }

    /// Whether the word holding `units[from..<to]` holds `://`.
    private static func inURL(_ units: [UInt16], from: Int, to: Int) -> Bool {
        var start = from
        while start > 0, !isASCIISpace(units[start - 1]) { start -= 1 }
        var end = to
        while end < units.count, !isASCIISpace(units[end]) { end += 1 }
        guard end - start >= 3 else { return false }
        return (start..<(end - 2)).contains {
            units[$0] == colon && units[$0 + 1] == slash && units[$0 + 2] == slash
        }
    }

    /// Tab, line feed, vertical tab, form feed, return or space.
    private static func isASCIISpace(_ u: UInt16) -> Bool { (0x09...0x0D).contains(u) || u == 0x20 }

    private static let dot = UInt16(UInt8(ascii: "."))
    private static let colon = UInt16(UInt8(ascii: ":"))
    private static let slash = UInt16(UInt8(ascii: "/"))
    private static let hyphen = UInt16(UInt8(ascii: "-"))
    private static let underscore = UInt16(UInt8(ascii: "_"))

    private static func isASCIILetter(_ u: UInt16) -> Bool {
        (0x41...0x5A).contains(u) || (0x61...0x7A).contains(u)
    }

    private static func isASCIIDigit(_ u: UInt16) -> Bool { (0x30...0x39).contains(u) }

    /// Whether a code unit beside a key joins it to a longer word: a letter
    /// or a decimal digit in any script, `_` or `-`. Kotlin's
    /// `Char.isLetterOrDigit` exactly: the letter categories and Nd, and a
    /// lone surrogate is neither.
    private static func joins(_ u: UInt16) -> Bool {
        if u == hyphen || u == underscore { return true }
        guard let scalar = Unicode.Scalar(u) else { return false }
        switch scalar.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter, .decimalNumber:
            return true
        default:
            return false
        }
    }
}

/// What a view's text links keys to, and what opening one does: set where
/// the runner is known, read by `MarkdownText` and a task's own lines.
public struct TaskKeyLinker: Equatable, Sendable {
    public var index: TaskKeyIndex
    /// What each key's hovercard or preview shows (ov-299), from the same
    /// reads as `index`. Empty where no screen built them: the keys still
    /// link, and show no card.
    public var cards: TaskKeyCards
    public var open: @MainActor (TaskKeyTarget) -> Void

    public init(
        index: TaskKeyIndex, cards: TaskKeyCards = .empty, open: @escaping @MainActor (TaskKeyTarget) -> Void
    ) {
        self.index = index
        self.cards = cards
        self.open = open
    }

    /// Equal when they link the same keys to the same tasks, and show the
    /// same cards. The opener is left out, as a closure can't be compared: a
    /// screen makes a new one on every pass, and a transcript's text
    /// shouldn't redraw for that.
    public static func == (a: TaskKeyLinker, b: TaskKeyLinker) -> Bool { a.index == b.index && a.cards == b.cards }

    /// The card for a key this linker links, or nil: a key it doesn't link,
    /// or cards built for another runner.
    public func card(forKey key: String) -> TaskKeyCard? {
        guard index.targets[key] != nil, cards.runner == index.runner else { return nil }
        return cards.card(for: key)
    }

    /// The card a task link names, when it's this linker's runner's.
    public func card(for url: URL) -> TaskKeyCard? {
        guard let (runner, key) = TaskKeyLinks.parse(url), runner == index.runner else { return nil }
        return card(forKey: key)
    }

    /// Open the task under `key`, when this linker links it.
    @MainActor
    public func open(key: String) {
        if let target = index.targets[key] { open(target) }
    }

    /// No keys, nothing to open: what a view gets when no one set it.
    public static let none = TaskKeyLinker(index: .empty, open: { _ in })

    /// `text` with its keys linked.
    public func linked(_ text: AttributedString) -> AttributedString {
        TaskKeyLinks.linked(text, index: index)
    }

    /// The tasks `linked` (text this linker linked) links to, once each in
    /// the order they're first linked: one accessibility action apiece, as
    /// a link inside a row that speaks as one element can't be reached by
    /// itself (`taskKeyActions(_:linker:)`).
    public func targets(in linked: AttributedString) -> [TaskKeyTarget] {
        var seen = Set<String>()
        return linked.runs.compactMap { run -> TaskKeyTarget? in
            guard let url = run.link, let (runner, key) = TaskKeyLinks.parse(url), runner == index.runner,
                let target = index.targets[key], seen.insert(key).inserted
            else { return nil }
            return target
        }
    }

    /// Open the task a link names, when it's on this runner's boards: true
    /// when it named one, opened or not, so the guard never hands a task
    /// link to the system.
    @MainActor
    @discardableResult
    public func follow(_ url: URL) -> Bool {
        guard let (runner, key) = TaskKeyLinks.parse(url) else { return false }
        if runner == index.runner, let target = index.targets[key] { open(target) }
        return true
    }
}

extension EnvironmentValues {
    /// The keys this view's text links, and where they go. `.none` unless a
    /// screen that knows its runner sets it.
    @Entry public var taskKeyLinker: TaskKeyLinker = .none
}

extension TaskKeyTarget {
    /// Where a phone opens it: the task, pushed over the screen showing, as
    /// a pane's task chip opens one (`ShellPaneBar.chipTitle`).
    var phoneRoute: PhoneRoute { .task(PhoneWorkspace(runner: runner, workspace: workspace), task: task) }
}

extension TaskKeyLinker {
    /// The phone's linker for one runner: its workspaces' prefixes and the
    /// boards it has read, each link opening its task's route through
    /// `open` (the navigator's push). Links nothing before the runner is
    /// known or without a way to open. Here rather than in the app so
    /// `swift test` holds the wiring: the iOS target has no unit tests.
    static func phone(
        runner: String?, workspaces: [WorkspaceSummary], boards: [String: TaskBoardModel],
        cards: TaskKeyCards = .empty, open: (@MainActor (PhoneRoute) -> Void)?
    ) -> TaskKeyLinker {
        guard let runner, let open else { return .none }
        let index = TaskKeyIndex(runner: runner, workspaces: workspaces, boards: boards)
        return TaskKeyLinker(index: index, cards: cards) { open($0.phoneRoute) }
    }
}

extension View {
    /// One accessibility action per task `linked` links, "Open ov-190,
    /// Fix the login" (its title, when its card is known: ov-299), for a row
    /// that speaks as one element: VoiceOver can't reach a link inside such
    /// a row's text, and these open the same task a tap would.
    public func taskKeyActions(_ linked: AttributedString, linker: TaskKeyLinker) -> some View {
        accessibilityActions {
            ForEach(linker.targets(in: linked), id: \.key) { target in
                Button(TaskKeyLinks.openLabel(target.key, card: linker.card(forKey: target.key))) {
                    linker.open(target)
                }
            }
        }
    }
}
