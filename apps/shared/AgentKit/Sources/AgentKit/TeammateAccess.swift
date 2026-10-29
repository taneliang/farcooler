import Foundation

// Add Teammate with a person and an access (ov-856, §8 of the design): who
// this device is on a runner, whether the runner already has the teammate,
// what to offer, and the one request a step-up signs. Android's
// `TeammateAccess.kt` holds the same rules and words.

/// A person a runner already has under the scanned account.
public struct ExistingPerson: Equatable, Sendable {
    public let handle: String
    public let name: String
    public let access: PersonAccess

    public init(handle: String, name: String, access: PersonAccess) {
        self.handle = handle
        self.name = name
        self.access = access
    }
}

/// What Add Teammate needs to know about one runner, read from `member.list`:
/// what the person adding is there, and who is there already.
public struct TeammateRunnerFacts: Equatable, Sendable {
    /// What this device's person is on the runner. Nil when the runner can't
    /// say, which reads as unable to add.
    public var caller: PersonAccess?
    /// The people this device could be added to: the one the runner has under
    /// the scanned account, and any added by hand with no account yet.
    public var candidates: [ExistingPerson]
    /// The handle of the one under the scanned account, if any.
    public var matching: String?
    /// Every handle the runner has, so a new person's is never one of them.
    public var taken: Set<String>
    /// This device holds a trusted step-up key there, so adding asks Face ID
    /// once for that exact request and needs no other confirmation.
    public var signs: Bool

    public init(
        caller: PersonAccess?, candidates: [ExistingPerson] = [], matching: String? = nil, taken: Set<String> = [],
        signs: Bool = false
    ) {
        self.caller = caller
        self.candidates = candidates
        self.matching = matching
        self.taken = taken
        self.signs = signs
    }

    /// Read from a `member.list` answer for the teammate's `account`.
    /// Nil when it isn't one.
    public init?(members: Data, account: String, thisDevice: String = "", isSocket: Bool = false) {
        guard let people = RunnerPeople(json: members),
            let access = PeopleAccess(members: members, thisDevice: thisDevice, isSocket: isSocket)
        else { return nil }
        let recorded = access.recorded
        let fits = people.members.filter { $0.account == account || $0.account.isEmpty }
        self.init(
            caller: recorded ? access.yourAccess : nil,
            candidates: fits.map { member in
                ExistingPerson(
                    handle: member.handle, name: member.name,
                    access: access.people.first { $0.handle == member.handle }?.access ?? .member)
            },
            matching: account.isEmpty ? nil : people.members.first { $0.account == account }?.handle,
            taken: Set(people.members.map(\.handle)), signs: access.thisDevice?.stepUp == .trusted)
    }
}

/// Add Teammate's rules, once for the phones and the Mac.
public enum TeammateAccessRules {
    /// What the Access menu offers when adding to runners where the caller is
    /// `callers`: Admin only if the caller is the Owner on every one of them.
    public static func choices(callers: [PersonAccess?]) -> [PersonAccess] {
        callers.allSatisfy { $0 == .owner } ? PersonAccess.choices(for: .owner) : PersonAccess.choices(for: .admin)
    }

    /// Why this runner can't be ticked, or nil when it can. Only the Owner and
    /// an Admin add people.
    public static func blocked(_ facts: TeammateRunnerFacts?, runner: String) -> String? {
        switch facts?.caller {
        case .owner, .admin: nil
        case .member, .viewer:
            "Only an Owner or an Admin can add people to \(runner)."
        case nil:
            "Update Far Cooler on \(runner) to add people from here."
        }
    }

    /// The person and access a runner is sent: the existing person's when
    /// they're there and picked, else a new one with a handle no one has.
    public static func bind(_ choice: TeammateChoice, facts: TeammateRunnerFacts?) -> [String: Any] {
        let taken = facts?.taken ?? []
        if let handle = choice.person, let existing = facts?.candidates.first(where: { $0.handle == handle }) {
            return [
                "handle": existing.handle, "name": existing.name, "account": choice.who.account,
                "email": choice.who.email, "access": existing.access.rawValue,
            ]
        }
        return [
            "handle": TeammateNames.unique(choice.handle, taken: taken),
            "name": choice.name.trimmingCharacters(in: .whitespaces), "account": choice.who.account,
            "email": choice.who.email, "access": choice.access.rawValue,
        ]
    }

    /// The signed `client.enroll` for one runner: the line at the access's
    /// scope, `for_teammate`, with the person and access in the same request.
    public static func arguments(
        publicKey: String, label: String, clientId: String, nodeKey: String, teammate: TeammateChoice,
        facts: TeammateRunnerFacts?
    ) -> [String: Any] {
        let bound = bind(teammate, facts: facts)
        let viewer = (bound["access"] as? String) == PersonAccess.viewer.rawValue
        return [
            "publicKey": publicKey, "label": label, "clientId": clientId, "nodeKey": nodeKey,
            "scope": viewer ? "read" : "control", "forTeammate": true, "teammate": bound,
        ]
    }

    /// What a refusal of Add Teammate says, in words that name no runner error.
    public static func refusal(_ word: String, runner: String, device: String) -> String {
        switch word {
        case "not_admin", "not_owners_device":
            "Only an Owner or an Admin can add people to \(runner)."
        default:
            StepUpWords.sentence(forRefusal: word, runner: runner, device: device)
        }
    }

    /// What the Face ID prompt says.
    public static func reason(device: String, runner: String, access: String) -> String {
        StepUpReason.add(device: device, runner: runner, access: PersonAccess(rawValue: access) ?? .member)
    }

    /// "Dan keeps the access they have on studio."
    public static func keeps(_ person: ExistingPerson, runner: String) -> String {
        "\(person.name) is already on \(runner) as \(article(person.access.title)) \(person.access.title), and keeps that access."
    }

    private static func article(_ word: String) -> String { "AEIOU".contains(word.first ?? " ") ? "an" : "a" }

    /// The note under the Access menu.
    public static func detail(_ access: PersonAccess) -> String { access.summary }
}
