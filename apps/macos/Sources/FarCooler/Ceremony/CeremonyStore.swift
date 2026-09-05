import AgentKit
import CFarCoolerClient
import Foundation
import LocalAuthentication

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
/// `crates/client/src/ceremony.rs` byte for byte and a `CodingKeys` table is a
/// second place for a name to be wrong.
struct CeremonyRunner: Codable, Equatable, Identifiable {
    var id: String
    var label: String
    var alias: String
    var user: String
    var host_key: String
    var reach: CeremonyReach
    var pending: Bool
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
    /// The code decoded, and it belongs to another account. Decided here rather
    /// than in Rust because `farcooler_client_ceremony_scan` takes no account
    /// to compare against — see the note on `CeremonyStore.scan`.
    case otherAccount(String)
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
        case .otherAccount: return "This device uses a different account"
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
        case .otherAccount(let email):
            return "Sign in to \(email) on the new device, then show its code again."
        case .unreadable:
            return "Show a new code, then try again."
        }
    }

    /// A refusal the person can retry from, versus one where the way forward is
    /// somewhere else entirely.
    var retryable: Bool {
        switch self {
        case .version, .channel, .otherAccount: return false
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
        // A word this build does not have. It is still a refusal — reporting it
        // as a successful scan would be the one wrong answer.
        default: return .unreadable
        }
    }
}

// MARK: - The four entry points

/// The ceremony, as four calls into `crates/client/src/ceremony.rs`.
///
/// Every one of them takes the buffer contract `farcooler_client_generate_key`
/// has used since the first release: JSON into a buffer, returning the bytes
/// needed, writing nothing when the buffer is short. So `spill` calls twice at
/// most and never truncates — a half-written payload parses as nothing and
/// looks exactly like a corrupt scan.
enum CeremonyFFI {
    static func offer(name: String, account: String, keyA: String, keyB: String?) -> String? {
        name.withCString { name in
            account.withCString { account in
                keyA.withCString { keyA in
                    withOptionalCString(keyB) { keyB in
                        spill { farcooler_client_ceremony_offer(name, account, keyA, keyB, $0, $1) }
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

    static func reply(offer: String, runners: [CeremonyRunner], budget: Int) -> String? {
        guard let runnersJSON = try? JSONEncoder().encode(runners) else { return nil }
        return offer.withCString { offer in
            String(decoding: runnersJSON, as: UTF8.self).withCString { runners in
                spill { farcooler_client_ceremony_reply(offer, runners, budget, $0, $1) }
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
    enum Phase: Equatable {
        /// This Mac is the new device: here is its code.
        case showingOffer(String)
        /// This Mac is the trusted device: point the camera at the other one.
        case scanning
        /// A code was read and accepted. Pick runners and confirm.
        case confirming(Confirmation)
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

    private var alreadyTaken = false

    // MARK: This Mac as the new device

    /// Show this Mac's code.
    ///
    /// The string the FFI returns is BOTH what goes in the code and what this
    /// Mac keeps, to hand back as `expecting` when the reply arrives. Keeping
    /// one string rather than re-encoding the struct is what stops what is on
    /// screen and what is remembered from drifting apart.
    func present(name: String, account: String, keyA: String, keyB: String?) {
        guard let answer = CeremonyFFI.offer(name: name, account: account, keyA: keyA, keyB: keyB)
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
        return manifest
    }

    // MARK: This Mac as the trusted device

    /// Read a scanned offer and, if this build may act on it, open the
    /// confirmation.
    ///
    /// **This app makes no rule.** The account comparison used to be here,
    /// because `ceremony_scan` was never told which account was asking; it now
    /// takes one and answers `wrong_account`, so the only thing left is which
    /// screen a refusal leads to. The mismatch is still terminal and still
    /// ahead of the runner list — reaching that list with a mismatched code
    /// would put the fleet on screen with only a fingerprint between it and a
    /// stranger.
    func scan(_ scanned: String, at: Date, account: String, email: String, runners: [RunnerRow]) {
        guard
            let answer = CeremonyFFI.scan(
                scanned, expectingAccount: account, heldFor: Date().timeIntervalSince(at))
        else {
            return refuse(.unreadable)
        }
        if case .wrongAccount = Refusal.from(answer) { return refuse(.otherAccount(email)) }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        guard let data = answer.data(using: .utf8),
            let offer = try? JSONDecoder().decode(CeremonyOffer.self, from: data)
        else { return refuse(.unreadable) }

        phase = .confirming(
            Confirmation(
                offerJSON: answer, offer: offer, scannedAt: at, rows: runners,
                shell: offer.key_b == nil ? nil : ShellKeyChoice(deviceName: offer.name)))
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

        // A fingerprint, at the moment of the tap. This is what someone
        // standing at an unlocked laptop runs into, and it is the only thing
        // between them and an enrolled device — an account check does not help
        // there, because that laptop is signed in and they would be using its
        // session. Cancelled or failed enrolls nothing.
        guard await authenticated(reason: reason) else { return }

        // Re-scanned rather than re-used: the answer is the same offer, and the
        // point of the call is the freshness rule attached to it.
        guard
            let rechecked = CeremonyFFI.scan(
                confirmation.offerJSON,
                expectingAccount: confirmation.offer.account,
                heldFor: Date().timeIntervalSince(confirmation.scannedAt))
        else { return refuse(.unreadable) }
        if let refusal = Refusal.from(rechecked) { return refuse(refusal) }

        phase = .enrolling
        let wanted = confirmation.rows.filter(\.granted).map(\.runner)
        let outcome = await enroll(wanted)
        // The FILE's state, not this Mac's intention. Every record arrives here
        // pending — see `RunnerFacts` — and only a runner that answered loses
        // the flag.
        let granted = outcome.granting(wanted)
        transcript = outcome.transcript
        note = Enrollment.note(about: granted, outcome: outcome)

        guard
            let answer = CeremonyFFI.reply(
                offer: confirmation.offerJSON, runners: granted, budget: codeBudgetBytes)
        else { return refuse(.unreadable) }
        if let refusal = Refusal.from(answer) { return refuse(refusal) }
        phase = .showingManifest(answer)
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
        transcript = nil
        note = nil
        phase = .scanning
    }

    func refuse(_ refusal: Refusal) {
        phase = .refused(refusal)
    }

    /// Touch ID, or this Mac's password. Never nothing.
    ///
    /// `.deviceOwnerAuthentication` rather than `…WithBiometrics` so a Mac with
    /// no Touch ID falls back to the password instead of being unable to add a
    /// device at all. Any error — cancelled, failed, no passcode set — is a no.
    private func authenticated(reason: String) async -> Bool {
        let context = LAContext()
        var problem: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
            return false
        }
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason))
            ?? false
    }
}
