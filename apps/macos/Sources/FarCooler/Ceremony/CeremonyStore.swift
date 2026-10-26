import AgentKit
import CFarCoolerClient
import Foundation
import LocalAuthentication
import os

// MARK: - What crosses the FFI

/// The first code: what a new device shows.
///
/// Decoded here only to put names on a screen. Nothing in this struct is
/// checked in Swift — `farcooler_client_ceremony_scan` has already refused
/// anything this build must not act on, and a second opinion in Swift would be
/// a second thing to get wrong on one of three platforms.
struct CeremonyOffer: Codable, Equatable {
    var v: Int
    var key_a: String
    var key_b: String?
    var name: String
    var account: String
    var channel: String
    var ceremony: String
    /// The device's tailcat node public key: 43 characters of unpadded
    /// base64-URL, or empty from a device that has none.
    ///
    /// **Optional here, not on the wire.** `crates/ceremony/src/lib.rs`
    /// always serializes it, so a code this build scanned always carries the
    /// key. Decoding it as optional anyway costs one `?? ""` and buys the one
    /// thing that matters: a missing field turns into an empty node key rather
    /// than into `.unreadable`, which would refuse the whole ceremony over a
    /// field that only ever decides whether a tunnel is offered.
    ///
    /// Empty is not "unknown". It is a device that cannot be admitted to a
    /// tunnel — a v=1 offer, or a phone whose own mint failed — and the reply
    /// grants it addresses only. `ceremony::can_be_granted_a_tunnel` reads the
    /// same field for the same reason on the other side of the exchange.
    var node_key: String?

    /// `SHA256:t7Xq…9Vd`, for the person holding both devices to compare.
    ///
    /// The only thing on the confirmation that is not a name somebody typed. It
    /// is worth showing in full behind a disclosure and abbreviated in the
    /// line, because an abbreviated fingerprint compared carefully is worth
    /// more than a full one nobody reads.
    var fingerprint: String? { RunnerFacts.fingerprint(ofOpenSSHKey: key_a) }
}

/// One runner in a reply: everything a device needs to reach it, and nothing it
/// needs to trust it with.
///
/// Field names are the wire's, not Swift's, because this round-trips through
/// `crates/ceremony/src/lib.rs` byte for byte and a `CodingKeys` table is a
/// second place for a name to be wrong.
struct CeremonyRunner: Codable, Equatable, Identifiable {
    var id: String
    var label: String
    var alias: String
    var user: String
    var host_key: String
    var reach: CeremonyReach
    var pending: Bool
    /// A tunneled runner's address, kept as the route a person may choose when
    /// the tunnel fails (ov-485). Nil beside an address, and from older builds.
    /// Never dialed on its own: see `TunnelRoute`.
    var alternative: CeremonyReach? = nil
}

/// How a granted runner is reached: an address, or the tunnel.
///
/// One or the other and never both — two optional fields would admit "both set"
/// and "neither set", and then something here would have to pick a winner. The
/// wire is tagged on `kind` so a third kind is additive, and an unrecognized one
/// throws rather than decoding to a default: the core has already accepted the
/// manifest by the time this runs, so a tag this does not know is this app
/// failing, and ``Refusal/unreadable`` is what says so.
enum CeremonyReach: Codable, Equatable {
    case direct(host: String, port: Int)
    case tailcat(token: String)

    private enum Field: String, CodingKey {
        case kind, host, port, token
    }

    init(from decoder: Decoder) throws {
        let wire = try decoder.container(keyedBy: Field.self)
        switch try wire.decode(String.self, forKey: .kind) {
        case "direct":
            self = .direct(
                host: try wire.decode(String.self, forKey: .host),
                port: try wire.decode(Int.self, forKey: .port))
        case "tailcat":
            self = .tailcat(token: try wire.decode(String.self, forKey: .token))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: wire, debugDescription: "unknown reach \(other)")
        }
    }

    func encode(to encoder: Encoder) throws {
        var wire = encoder.container(keyedBy: Field.self)
        switch self {
        case .direct(let host, let port):
            try wire.encode("direct", forKey: .kind)
            try wire.encode(host, forKey: .host)
            try wire.encode(port, forKey: .port)
        case .tailcat(let token):
            try wire.encode("tailcat", forKey: .kind)
            try wire.encode(token, forKey: .token)
        }
    }

    /// The address, for the things that are genuinely about one — judging how
    /// far it travels, writing a `HostName`. Nil for a tunnel, which has none,
    /// so a caller has to say what it does about that rather than inherit an
    /// empty string.
    var host: String? {
        if case .direct(let host, _) = self { return host }
        return nil
    }

    var port: Int? {
        if case .direct(_, let port) = self { return port }
        return nil
    }

    /// The same reach at a different address. Used where this Mac finds a name
    /// for the same runner that survives leaving the building; a tunnel already
    /// does, so it comes back unchanged.
    func replacingHost(_ host: String) -> CeremonyReach {
        guard case .direct(_, let port) = self else { return self }
        return .direct(host: host, port: port)
    }

    /// What a person sees where an address would go.
    ///
    /// Never the token: it is long, it is meaningless to read, and it is the one
    /// field here worth stealing.
    var display: String {
        switch self {
        case .direct(let host, _): return host
        case .tailcat: return "Through the tunnel"
        }
    }

    /// A name for a sentence, when the granting side sent no label or alias.
    func name(user: String) -> String {
        switch self {
        case .direct(let host, _): return "\(user)@\(host)"
        case .tailcat: return "a tunneled runner"
        }
    }
}

/// The reply: the runners a trusted device granted, addressed to one ceremony.
struct CeremonyManifest: Codable, Equatable {
    var v: Int
    var ceremony: String
    var account: String
    var channel: String
    var target: String
    var runners: [CeremonyRunner]
    /// Whose runners these are, on a reply from a teammate's runner admin
    /// (ov-525): their email. Nil on a same-account reply. Shown, never
    /// trusted: the reply proves nothing about who made it.
    var granted_by: String? = nil
}

/// Who the team directory says a teammate is (ov-525): the account their
/// code named, and the email and team the DIRECTORY has for it. Nothing here
/// came from the code but the account it was looked up by.
struct CeremonyTeammate: Codable, Equatable {
    var account: String
    var email: String
    var team: String
}

/// A team whose roster holds a teammate's device while it waits out the hold
/// after their account's trust reset (ruling R-76, ov-565), and when the hold
/// ends, in milliseconds by the relay's clock. Shown, never judged here.
struct CeremonyHold: Codable, Equatable {
    var team: String
    var until: Double

    var ends: Date { Date(timeIntervalSince1970: until / 1000) }

    /// What the confirmation says about it, above Add Teammate: why the
    /// device is held, and what adding it now vouches for.
    static func sentence(_ holds: [CeremonyHold], email: String) -> String {
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
enum TeammateScope: String, CaseIterable, Identifiable, Equatable {
    case control, read

    var id: String { rawValue }

    var title: String {
        switch self {
        case .control: return "Control"
        case .read: return "Read"
        }
    }

    /// What it lets them do, under the picker.
    var detail: String {
        switch self {
        case .control: return "They can run agents and commands on the runners you select."
        case .read: return "They can follow the work on the runners you select, but can’t change it."
        }
    }
}

// MARK: - Refusals

/// Why a code was refused, and the sentence that belongs to it.
///
/// **The FFI returns a word; this file owns the sentence.** A Rust error string
/// must never reach a screen — it names types and crates, it is not localized,
/// and it tells someone nothing they can act on. Every case here maps a stable
/// code from `CeremonyError::code` to copy written for the person holding the
/// two devices.
enum Refusal: Equatable {
    case version(Int)
    case channel(String)
    case malformed
    case wrongCeremony
    case wrongAccount
    case wrongTarget
    case stale
    case alreadyTaken
    case tooLarge
    /// The reply granted a runner reachable only through the tunnel, and this
    /// device named no node key a tunnel would admit. The granting side's bug,
    /// refused whole rather than handed on as a runner that can only fail.
    case noTunnel
    /// The team directory couldn't be asked whether this code's account is on
    /// one of yours, so it was refused rather than waved through (ov-525).
    case directoryUnreachable
    /// The code's account is on none of your teams. Carries the signed-in
    /// email, for "sign in to … on that device", which is the other way in.
    case notATeammate(String)
    /// A teammate's Mac asked for shell access, which a teammate never gets.
    case teammateShell
    /// A teammate's device waiting out a trust reset's hold (R-76), which the
    /// relay didn't record this admin confirming, so nothing was added.
    case holdNotConfirmed
    /// The core refused a held key because the caller didn't say it would
    /// confirm it. This app always says so; a bug if it's ever shown.
    case heldKey
    /// The directory lists that teammate's devices, and this isn't one.
    case unregisteredKey
    /// The FFI answered nothing at all. A bug, not a refusal, and it says so
    /// rather than inventing a cause.
    case unreadable

    /// The headline.
    var title: String {
        switch self {
        case .version: return "Update Far Cooler to scan this code"
        case .channel: return "This code is from a different version of Far Cooler"
        case .malformed: return "This isn’t a Far Cooler code"
        case .wrongCeremony: return "This code is for a different request"
        case .wrongAccount: return "This code is for a different account"
        case .wrongTarget: return "This code is for another device"
        case .stale: return "This code expired"
        case .alreadyTaken: return "This code was already used"
        case .tooLarge: return "Too many runners selected"
        case .noTunnel: return "This Mac can’t reach one of those runners"
        case .directoryUnreachable: return "Couldn’t check your teams"
        case .notATeammate: return "This device’s account isn’t on any of your teams"
        case .teammateShell: return "Teammates can’t have shell access"
        case .holdNotConfirmed: return "Couldn’t confirm their device"
        case .heldKey: return "Their device is waiting out a hold"
        case .unregisteredKey: return "Their account hasn’t confirmed that device"
        case .unreadable: return "Couldn’t read this code"
        }
    }

    /// The sentence under it, which says what to do next.
    var detail: String {
        switch self {
        case .version:
            return "Update Far Cooler on this Mac, then show the code again."
        case .channel(let name):
            return "The other device is using Far Cooler \(name.capitalized). Both devices must "
                + "use the same version."
        case .malformed:
            return "Scan the code shown in Far Cooler on the other device."
        case .wrongCeremony:
            return "Show a new code on this Mac, then scan the code on the other device."
        case .wrongAccount:
            return "Both devices must be signed in to the same account."
        case .wrongTarget:
            return "Show a new code on this Mac, then scan the code on the other device."
        case .stale:
            return "Show a new code, then scan it within two minutes."
        case .alreadyTaken:
            return "Show a new code, then try again."
        case .tooLarge:
            return "Select fewer runners. You can add the others later."
        case .noTunnel:
            return "One of them is reachable only through a tunnel, and this Mac isn’t on "
                + "one. Try again, choosing runners it can reach."
        case .directoryUnreachable:
            return "That device uses another account, and Far Cooler couldn’t reach your team directory to "
                + "check it’s a teammate’s, so nothing was added. Check your connection, then try again."
        case .notATeammate(let email):
            return "Invite its owner to one of your teams, or sign in to \(email) on that device, then show "
                + "its code again."
        case .teammateShell:
            return "On that Mac, turn off shell access, then show its code again."
        case .heldKey:
            return "Their account confirmed it after it had no confirmed device left, so your team can add it once "
                + "the hold ends. Update Far Cooler on this Mac, then scan its code again."
        case .holdNotConfirmed:
            return "Far Cooler couldn’t record with your team that you confirmed their device, so nothing was "
                + "added. Check your connection, then scan its code again."
        case .unregisteredKey:
            // The next step, now there is one (review L2, ov-550). A first
            // device is confirmed by signing in on it (`keys: []`); a later
            // or reinstalled one by their scanning its code from a device
            // already confirmed, in the same-account ceremony.
            // "That device", never "this": this Mac is the admin's.
            return "Far Cooler adds a teammate’s device only once their account has confirmed it. "
                + "Ask them to sign in to Far Cooler on it, then scan its code from another of their "
                + "devices that’s already confirmed. If it’s their first device, signing in is enough. "
                + "Then scan its code here again."
        case .unreadable:
            return "Show a new code, then try again."
        }
    }

    /// A refusal the person can retry from, versus one where the way forward is
    /// somewhere else entirely.
    var retryable: Bool {
        switch self {
        case .version, .channel, .notATeammate: return false
        default: return true
        }
    }

    /// Read `{"error":"stale"}` and friends. Nil when the payload is not a
    /// refusal, which is how the callers tell an answer from a rejection.
    static func from(_ json: String) -> Refusal? {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let code = object["error"] as? String
        else { return nil }
        switch code {
        case "version": return .version(object["version"] as? Int ?? 0)
        case "channel": return .channel(object["channel"] as? String ?? "another channel")
        case "malformed": return .malformed
        case "wrong_ceremony": return .wrongCeremony
        case "wrong_account": return .wrongAccount
        case "wrong_target": return .wrongTarget
        case "stale": return .stale
        case "already_taken": return .alreadyTaken
        case "too_large": return .tooLarge
        case "no_tunnel": return .noTunnel
        case "directory_unreachable": return .directoryUnreachable
        // The email is the caller's to fill in: see `CeremonyStore.checkTeammate`.
        case "not_a_teammate": return .notATeammate("")
        case "teammate_shell": return .teammateShell
        case "unregistered_key": return .unregisteredKey
        case "held_key": return .heldKey
        // A word this build does not have. It is still a refusal — reporting it
        // as a successful scan would be the one wrong answer.
        default: return .unreadable
        }
    }
}

// MARK: - The four entry points

/// The ceremony, as four calls into `crates/ceremony/src/lib.rs`.
///
/// Every one of them takes the buffer contract `farcooler_client_generate_key`
/// has used since the first release: JSON into a buffer, returning the bytes
/// needed, writing nothing when the buffer is short. So `spill` calls twice at
/// most and never truncates — a half-written payload parses as nothing and
/// looks exactly like a corrupt scan.
enum CeremonyFFI {
    /// `nodeKey` is this Mac's tunnel public key, or nil. Nil is the `v=1`
    /// offer: direct runners granted, tunneled ones refused by the core rather
    /// than half-accepted here. See ``MacNodeIdentity``.
    static func offer(
        name: String, account: String, keyA: String, keyB: String?, nodeKey: String? = nil
    ) -> String? {
        name.withCString { name in
            account.withCString { account in
                keyA.withCString { keyA in
                    withOptionalCString(keyB) { keyB in
                        withOptionalCString(nodeKey) { nodeKey in
                            spill {
                                farcooler_client_ceremony_offer(
                                    name, account, keyA, keyB, nodeKey, $0, $1)
                            }
                        }
                    }
                }
            }
        }
    }

    /// `expectingAccount` is which account is asking; the core answers
    /// `wrong_account` itself. This app used to make that comparison, which
    /// meant three apps each held their own copy of a security rule.
    static func scan(
        _ encoded: String, expectingAccount: String, heldFor: TimeInterval
    ) -> String? {
        encoded.withCString { encoded in
            expectingAccount.withCString { account in
                spill {
                    farcooler_client_ceremony_scan(encoded, account, milliseconds(heldFor), $0, $1)
                }
            }
        }
    }

    /// `grantedBy` is the email of the account granting, for a teammate's
    /// reply (ov-525), and nil for one of your own devices, whose reply is
    /// built exactly as it always was.
    static func reply(offer: String, runners: [CeremonyRunner], budget: Int, grantedBy: String? = nil) -> String? {
        guard let runnersJSON = try? JSONEncoder().encode(runners) else { return nil }
        return offer.withCString { offer in
            String(decoding: runnersJSON, as: UTF8.self).withCString { runners in
                guard let grantedBy else {
                    return spill { farcooler_client_ceremony_reply(offer, runners, budget, $0, $1) }
                }
                return grantedBy.withCString { grantedBy in
                    spill { farcooler_client_ceremony_teammate_reply(offer, runners, grantedBy, budget, $0, $1) }
                }
            }
        }
    }

    /// A code from another account, checked against the team directory's own
    /// answers (`directory`, nil when it couldn't be asked). The core decides;
    /// this passes the bytes along.
    static func scanTeammate(
        _ encoded: String, expectingAccount: String, directory: String?, heldFor: TimeInterval
    ) -> String? {
        encoded.withCString { encoded in
            expectingAccount.withCString { account in
                withOptionalCString(directory) { directory in
                    spill {
                        // True: this app says a hold on the sheet and confirms
                        // it with each team before writing (`confirm`).
                        farcooler_client_ceremony_scan_teammate(
                            encoded, account, directory, milliseconds(heldFor), true, $0, $1)
                    }
                }
            }
        }
    }

    static func accept(
        _ encoded: String, expecting: String, alreadyTaken: Bool, heldFor: TimeInterval
    ) -> String? {
        encoded.withCString { encoded in
            expecting.withCString { expecting in
                spill {
                    farcooler_client_ceremony_accept(
                        encoded, expecting, alreadyTaken, milliseconds(heldFor), $0, $1)
                }
            }
        }
    }

    /// Negative elapsed time is not a thing, and `UInt64(-1)` is a very long
    /// time indeed — which would read as fresh forever. A clock that moved
    /// backwards clamps to zero, and the freshness window still bounds how long
    /// the sheet may stay open.
    private static func milliseconds(_ interval: TimeInterval) -> UInt64 {
        UInt64(max(0, interval) * 1000)
    }

    private static func withOptionalCString<T>(
        _ value: String?, _ body: (UnsafePointer<CChar>?) -> T
    ) -> T {
        guard let value else { return body(nil) }
        return value.withCString(body)
    }

    private static func spill(_ call: (UnsafeMutablePointer<UInt8>?, Int) -> Int) -> String? {
        // Ample for an offer, and for a manifest of a dozen runners. The second
        // call exists for the case it is not.
        var buffer = [UInt8](repeating: 0, count: 4096)
        var needed = buffer.withUnsafeMutableBufferPointer { call($0.baseAddress, $0.count) }
        if needed > buffer.count {
            buffer = [UInt8](repeating: 0, count: needed)
            needed = buffer.withUnsafeMutableBufferPointer { call($0.baseAddress, $0.count) }
        }
        guard needed > 0, needed <= buffer.count else { return nil }
        return String(decoding: buffer[0..<needed], as: UTF8.self)
    }
}

// MARK: - The state machine

/// The ceremony on this Mac, in both directions.
///
/// One object for both because they are one conversation: the Mac that scans a
/// phone's code and the Mac that shows its own are running the same four
/// moments from opposite ends, and splitting them would duplicate the freshness
/// bookkeeping — which is the part that must not be duplicated.
///
/// **This makes no decision about whether a scan is acceptable.** Every phase
/// change below is the answer the FFI gave, rendered.
@MainActor
final class CeremonyStore: ObservableObject {
    /// The code this Mac was showing when a teammate's reply came in.
    private var heldOffer = ""
    enum Phase: Equatable {
        /// This Mac is the new device: here is its code.
        case showingOffer(String)
        /// This Mac is the trusted device: point the camera at the other one.
        case scanning
        /// A code was read and accepted. Pick runners and confirm.
        case confirming(Confirmation)
        /// A code from another account, being checked with the team
        /// directory before anything about it is shown (ov-525).
        case checkingTeam
        /// A reply from another account's runner admin was taken and is held
        /// until the person says yes (ov-543). Nothing is added before that.
        case confirmingJoin(CeremonyManifest)
        /// Writing keys to runners.
        case enrolling
        /// The reply, for the new device to scan.
        case showingManifest(String)
        case done(String)
        case refused(Refusal)
    }

    /// What the confirmation sheet is about.
    struct Confirmation: Equatable {
        /// The offer exactly as the FFI re-encoded it. Passed back to
        /// `ceremony_reply` verbatim, never rebuilt from the decoded struct:
        /// what was shown and what is answered must not be able to drift.
        var offerJSON: String
        var offer: CeremonyOffer
        /// When this device scanned, by its own clock. The only clock that
        /// counts — a timestamp inside a code is controlled by whoever is
        /// showing it.
        var scannedAt: Date
        var rows: [RunnerRow]
        /// Present when the device being added is a Mac, which is the only
        /// device with a second key.
        var shell: ShellKeyChoice?
        /// Present when the code came from a teammate's account (ov-525).
        var teammate: TeammateChoice? = nil
    }

    /// A teammate's code: who the directory says they are, and what the
    /// person adding them chose.
    struct TeammateChoice: Equatable {
        var who: CeremonyTeammate
        /// The directory's answers the scan was checked against, kept to
        /// check the same code again at the tap, as the same-account path
        /// re-scans.
        var directory: String
        /// The account signed in on this Mac, which the check is made for.
        var scanner: String
        /// This Mac's email, written into the reply as whose runners these are.
        var grantedBy: String
        /// The teams whose rosters hold this device while it waits out a
        /// trust reset's hold (R-76): said on the sheet, and confirmed with
        /// each team at the tap, before anything is written.
        var held: [CeremonyHold] = []
        var handle: String
        var name: String
        var scope: TeammateScope = .control

        /// Whether what was typed is something a runner will take.
        var isComplete: Bool {
            TeammateNames.isValidHandle(handle) && !name.trimmingCharacters(in: .whitespaces).isEmpty
        }
    }

    /// One runner, and whether this ceremony grants it.
    struct RunnerRow: Equatable, Identifiable {
        var runner: CeremonyRunner
        var granted: Bool
        /// "this Mac", or the account this runner was reached through when it
        /// is not the one signing. A runner reached through a different account
        /// is listed and labeled, never hidden.
        var note: String?
        var id: String { runner.id }
    }

    @Published private(set) var phase: Phase = .scanning
    /// The transcript of the enrollment, in the CLI's own words, for the row
    /// that could not be granted. Shown in a `DetailBox`, the way an install's
    /// output already is — it is the thing that talked to the runner.
    @Published private(set) var transcript: String?
    /// The sentence above that transcript, chosen from what the enrollment
    /// actually did — see ``Enrollment/note(about:outcome:)``. Nil when every
    /// selected runner took the key, which is the ordinary case.
    @Published private(set) var note: String?
    /// The sentence about runners the reply granted through the tunnel, or nil
    /// when it granted none that way — which is every ordinary pairing.
    ///
    /// Its own property rather than a second value in ``note`` because it is not
    /// a warning: ``note`` is drawn in orange and says something went wrong,
    /// while this says the ceremony routed around a runner's dead address. Both
    /// can be true at once, on different runners, so neither may replace the
    /// other.
    @Published private(set) var tunnelNote: String?
    /// The runners the reply had to grant at their address alone, by name, or
    /// nil (ov-485 N1). A warning, drawn in orange: the new device can't use
    /// Tailcat for them.
    @Published private(set) var addressOnlyNote: String?

    private static let log = Logger(subsystem: "com.farcooler.FarCooler", category: "ceremony")

    private var alreadyTaken = false

    /// The team directory's answers, or nil when it couldn't be asked. A test
    /// answers it; the app asks the relay.
    var teamDirectory: () async -> Data? = { await Account.shared.directoryClient.teamDirectory() }
    /// The teammate check in flight after a code from another account.
    private(set) var teamCheck: Task<Void, Never>?
    /// Vouch for the device just added to this Mac's own account (ov-550):
    /// sign a certificate for the Key A its code carried and post it, so the
    /// directory lists it to a team's admins. A test records the call; the
    /// app signs with this Mac's key for the account.
    var certify: (_ account: String, _ keyA: String) async -> Void = {
        await DeviceCertification.shared.certifyScanned(account: $0, keyA: $1)
    }
    /// An admin confirming, in person, a teammate's device that waits out a
    /// trust reset's hold (R-76): true once the relay has recorded it for
    /// `team`. A test records the call; the app asks the relay.
    var confirmHold: (_ team: String, _ user: String, _ keyA: String) async -> Bool = { team, user, keyA in
        if case .success = await Account.shared.directoryClient.confirmHeldDevice(team: team, user: user, keyA: keyA) {
            return true
        }
        return false
    }
    /// The certification started by the last confirmed same-account reply.
    private(set) var certification: Task<Void, Never>?
    /// Which scan the check in flight belongs to (review F3). Every scan takes
    /// a new one, so a slow check for an earlier code can never open its
    /// teammate's sheet over a later code.
    private var scanNumber = 0

    // MARK: This Mac as the new device

    /// Show this Mac's code.
    ///
    /// The string the FFI returns is BOTH what goes in the code and what this
    /// Mac keeps, to hand back as `expecting` when the reply arrives. Keeping
    /// one string rather than re-encoding the struct is what stops what is on
    /// screen and what is remembered from drifting apart.
    func present(
        name: String, account: String, keyA: String, keyB: String?, nodeKey: String? = nil
    ) {
        guard
            let answer = CeremonyFFI.offer(
                name: name, account: account, keyA: keyA, keyB: keyB, nodeKey: nodeKey)
        else { return refuse(.unreadable) }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        alreadyTaken = false
        phase = .showingOffer(answer)
    }

    /// Take the reply this Mac was shown, or refuse it.
    func accept(_ scanned: String, at: Date) -> CeremonyManifest? {
        guard case .showingOffer(let expecting) = phase else { return nil }
        guard
            let answer = CeremonyFFI.accept(
                scanned, expecting: expecting, alreadyTaken: alreadyTaken,
                heldFor: Date().timeIntervalSince(at))
        else {
            refuse(.unreadable)
            return nil
        }
        if let refusal = Refusal.from(answer) {
            refuse(refusal)
            return nil
        }
        // One reply per ceremony, recorded the moment one is taken, so a forged
        // reply cannot follow a real one. Rust enforces it; this is the state
        // Rust is enforcing it against.
        alreadyTaken = true
        guard let data = answer.data(using: .utf8),
            let manifest = try? JSONDecoder().decode(CeremonyManifest.self, from: data)
        else {
            refuse(.unreadable)
            return nil
        }
        // Another account's runners: asked about, never joined silently. The
        // caller gets nothing back until `join()` says the person agreed.
        if manifest.grantor != nil {
            heldOffer = expecting
            phase = .confirmingJoin(manifest)
            return nil
        }
        return manifest
    }

    /// The person said yes to a teammate's runners: they're this Mac's now.
    /// Nil when no teammate's reply is waiting.
    func join() -> CeremonyManifest? {
        guard case .confirmingJoin(let manifest) = phase else { return nil }
        phase = .showingOffer(heldOffer)
        return manifest
    }

    /// The person said no. Nothing was added, and the reply stays consumed:
    /// a new code is the way back.
    func cancelJoin() {
        guard case .confirmingJoin = phase else { return }
        phase = .showingOffer(heldOffer)
    }

    // MARK: This Mac as the trusted device

    /// Read a scanned offer and, if this build may act on it, open the
    /// confirmation.
    ///
    /// **This app makes no rule.** The account comparison used to be here,
    /// because `ceremony_scan` was never told which account was asking; it now
    /// takes one and answers `wrong_account`, so the only thing left is which
    /// screen a refusal leads to.
    ///
    /// A code from another account goes on to the teammate check (ov-525),
    /// and nothing about it is shown until the core has said yes: reaching
    /// the runner list with a stranger's code would put the fleet on screen
    /// with only a fingerprint between it and them.
    func scan(_ scanned: String, at: Date, account: String, email: String, runners: [RunnerRow]) {
        scanNumber += 1
        guard
            let answer = CeremonyFFI.scan(
                scanned, expectingAccount: account, heldFor: Date().timeIntervalSince(at))
        else {
            return refuse(.unreadable)
        }
        if case .wrongAccount = Refusal.from(answer) {
            phase = .checkingTeam
            let number = scanNumber
            teamCheck = Task {
                await checkTeammate(scanned, at: at, account: account, email: email, runners: runners, scan: number)
            }
            return
        }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        guard let data = answer.data(using: .utf8),
            let offer = try? JSONDecoder().decode(CeremonyOffer.self, from: data)
        else { return refuse(.unreadable) }

        phase = .confirming(
            Confirmation(
                offerJSON: answer, offer: offer, scannedAt: at, rows: runners,
                shell: offer.key_b == nil ? nil : ShellKeyChoice(deviceName: offer.name)))
    }

    /// Ask the directory, then let the core decide whether a code from
    /// another account is a teammate's (ov-525).
    ///
    /// The time the directory took counts against the code's freshness, since
    /// the clock started at the scan. An unreachable directory is the core's
    /// `directory_unreachable`, never a pass.
    func checkTeammate(
        _ scanned: String, at: Date, account: String, email: String, runners: [RunnerRow], scan: Int
    ) async {
        let directory = await teamDirectory().map { String(decoding: $0, as: UTF8.self) }
        // Cancelled, or scanned again, while the directory was answering.
        // And a check for an earlier code, overtaken by a later one, drops its
        // answer rather than opening that teammate's sheet over this code.
        guard !Task.isCancelled, scan == scanNumber, phase == .checkingTeam else { return }
        guard
            let answer = CeremonyFFI.scanTeammate(
                scanned, expectingAccount: account, directory: directory, heldFor: Date().timeIntervalSince(at))
        else { return refuse(.unreadable) }
        if case .notATeammate = Refusal.from(answer) { return refuse(.notATeammate(email)) }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        struct Accepted: Decodable {
            let offer: String
            let teammate: CeremonyTeammate
            let held: [CeremonyHold]?
        }
        guard let accepted = try? JSONDecoder().decode(Accepted.self, from: Data(answer.utf8)),
            let offer = try? JSONDecoder().decode(CeremonyOffer.self, from: Data(accepted.offer.utf8))
        else { return refuse(.unreadable) }
        let who = accepted.teammate
        // Nothing is granted to someone else by default, this Mac least of all
        // (review F1): which runners a teammate gets is a choice made on
        // purpose. This Mac doesn't know which runners are in a team the two
        // share, so none starts checked.
        let rows = runners.map { RunnerRow(runner: $0.runner, granted: false, note: $0.note) }
        phase = .confirming(
            Confirmation(
                offerJSON: accepted.offer, offer: offer, scannedAt: at, rows: rows, shell: nil,
                teammate: TeammateChoice(
                    who: who, directory: directory ?? "", scanner: account, grantedBy: email,
                    held: accepted.held ?? [],
                    handle: TeammateNames.handle(email: who.email, taken: []),
                    name: TeammateNames.name(email: who.email))))
    }

    /// The person adding a teammate edits what they'll be called, and what
    /// they may do.
    func setTeammate(_ change: (inout TeammateChoice) -> Void) {
        guard case .confirming(var confirmation) = phase, var teammate = confirmation.teammate else { return }
        change(&teammate)
        confirmation.teammate = teammate
        phase = .confirming(confirmation)
    }

    func setGrant(_ granted: Bool, for id: String) {
        guard case .confirming(var confirmation) = phase else { return }
        guard let index = confirmation.rows.firstIndex(where: { $0.id == id }) else { return }
        confirmation.rows[index].granted = granted
        phase = .confirming(confirmation)
    }

    func setShell(_ shell: ShellKeyChoice) {
        guard case .confirming(var confirmation) = phase else { return }
        confirmation.shell = shell
        phase = .confirming(confirmation)
    }

    /// Confirm, behind a fingerprint, and build the reply.
    ///
    /// The freshness check runs AGAIN here, on the same scan, because the
    /// window bounds how long a confirmation may sit open — not how old the
    /// photograph was. A sheet left open over lunch refuses rather than
    /// enrolling.
    /// The keys go in BEFORE the reply is built, and the reply is built FROM
    /// what they did. That ordering is what makes `pending` a fact rather than
    /// a hope: a runner named in that code as ready is one the new device will
    /// try to connect to, and a claim no line was written for is the one lie in
    /// this flow that a person cannot detect — the phone simply fails to
    /// connect, with nothing anywhere saying why. iOS states the same rule in
    /// its own `confirm()`.
    func confirm(
        reason: String,
        enroll: @Sendable @escaping ([CeremonyRunner]) async -> Enrollment.Outcome
    ) async {
        guard case .confirming(let confirmation) = phase else { return }
        // A teammate with no usable handle or name can't be recorded on a
        // runner, so nothing is asked of the fingerprint either. The button is
        // off too; this is the rule, and that is the screen.
        if let teammate = confirmation.teammate, !teammate.isComplete { return }

        // A fingerprint, at the moment of the tap. This is what someone
        // standing at an unlocked laptop runs into, and it is the only thing
        // between them and an enrolled device — an account check does not help
        // there, because that laptop is signed in and they would be using its
        // session. Cancelled or failed enrolls nothing.
        guard await authenticate(reason) else { return }

        // Re-scanned rather than re-used: the answer is the same offer, and the
        // point of the call is the freshness rule attached to it. A teammate's
        // goes back through the teammate check, against the same answers.
        let held = Date().timeIntervalSince(confirmation.scannedAt)
        let teammate = confirmation.teammate
        let rechecked: String?
        if let teammate {
            rechecked = CeremonyFFI.scanTeammate(
                confirmation.offerJSON, expectingAccount: teammate.scanner, directory: teammate.directory,
                heldFor: held)
        } else {
            rechecked = CeremonyFFI.scan(
                confirmation.offerJSON, expectingAccount: confirmation.offer.account, heldFor: held)
        }
        guard let rechecked else { return refuse(.unreadable) }
        if let refusal = Refusal.from(rechecked) { return refuse(refusal) }
        let grantedBy = teammate?.grantedBy

        let wanted = confirmation.rows.filter(\.granted).map(\.runner)
        // Measured BEFORE anything is written (ov-485 H1). The core degrades
        // a reply over budget down to the address-only one, which no token
        // changes, so a refusal here is the only one the reply below can give:
        // asked now, it comes before a key reaches any runner.
        if let refusal = Self.refusalBeforeWriting(
            offer: confirmation.offerJSON, runners: wanted, budget: codeBudgetBytes, grantedBy: grantedBy)
        {
            return refuse(refusal)
        }

        // Held after a trust reset (R-76): this tap, in person, behind a
        // fingerprint, is the admin's confirmation, recorded with each team
        // that holds the device before any key is written. The holds come
        // from the re-scan, so they're the ones the key was just checked
        // against. Not recorded is nothing written.
        if let teammate {
            // Unreadable is not "none": a hold this can't read can't be
            // confirmed, so nothing is written (fix round 1, Low 2).
            guard let holds = Self.holds(in: rechecked) else { return refuse(.holdNotConfirmed) }
            for hold in holds {
                let confirmed = await confirmHold(hold.team, teammate.who.account, confirmation.offer.key_a)
                guard confirmed else { return refuse(.holdNotConfirmed) }
            }
        }

        phase = .enrolling
        let outcome = await enroll(wanted)
        // The FILE's state, not this Mac's intention. Every record arrives here
        // pending — see `RunnerFacts` — and only a runner that answered loses
        // the flag.
        let granted = outcome.granting(wanted)
        transcript = outcome.transcript
        note = Enrollment.note(about: granted, outcome: outcome)
        tunnelNote = outcome.tunneled.isEmpty ? nil : Enrollment.reachedThroughTheTunnel(others: false)

        guard
            let answer = CeremonyFFI.reply(
                offer: confirmation.offerJSON, runners: granted, budget: codeBudgetBytes, grantedBy: grantedBy)
        else { return refuse(.unreadable) }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        let fellBack = Self.addressOnly(granted: granted, reply: answer)
        addressOnlyNote = TunnelRoute.addressOnlySentence(fellBack.map(\.label))
        if !fellBack.isEmpty {
            tunnelNote = fellBack.count == granted.filter(\.isTunneled).count
                ? nil : Enrollment.reachedThroughTheTunnel(others: true)
        }
        phase = .showingManifest(answer)

        // Your own device, confirmed with a fingerprint just now: vouch for
        // it in the directory too, with the key from the code this Mac read
        // and never one the relay names. A teammate's device is never this
        // Mac's to vouch for. Off the main path: the reply is on screen
        // whatever the relay says, and a new device that hasn't registered
        // yet is waited for.
        if teammate == nil {
            let (account, keyA, certify) = (confirmation.offer.account, confirmation.offer.key_a, self.certify)
            certification = Task { await certify(account, keyA) }
        }
    }

    /// The holds a teammate scan's answer names (R-76): none when it names
    /// none, nil when its `held` can't be read, which the caller must treat
    /// as a hold it can't confirm (fix round 1, Low 2).
    static func holds(in answer: String) -> [CeremonyHold]? {
        struct Answer: Decodable { let held: [CeremonyHold]? }
        guard let decoded = try? JSONDecoder().decode(Answer.self, from: Data(answer.utf8)) else { return nil }
        return decoded.held ?? []
    }

    /// The granted runners the reply carries at their address alone, though
    /// the grant meant Tailcat (ov-485 N1).
    ///
    /// A reply that doesn't decode names nobody, so a fallback in it would go
    /// unnamed. It comes from our own core and should never happen; when it
    /// does, `undecodable` is told (the log, in the app) so it isn't silent
    /// (ov-530 R2-4).
    static func addressOnly(
        granted: [CeremonyRunner], reply: String,
        undecodable: (Error) -> Void = { log.error("the ceremony reply didn’t decode: \($0)") }
    ) -> [CeremonyRunner] {
        let manifest: CeremonyManifest
        do {
            manifest = try JSONDecoder().decode(CeremonyManifest.self, from: Data(reply.utf8))
        } catch {
            undecodable(error)
            return []
        }
        let ids = TunnelRoute.addressOnly(
            meant: granted.map { (id: $0.id, reach: $0.reach) },
            replied: manifest.runners.map { (id: $0.id, reach: $0.reach) },
            isTunnel: { if case .tailcat = $0 { return true } else { return false } })
        return granted.filter { ids.contains($0.id) }
    }

    /// Which of these rows the reply would grant at their address alone if
    /// every one of them answered with a token: what the confirm screen warns
    /// about before anything is written (ov-485 N1). Measured by the core, on a
    /// token of production size.
    static func predictedAddressOnly(
        offer: String, rows: [RunnerRow], budget: Int, grantedBy: String? = nil
    ) -> Set<String> {
        let token = String(repeating: "t", count: 184)
        let meant = rows.filter(\.granted).map { row -> CeremonyRunner in
            var runner = row.runner
            if case .direct = runner.reach {
                runner.alternative = runner.reach
                runner.reach = .tailcat(token: token)
            }
            return runner
        }
        guard let answer = CeremonyFFI.reply(offer: offer, runners: meant, budget: budget, grantedBy: grantedBy),
            Refusal.from(answer) == nil
        else { return [] }
        return Set(addressOnly(granted: meant, reply: answer).map(\.id))
    }

    /// The refusal the reply would give these runners as picked, or nil.
    static func refusalBeforeWriting(
        offer: String, runners: [CeremonyRunner], budget: Int, grantedBy: String? = nil
    ) -> Refusal? {
        guard let answer = CeremonyFFI.reply(offer: offer, runners: runners, budget: budget, grantedBy: grantedBy) else {
            return .unreadable
        }
        return Refusal.from(answer)
    }

    // MARK: - Both directions

    func finish(_ sentence: String) {
        phase = .done(sentence)
    }

    /// Back to the camera after a refusal.
    ///
    /// The refused scan is discarded rather than re-decoded. A code that was
    /// refused stays refused — the way forward is a fresh look at a fresh code,
    /// and re-submitting the same bytes with a newer `held_ms` would be an app
    /// arguing with the freshness rule.
    func scanAgain() {
        teamCheck?.cancel()
        teamCheck = nil
        scanNumber += 1
        transcript = nil
        note = nil
        tunnelNote = nil
        addressOnlyNote = nil
        phase = .scanning
    }

    func refuse(_ refusal: Refusal) {
        phase = .refused(refusal)
    }

    /// The fingerprint at the tap. A test answers it; the app asks Touch ID.
    var authenticate: (String) async -> Bool = { await CeremonyStore.authenticated(reason: $0) }

    /// Touch ID, or this Mac's password. Never nothing.
    ///
    /// `.deviceOwnerAuthentication` rather than `…WithBiometrics` so a Mac with
    /// no Touch ID falls back to the password instead of being unable to add a
    /// device at all. Any error — cancelled, failed, no passcode set — is a no.
    private static func authenticated(reason: String) async -> Bool {
        let context = LAContext()
        var problem: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
            return false
        }
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason))
            ?? false
    }
}
