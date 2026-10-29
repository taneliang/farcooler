import LocalAuthentication
import SwiftUI

/// Whether the app lock is on (ov-856, §3a). On unless the person turned it
/// off in Settings. A launch argument `-appLock NO` overrides it for a run,
/// which is how the UI tests keep their own screens clear.
enum AppLockSetting {
    static let key = "appLock"

    static var isOn: Bool {
        // `bool(forKey:)` reads the launch argument's "NO" as false, where
        // `object(forKey:) as? Bool` would see a string and fall to the default.
        UserDefaults.standard.object(forKey: key) == nil ? true : UserDefaults.standard.bool(forKey: key)
    }
}

/// Face ID or the passcode, asked once, with nothing remembered between two.
enum AppLockAuth {
    /// Whether this device can ask at all: with no passcode set there is
    /// nothing to ask with, and a lock that can't open would only lock the
    /// person out of their own runners.
    static var canAsk: Bool {
        LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
    }

    static func ask(_ reason: String = AppLock.prompt) async -> Bool {
        let context = LAContext()
        context.touchIDAuthenticationAllowableReuseDuration = 0
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) ?? false
    }

    /// Whether the person just proved they are who holds the phone: true at
    /// once on a device with nothing to ask with.
    static func verify(_ reason: String) async -> Bool {
        canAsk ? await ask(reason) : true
    }
}

/// The one lock the whole app reads: the gate draws it, and every way in
/// (a tapped notification, a link) waits on it.
@MainActor
final class AppLockCenter: ObservableObject {
    static let shared = AppLockCenter()
    @Published var lock = AppLock.atLaunch(isOn: AppLockSetting.isOn)
}

/// The lock: Face ID when the app resumes after five minutes away, and at
/// launch. The SSH key is a `ThisDeviceOnly` keychain item, so the person
/// holding an unlocked phone can use it only through this app; this is what
/// stops them, shell included.
///
/// The cover is a window of its own above alerts, because a SwiftUI overlay
/// is drawn under every sheet and full-screen cover, and the terminal is one.
struct AppLockGate: ViewModifier {
    @Environment(\.scenePhase) private var phase
    @ObservedObject private var center = AppLockCenter.shared
    @State private var asking = false
    @State private var declined = false
    @State private var window = LockWindow()

    func body(content: Content) -> some View {
        content
            .onChange(of: phase) { _, now in
                switch now {
                case .background:
                    center.lock.isOn = AppLockSetting.isOn
                    center.lock.didLeave(at: .now)
                case .active:
                    center.lock.didReturn(at: .now)
                    if center.lock.isLocked { attempt() }
                default: break
                }
                refresh()
            }
            .onChange(of: center.lock) { _, _ in refresh() }
            .onChange(of: declined) { _, _ in refresh() }
            .task {
                refresh()
                if center.lock.isLocked { attempt() }
            }
    }

    /// Cover the app while it is locked, and while it is away from the
    /// screen with the lock on, so the app switcher's snapshot shows no
    /// terminal.
    private func refresh() {
        let locked = center.lock.isLocked
        if locked || (center.lock.isOn && phase != .active) {
            if locked { UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil) }
            window.show(locked: locked, declined: declined, unlock: attempt)
        } else {
            window.hide()
        }
    }

    private func attempt() {
        guard !asking else { return }
        #if DEBUG
        // A UI test that must see the lock stay up: no face to give it.
        if CommandLine.arguments.contains("-appLockHold") { return }
        #endif
        guard AppLockAuth.canAsk else {
            center.lock.unlock()
            return
        }
        asking = true
        Task {
            let opened = await AppLockAuth.ask()
            asking = false
            declined = !opened
            if opened { center.lock.unlock() }
        }
    }
}

/// The lock's window, above alerts and so above every sheet and cover.
@MainActor
final class LockWindow {
    private var window: UIWindow?
    private var host: UIHostingController<LockedCover>?
    private weak var previous: UIWindow?

    func show(locked: Bool, declined: Bool, unlock: @escaping () -> Void) {
        let cover = LockedCover(declined: declined, locked: locked, unlock: unlock)
        if let host, let window {
            host.rootView = cover
            window.isHidden = false
            if locked, !window.isKeyWindow { window.makeKey() }
            return
        }
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first else { return }
        previous = scene.windows.first { $0.isKeyWindow }
        let host = UIHostingController(rootView: cover)
        host.view.backgroundColor = .clear
        let window = UIWindow(windowScene: scene)
        window.windowLevel = .alert + 1
        window.rootViewController = host
        window.isHidden = false
        if locked { window.makeKey() }
        self.host = host
        self.window = window
    }

    func hide() {
        guard let window else { return }
        window.isHidden = true
        window.rootViewController = nil
        self.window = nil
        host = nil
        previous?.makeKey()
    }
}

/// What covers the app while it is locked: nothing of the runner shows.
struct LockedCover: View {
    let declined: Bool
    /// False while the cover is only the app switcher's: nothing to press yet.
    let locked: Bool
    let unlock: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.fill").font(.system(size: 36)).foregroundStyle(.secondary)
            Text("Far Cooler is locked").font(.title3.weight(.semibold))
            if declined {
                Text("Face ID or your passcode didn’t work. Try again.")
                    .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            if locked {
                Button(AppLock.unlockButton, action: unlock)
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("app-lock-unlock")
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("app-lock")
    }
}

extension View {
    /// Put the app lock over this view.
    func appLocked() -> some View { modifier(AppLockGate()) }
}
