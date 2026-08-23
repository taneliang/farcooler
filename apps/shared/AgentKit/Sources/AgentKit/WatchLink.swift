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
/// The vocabulary is deliberately small — four requests and four replies. The
/// watch holds no SSH identity and decides nothing; it asks the phone, which
/// performs each action through the code paths the phone already has, so a
/// watch and a phone cannot answer the same agent differently.
///
/// Three of the four requests CHANGE something and the fourth only reads. That
/// division is worth keeping as the vocabulary grows: an action needs a
/// confirmation screen, a guard against a second tap and a sentence for a
/// failure that may or may not have landed, and a read needs none of the three.
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
        /// The messages a transcript reply carries.
        static let entries = "entries"
        /// Who said one of them.
        static let role = "role"
        /// Whether the entries are the whole conversation.
        static let complete = "complete"
    }

    /// The values `Key.kind` takes. Constants for the same reason the keys are.
    ///
    /// `transcript` is the one word that names both a request and a reply, and
    /// that is deliberate rather than an oversight. Every other pair here does
    /// something to an agent, so the asking and the outcome are different words
    /// — `prompt` and `sent`, `pendingPermission` and `permission`. This pair
    /// asks for a thing and answers with that thing, and inventing a second
    /// noun for the answer would be vocabulary with nothing to say. The two
    /// cannot be confused for each other in any case: a request carries
    /// `terminal` and no `entries`, a reply carries `entries` and no
    /// `terminal`, and both initializers below refuse a dictionary missing what
    /// they need.
    enum Kind {
        static let prompt = "prompt"
        static let answer = "answer"
        static let pendingPermission = "pendingPermission"
        static let transcript = "transcript"
        static let sent = "sent"
        static let failed = "failed"
        static let permission = "permission"
    }
}

/// One thing the watch asks the phone to do.
///
/// Sent by `sendMessage`, which wakes the phone app in the background if it is
/// not running. Four cases, and adding a fifth is a decision about what the
/// watch is allowed to change — not a formality.
///
/// The fourth changes nothing, which is why it could be added at all.
/// `transcript` reads; it does not touch an agent, cannot be sent twice by
/// accident to any cost, and needs no confirmation screen. That is a different
/// kind of message from the three above it and the rest of this file should be
/// read with the difference in mind.
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
    /// What this agent has actually SAID, most recent last.
    ///
    /// Asked because there is nowhere else to get it. `FleetSnapshot.Agent`
    /// carries `headline`, `line` and a three-line `feed`, all of them
    /// truncated on the host to fit a widget — the fleet's own summary of the
    /// agent, and not one word the agent wrote. The conversation exists only in
    /// the agent's event stream, which lives on the runner, which the watch
    /// cannot reach.
    ///
    /// There is no cursor and no paging. One question, one screenful, and the
    /// rest is on the phone — see `WatchTranscript`, which explains what is
    /// dropped and which end drops it.
    case transcript(terminal: String)

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
        case let .transcript(terminal):
            [
                WatchLink.Key.kind: WatchLink.Kind.transcript,
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
        case WatchLink.Kind.transcript:
            guard let terminal = dictionary[WatchLink.Key.terminal] as? String
            else { return nil }
            self = .transcript(terminal: terminal)
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
    /// What the agent said, as much of it as fits.
    ///
    /// Not optional, unlike `permission`. An agent that has said nothing is a
    /// `WatchTranscript` with no entries and `complete` true, which is a
    /// perfectly good answer and one the screen renders as such; there is no
    /// second kind of nothing here for a `nil` to mean.
    case transcript(WatchTranscript)

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
        case let .transcript(transcript):
            [
                WatchLink.Key.kind: WatchLink.Kind.transcript,
                WatchLink.Key.entries: transcript.entries.map(\.dictionary),
                WatchLink.Key.complete: transcript.complete,
            ]
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
        case WatchLink.Kind.transcript:
            guard let transcript = WatchTranscript(dictionary: dictionary) else { return nil }
            self = .transcript(transcript)
        default:
            return nil
        }
    }
}

/// What an agent has said, cut down to what a wrist can carry.
///
/// **The phone decides what fits, and the watch never has to.** Two reasons,
/// and the first is transport: `sendMessage` rejects an oversized payload at
/// runtime with `WCErrorCodePayloadTooLarge`, and a rejected reply is not a
/// truncated reply — it is no reply at all, reaching the person as
/// `WatchLinkClient.nothingSent` about a question that went nowhere. Nothing
/// can be trimmed after the fact, so the payload has to fit by construction.
/// The second is that the phone is the only side that knows what it left out:
/// the watch cannot report on words it never received.
///
/// **The ceiling is chosen, not measured**, and it is chosen with a wide
/// margin. WatchConnectivity does not publish the exact number it enforces;
/// what it publishes is the error. So the budget here is `byteBudget`, well
/// under any figure the framework has been observed to allow, leaving room for
/// the binary property list's own keys and framing — which are counted toward
/// the limit and are not counted here.
///
/// It is also, deliberately, far more than the screen can use. Sixteen
/// kilobytes is a couple of thousand words, and nobody reads a couple of
/// thousand words on a wrist in the ninety seconds this feature exists to
/// serve. The cut that will actually bite is `entryLimit`, which is about
/// attention rather than bytes; the byte budget is a backstop against one
/// enormous message, and its job is to fail the transport never.
public struct WatchTranscript: Sendable, Equatable {
    /// The messages, OLDEST FIRST — the order they were said in.
    ///
    /// Chronological even though the newest is what somebody opened this for,
    /// because a conversation read backwards is a conversation where every
    /// answer precedes its question. The screen solves the other half by
    /// opening scrolled to the bottom, so the newest is what is on screen and
    /// scrolling up walks back in time. Reversing the array would have made the
    /// first line right and every line after it wrong.
    public let entries: [WatchTranscriptEntry]

    /// Whether these are all the words there were.
    ///
    /// False for three different reasons — the entry count was capped, the byte
    /// budget clipped a message, or the phone's own copy of the conversation
    /// has a gap in it — and the screen says the same sentence for all three,
    /// because the answer to all three is the same: open it on the phone.
    /// Distinguishing them would be three sentences that lead to one action.
    ///
    /// True is a real claim and is made carefully. An agent that has said
    /// nothing yet is `complete` — nothing is missing from nothing — which is
    /// why the caller's `whole` is a judgement it has to make rather than a
    /// flag it can leave set. See `WatchLinkHost.transcript`, which reads
    /// `GapReason` one case at a time to decide it.
    ///
    /// Spelled like `FleetSnapshot.complete`, and meaning the same kind of
    /// thing: not "this is fresh" but "this is all of it".
    public let complete: Bool

    public init(entries: [WatchTranscriptEntry], complete: Bool) {
        self.entries = entries
        self.complete = complete
    }

    /// How many messages a wrist is handed, at most.
    ///
    /// Twelve. A watch list of twelve messages is already a long turn of the
    /// crown, and the situations this app was designed for — a light, a
    /// hallway, ninety seconds out of a chair — do not contain a thirteenth.
    /// It also bounds the array whatever the messages weigh, which
    /// `byteBudget` alone cannot: a hundred one-word exchanges cost almost
    /// nothing and are still a hundred rows.
    public static let entryLimit = 12

    /// How many bytes of message text a wrist is handed, at most. See the type
    /// comment for where the number comes from and what it is measured in:
    /// UTF-8 bytes of the text itself, with the property list's framing NOT
    /// counted and the margin covering it.
    public static let byteBudget = 16 * 1024

    /// Marks the one message that was cut mid-sentence.
    private static let clipped = "…"

    /// The tail of a conversation that fits, and whether anything was left out.
    ///
    /// `whole` is what the CALLER knows and this cannot: whether the
    /// conversation it is handing over is itself the whole one. A transcript
    /// replayed from the daemon's retained window is bounded at
    /// `TRANSCRIPT_LIMIT` events, and a stream with a gap in it has lost
    /// something upstream of here. Either way `complete` has to come back
    /// false, and only the caller can say so.
    ///
    /// Backwards from the end, because the end is what was asked for. The
    /// newest message is kept whatever it costs — if it alone exceeds the
    /// budget it is clipped and it is the only entry, which is the right
    /// outcome: what the agent just said is the thing somebody raised their
    /// wrist for, and four older messages in its place would be four messages
    /// nobody wanted.
    ///
    /// **A clipped message keeps its HEAD.** Prose is read from the top, and a
    /// person who wants the end of it opens their phone — which the screen
    /// tells them to do. Keeping the tail instead would start every long answer
    /// mid-sentence, which is unreadable rather than merely incomplete.
    public static func fitting(
        _ entries: [WatchTranscriptEntry], whole: Bool
    ) -> WatchTranscript {
        var kept: [WatchTranscriptEntry] = []
        var used = 0
        var clippedOne = false

        for entry in entries.reversed() {
            guard kept.count < entryLimit else { break }
            let size = entry.text.utf8.count
            if used + size <= byteBudget {
                kept.append(entry)
                used += size
                continue
            }
            // The newest message on its own is bigger than the whole budget.
            // It is still the message that was asked for, so it is kept and
            // cut; anything older than it is not reached at all.
            if kept.isEmpty {
                kept.append(
                    WatchTranscriptEntry(
                        role: entry.role,
                        text: clip(entry.text, to: byteBudget - clipped.utf8.count) + clipped))
                clippedOne = true
            }
            break
        }

        return WatchTranscript(
            entries: kept.reversed(),
            complete: whole && !clippedOne && kept.count == entries.count)
    }

    /// The first `budget` UTF-8 bytes of `text`, cut between characters.
    ///
    /// Character by character rather than by slicing the UTF-8 view, because a
    /// slice can land inside a multi-byte scalar and `String(decoding:as:)`
    /// turns that into a replacement character — a black diamond at the end of
    /// every clipped message, in exactly the languages least able to spare one.
    /// A `Character` is a grapheme cluster, so this also declines to cut a
    /// family emoji in half.
    private static func clip(_ text: String, to budget: Int) -> String {
        var out = ""
        var used = 0
        for character in text {
            let size = String(character).utf8.count
            if used + size > budget { break }
            out.append(character)
            used += size
        }
        return out
    }

    /// One unreadable entry costs the whole transcript, for the reason
    /// `WatchPermission` gives for its options: a shorter list than the phone
    /// sent, with nothing to say so, is worse than a screen that admits it
    /// cannot show this. Here it is milder — nobody is choosing from these —
    /// but the failure is the same shape, and one rule read twice is better
    /// than two rules.
    public init?(dictionary: [String: Any]) {
        guard let raw = dictionary[WatchLink.Key.entries] as? [[String: Any]],
            let complete = dictionary[WatchLink.Key.complete] as? Bool
        else { return nil }
        var entries: [WatchTranscriptEntry] = []
        for item in raw {
            guard let entry = WatchTranscriptEntry(dictionary: item) else { return nil }
            entries.append(entry)
        }
        self.init(entries: entries, complete: complete)
    }
}

/// One message, as its speaker wrote it.
public struct WatchTranscriptEntry: Sendable, Equatable {
    /// Who said it — `AgentKit.Role`'s raw value, carried and not interpreted.
    ///
    /// A String for a reason the rest of this file will recognize but with a
    /// twist of its own: `Role` is declared in `AgentEvent.swift`, which the
    /// watch target does not compile. So the watch could not name the type even
    /// if this wanted it to, and a word it has never met has to arrive as a
    /// word. See `isYours`, which is the only question anything asks of it.
    public let role: String
    /// The message itself, in whatever the agent wrote — usually Markdown.
    ///
    /// Carried raw and rendered as plain text on the watch. See
    /// `TranscriptView` for why nothing renders the Markdown.
    public let text: String

    public init(role: String, text: String) {
        self.role = role
        self.text = text
    }

    /// Whether the wearer said this.
    ///
    /// A yes-or-no rather than a switch over roles, and the asymmetry is
    /// deliberate: "was this me" has exactly one right answer for a word this
    /// build has never met, which is no. A newer phone that sends a role
    /// invented after this watch shipped gets that message drawn as somebody
    /// else's, which is true, rather than attributed to the person holding the
    /// watch, which would not be.
    ///
    /// `"User"` is `Role.user.rawValue`, spelled out because the watch target
    /// does not compile the file `Role` lives in. If that raw value ever
    /// changes, this is the second place to change — and `WatchLinkTests` pins
    /// the two together so it cannot be missed.
    public var isYours: Bool { role == "User" }

    public var dictionary: [String: Any] {
        [WatchLink.Key.role: role, WatchLink.Key.text: text]
    }

    public init?(dictionary: [String: Any]) {
        guard let role = dictionary[WatchLink.Key.role] as? String,
            let text = dictionary[WatchLink.Key.text] as? String
        else { return nil }
        self.init(role: role, text: text)
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
