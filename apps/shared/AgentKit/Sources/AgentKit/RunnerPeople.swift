import Foundation

/// The people on a shared runner, their reps, and whose Claude account the
/// work runs on (ov-508).
///
/// Decoded from `member.list`'s answer as both roads carry it: the Mac reads
/// `farcooler member list --json`, the phones read the bridge, and both print
/// `farcooler_client::members_json`'s shape.
///
/// The words are here rather than in each app, so the Mac and the iPhone say
/// the same thing about the same runner, and the tests pin them once.
public struct RunnerPeople: Equatable, Sendable {
    /// A member's rep: its pane, and the workspace it works in ("" from a
    /// runner that didn't say).
    public struct Rep: Equatable, Hashable, Sendable, Decodable {
        public let terminal: String
        public let workspace: String

        public init(terminal: String, workspace: String) {
            self.terminal = terminal
            self.workspace = workspace
        }
    }

    /// A person on the runner who isn't its owner.
    public struct Member: Equatable, Identifiable, Sendable {
        public let handle: String
        public let name: String
        public let reps: [Rep]
        /// Their enrolled devices, by client id.
        public let devices: [String]
        /// The team directory account they were added under by QR (ov-525),
        /// or "" for a member added by hand.
        public let account: String
        /// That account's email, or "".
        public let email: String
        public var id: String { handle }

        public init(
            handle: String, name: String, reps: [Rep] = [], devices: [String] = [], account: String = "",
            email: String = ""
        ) {
            self.handle = handle
            self.name = name
            self.reps = reps
            self.devices = devices
            self.account = account
            self.email = email
        }
    }

    public let members: [Member]
    /// The account's email, or "" when the runner couldn't read it or this
    /// device is enrolled below Control, which isn't told whose it is.
    public let claudeAccount: String
    /// Who this device is: a member's handle, "" for the owner, or
    /// `(released)` for a device that was a member's and is nobody's now.
    public let you: String

    public init(members: [Member], claudeAccount: String, you: String) {
        self.members = members
        self.claudeAccount = claudeAccount
        self.you = you
    }

    /// `member.list`'s JSON, or nil when it isn't that.
    public init?(json data: Data) {
        guard let wire = try? JSONDecoder().decode(Wire.self, from: data) else { return nil }
        self.init(
            members: wire.members.map {
                Member(
                    handle: $0.handle, name: $0.name, reps: $0.rep_panes ?? [], devices: $0.devices ?? [],
                    account: $0.account ?? "", email: $0.email ?? "")
            },
            claudeAccount: wire.claude_account.trimmingCharacters(in: .whitespaces),
            you: wire.you)
    }

    /// Whether anyone but the owner works here: the only runner the people,
    /// and whose account it is, are worth a line for.
    public var isShared: Bool { !members.isEmpty || isMember }

    /// Whether this device is a member's rather than the owner's.
    public var isMember: Bool { !you.isEmpty && you != Self.released }

    /// Whether `member` is the person holding this device.
    public func isYou(_ member: Member) -> Bool { isMember && member.handle == you }

    /// The one line that says whose Claude account the work runs on: every
    /// agent here, each member's rep included, runs on it, so its usage and
    /// limits are shared.
    ///
    /// "Your" only on the owner's own device (`you` empty): a released
    /// device keeps its scope and is told the email, but it's nobody's, so
    /// it reads the account as someone else's, as a member does.
    public var accountLine: String {
        if claudeAccount.isEmpty { return "Work here runs on the Claude account signed in on this runner." }
        if you.isEmpty { return "Work here runs on your Claude account, \(claudeAccount)." }
        return "Work here runs on \(claudeAccount)’s Claude account."
    }

    /// What a rep is for, said once where reps first appear.
    public static let repPurpose =
        "A rep passes a person’s asks to the orchestrator and brings its questions back. It never answers for them."

    /// Where `member`'s reps work, by workspace name: "Rep in Billing",
    /// "Reps in Billing and Main". A rep whose workspace isn't in `names` is
    /// counted instead, so the line never names a board that isn't there.
    public static func reps(of member: Member, workspaceNames names: [String: String]) -> String {
        let named = member.reps.compactMap { names[$0.workspace] }
        switch member.reps.count {
        case 0: return "No rep"
        case named.count:
            let list = ListFormatter.localizedString(byJoining: named)
            return named.count == 1 ? "Rep in \(list)" : "Reps in \(list)"
        case 1: return "Has a rep"
        case let n: return "Has \(n) reps"
        }
    }

    /// A member's second line: their handle, their email when they were
    /// added by QR, and where their reps work.
    public static func detail(of member: Member, reps: String) -> String {
        [member.handle, member.email, reps].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    static let released = "(released)"

    private struct Wire: Decodable {
        struct Member: Decodable {
            let handle: String
            let name: String
            let rep_panes: [Rep]?
            let devices: [String]?
            let account: String?
            let email: String?
        }
        let members: [Member]
        let claude_account: String
        let you: String
    }
}
