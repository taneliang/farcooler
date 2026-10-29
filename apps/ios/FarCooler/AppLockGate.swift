import AgentKit
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

    static func ask() async -> Bool {
        let context = LAContext()
        context.touchIDAuthenticationAllowableReuseDuration = 0
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: AppLock.prompt)) ?? false
    }
}

/// The lock: Face ID when the app resumes after five minutes away, and at
/// launch. The SSH key is a `ThisDeviceOnly` keychain item, so the person
/// holding an unlocked phone can use it only through this app; this is what
/// stops them, shell included.
struct AppLockGate: ViewModifier {
    @Environment(\.scenePhase) private var phase
    @State private var lock = AppLock.atLaunch(isOn: AppLockSetting.isOn)
    @State private var asking = false
    @State private var declined = false

    func body(content: Content) -> some View {
        content
            .overlay {
                if lock.isLocked { LockedCover(declined: declined, unlock: attempt) }
            }
            .onChange(of: phase) { _, now in
                switch now {
                case .background:
                    lock.isOn = AppLockSetting.isOn
                    lock.didLeave(at: .now)
                case .active:
                    lock.didReturn(at: .now)
                    if lock.isLocked { attempt() }
                default: break
                }
            }
            .task { if lock.isLocked { attempt() } }
    }

    private func attempt() {
        guard !asking else { return }
        guard AppLockAuth.canAsk else {
            lock.unlock()
            return
        }
        asking = true
        Task {
            let opened = await AppLockAuth.ask()
            asking = false
            declined = !opened
            if opened { lock.unlock() }
        }
    }
}

/// What covers the app while it is locked: nothing of the runner shows.
private struct LockedCover: View {
    let declined: Bool
    let unlock: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.fill").font(.system(size: 36)).foregroundStyle(.secondary)
            Text("Far Cooler is locked").font(.title3.weight(.semibold))
            if declined {
                Text("Face ID or your passcode didn’t work. Try again.")
                    .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            Button(AppLock.unlockButton, action: unlock)
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("app-lock-unlock")
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
