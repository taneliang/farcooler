import Foundation
import os

/// What this device's Key A signed for the directory: the two fields
/// `directory/devices/register` takes beside the label and the challenge.
///
/// Made by `farcooler_client_device_registration` in the app, which holds the
/// key; AgentKit never sees a private key. Neither field is secret, and
/// neither is logged.
public struct DeviceKeyProof: Equatable, Sendable {
    public let keyA: String
    public let proofA: String

    public init(keyA: String, proofA: String) {
        self.keyA = keyA
        self.proofA = proofA
    }

    /// The FFI's JSON answer, or nil for a refusal (`{"error": …}`) or
    /// anything else.
    public init?(answer: Data) {
        guard let object = try? JSONSerialization.jsonObject(with: answer) as? [String: Any],
            let keyA = object["keyA"] as? String, keyA.hasPrefix("ssh-ed25519 "),
            let proofA = object["proofA"] as? String, !proofA.isEmpty
        else { return nil }
        self.init(keyA: keyA, proofA: proofA)
    }
}

/// Where the directory has this device (ov-480 §3): the first device a person
/// registers is trusted; every later one waits for another of theirs to
/// certify it.
public enum DeviceKeyState: String, Equatable, Sendable {
    case trusted, pending, certified
}

extension DirectoryClient {
    /// Register this device's Key A (ov-549): ask the relay for a challenge,
    /// have `prove` sign it, and send the proof.
    ///
    /// `prove` gets only the relay's challenge. The account it signs for is
    /// the caller's own, captured in the closure, never anything the relay
    /// said. Nil from it (no key, or the core refused the challenge) sends
    /// nothing more.
    public func registerDevice(
        label: String, prove: (_ challenge: String) -> DeviceKeyProof?
    ) async -> Result<DeviceKeyState, DirectoryFailure> {
        let challenge: String
        switch await post("/v1/directory/devices/challenge", [:]) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            // A relay with no directory, or none of this route yet.
            if reply.status == 404 { return .failure(.account(.endpointMissing)) }
            guard reply.status == 200 else { return .failure(Self.failure(reply)) }
            guard let minted = reply.body["challenge"] as? String, !minted.isEmpty else {
                return .failure(.account(.malformedResponse))
            }
            challenge = minted
        }
        guard let proof = prove(challenge) else { return .failure(.proofRefused) }
        let body: [String: Any] = [
            "label": Self.limited(label), "challenge": challenge,
            "keyA": proof.keyA, "proofA": proof.proofA,
        ]
        switch await post("/v1/directory/devices/register", body) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            // A 403 here is the proof or the challenge, never a role.
            guard reply.status == 200 else {
                return .failure(reply.status == 403 ? .proofRefused : Self.failure(reply))
            }
            guard let raw = reply.body["state"] as? String, let state = DeviceKeyState(rawValue: raw) else {
                return .failure(.account(.malformedResponse))
            }
            return .success(state)
        }
    }
}

/// Registering this device's Key A with the directory, on sign-in and on each
/// launch while signed in (ov-549, ov-480 §6: devices signed in before this
/// shipped register on their next launch).
///
/// Each app supplies the label and the signer, since the key is the app's: a
/// file per account on a Mac, the Keychain on an iPhone, the Keystore-wrapped
/// store on Android. Once per account per launch: registering again is
/// harmless (the relay answers the same device) but costs two requests.
///
/// Nothing here reaches a screen. A device that couldn't register still
/// works exactly as before; what it misses is being listed to a team's
/// admins, which a teammate's scan then refuses, by name.
@MainActor
public final class DeviceKeyRegistration {
    public static let shared = DeviceKeyRegistration()

    /// This device's name, as the person will read it on another screen.
    public var label: () -> String = { "Device" }
    /// Sign the relay's challenge for `account` with this device's Key A.
    public var sign: (_ account: String, _ challenge: String) -> DeviceKeyProof? = { _, _ in nil }
    /// This device's Key A for `account`, public half, so Settings can mark
    /// which of the account's device keys is this one (ov-550). Nil when
    /// there is none yet.
    public var publicKey: (_ account: String) -> String? = { _ in nil }

    /// Where the directory has this device, after the last attempt this launch.
    public private(set) var state: DeviceKeyState?
    private var registered: String?
    private var running = false
    /// An account asked for while another registration was in flight: run
    /// once that one ends, never dropped (review L5). The latest ask wins.
    private var queued: (account: String, directory: DirectoryClient)?

    private static let log = Logger(subsystem: "com.farcooler.agentkit", category: "device-keys")

    public init() {}

    /// Register for the signed-in account, unless that already happened this
    /// launch. `account` is this device's own record of who is signed in.
    ///
    /// A call that arrives while another is in flight (a sign-in during the
    /// launch's registration) is queued and run when that one ends, and
    /// answers nil itself; the call already running answers the last outcome.
    @discardableResult
    public func registerIfNeeded(
        account: String, directory: DirectoryClient
    ) async -> Result<DeviceKeyState, DirectoryFailure>? {
        guard !account.isEmpty, registered != account else { return nil }
        if running {
            queued = (account, directory)
            return nil
        }
        running = true
        defer { running = false }
        var current = (account: account, directory: directory)
        while true {
            let outcome = await attempt(account: current.account, directory: current.directory)
            guard let next = queued else { return outcome }
            queued = nil
            if next.account == registered { return outcome }
            current = next
        }
    }

    private func attempt(
        account: String, directory: DirectoryClient
    ) async -> Result<DeviceKeyState, DirectoryFailure> {
        let sign = self.sign
        let outcome = await directory.registerDevice(label: label()) { sign(account, $0) }
        switch outcome {
        case .success(let state):
            self.state = state
            registered = account
        case .failure(let why):
            // By kind only: no body, no key, no challenge.
            Self.log.notice("device key not registered: \(String(describing: why), privacy: .public)")
        }
        return outcome
    }

    /// For `Account.shared`: the call each app makes after sign-in. Always
    /// registers, even if this launch already did: when none of the account's
    /// devices is confirmed, signing in is how a person makes this one their
    /// confirmed device (R-75), which is what Settings tells them to do.
    public func registerAfterSignIn() async {
        let account = Account.shared
        guard account.isSignedIn else { return }
        await registerAgain(account: account.userId, directory: account.directoryClient)
    }

    /// Register for `account` whether or not this launch already did.
    @discardableResult
    public func registerAgain(
        account: String, directory: DirectoryClient
    ) async -> Result<DeviceKeyState, DirectoryFailure>? {
        registered = nil
        return await registerIfNeeded(account: account, directory: directory)
    }

    /// For `Account.shared`: the call each app makes at launch.
    public func registerIfSignedIn() async {
        let account = Account.shared
        guard account.isSignedIn else { return }
        await registerIfNeeded(account: account.userId, directory: account.directoryClient)
    }
}
