import Foundation
import os

/// A phone's key slots: the iPhone Keychain, or the Android Keystore's
/// wrapped store. Only public halves cross into AgentKit; the app signs.
///
/// Three slots during a re-key (ov-569):
/// - `current` is the key the phone connects with and registers;
/// - `next` is the new key, made but not in use yet;
/// - `previous` is the old key, kept only for runners that haven't moved yet.
@MainActor
public protocol PhoneKeySlots: AnyObject {
    var currentPublicKey: String? { get }
    var nextPublicKey: String? { get }
    var previousPublicKey: String? { get }
    /// Make the new key and keep it in `next`. Nil when the store refused.
    func makeNext() -> String?
    /// `next` becomes `current`, and `current` becomes `previous`. False when
    /// there was no `next`, or the store refused, and then nothing moved.
    func promote() -> Bool
    /// Delete the old key for good.
    func forgetPrevious()
    /// The relay's challenge, signed by the `next` key for `account`.
    func proveNext(account: String, challenge: String) -> DeviceKeyProof?
    /// A certificate, signed by the `current` key for `account`, for `keyA`.
    func certifyByCurrent(account: String, keyA: String) -> String?
    /// The runners (by id) that still hold only the old key.
    var runnersOnPrevious: Set<String> { get set }
}

/// One of the runners saved on this phone.
public struct RekeyRunner: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

/// How moving one runner to the new key ended.
public enum RunnerMove: Equatable, Sendable {
    /// The runner now lets this phone in with the new key only.
    case moved
    /// The phone couldn't reach it. It moves the next time the phone does.
    case notReached
    /// The runner's Far Cooler is too old to move a key.
    case needsUpdate
    /// The runner refused: the phone's old key isn't on it any more, or it
    /// wouldn't take the new one.
    case refused
    /// Not tried yet.
    case waiting

    /// What the runner's row says. Plain words, no runner error text.
    public var detail: String {
        switch self {
        case .moved: return "Moved to the new key."
        case .notReached: return "Moves the next time this phone reaches it."
        case .needsUpdate: return "Needs a newer Far Cooler. Update it, then open it on this phone."
        case .refused:
            return "Didn’t take the new key. Add this phone to it again from a device that’s already on it."
        case .waiting: return "Waiting to move."
        }
    }
}

/// Why a re-key stopped.
public enum PhoneRekeyFailure: Equatable, Sendable {
    /// The key store wouldn't make or keep the new key.
    case keyStore
    /// The relay didn't take the new key.
    case register(AccountError?)
    /// The relay didn't take the old key's removal.
    case removeOld(AccountError?)

    public var message: String {
        switch self {
        case .keyStore:
            return "This phone couldn’t store a new key, so nothing changed. Try again."
        case .register(let why):
            return "Couldn’t add the new key to your account. " + (why?.message ?? "Try again.")
        case .removeOld(let why):
            return "Couldn’t remove the old key from your account. " + (why?.message ?? "Try again.")
        }
    }
}

/// Where a re-key is.
public enum PhoneRekeyStage: Equatable, Sendable {
    case notStarted
    case working
    /// Everything that could be done is done. Runners not reached yet move
    /// the next time the phone reaches them.
    case finished
    case failed(PhoneRekeyFailure)
}

/// "Make a New Key" on a phone (ov-569).
///
/// A phone's Key A is its SSH identity on every runner and its device at the
/// relay. Once a key is removed at the relay it can never be registered again,
/// so the phone can't be listed to teams. This replaces it:
///
/// 1. Make a new key, keeping the old one until the swap is done.
/// 2. Register it. It's pending unless R-75's reset trusts it.
/// 3. Confirm it: the old key certifies it when the old key is still
///    confirmed; otherwise the person scans this phone's code from a confirmed
///    device, which the new key's code now shows.
/// 4. Use it, and move each runner's line to it (`client.rekey`). A runner the
///    phone can't reach moves later, with the old key kept for it alone.
/// 5. Remove the old key at the relay, naming the new one as its successor
///    when the old key confirmed it, so the person's other devices stay
///    confirmed.
///
/// Each step can be resumed: the key slots say where it got to.
///
/// **What it never trusts.** The new key is the phone's own, never one the
/// relay names. Its rows in `me` are found by the keys this phone holds
/// (`DeviceKeyRow.isHeld`), not by label or id.
@MainActor
public final class PhoneRekey: ObservableObject {
    public static let shared = PhoneRekey()

    /// The phone's key slots. Nil on a build that has none (the Mac).
    public var keys: PhoneKeySlots?
    /// Who is signed in here, and the relay to ask. Nil when nobody is: the
    /// key still moves on runners, and registers at the next sign-in.
    public var signedIn: () -> (account: String, directory: DirectoryClient)? = {
        let account = Account.shared
        return account.isSignedIn ? (account.userId, account.directoryClient) : nil
    }
    /// What this device is called on another screen.
    public var label: () -> String = { "Phone" }
    /// The runners saved on this phone.
    public var runners: () -> [RekeyRunner] = { [] }
    /// Move one runner's line to the current key, with the previous one.
    public var move: (RekeyRunner) async -> RunnerMove = { _ in .notReached }

    @Published public private(set) var stage: PhoneRekeyStage = .notStarted
    /// Where the directory has the new key. Nil before it registered, or
    /// with nobody signed in.
    @Published public private(set) var registered: DeviceKeyState?
    /// Whether the new key still needs scanning from a confirmed device.
    @Published public private(set) var needsScan = false
    @Published public private(set) var moves: [String: RunnerMove] = [:]
    @Published public private(set) var oldRemoved = false

    private var running = false
    private static let log = Logger(subsystem: "com.farcooler.agentkit", category: "rekey")

    public init() {}

    /// Whether this build can make a new key at all.
    public var isAvailable: Bool { keys != nil }

    /// Whether a re-key was started and hasn't finished: a new key not in use
    /// yet, or an old one still kept for some runner.
    public var isUnderway: Bool {
        guard let keys else { return false }
        return keys.nextPublicKey != nil || keys.previousPublicKey != nil
    }

    /// The runners still on the old key, in the order the phone lists them.
    public var runnersLeft: [RekeyRunner] {
        guard let keys else { return [] }
        return runners().filter { keys.runnersOnPrevious.contains($0.id) }
    }

    /// Start, or carry on from wherever the slots say it got to.
    public func run() async {
        guard let keys, !running else { return }
        running = true
        defer { running = false }
        stage = .working
        if let failure = await steps(keys) {
            stage = .failed(failure)
            Self.log.notice("re-key stopped: \(String(describing: failure), privacy: .public)")
        } else {
            stage = .finished
        }
    }

    /// Move one runner, after the phone reached it with the old key. What each
    /// connection calls on connecting to a runner still on the old key.
    public func moveIfNeeded(runnerID: String) async {
        guard let keys, keys.runnersOnPrevious.contains(runnerID), keys.previousPublicKey != nil,
            let runner = runners().first(where: { $0.id == runnerID })
        else { return }
        await moveOne(runner, keys)
        forgetPreviousIfDone(keys)
    }

    /// Give up on the runners left on the old key, and delete it.
    public func forgetOldKey() {
        guard let keys else { return }
        keys.runnersOnPrevious = []
        keys.forgetPrevious()
    }

    private func steps(_ keys: PhoneKeySlots) async -> PhoneRekeyFailure? {
        // 1. A new key, unless one was made already, or already put to use.
        if keys.previousPublicKey == nil, keys.nextPublicKey == nil {
            guard keys.makeNext() != nil else { return .keyStore }
        }
        if keys.nextPublicKey != nil {
            // 2 and 3, before the new key is used, while the old one can
            // still vouch for it.
            if let failure = await registerAndConfirm(keys) { return failure }
            // 4. Use it. Every saved runner holds only the old key until it
            // moves, and is marked so BEFORE the new key is in use (fix round
            // 1, M2): stopping in between must never leave the old key in
            // `previous` with no runner marked, which would read as every
            // runner moved and delete it.
            keys.runnersOnPrevious.formUnion(runners().map(\.id))
            guard keys.promote() else { return .keyStore }
        }
        for runner in runners() where keys.runnersOnPrevious.contains(runner.id) {
            await moveOne(runner, keys)
        }
        // 5. Off the account.
        if let failure = await removeOld(keys) { return failure }
        forgetPreviousIfDone(keys)
        return nil
    }

    private func registerAndConfirm(_ keys: PhoneKeySlots) async -> PhoneRekeyFailure? {
        guard let here = signedIn() else { return nil }
        let outcome = await here.directory.registerDevice(label: label()) {
            keys.proveNext(account: here.account, challenge: $0)
        }
        switch outcome {
        case .failure(let why):
            if case .account(let account) = why { return .register(account) }
            return .register(nil)
        case .success(let state):
            registered = state
        }
        guard registered == .pending else { return nil }
        // The old key confirms the new one, if the directory still has the old
        // one confirmed. Its row is found by the key this phone holds.
        let rows = (try? await here.directory.deviceKeys().get()) ?? []
        let old = rows.first { $0.isHeld(by: keys.currentPublicKey) }
        if let old, old.isConfirmed, let next = keys.nextPublicKey,
            let certificate = keys.certifyByCurrent(account: here.account, keyA: next)
        {
            switch await here.directory.certifyDevice(keyA: next, certificate: certificate) {
            case .certified, .alreadyConfirmed:
                registered = .certified
                return nil
            default:
                break
            }
        }
        // Scanned from another confirmed device, as any new device is. The
        // phone's code shows the new key once it's in use.
        needsScan = true
        return nil
    }

    private func moveOne(_ runner: RekeyRunner, _ keys: PhoneKeySlots) async {
        moves[runner.id] = .waiting
        let outcome = await move(runner)
        moves[runner.id] = outcome
        if outcome == .moved { keys.runnersOnPrevious.remove(runner.id) }
    }

    private func removeOld(_ keys: PhoneKeySlots) async -> PhoneRekeyFailure? {
        guard let previous = keys.previousPublicKey, let here = signedIn() else { return nil }
        let rows: [DeviceKeyRow]
        switch await here.directory.deviceKeys(includingRemoved: true) {
        case .failure(let why):
            if case .account(let account) = why { return .removeOld(account) }
            return .removeOld(nil)
        case .success(let loaded):
            rows = loaded
        }
        let new = rows.first { $0.isHeld(by: keys.currentPublicKey) }
        if new?.isConfirmed == true { needsScan = false }
        guard let old = rows.first(where: { $0.isHeld(by: previous) }), !old.isRemoved else {
            oldRemoved = true
            return nil
        }
        // The new key takes the old one's place only if the old one confirmed it.
        let successor = new.flatMap { $0.certifiedBy == old.id && $0.state == "certified" ? $0.id : nil }
        switch await here.directory.removeDeviceKey(id: old.id, successor: successor) {
        case .success:
            oldRemoved = true
            return nil
        case .failure(let why):
            if case .account(let account) = why { return .removeOld(account) }
            return .removeOld(nil)
        }
    }

    private func forgetPreviousIfDone(_ keys: PhoneKeySlots) {
        if keys.runnersOnPrevious.isEmpty, keys.previousPublicKey != nil { keys.forgetPrevious() }
    }
}

/// Which of a phone's keys to connect to a runner with, during a re-key.
public enum PhoneKeyChoice: Equatable, Sendable {
    case current
    case previous

    /// The old key for a runner that hasn't moved yet, while the phone still
    /// has it; the current key otherwise.
    public static func forRunner(_ id: String, onPrevious: Set<String>, hasPrevious: Bool) -> PhoneKeyChoice {
        hasPrevious && onPrevious.contains(id) ? .previous : .current
    }

    /// The other key to try once, after a runner refused this one, or nil.
    /// A runner that moved when its answer was lost refuses the old key, and
    /// one that never moved refuses the new one.
    public func alternate(hasPrevious: Bool) -> PhoneKeyChoice? {
        guard hasPrevious else { return nil }
        return self == .current ? .previous : .current
    }
}
