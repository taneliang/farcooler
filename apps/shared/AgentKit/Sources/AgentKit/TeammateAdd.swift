import Foundation

// Adding a teammate's device, from the Mac and from the iPhone (ov-525,
// ov-854): who the directory says they are, what they may do, and what to say
// about each runner. The Mac and the iPhone show the same sentences, and
// Android's `TeammateAdd.kt` holds the same words.

/// Who the team directory says a teammate is (ov-525): the account their
/// code named, and the email and team the DIRECTORY has for it. Nothing here
/// came from the code but the account it was looked up by.
public struct CeremonyTeammate: Codable, Equatable, Sendable {
    public var account: String
    public var email: String
    public var team: String

    public init(account: String, email: String, team: String) {
        self.account = account
        self.email = email
        self.team = team
    }
}

/// A team whose roster holds a teammate's device while it waits out the hold
/// after their account's trust reset (ruling R-76, ov-565), and when the hold
/// ends, in milliseconds by the relay's clock. Shown, never judged here.
public struct CeremonyHold: Codable, Equatable, Sendable {
    public var team: String
    public var until: Double

    public init(team: String, until: Double) {
        self.team = team
        self.until = until
    }

    public var ends: Date { Date(timeIntervalSince1970: until / 1000) }

    /// What the confirmation says about it, above Add Teammate: why the
    /// device is held, and what adding it now vouches for.
    public static func sentence(_ holds: [CeremonyHold], email: String) -> String {
        let ends = (holds.map(\.ends).max() ?? Date()).formatted(date: .abbreviated, time: .shortened)
        return "Their device is waiting out a hold until \(ends), because \(email)’s account had no confirmed "
            + "device left and confirmed it when it signed in. That’s also what someone with a stolen sign-in "
            + "could do. Add it only if \(email) is with you and says they did that themselves: adding it "
            + "confirms the device for your team now."
    }
}

/// A teammate's access on the runners they're added to: Control or Read,
/// never a shell (ruling R-65). The core refuses a teammate's code that asks
/// for a shell key, and this is the only choice left.
public enum TeammateScope: String, CaseIterable, Identifiable, Equatable, Sendable {
    case control, read

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .control: return "Control"
        case .read: return "Read"
        }
    }

    /// What it lets them do, under the picker.
    public var detail: String {
        switch self {
        case .control: return "They can run agents and commands on the runners you select."
        case .read: return "They can follow the work on the runners you select, but can’t change it."
        }
    }
}

/// A teammate's code: who the directory says they are, and what the person
/// adding them chose.
public struct TeammateChoice: Equatable, Sendable {
    public var who: CeremonyTeammate
    /// The directory's answers the scan was checked against, kept to
    /// check the same code again at the tap, as the same-account path
    /// re-scans.
    public var directory: String
    /// The account signed in on the device doing the adding, which the
    /// check is made for.
    public var scanner: String
    /// That device's email, written into the reply as whose runners these are.
    public var grantedBy: String
    /// The teams whose rosters hold this device while it waits out a
    /// trust reset's hold (R-76): said on the sheet, and confirmed with
    /// each team at the tap, before anything is written.
    public var held: [CeremonyHold] = []
    public var handle: String
    public var name: String
    public var scope: TeammateScope = .control

    public init(
        who: CeremonyTeammate, directory: String, scanner: String, grantedBy: String,
        held: [CeremonyHold] = [], handle: String, name: String, scope: TeammateScope = .control
    ) {
        self.who = who
        self.directory = directory
        self.scanner = scanner
        self.grantedBy = grantedBy
        self.held = held
        self.handle = handle
        self.name = name
        self.scope = scope
    }

    /// What the directory suggests a teammate is called, before the person
    /// editing it: a handle from their email and a name to read.
    public init(
        who: CeremonyTeammate, directory: String, scanner: String, grantedBy: String, held: [CeremonyHold] = []
    ) {
        self.init(
            who: who, directory: directory, scanner: scanner, grantedBy: grantedBy, held: held,
            handle: TeammateNames.handle(email: who.email, taken: []), name: TeammateNames.name(email: who.email))
    }

    /// Whether what was typed is something a runner will take.
    public var isComplete: Bool {
        TeammateNames.isValidHandle(handle) && !name.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

/// The core's answer to a teammate's code it accepted
/// (`farcooler_client_ceremony_scan_teammate`): the offer re-encoded, who the
/// directory says they are, and any hold on their device.
public struct TeammateAccepted: Decodable, Sendable {
    public let offer: String
    public let teammate: CeremonyTeammate
    public let held: [CeremonyHold]?
}

/// What became of one runner's half of Add Teammate: `teammate.enroll`'s
/// answer on a phone (ov-854), which `crates/client/src/session/teammate.rs`
/// words and this decodes.
public enum TeammateAddOutcome: Equatable, Sendable {
    /// Key A is on the runner and bound to the teammate. `tunnel` is the
    /// runner's token when it admitted the device's node key, else empty.
    case written(tunnel: String)
    /// This phone isn't a `host_admin` device there; nothing was written.
    case needsAdmin
    /// The runner's Far Cooler is too old to keep a member's account.
    case tooOld
    /// Who's on the runner couldn't be read, so nothing was written.
    case couldNotRead
    /// The device is already another member's there.
    case someoneElses
    /// The write failed, and the key was taken off again (`stillThere`: or
    /// couldn't be).
    case notAdded(stillThere: Bool)
    /// The key went on and couldn't be bound, so it was taken off again.
    case notRecorded(stillThere: Bool)
    /// The device was bound before, and its record couldn't be updated.
    case notUpdated
    /// The phone never reached the runner, or the runner refused the call:
    /// nothing is known to have been written.
    case notReached

    /// Decode `teammate.enroll`'s JSON. An answer that doesn't read is
    /// ``notReached``, never a guess at success.
    public static func decode(_ data: Data) -> TeammateAddOutcome {
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let word = object["outcome"] as? String
        else { return .notReached }
        let stillThere = object["stillThere"] as? Bool ?? false
        switch word {
        case "written": return .written(tunnel: object["tunnel"] as? String ?? "")
        case "needs_admin": return .needsAdmin
        case "too_old": return .tooOld
        case "could_not_read": return .couldNotRead
        case "someone_elses": return .someoneElses
        case "not_added": return .notAdded(stillThere: stillThere)
        case "not_recorded": return .notRecorded(stillThere: stillThere)
        case "not_updated": return .notUpdated
        default: return .notReached
        }
    }

    /// Whether the teammate's device can connect to this runner now.
    public var isWritten: Bool {
        if case .written = self { return true }
        return false
    }
}

/// The sentences for Add Teammate on a phone. The first four are the Mac's,
/// word for word (`TeammateEnrollment`); the rest say what a phone can't do.
public enum TeammateAddCopy {
    /// What to say about the runners that didn't take the teammate's device,
    /// one sentence for each, or nil when they all did. `device` is what this
    /// phone is called in a sentence ("iPhone").
    public static func note(about outcomes: [(runner: String, outcome: TeammateAddOutcome)], device: String) -> String? {
        let said = outcomes.compactMap { sentence($0.outcome, runner: $0.runner, device: device) }
        return said.isEmpty ? nil : said.joined(separator: "\n\n")
    }

    /// One runner's sentence, or nil when the device is there and bound.
    public static func sentence(_ outcome: TeammateAddOutcome, runner: String, device: String) -> String? {
        switch outcome {
        case .written:
            return nil
        case .needsAdmin:
            return "This \(device) can only control \(runner), so it can’t add a teammate there. "
                + "Add your teammate from your Mac."
        case .tooOld:
            return "\(runner)’s Far Cooler is too old to record whose device this is, so nothing was added there. "
                + "Update Far Cooler on it, then add your teammate again."
        case .couldNotRead:
            return "Far Cooler couldn’t read who’s on \(runner), so it didn’t add your teammate’s device there."
        case .someoneElses:
            return "This device is already someone else’s on \(runner), so it wasn’t added for your teammate."
        case .notAdded(let stillThere):
            return "Far Cooler couldn’t add your teammate’s device to \(runner), and any access it had there before is gone. "
                + (stillThere ? Self.stillThere : Self.takenOff)
        case .notRecorded(let stillThere):
            return "Far Cooler couldn’t record this device as your teammate’s on \(runner). "
                + (stillThere ? Self.stillThere : Self.takenOff)
        case .notUpdated:
            return "Far Cooler couldn’t update your teammate’s record on \(runner), but their device still has "
                + "the access you chose. Add your teammate again to finish updating it."
        case .notReached:
            return "This \(device) couldn’t reach \(runner), so your teammate’s device wasn’t added there."
        }
    }

    /// Taking the key off worked.
    static let takenOff = "It took the device’s key off again, so it has no access there."
    /// Taking the key off failed too: loud.
    static let stillThere =
        "It also couldn’t make sure the device’s key isn’t on that runner. Check Settings > Devices for it."
}

/// What a teammate's code is refused with, in the Mac's words (ov-525) and now
/// the phone's. The core's word picks the case; these are its sentences.
public enum TeammateRefusalCopy {
    public static let directoryUnreachableTitle = "Couldn’t check your teams"
    public static let directoryUnreachable =
        "That device uses another account, and Far Cooler couldn’t reach your team directory to "
        + "check it’s a teammate’s, so nothing was added. Check your connection, then try again."

    public static let notATeammateTitle = "This device’s account isn’t on any of your teams"
    public static func notATeammate(email: String) -> String {
        "Invite its owner to one of your teams, or sign in to \(email) on that device, then show "
            + "its code again."
    }

    public static let teammateShellTitle = "Teammates can’t have shell access"
    public static let teammateShell = "On that Mac, turn off shell access, then show its code again."

    public static let unregisteredKeyTitle = "Their account hasn’t confirmed that device"
    /// The next step, now there is one (ov-550 review L2). A first device is
    /// confirmed by signing in on it; a later or reinstalled one by their
    /// scanning its code from a device already confirmed.
    public static let unregisteredKey =
        "Far Cooler adds a teammate’s device only once their account has confirmed it. "
        + "Ask them to sign in to Far Cooler on it, then scan its code from another of their "
        + "devices that’s already confirmed. If it’s their first device, signing in is enough. "
        + "Then scan its code here again."

    /// A phone has no way to confirm a held device in person, so it refuses
    /// one and sends the person to the Mac, which does.
    public static let heldKeyTitle = "Their device is waiting out a hold"
    public static let heldKeyOnAPhone =
        "Their account confirmed it after it had no confirmed device left, so your team can add it once "
        + "the hold ends. Add it from your Mac, where you can confirm it in person."
}

/// The call a phone makes to a runner for one Add Teammate (ov-854), in one
/// place so the iPhone's call and its test read the same words. Android sends
/// the same names from `TeammateAdd.kt`.
///
/// **One call, not `client.enroll` and then a bind.** `teammate.enroll` is the
/// client core's: it writes the key `for_teammate` (so the runner holds it
/// nobody's until it is bound), binds it to the member and takes it off again
/// if either step fails. A phone that sent a plain `client.enroll` would leave
/// the teammate's key the owner's.
public enum TeammateAddRequest {
    public static let method = "teammate.enroll"

    public static func arguments(
        publicKey: String, label: String, clientId: String, nodeKey: String, teammate: TeammateChoice
    ) -> [String: Any] {
        [
            "publicKey": publicKey,
            "label": label,
            "clientId": clientId,
            // Control or Read; the core refuses anything wider.
            "scope": teammate.scope.rawValue,
            "nodeKey": nodeKey,
            "handle": teammate.handle,
            "name": teammate.name.trimmingCharacters(in: .whitespaces),
            // The directory's, never the code's.
            "account": teammate.who.account,
            "email": teammate.who.email,
        ]
    }
}
