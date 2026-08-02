import Foundation

/// Where APNs delivers this device's address, and where it goes next.
///
/// Shared because the two apps had this twice, near-verbatim: the same
/// token-hexing, the same hold-until-signed-in field, the same `sendIfPossible`,
/// the same failure comment. Only the device label differed, so that is the one
/// thing left to the caller.
///
/// The local `Notifier` in each app covers the case where the phone is awake and
/// the app is running. This covers the case the product exists for: the app is
/// not running, the device is asleep, and an agent on a machine three time zones
/// away is stuck. Nothing local can know that — only the daemon can, and the
/// only route to a sleeping device is APNs.
@MainActor
public final class PushRegistration: ObservableObject {
    public static let shared = PushRegistration()

    /// Whether the relay has this device's address.
    ///
    /// Published rather than swallowed, because the failure is the product's
    /// central promise going quietly missing — the device is never filed, the
    /// daemon's pushes reach nobody, and the settings screen goes on saying
    /// "Notifications can reach this device."
    @Published public private(set) var registered = false
    @Published public private(set) var lastError: String?

    /// How this device names itself. Set once at launch by each app, because
    /// `UIDevice.current.name` and `Host.current().localizedName` are the only
    /// part of this that is platform-specific.
    public var label: () -> String = { "Device" }

    /// The APNs platform for this build. `apns` on Apple; the Android client
    /// will set `fcm`.
    public var platform = "apns"

    /// The last token APNs gave us, held until there is an account to file it
    /// under. Registration and sign-in finish in either order, and whichever is
    /// second completes the pair.
    private var token: String?

    private init() {}

    public func received(_ deviceToken: Data) {
        token = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { await sendIfPossible() }
    }

    /// Called after signing in, for the case where the token arrived first.
    public func sendIfPossible() async {
        guard let token, Account.shared.isSignedIn else { return }
        let ok = await Account.shared.registerDevice(
            pushToken: token, platform: platform, label: label())
        registered = ok
        lastError = ok ? nil : "Could not tell the relay how to reach this device."
    }

    /// Not surfaced as an error by the caller: registration fails on the
    /// simulator and in any build without a push entitlement, neither of which
    /// is worth telling someone about, and local notifications keep working.
    public func unavailable(_ error: Error) {
        print("remote notifications unavailable: \(error.localizedDescription)")
    }
}
