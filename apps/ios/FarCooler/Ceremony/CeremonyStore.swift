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
                ? "That code is from a newer Far Cooler"
                : "That code is from an older Far Cooler"
        case .channel: return "That code is from a different Far Cooler"
        case .malformed: return "That isn’t a Far Cooler code"
        case .wrongCeremony: return "That code answers a different device"
        case .wrongAccount: return "That code is for a different account"
        case .wrongTarget: return "That code is meant for another device"
        case .stale: return "That code expired"
        case .alreadyTaken: return "That code has already been used"
        case .tooLarge: return "Too many runners for one code"
        case .unknown: return "Something went wrong"
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
            return "That device is running \(named). Both devices have to be running the same one."
        case .malformed:
            return "Point the camera at the code the other device is showing."
        case .wrongCeremony:
            return "Show this device’s code again, then scan the reply it gets."
        case .wrongAccount:
            return "Both devices have to be signed into the same account."
        case .wrongTarget:
            return "Show this device’s code again, then scan the reply it gets."
        case .stale:
            return "A code is good for two minutes. Show a new one and scan it again."
        case .alreadyTaken:
            return "Show a new code to add this device again."
        case .tooLarge:
            return "Pick fewer runners. You can grant the rest by adding this device again."
        case .unknown:
            return "Far Cooler couldn’t finish adding this device. Try again."
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

        switch CeremonyCore.reply(offer: rescanned, runners: picked()) {
        case .refused(let refusal):
            phase = .refused(refusal)
        case .payload(let data):
            code = String(decoding: data, as: UTF8.self)
            phase = .showingManifest
        }
    }

    /// The ticked runners, as the reply's records.
    ///
    /// `pending` is true for every one of them, and that is not a placeholder:
    /// it is what "the trusted device has not yet written this key into that
    /// runner's `~/.ssh/authorized_keys`" means, and nothing has, because
    /// `client.enroll` is not reachable from the client core yet — the daemon
    /// serves it, `crates/client`'s dispatch does not route it. A runner
    /// claimed as granted when no line was written would be the one lie in this
    /// flow that a person could not detect.
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
