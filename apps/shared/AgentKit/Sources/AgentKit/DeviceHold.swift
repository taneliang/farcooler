import Foundation

/// The hold on a device trusted by a reset (ruling R-76, ov-565).
///
/// An account with no confirmed device trusts its next one (R-75), and a
/// stolen session can bring that about. So the relay holds that device, and
/// any it confirms, out of the keys a team's admins are sent for 72 hours,
/// unless an admin of the team confirms it with the person in front of them
/// (`services/relay/src/directory/hold.ts`). This is what each app says about
/// it, and the admin's half of confirming it.
extension DeviceKeyRow {
    /// When its hold ends, if it's still waiting it out at `now`. The relay
    /// sends `heldUntil` only while a hold lasts; `now` only stops a row
    /// loaded before the end from saying so after it.
    public func waitingOutHold(at now: Date) -> Date? {
        guard let heldUntil, heldUntil > now else { return nil }
        return heldUntil
    }

    /// What a held device means for the person's teams, with the day and
    /// time it ends.
    public static func holdSentence(until: Date) -> String {
        "Your team admins can’t add it to runners until "
            + "\(until.formatted(date: .abbreviated, time: .shortened)), unless one confirms it with you in person."
    }
}

extension DirectoryClient {
    /// A team admin confirms, with its person in front of them, that a
    /// teammate's held device is theirs: the relay lists it to `team` from
    /// now on. `keyA` is the key from the code this device scanned, never one
    /// the relay named.
    public func confirmHeldDevice(team: String, user: String, keyA: String) async -> Result<Void, DirectoryFailure> {
        switch await post("/v1/directory/teams/devices/confirm", ["team": team, "user": user, "keyA": keyA]) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            guard reply.status == 200, reply.body["ok"] as? Bool == true else { return .failure(Self.failure(reply)) }
            return .success(())
        }
    }
}

/// A team's admin lifting a device's hold, as `directory/me` records it.
public struct HoldConfirmation: Equatable, Sendable {
    /// The team's name, as its admins chose it: quoted through
    /// `DeviceKeyRow.shown`.
    public let team: String
    public let at: Date

    public init(team: String, at: Date) {
        self.team = team
        self.at = at
    }
}
