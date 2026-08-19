import Foundation

/// Everything the watch asks the phone, and everything the phone answers.
///
/// The watch app and the phone app are two separate binaries that have to agree
/// about these messages down to the key names, which is the same reason
/// `AgentActivityAttributes` lives in one file both targets compile. A key
/// spelled `requestId` on one side and `request` on the other does not fail to
/// build — the message arrives, the receiver finds nothing under the key it
/// looked for, and the action silently never happens. Nobody sees an error:
/// the person taps Allow, the watch says it sent, and the agent goes on
/// waiting. One file, one spelling, no second place to drift.
///
/// **Everything here is coded to `[String: Any]` by hand, and not with
/// `Codable`.** `WCSession` does not carry `Data`; it carries a property-list
/// dictionary, and it rejects at RUNTIME any value that is not a property-list
/// type — `String`, `Int`, `Double`, `Bool`, `Data`, `Date`, `Array`,
/// `Dictionary`. There is no compiler diagnostic for getting that wrong, and
/// the punishment for getting it wrong is the silent non-delivery described
/// above. So the dictionary is written out where it can be read and tested,
/// rather than produced by a synthesized encoder whose output nobody inspects.
/// `WatchLinkTests` checks the whole structure recursively; that test is the
/// only enforcement this rule has.
///
/// Hand-written coding also buys the thing `Codable` cannot: an unrecognized
/// `kind` decodes to `nil` instead of throwing. A watch one release behind its
/// phone meets a word it has not learned yet, ignores that one message, and
/// keeps working — where a throw inside a `WCSession` delegate callback would
/// take the watch app down over a message it was free to disregard.
///
/// The vocabulary is deliberately three requests and three replies. The watch
/// holds no SSH identity and decides nothing; it asks the phone, which performs
/// each action through the code paths the phone already has, so a watch and a
/// phone cannot answer the same agent differently.
enum WatchLink {
    /// Every string key that crosses this seam, spelled once.
    ///
    /// A literal at each use site is how two spellings of one key get into a
    /// codebase, and this file exists to prevent exactly that.
    enum Key {
        /// Which message this is. Present on every request and every reply.
        static let kind = "kind"
        static let terminal = "terminal"
        static let text = "text"
        /// The permission's own id, as the agent's event named it.
        static let request = "request"
        /// Which of the offered options was chosen.
        static let option = "option"
        static let reason = "reason"
        static let permission = "permission"
        static let id = "id"
        static let toolCall = "toolCall"
        static let options = "options"
        static let name = "name"
    }

    /// The values `Key.kind` takes. Constants for the same reason the keys are.
    enum Kind {
        static let prompt = "prompt"
        static let answer = "answer"
        static let pendingPermission = "pendingPermission"
        static let sent = "sent"
        static let failed = "failed"
        static let permission = "permission"
    }
}

/// One thing the watch asks the phone to do.
///
/// Sent by `sendMessage`, which wakes the phone app in the background if it is
/// not running. Three cases, and adding a fourth is a decision about what the
/// watch is allowed to change — not a formality.
public enum WatchRequest: Sendable, Equatable {
    /// Send text to an agent, exactly as the phone's composer would.
    case prompt(terminal: String, text: String)
    /// Answer a pending permission: which permission, and which of the options
    /// the agent offered.
    case answer(terminal: String, request: String, option: String)
    /// What, if anything, this agent is blocked on.
    ///
    /// Asked when the watch opens a `blocked` agent, because a permission's id
    /// and options come from the agent's event stream and not from the fleet
    /// snapshot — so the watch cannot know them and must not invent them.
    case pendingPermission(terminal: String)

    public var dictionary: [String: Any] {
        switch self {
        case let .prompt(terminal, text):
            [
                WatchLink.Key.kind: WatchLink.Kind.prompt,
                WatchLink.Key.terminal: terminal,
                WatchLink.Key.text: text,
            ]
        case let .answer(terminal, request, option):
            [
                WatchLink.Key.kind: WatchLink.Kind.answer,
                WatchLink.Key.terminal: terminal,
                WatchLink.Key.request: request,
                WatchLink.Key.option: option,
            ]
        case let .pendingPermission(terminal):
            [
                WatchLink.Key.kind: WatchLink.Kind.pendingPermission,
                WatchLink.Key.terminal: terminal,
            ]
        }
    }

    /// `nil` for anything this build cannot read: an unknown `kind`, a missing
    /// key, a key of the wrong type. All three mean the same thing to the
    /// receiver — it does not understand this message — and the only safe
    /// response is to do nothing. Guessing at a half-read `prompt` would send
    /// an empty message to a live agent.
    public init?(dictionary: [String: Any]) {
        switch dictionary[WatchLink.Key.kind] as? String {
        case WatchLink.Kind.prompt:
            guard let terminal = dictionary[WatchLink.Key.terminal] as? String,
                let text = dictionary[WatchLink.Key.text] as? String
            else { return nil }
            self = .prompt(terminal: terminal, text: text)
        case WatchLink.Kind.answer:
            guard let terminal = dictionary[WatchLink.Key.terminal] as? String,
                let request = dictionary[WatchLink.Key.request] as? String,
                let option = dictionary[WatchLink.Key.option] as? String
            else { return nil }
            self = .answer(terminal: terminal, request: request, option: option)
        case WatchLink.Kind.pendingPermission:
            guard let terminal = dictionary[WatchLink.Key.terminal] as? String
            else { return nil }
            self = .pendingPermission(terminal: terminal)
        default:
            return nil
        }
    }
}

/// What the phone answers.
public enum WatchReply: Sendable, Equatable {
    /// The phone did it. Not "the agent acted on it" — only that the phone
    /// handed it on through the same path a tap on the phone would have taken.
    case sent
    /// It did not happen, and this is why, in words fit to put on a watch.
    /// Never a raw error from underneath.
    case failed(String)
    /// What the agent is blocked on, or `nil` for nothing pending.
    ///
    /// `nil` is a case rather than a `failed`, because an agent can be blocked
    /// on something that is not a permission — a trust gate, a plain question.
    /// "Nothing pending" is then the honest answer and the watch renders it as
    /// such. Reporting it as a failure would put an error in front of a person
    /// when nothing went wrong.
    case permission(WatchPermission?)

    /// Nothing pending travels as an ABSENT `permission` key, not as a null.
    ///
    /// `NSNull` is not a property-list type, so a null here would be rejected
    /// by `WCSession` and the reply would never arrive — the watch would sit
    /// waiting on an answer the phone believes it already gave.
    public var dictionary: [String: Any] {
        switch self {
        case .sent:
            [WatchLink.Key.kind: WatchLink.Kind.sent]
        case let .failed(reason):
            [WatchLink.Key.kind: WatchLink.Kind.failed, WatchLink.Key.reason: reason]
        case let .permission(permission):
            if let permission {
                [
                    WatchLink.Key.kind: WatchLink.Kind.permission,
                    WatchLink.Key.permission: permission.dictionary,
                ]
            } else {
                [WatchLink.Key.kind: WatchLink.Kind.permission]
            }
        }
    }

    public init?(dictionary: [String: Any]) {
        switch dictionary[WatchLink.Key.kind] as? String {
        case WatchLink.Kind.sent:
            self = .sent
        case WatchLink.Kind.failed:
            guard let reason = dictionary[WatchLink.Key.reason] as? String else { return nil }
            self = .failed(reason)
        case WatchLink.Kind.permission:
            // An absent key is "nothing pending". A key that is present and
            // unreadable is not: that is a permission this build failed to
            // understand, and rendering it as "nothing pending" would tell the
            // person their agent is not waiting when it is.
            guard let raw = dictionary[WatchLink.Key.permission] else {
                self = .permission(nil)
                return
            }
            guard let nested = raw as? [String: Any],
                let permission = WatchPermission(dictionary: nested)
            else { return nil }
            self = .permission(permission)
        default:
            return nil
        }
    }
}

/// What an agent is waiting to be told, on its way to the watch.
///
/// Field for field the `permission` event in `AgentEvent`, so the phone's
/// conversion is a copy: a field this dropped would show up as an unused one on
/// the other side rather than as nothing at all.
public struct WatchPermission: Sendable, Equatable, Identifiable {
    /// The permission's id, which `answer` sends straight back.
    public let id: String
    /// The tool call this permission gates, as an opaque correlation id — NOT
    /// a description of what the agent wants to do. Every producer defaults
    /// it to the empty string when the agent left it out: `tool_use_id` in
    /// `claude/normalize.rs`, `itemId` in `codex/normalize.rs`,
    /// `toolCall.toolCallId` in `acp/session.rs`. Its only consumer anywhere
    /// is an id comparison — `AgentView.swift`'s `names(_:_:)`, matching this
    /// permission to the `ToolRow` it blocks so the approval renders on the
    /// call it gates rather than in a panel elsewhere. Nothing renders the
    /// string itself: a surface that put it on screen would show `toolu_01…`
    /// where a sentence belongs, or nothing at all when the agent left it
    /// empty.
    public let toolCall: String
    /// The answers the agent offered, in the order it offered them.
    ///
    /// Rendered as buttons by their own names. There is no hardcoded
    /// Allow/Deny pair anywhere on the watch: the daemon declines to invent
    /// that vocabulary and so does every surface downstream of it.
    public let options: [WatchPermissionOption]

    public init(id: String, toolCall: String, options: [WatchPermissionOption]) {
        self.id = id
        self.toolCall = toolCall
        self.options = options
    }

    public var dictionary: [String: Any] {
        [
            WatchLink.Key.id: id,
            WatchLink.Key.toolCall: toolCall,
            WatchLink.Key.options: options.map(\.dictionary),
        ]
    }

    /// One unreadable option costs the whole permission.
    ///
    /// Skipping it instead would show the person a shorter list of answers than
    /// the agent actually offered, with nothing to say so — and they would pick
    /// from what they were shown, believing it was everything. Refusing the
    /// permission outright reaches them as "this build cannot show you this",
    /// which sends them to the phone rather than to the wrong answer.
    public init?(dictionary: [String: Any]) {
        guard let id = dictionary[WatchLink.Key.id] as? String,
            let toolCall = dictionary[WatchLink.Key.toolCall] as? String,
            let raw = dictionary[WatchLink.Key.options] as? [[String: Any]]
        else { return nil }
        var options: [WatchPermissionOption] = []
        for entry in raw {
            guard let option = WatchPermissionOption(dictionary: entry) else { return nil }
            options.append(option)
        }
        self.init(id: id, toolCall: toolCall, options: options)
    }
}

/// One answer the agent offered. `PermissionOption`'s three fields, spelled the
/// same way for the same reason the rest of this file is.
public struct WatchPermissionOption: Sendable, Equatable, Identifiable {
    public let id: String
    /// What the button says, as the agent worded it.
    public let name: String
    /// The agent's own category for this answer — `allow_once`, `reject_once`
    /// and whatever else it invents. Carried rather than interpreted: a build
    /// that switched on it would have to be taught every new one, and would
    /// render nothing for the one it had not met.
    public let kind: String

    public init(id: String, name: String, kind: String) {
        self.id = id
        self.name = name
        self.kind = kind
    }

    public var dictionary: [String: Any] {
        [
            WatchLink.Key.id: id,
            WatchLink.Key.name: name,
            WatchLink.Key.kind: kind,
        ]
    }

    public init?(dictionary: [String: Any]) {
        guard let id = dictionary[WatchLink.Key.id] as? String,
            let name = dictionary[WatchLink.Key.name] as? String,
            let kind = dictionary[WatchLink.Key.kind] as? String
        else { return nil }
        self.init(id: id, name: name, kind: kind)
    }
}
