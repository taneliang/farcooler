import FarCoolerClient
import Foundation
import LocalAuthentication
import UIKit

// The enrollment ceremony, as this app takes part in it.
//
// EVERY RULE THAT DECIDES WHETHER A SCAN IS ACCEPTABLE IS IN RUST, behind the
// four entry points below — version, channel, freshness, the three echoes, the
// byte budget. This file shows a code, points a camera at one, and turns the
// word that comes back into a sentence. If something here starts to look like
// validation, it belongs in `crates/client/src/ceremony.rs` instead.
//
// A refusal crosses as a stable word, never a sentence: `{"error":"stale"}`.
// The copy for each is `Refusal` below, and it is the app's, so no Rust error
// string can reach a screen.

// MARK: - What the codes carry

/// The first code: what a new device shows. Decoded only to draw the
/// confirmation — nothing is decided from these fields here.
struct CeremonyOffer: Decodable, Equatable {
    let v: Int
    let key_a: String  // swiftlint:disable:this identifier_name
    let key_b: String?  // swiftlint:disable:this identifier_name
    let name: String
    let account: String
    let channel: String
    let ceremony: String
}

/// One runner in a reply: everything a device needs to reach it, and nothing it
/// needs to trust it with. The field names are the wire's, because this is
/// serialized straight back into `farcooler_client_ceremony_reply`.
struct CeremonyRunner: Codable, Equatable, Identifiable {
    let id: String
    let label: String
    let alias: String
    let address: String
    let user: String
    let port: Int
    let host_key: String  // swiftlint:disable:this identifier_name
    let pending: Bool
}

/// The reply: the runners a trusted device granted, addressed to one ceremony.
struct CeremonyManifest: Decodable, Equatable {
    let v: Int
    let ceremony: String
    let account: String
    let channel: String
    let target: String
    let runners: [CeremonyRunner]
}

// MARK: - Refusals

/// Why a code was refused, and what a person is told about it.
///
/// The cases are the FFI's stable words. THE APP OWNS THE SENTENCE: the core
/// answers `malformed`, and what belongs on a screen is "That isn't a Far
/// Cooler code" — never a `serde_json` message, and never advice about an sshd
/// setting that was not the problem.
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
    /// The core answered nothing readable at all. Not one of its words: this is
    /// the app failing, and it says so without saying how.
    case unknown

    init(code: String, version: Int?, channel: String?) {
        switch code {
        case "version": self = .version(version ?? 0)
        case "channel": self = .channel(channel ?? "")
        case "malformed": self = .malformed
        case "wrong_ceremony": self = .wrongCeremony
        case "wrong_account": self = .wrongAccount
        case "wrong_target": self = .wrongTarget
        case "stale": self = .stale
        case "already_taken": self = .alreadyTaken
        case "too_large": self = .tooLarge
        default: self = .unknown
        }
    }

    var title: String {
        switch self {
        case .version(let made):
            return made > 1
                ? "This code requires a newer version of Far Cooler"
                : "This code is from an older version of Far Cooler"
        case .channel: return "This code is from a different version of Far Cooler"
        case .malformed: return "This isn’t a Far Cooler code"
        case .wrongCeremony: return "This code is for a different request"
        case .wrongAccount: return "This code is for a different account"
        case .wrongTarget: return "This code is for another device"
        case .stale: return "This code expired"
        case .alreadyTaken: return "This code was already used"
        case .tooLarge: return "Too many runners selected"
        case .unknown: return "Couldn’t add this device"
        }
    }

    var message: String {
        switch self {
        case .version(let made):
            return made > 1
                ? "Update Far Cooler on this device, then try again."
                : "Update Far Cooler on the other device, then try again."
        case .channel(let channel):
            let named = channel.isEmpty ? "a different version" : "Far Cooler \(channel.capitalized)"
            return "The other device is using \(named). Both devices must use the same version."
        case .malformed:
            return "Scan the code shown in Far Cooler on the other device."
        case .wrongCeremony:
            return "Show a new code on this device, then scan the code on the other device."
        case .wrongAccount:
            return "Both devices must be signed in to the same account."
        case .wrongTarget:
            return "Show a new code on this device, then scan the code on the other device."
        case .stale:
            return "Show a new code, then scan it within two minutes."
        case .alreadyTaken:
            return "Show a new code, then try again."
        case .tooLarge:
            return "Select fewer runners. You can add the others later."
        case .unknown:
            return "Try again."
        }
    }
}

// MARK: - The four entry points

/// Swift's view of the ceremony core: four calls, and the buffer contract they
/// all share.
///
/// Every one of them answers either the payload it was asked for or
/// `{"error":"…"}`, so every one of them comes back as `Answer`.
enum CeremonyCore {
    enum Answer {
        case payload(Data)
        case refused(Refusal)
    }

    /// Leg one, the displaying side. `keyB` is nil on a phone: there is no Zed
    /// on a phone, so there is no second key.
    static func offer(name: String, account: String, keyA: String, keyB: String? = nil) -> Answer {
        name.withCString { name in
            account.withCString { account in
                keyA.withCString { keyA in
                    withOptionalCString(keyB) { keyB in
                        answer {
                            farcooler_client_ceremony_offer(name, account, keyA, keyB, $0, $1)
                        }
                    }
                }
            }
        }
    }

    /// Leg one, the scanning side. `heldFor` is this device's own elapsed time,
    /// which is the only clock that counts — a code carries no timestamp
    /// because the device showing it would control that number.
    ///
    /// `expectingAccount` is which account is asking, and the core answers
    /// `wrong_account` itself. That comparison used to live here, which meant
    /// three apps each held a copy of a security rule — so it moved into
    /// `ceremony.rs` beside the same check on the reply leg.
    static func scan(_ encoded: String, expectingAccount: String, heldFor: TimeInterval) -> Answer {
        encoded.withCString { encoded in
            expectingAccount.withCString { account in
                answer {
                    farcooler_client_ceremony_scan(
                        encoded, account, milliseconds(heldFor), $0, $1
                    )
                }
            }
        }
    }

    /// Leg two, the trusted device's side.
    ///
    /// `budgetBytes` of 0 takes the core's conservative default. This app does
    /// not compute a budget of its own: CoreImage will encode whatever fits a
    /// version-40 code, which is more than the default, so the smaller number
    /// is the safe one and the one that keeps three platforms agreeing about
    /// which manifests are too big.
    static func reply(offer: Data, runners: [CeremonyRunner], budgetBytes: Int = 0) -> Answer {
        guard
            let runnersJSON = try? JSONEncoder().encode(runners),
            let runnersText = String(data: runnersJSON, encoding: .utf8),
            let offerText = String(data: offer, encoding: .utf8)
        else { return .refused(.unknown) }

        return offerText.withCString { offer in
            runnersText.withCString { runners in
                answer { farcooler_client_ceremony_reply(offer, runners, budgetBytes, $0, $1) }
            }
        }
    }

    /// Leg two, the new device's side. `alreadyTaken` is this device's own
    /// record that the ceremony has answered once — one reply per ceremony, so
    /// a forged one cannot follow a real one.
    static func accept(
        _ encoded: String, expecting: String, alreadyTaken: Bool, heldFor: TimeInterval
    ) -> Answer {
        encoded.withCString { encoded in
            expecting.withCString { expecting in
                answer {
                    farcooler_client_ceremony_accept(
                        encoded, expecting, alreadyTaken, milliseconds(heldFor), $0, $1)
                }
            }
        }
    }

    /// The same buffer contract, for the two entry points that answer raw text
    /// rather than JSON: `farcooler_client_fingerprint` and
    /// `farcooler_client_client_id`. Both return 0 when the input is not a
    /// public key, which is `nil` here rather than a guess a person would
    /// compare against a screen.
    static func text(
        of publicKey: String,
        _ call: (UnsafePointer<CChar>?, UnsafeMutablePointer<UInt8>?, Int) -> Int
    ) -> String? {
        publicKey.withCString { key in
            var buffer = [UInt8](repeating: 0, count: 128)
            var written = buffer.withUnsafeMutableBufferPointer {
                call(key, $0.baseAddress, $0.count)
            }
            if written > buffer.count {
                buffer = [UInt8](repeating: 0, count: written)
                written = buffer.withUnsafeMutableBufferPointer {
                    call(key, $0.baseAddress, $0.count)
                }
            }
            guard written > 0, written <= buffer.count else { return nil }
            return String(bytes: buffer[0..<written], encoding: .utf8)
        }
    }

    /// The buffer contract, once: JSON into a buffer, the call reporting the
    /// bytes it needed and writing nothing when that is more than it was given.
    private static func answer(_ call: (UnsafeMutablePointer<UInt8>?, Int) -> Int) -> Answer {
        var buffer = [UInt8](repeating: 0, count: 4096)
        var written = buffer.withUnsafeMutableBufferPointer { call($0.baseAddress, $0.count) }
        if written > buffer.count {
            buffer = [UInt8](repeating: 0, count: written)
            written = buffer.withUnsafeMutableBufferPointer { call($0.baseAddress, $0.count) }
        }
        guard written > 0, written <= buffer.count else { return .refused(.unknown) }

        let data = Data(buffer[0..<written])
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return .refused(.unknown)
        }
        // An `error` key is the only thing that distinguishes a refusal from a
        // payload, and it is the core's own word — never a sentence.
        if let code = object["error"] as? String {
            return .refused(
                Refusal(
                    code: code,
                    version: object["version"] as? Int,
                    channel: object["channel"] as? String))
        }
        return .payload(data)
    }

    /// A duration as the core wants it, and every awkward value it can be.
    ///
    /// A negative interval — a phone whose clock moved backwards mid-ceremony —
    /// counts as no time at all rather than wrapping into an enormous unsigned
    /// number. An infinite one is a caller saying it does not know when the
    /// code was read, and the honest answer to that is "older than any window",
    /// not a trap: `UInt64(Double.infinity)` crashes.
    private static func milliseconds(_ interval: TimeInterval) -> UInt64 {
        let ms = interval * 1000
        guard ms.isFinite else { return .max }
        guard ms > 0 else { return 0 }
        return UInt64(min(ms, 1e15))
    }

    private static func withOptionalCString<T>(
        _ text: String?, _ body: (UnsafePointer<CChar>?) -> T
    ) -> T {
        guard let text else { return body(nil) }
        return text.withCString { body($0) }
    }
}

// MARK: - The gate

/// A fingerprint, at the moment of the tap.
///
/// This is what someone standing at an unlocked laptop runs into, and it is the
/// only thing between them and an enrolled device. An account check does not
/// help there: that laptop is signed in, and they would be using its session.
///
/// Falls back to the device passcode, never to nothing — `deviceOwnerAuthentication`
/// is the policy that means "Face ID, Touch ID, or the passcode", so a phone
/// with no biometrics still asks for something. A cancelled or failed
/// evaluation enrolls nothing.
enum ConfirmingTap {
    enum Outcome {
        case confirmed
        /// Someone changed their mind. Not a failure, and not worth a screen.
        case cancelled
        /// It asked and did not get an answer it accepted, or it could not ask.
        case refused
    }

    static func ask(reason: String) async -> Outcome {
        let context = LAContext()
        var problem: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
            // No passcode set at all. Nothing to fall back to, so nothing is
            // enrolled — the alternative would be an unguarded tap, which is
            // the whole thing this gate exists to prevent.
            return .refused
        }
        do {
            let ok = try await context.evaluatePolicy(
                .deviceOwnerAuthentication, localizedReason: reason)
            return ok ? .confirmed : .refused
        } catch let error as LAError where error.code == .userCancel || error.code == .appCancel
            || error.code == .systemCancel
        {
            return .cancelled
        } catch {
            return .refused
        }
    }
}

// MARK: - Enrolling

/// Putting a device's key into `~/.ssh/authorized_keys` on the runners a
/// ceremony grants.
///
/// **The daemon owns the write.** Not a shell command appending a line, and
/// emphatically not anything in Swift: that file is the one whose corruption
/// costs somebody SSH access to their own machine, so the write is
/// descriptor-anchored, `O_NOFOLLOW`, locked, atomic and `fsync`ed twice in
/// `crates/daemon/src/enrollment.rs`. The app's part is to ask.
///
/// A closure rather than a protocol because there is one implementation and one
/// call site, and what a test needs to replace is a behavior, not an object.
/// It answers with what became of each runner, keyed by `CeremonyRunner.id`; a
/// runner with NO entry was never asked, which on a phone is most of them —
/// see `CeremonyStore.throughTheLiveConnection`. Both of those become `pending`
/// in the reply, because both mean the same thing about the file.
typealias Enroller = @MainActor (
    _ publicKey: String, _ label: String, _ clientId: String, _ runners: [CeremonyRunner]
) async -> [String: Connection.Enrollment]

// MARK: - The state machine

/// One runner, as a row someone can tick.
struct RunnerRow: Identifiable, Equatable {
    let runner: Runner
    var picked: Bool

    var id: UUID { runner.id }
    var label: String { runner.label }
    /// The second line. Which account a runner is reached through is not
    /// something this device records, so what is shown is what it does know:
    /// where the runner is and who it logs in as.
    var detail: String { "\(runner.user)@\(runner.address)" }
}

/// The ceremony, from this app's side of it — either side.
///
/// One store for both because they are one exchange: the device being added
/// shows a code and takes a reply; the device already trusted reads a code and
/// gives one back. Which methods a screen calls is what makes it one or the
/// other.
@MainActor
final class CeremonyStore: ObservableObject {
    enum Phase: Equatable {
        case idle
        /// The new device's own code is on screen.
        case showingOffer
        case scanning
        /// The scanned code belongs to another account. It appears BEFORE the
        /// runner list, so the runners are never on screen with only a
        /// fingerprint between a stranger and them.
        case mismatch
        case confirming
        /// The tap has been confirmed and the reply is being built.
        case enrolling
        /// The reply is on screen for the new device to scan.
        case showingManifest
        case done
        case refused(Refusal)
    }

    @Published private(set) var phase: Phase = .idle
    /// The runners on offer, and which are ticked. Published beside `phase`
    /// rather than carried inside `.confirming` so a tick is one row changing
    /// rather than the whole screen's state being replaced.
    @Published var rows: [RunnerRow] = []
    /// What to draw as a code: this device's offer, or the reply it built.
    @Published private(set) var code: String = ""
    /// The device asking to be added, once its code has been read.
    @Published private(set) var offer: CeremonyOffer?
    /// The fingerprint of the key that device is offering, as Rust computed it.
    @Published private(set) var fingerprint: String?
    /// The runners a reply granted, for the device that took it.
    @Published private(set) var granted: [CeremonyRunner] = []
    /// The gate said no. Not a refusal — nothing was scanned wrong and no rule
    /// was broken — so it is an alert over the confirmation rather than a
    /// screen that replaces it.
    @Published var declined = false
    /// What to say about the runners whose `authorized_keys` this device did
    /// not write to, or nil when every granted runner took the key.
    ///
    /// A sentence rather than a flag because this file is where the ceremony's
    /// copy lives — `Refusal` above owns every other sentence in the flow for
    /// the same reason — and because the two cases have different answers for a
    /// person. Never a cause where the cause is not knowable: a runner asleep,
    /// a daemon not installed and a fence that could not be rewritten look
    /// identical from here, and a screen that guesses is how an app ends up
    /// telling somebody to loosen an sshd setting that was never the problem.
    @Published private(set) var pendingNote: String?

    /// Where a grant actually lands.
    ///
    /// Injected so `confirm()` can be driven with no SSH session anywhere near
    /// it, and defaulted so the two screens that build this store keep working
    /// without having to know that the ceremony writes keys at all — neither of
    /// them is handed a `Connection` to pass on.
    var enroller: Enroller = CeremonyStore.throughTheLiveConnection

    /// Who this device is, so it can build an offer and recognize one.
    let account: String
    let accountEmail: String
    let deviceName: String

    /// The offer this device is showing, kept exactly as the core returned it:
    /// what goes in the code and what is passed back as `expecting` are the
    /// same string, so what it shows and what it remembers cannot drift apart.
    private var showing: String = ""
    private var scannedCode: String = ""
    /// When the scanned code was read, and when this device's own code went up.
    /// Both are this device's clock, which is the only one that counts.
    private var scannedAt: Date?
    private var showingSince: Date?
    /// One reply per ceremony, recorded here because "have I already taken
    /// one" is state on the device rather than anything a code can say.
    private var alreadyTaken = false

    init(account: String, accountEmail: String, deviceName: String) {
        self.account = account
        self.accountEmail = accountEmail
        self.deviceName = deviceName
    }

    // MARK: The device being added

    /// Build and show this device's code.
    func showOffer(publicKey: String?) {
        guard let publicKey, !publicKey.isEmpty else {
            phase = .refused(.unknown)
            return
        }
        alreadyTaken = false
        switch CeremonyCore.offer(
            name: deviceName, account: account, keyA: publicKey, keyB: nil)
        {
        case .refused(let refusal):
            phase = .refused(refusal)
        case .payload(let data):
            showing = String(decoding: data, as: UTF8.self)
            code = showing
            showingSince = Date()
            phase = .showingOffer
        }
    }

    /// Take the reply this device scanned, or refuse it.
    ///
    /// `heldFor` is measured from the moment this device's own code went up
    /// rather than from the instant of the scan. The two readings differ by
    /// whatever the exchange took, and this is the one that does any work: a
    /// code left on a screen for an hour is exactly the case the freshness
    /// window is about, and a reply arriving then is refused.
    func takeReply(_ encoded: String) {
        let heldFor = showingSince.map { Date().timeIntervalSince($0) } ?? .infinity
        switch CeremonyCore.accept(
            encoded, expecting: showing, alreadyTaken: alreadyTaken, heldFor: heldFor)
        {
        case .refused(let refusal):
            // Refused without being consumed — the core takes nothing it did
            // not accept — so showing a fresh code and trying again is the
            // whole recovery.
            phase = .refused(refusal)
        case .payload(let data):
            guard let manifest = try? JSONDecoder().decode(CeremonyManifest.self, from: data) else {
                phase = .refused(.unknown)
                return
            }
            alreadyTaken = true
            granted = manifest.runners
            phase = .done
        }
    }

    /// Back to the code already on screen, without minting another.
    ///
    /// Cancelling a scan must not change the code: the other device may have
    /// read it a moment ago and be picking runners for exactly that ceremony,
    /// and a fresh id here would turn their reply into a refusal.
    func showCodeAgain() {
        guard !showing.isEmpty else { return }
        code = showing
        phase = .showingOffer
    }

    // MARK: The device already trusted

    func beginScanning() {
        offer = nil
        fingerprint = nil
        phase = .scanning
    }

    /// Read a scanned code, and decide which screen it leads to.
    ///
    /// There is no judgement in this file. The account comparison used to be
    /// here, because `ceremony_scan` was never told which account was asking —
    /// which meant three apps each held their own copy of a security rule. It
    /// now takes `expectingAccount` and answers `wrong_account` itself, and the
    /// only thing left here is which screen a refusal leads to.
    func read(_ encoded: String, runners: [Runner], grantingFrom: Runner?) {
        let now = Date()
        switch CeremonyCore.scan(encoded, expectingAccount: account, heldFor: 0) {
        case .refused(.wrongAccount):
            phase = .mismatch
        case .refused(let refusal):
            phase = .refused(refusal)
        case .payload(let data):
            guard let decoded = try? JSONDecoder().decode(CeremonyOffer.self, from: data) else {
                phase = .refused(.unknown)
                return
            }
            scannedCode = encoded
            scannedAt = now
            offer = decoded

            // Only the runner being granted from is ticked. Everything else is
            // listed and unticked: granting more than was asked for is not
            // something a default gets to do.
            rows = runners.map { RunnerRow(runner: $0, picked: $0.id == grantingFrom?.id) }
            fingerprint = Self.fingerprint(of: data)
            phase = .confirming
        }
    }

    /// The fingerprint of the key on offer, computed in Rust.
    ///
    /// Asked for directly now. This used to build an empty reply and read its
    /// `target`, which worked and was a hack; `farcooler_client_fingerprint` is
    /// the same computation with a name. This app implements no cryptography.
    ///
    /// Computed once, when the code is read, rather than every time a screen
    /// redraws.
    private static func fingerprint(of offer: Data) -> String? {
        guard let decoded = try? JSONDecoder().decode(CeremonyOffer.self, from: offer) else {
            return nil
        }
        return CeremonyCore.text(of: decoded.key_a, farcooler_client_fingerprint)
    }

    /// The client id this device will be enrolled under.
    ///
    /// Derived in Rust from the key itself, not invented here. The daemon's
    /// "already enrolled" check compares client ids, so an id invented per
    /// platform means one device enrolls twice under two names and the daemon
    /// can no longer say which session arrived on which key.
    private static func clientId(of offer: Data) -> String? {
        guard let decoded = try? JSONDecoder().decode(CeremonyOffer.self, from: offer) else {
            return nil
        }
        return CeremonyCore.text(of: decoded.key_a, farcooler_client_client_id)
    }

    /// Confirm the grant: the gate, then the freshness check, then the reply.
    func confirm() async {
        guard let offer, !scannedCode.isEmpty else {
            phase = .refused(.unknown)
            return
        }

        switch await ConfirmingTap.ask(reason: "Confirm adding \(offer.name) to your runners") {
        case .cancelled:
            // Changing your mind is not a failure and gets no screen.
            return
        case .refused:
            declined = true
            return
        case .confirmed:
            break
        }

        phase = .enrolling

        // The freshness check happens HERE, at the confirmation, not only at
        // the scan: a sheet left open past the window has to refuse rather than
        // enroll, and this call is what makes it.
        let heldFor = scannedAt.map { Date().timeIntervalSince($0) } ?? .infinity
        let rescanned: Data
        switch CeremonyCore.scan(scannedCode, expectingAccount: account, heldFor: heldFor) {
        case .refused(let refusal):
            phase = .refused(refusal)
            return
        case .payload(let data):
            rescanned = data
        }

        // The keys go in BEFORE the reply is built, which is what makes
        // `pending` a fact rather than a hope: a runner named in that code is
        // claimed as granted, and a claim no line was written for is the one
        // lie in this flow that a person could not detect. It is also what
        // makes `.enrolling` — already on screen — name what is happening,
        // rather than a step this app skipped and said nothing about.
        let wanted = picked()
        // The id this device will be enrolled under, derived in Rust from the
        // key in the code rather than invented here. See `clientId(of:)`: the
        // daemon's "already enrolled" arm compares client ids, so an id
        // invented per platform means one device enrolls twice under two names.
        // No id means nothing to enroll under, so nothing is enrolled and every
        // runner travels pending — which is a true statement about the file.
        let outcomes: [String: Connection.Enrollment]
        if let clientId = Self.clientId(of: rescanned) {
            outcomes = await enroller(offer.key_a, offer.name, clientId, wanted)
        } else {
            outcomes = [:]
        }

        let granting = wanted.map { runner in
            CeremonyRunner(
                id: runner.id,
                label: runner.label,
                alias: runner.alias,
                address: runner.address,
                user: runner.user,
                port: runner.port,
                host_key: runner.host_key,
                // The FILE's state, not this app's intention: pending unless
                // that runner answered that the line is there.
                pending: outcomes[runner.id] != .written)
        }
        pendingNote = Self.note(about: granting, outcomes: outcomes)

        switch CeremonyCore.reply(offer: rescanned, runners: granting) {
        case .refused(let refusal):
            phase = .refused(refusal)
        case .payload(let data):
            code = String(decoding: data, as: UTF8.self)
            phase = .showingManifest
        }
    }

    /// The runners this device can actually write to: at most one of them.
    ///
    /// A Mac reaches every granted runner, because its `Enrollment` shells out
    /// to `farcooler --runner <target> client enroll` and inherits the agent,
    /// the passphrase prompt, `ProxyJump` and everything else ssh already
    /// knows. **A phone has no ssh at all.** What it has is one `Connection` —
    /// the session `FleetView` is running, to the runner whose fleet is on
    /// screen — so for every other granted runner the honest answer is no entry
    /// at all, and `confirm()` marks those pending. That is not a shortfall
    /// being hidden: pending is exactly "the trusted device has not yet written
    /// this key into that runner's `authorized_keys`", and it is what the new
    /// device's screen reads.
    ///
    /// Matched by id, and by the ceremony's spelling of one: these records came
    /// from this device's own runner list a moment ago in `picked()`, so the id
    /// in the manifest IS the id of the connection that reaches it.
    @MainActor
    private static func throughTheLiveConnection(
        publicKey: String, label: String, clientId: String, runners: [CeremonyRunner]
    ) async -> [String: Connection.Enrollment] {
        guard let connection = Connection.current,
            let reachable = connection.hostId?.uuidString,
            runners.contains(where: { $0.id == reachable })
        else { return [:] }
        return [
            reachable: await connection.enroll(
                publicKey: publicKey, label: label, clientId: clientId)
        ]
    }

    /// What to say about the runners that did not take the key, or nil when
    /// they all did.
    ///
    /// The too-old case is named because it is the one cause this device
    /// actually knows — the capability list arrives in the handshake — and
    /// because it is the one with a different answer: updating Far Cooler over
    /// there, rather than trying again from somewhere else. Everything else
    /// gets the sentence that promises nothing. It deliberately does NOT say
    /// the key will be written later: nothing in this app retries an
    /// enrollment, and a sentence saying otherwise would leave somebody waiting
    /// for a write that is never attempted.
    private static func note(
        about granting: [CeremonyRunner], outcomes: [String: Connection.Enrollment]
    ) -> String? {
        guard granting.contains(where: \.pending) else { return nil }
        if let old = granting.first(where: { outcomes[$0.id] == .tooOld }) {
            return "Far Cooler on \(old.label) is too old to add devices. "
                + "Update it there, then add this device again."
        }
        return "Some runners don’t have this device’s key yet. You can add it later "
            + "from a device that can reach them."
    }

    /// The ticked runners, as the reply's records.
    ///
    /// `pending` is true for every one of them here, and that is not a
    /// placeholder: it is what "the trusted device has not yet written this key
    /// into that runner's `~/.ssh/authorized_keys`" means, and at this point
    /// nothing has. `confirm()` corrects each one against what that runner
    /// answered, and only a runner that answered loses the flag.
    ///
    /// It used to be left true for every runner unconditionally, because
    /// `client.enroll` was unrouted in the client core — the daemon served it
    /// and `crates/client`'s dispatch had no arm — so the confirmation screen's
    /// promise that Far Cooler adds this device's key was false and no key was
    /// ever written. `ffi.rs` routes all three of `client.list`,
    /// `client.enroll` and `client.revoke` now, with
    /// `crates/client/tests/every_method_is_routed.rs` as the guard against a
    /// repeat.
    private func picked() -> [CeremonyRunner] {
        rows.filter(\.picked).map { row in
            CeremonyRunner(
                id: row.runner.id.uuidString,
                label: row.runner.label,
                // The `~/.ssh/config` alias, which the Mac writes. Derived from
                // the label here; the Rust writer owns collisions and suffixes.
                alias: alias(for: row.runner.label),
                address: row.runner.address,
                user: row.runner.user,
                port: row.runner.port,
                // The host key this device pinned, or nothing when it never
                // has. Empty travels honestly: the new device then meets the
                // ordinary first-contact screen and a person looks at a
                // fingerprint, rather than being handed a pin nobody verified.
                host_key: row.runner.fingerprint == "accept-any"
                    ? "" : (row.runner.fingerprint ?? ""),
                // Corrected in `confirm()`, once the enrollment has answered.
                pending: true)
        }
    }

    private func alias(for label: String) -> String {
        let slug = label.lowercased().map { character -> Character in
            character.isLetter || character.isNumber || character == "-" || character == "."
                ? character : "-"
        }
        let joined = String(slug).trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return joined.isEmpty ? "runner" : joined
    }

    func finish() {
        phase = .done
    }

    func reset() {
        phase = .idle
        offer = nil
        fingerprint = nil
        scannedCode = ""
        code = ""
        rows = []
        granted = []
        declined = false
        pendingNote = nil
    }
}

extension CeremonyRunner {
    /// This runner as something the app can connect to.
    ///
    /// A host key that came across empty stays nil, which is what makes the
    /// first connection report the fingerprint instead of trusting it.
    var asRunner: Runner {
        Runner(
            id: UUID(uuidString: id) ?? UUID(),
            label: label,
            address: address,
            port: port,
            user: user,
            fingerprint: host_key.isEmpty ? nil : host_key)
    }
}
