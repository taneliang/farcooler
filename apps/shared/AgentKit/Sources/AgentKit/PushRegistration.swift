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
/// not running, the device is asleep, and an agent on a runner three time zones
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

    /// Whether this device wants to hear that a turn ended — the "finishes or
    /// fails" toggle, read at registration time.
    ///
    /// A closure and not a value, and set once at launch by each app the way
    /// `label` is, because the two apps store the preference under different
    /// keys and neither of them is AgentKit's to know. Defaults to true: an
    /// app that never sets it registers as wanting notifications, which is what
    /// every install has always had.
    public var notifyOnDone: () -> Bool = { true }

    /// ActivityKit's push-to-start token, when the app has one.
    ///
    /// Filed with the device rather than on a route of its own because it is
    /// another address for the same phone, and because the two arrive in either
    /// order — re-registering when it lands is one line here instead of a second
    /// piece of hold-until-signed-in bookkeeping.
    ///
    /// Nil on the Mac, permanently. There are no Live Activities there and the
    /// relay's COALESCE is what keeps that from erasing a phone's token.
    public var liveActivityStartToken: String? {
        didSet {
            guard liveActivityStartToken != oldValue else { return }
            Task { await sendIfPossible() }
        }
    }

    /// The last token APNs gave us, held until there is an account to file it
    /// under. Registration and sign-in finish in either order, and whichever is
    /// second completes the pair.
    private var token: String?

    private init() {}

    public func received(_ deviceToken: Data) {
        token = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { await sendIfPossible() }
    }

    /// Called after signing in, for the case where the token arrived first —
    /// and after the notification toggles change, for the case below.
    ///
    /// Registration runs when a push token arrives, which is at launch. A
    /// preference the relay only hears about then is one that appears to do
    /// nothing until the app happens to re-register, so the settings screen
    /// calls this on the setter. That is the same bug this whole change is
    /// about, one level up.
    public func sendIfPossible() async {
        guard let token, Account.shared.isSignedIn else { return }
        switch await Account.shared.registerDevice(
            pushToken: token,
            platform: platform,
            label: label(),
            environment: Self.environment,
            liveActivityStartToken: liveActivityStartToken,
            notifyOnDone: notifyOnDone())
        {
        case .success:
            registered = true
            lastError = nil
        case .failure(let why):
            // The lead sentence says what did not happen; the cause says why,
            // in the relay's own vocabulary. This used to be one fixed string
            // for every failure, so a phone with no network and a phone whose
            // session had expired were told the same thing.
            registered = false
            lastError = "Couldn’t tell the relay how to reach this device. \(why.message)"
        }
    }

    /// Which APNs issued this device's token: `development` or `production`.
    ///
    /// Read out of the embedded provisioning profile rather than compiled in
    /// behind `#if DEBUG`, because those two do not line up. A Release build
    /// installed from Xcode is still development-signed, and the same source
    /// through TestFlight is not — the build configuration is simply not the
    /// question being asked. The entitlement is.
    ///
    /// This matters because getting it wrong is silent. A sandbox token sent to
    /// production APNs is refused with `BadDeviceToken`; nothing crashes,
    /// nothing logs on the phone, and the relay records an attempt it believes
    /// it made correctly. The phone just never rings.
    ///
    /// Production whenever the answer cannot be found, because that is the App
    /// Store case — the one where there is no developer watching to notice.
    public static let environment: String = {
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
            let data = try? Data(contentsOf: url)
        else { return "production" }

        // The profile is a CMS signature wrapping a plain XML plist. We are not
        // the audience for the signature — the OS already checked it, and this
        // is reading our own bundle — so this finds the payload rather than
        // pulling in Security to unwrap it properly.
        guard let start = data.range(of: Data("<plist".utf8)),
            let end = data.range(of: Data("</plist>".utf8), in: start.upperBound..<data.endIndex),
            let parsed = try? PropertyListSerialization.propertyList(
                from: data[start.lowerBound..<end.upperBound], format: nil) as? [String: Any],
            let entitlements = parsed["Entitlements"] as? [String: Any],
            let environment = entitlements["aps-environment"] as? String
        else { return "production" }
        return environment
    }()

    /// Not surfaced as an error by the caller: registration fails on the
    /// simulator and in any build without a push entitlement, neither of which
    /// is worth telling someone about, and local notifications keep working.
    public func unavailable(_ error: Error) {
        print("remote notifications unavailable: \(error.localizedDescription)")
    }
}
