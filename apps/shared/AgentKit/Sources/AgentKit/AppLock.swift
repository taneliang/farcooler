import Foundation

/// The app lock (ov-856, §3a of the design): Face ID when the app resumes
/// after five minutes away, before any terminal or admin screen.
///
/// The SSH key is a `ThisDeviceOnly` keychain item, so the person holding an
/// unlocked phone can use it only through the app. This is the control that
/// matters against a phone thief, and unlike step-up it covers the shell.
///
/// Pure state: the clock is passed in, so a test goes five minutes in no time.
public struct AppLock: Equatable, Sendable {
    /// How long the app may be away before it asks again.
    public static let idleLimit: TimeInterval = 5 * 60

    /// The screens that wait for the lock: anything that can type into a
    /// shell, and the screens that change who has access.
    public enum Screen: Equatable, Sendable {
        case terminal, admin, other
    }

    public private(set) var isLocked: Bool
    private var leftAt: Date?
    /// Off when the person has turned it off; a device with no passcode
    /// can't ask (the system has nothing to ask with).
    public var isOn: Bool

    public init(isOn: Bool = true, isLocked: Bool = false) {
        self.isOn = isOn
        self.isLocked = isLocked && isOn
    }

    /// A fresh launch: locked when it's on, because nothing has verified
    /// the person yet.
    public static func atLaunch(isOn: Bool = true) -> AppLock { AppLock(isOn: isOn, isLocked: true) }

    public mutating func didLeave(at now: Date) {
        leftAt = now
    }

    /// The app is back: locks when it was away long enough. A resume with
    /// no record of leaving (a crash restore) locks too.
    public mutating func didReturn(at now: Date) {
        guard isOn else { return }
        guard let leftAt else {
            isLocked = true
            return
        }
        if now.timeIntervalSince(leftAt) >= Self.idleLimit { isLocked = true }
        self.leftAt = nil
    }

    public mutating func unlock() {
        isLocked = false
    }

    /// Whether `screen` is covered right now.
    public func covers(_ screen: Screen) -> Bool {
        isOn && isLocked && screen != .other
    }

    public static let prompt = "Unlock Far Cooler to use your terminals and change who has access."
    public static let unlockButton = "Unlock"
}
