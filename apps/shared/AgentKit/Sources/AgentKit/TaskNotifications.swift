import CryptoKit
import Foundation

#if canImport(UserNotifications) && !os(watchOS)
    import UserNotifications
#endif

/// Notifications about tasks rather than agents (ov-94).
///
/// The runner decides, words and identifies every task notice: one thread per
/// task, `t:<runner id>:<task key>`, so a newer notice about a task replaces
/// the older one on every platform. Each device chooses which of five classes
/// it hears, and the relay filters by that choice
/// (`docs/superpowers/specs/2026-10-02-task-notifications-design.md`, Q4).
/// This file is the apps' half: the five switches and their defaults, reading
/// a task notice off a push, and a decision's answer buttons. Shared by the
/// Mac, the phone and the phone's notification service extension, so none of
/// them can disagree about which class is on or which button sends which
/// answer.
public enum TaskNoticeEvent: String, CaseIterable, Sendable {
    case decision
    case review
    case blocked
    case done
    case new

    /// The switch's name in Settings, in the order the section lists them.
    public var title: String {
        switch self {
        case .decision: "Needs a Decision"
        case .review: "Ready for Review"
        case .blocked: "Blocked"
        case .done: "Done"
        case .new: "New Task"
        }
    }

    /// Where the switch is kept. The same key on the Mac and the phone.
    public var defaultsKey: String { "notifications.tasks.\(rawValue)" }

    /// Needs a Decision, Ready for Review and Blocked are on until turned off:
    /// each is somebody's work waiting on a person. Done and New Task are news
    /// that can wait.
    public var onByDefault: Bool {
        switch self {
        case .decision, .review, .blocked: true
        case .done, .new: false
        }
    }
}

/// The five switches as the apps read them.
public enum TaskNotifications {
    /// Whether `event`'s switch is on: what was set, else its default.
    public static func isOn(_ event: TaskNoticeEvent, in defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: event.defaultsKey) as? Bool ?? event.onByDefault
    }

    /// The classes registration sends the relay as `notifyEvents`.
    ///
    /// `master` is the app's "all notifications" switch. With it off the relay
    /// is told no classes at all, so a phone in a pocket stays as quiet as the
    /// switch says, which it didn't while that switch was only read locally.
    public static func events(master: Bool, in defaults: UserDefaults = .standard) -> [String] {
        guard master else { return [] }
        return TaskNoticeEvent.allCases.filter { isOn($0, in: defaults) }.map(\.rawValue)
    }

    /// Whether a task notice of class `raw` may be shown here. A class this
    /// build doesn't know is not shown.
    public static func wants(_ raw: String?, master: Bool, in defaults: UserDefaults = .standard) -> Bool {
        guard master, let raw, let event = TaskNoticeEvent(rawValue: raw) else { return false }
        return isOn(event, in: defaults)
    }
}

/// A task notice, as a push's payload (or a local post's `userInfo`) carries
/// it: `kind: "task"`, the task's key, the runner it's on, its class, its id,
/// and a decision's answer options.
public struct TaskNotice: Equatable, Sendable {
    public let key: String
    public let runner: String?
    public let event: TaskNoticeEvent?
    public let noticeId: String?
    public let options: [String]

    public init(key: String, runner: String?, event: TaskNoticeEvent?, noticeId: String?, options: [String]) {
        self.key = key
        self.runner = runner
        self.event = event
        self.noticeId = noticeId
        self.options = options
    }

    /// `nil` unless this is a task notice naming a task. `options` is a list
    /// from APNs and from a local post, and a JSON string from FCM, whose data
    /// values are strings only.
    ///
    /// A legacy decision (`kind: "decision"`) carrying `event` is one too: the
    /// runner sends status decisions that way for one stable release, so
    /// older relays and apps still read them (ov-94).
    public init?(userInfo: [AnyHashable: Any]) {
        let kind = userInfo["kind"] as? String
        guard kind == "task" || (kind == "decision" && userInfo["event"] as? String == "decision"),
            let key = userInfo["task"] as? String, !key.isEmpty
        else { return nil }
        let options: [String]
        if let list = userInfo["options"] as? [String] {
            options = list
        } else if let text = userInfo["options"] as? String,
            let list = try? JSONDecoder().decode([String].self, from: Data(text.utf8))
        {
            options = list
        } else {
            options = []
        }
        self.init(
            key: key,
            runner: (userInfo["runner"] as? String).flatMap { $0.isEmpty ? nil : $0 },
            event: (userInfo["event"] as? String).flatMap(TaskNoticeEvent.init(rawValue:)),
            noticeId: (userInfo["noticeId"] as? String).flatMap { $0.isEmpty ? nil : $0 },
            options: options)
    }

    /// What a local post files, in the push's own spelling, so a tap or an
    /// answer on it is read by the same code as one on a push.
    public var userInfo: [String: Any] {
        var info: [String: Any] = ["kind": "task", "task": key]
        if let runner { info["runner"] = runner }
        if let event { info["event"] = event.rawValue }
        if let noticeId { info["noticeId"] = noticeId }
        if !options.isEmpty { info["options"] = options }
        return info
    }
}

/// A task decision's buttons: one per option, then "Answer…" to type one.
///
/// The options are the task's QUESTION note's, the same ones the Needs You
/// rows offer. Never an agent's permission ask: those option names can be a
/// command line, and the runner never sends them.
public enum TaskDecisionActions {
    /// The typed answer's action.
    public static let textAction = "answer.text"
    /// How many options get a button, as `TaskQuestion.buttonLimit` draws them.
    public static let optionLimit = 3

    /// One button.
    public struct Action: Equatable, Sendable {
        public let id: String
        public let title: String
    }

    /// The category a notification with these options is filed under.
    ///
    /// Named for the options, in order, because a category's actions are fixed
    /// once registered: two decisions with different options need two
    /// categories, and two with the same can share one.
    public static func category(for options: [String]) -> String {
        let digest = SHA256.hash(data: Data(options.joined(separator: "\u{1f}").utf8))
        return "decision." + digest.prefix(8).map { String(format: "%02x", $0) }.joined()
    }

    /// The buttons, in order: the options first, then "Answer…".
    public static func actions(for options: [String]) -> [Action] {
        options.prefix(optionLimit).enumerated().map { Action(id: "answer.\($0.offset)", title: $0.element) }
            + [Action(id: textAction, title: "Answer…")]
    }

    /// The answer an action sends, or `nil` for one that sends nothing: the
    /// default tap, a dismissal, an option this notification doesn't have, or
    /// a typed answer that's only whitespace.
    public static func answer(action: String, options: [String], typed: String?) -> String? {
        if action == textAction {
            let text = typed?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return text.isEmpty ? nil : text
        }
        guard action.hasPrefix("answer."), let index = Int(action.dropFirst("answer.".count)),
            options.prefix(optionLimit).indices.contains(index)
        else { return nil }
        return options[index]
    }

    #if canImport(UserNotifications) && !os(watchOS)
        /// The category for `options`, with each button asking to unlock the
        /// device first: an answer moves somebody's work.
        public static func makeCategory(options: [String]) -> UNNotificationCategory {
            let actions: [UNNotificationAction] = actions(for: options).map { action in
                action.id == textAction
                    ? UNTextInputNotificationAction(
                        identifier: action.id, title: action.title, options: [.authenticationRequired],
                        textInputButtonTitle: "Send", textInputPlaceholder: "Your answer")
                    : UNNotificationAction(identifier: action.id, title: action.title, options: [.authenticationRequired])
            }
            return UNNotificationCategory(
                identifier: category(for: options), actions: actions, intentIdentifiers: [], options: [])
        }

        /// Register `options`' category beside every category already
        /// registered, and answer its identifier. `setNotificationCategories`
        /// replaces the whole set, so it's read and merged first.
        public static func register(options: [String], in center: UNUserNotificationCenter = .current()) async
            -> String
        {
            let made = makeCategory(options: options)
            var categories = await center.notificationCategories()
            if !categories.contains(where: { $0.identifier == made.identifier }) {
                categories.insert(made)
                center.setNotificationCategories(categories)
            }
            return made.identifier
        }
    #endif
}

/// An agent's alert push, as the notification service extension folds it into
/// the widget snapshot: the pane, its status word, the agent's name, and
/// whether its turn ended badly.
///
/// The relay's `sendApns` writes these four keys beside `aps`; nil for a push
/// with no terminal or no status, which is a task notice or a push from a
/// runner too old to send a status. Here rather than in the extension so
/// AgentKit's tests can decode `test/fixtures/contracts/push/apns/` with the
/// same code the extension runs (ov-121). Lives in this file because the
/// extension already compiles it (`apps/ios/generate-project.py`).
public struct AgentPush: Equatable, Sendable {
    public let terminal: String
    public let status: String
    /// Empty when the push names none.
    public let label: String
    /// Absent reads as `false`: a runner or relay older than the field, and
    /// the behavior the extension always had.
    public let failed: Bool

    public init(terminal: String, status: String, label: String, failed: Bool) {
        self.terminal = terminal
        self.status = status
        self.label = label
        self.failed = failed
    }

    public init?(userInfo: [AnyHashable: Any]) {
        guard let terminal = userInfo["terminal"] as? String, !terminal.isEmpty,
            let status = userInfo["status"] as? String, !status.isEmpty
        else { return nil }
        self.init(
            terminal: terminal, status: status, label: userInfo["label"] as? String ?? "",
            failed: userInfo["failed"] as? Bool ?? false)
    }
}
