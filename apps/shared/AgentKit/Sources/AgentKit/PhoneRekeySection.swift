import SwiftUI

/// One line of a re-key's progress, as Settings shows it.
public struct PhoneRekeyLine: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let detail: String
    public let done: Bool
}

extension PhoneRekey {
    /// What Settings lists while a re-key runs and after: plain words, one line
    /// per step and per runner (ov-569).
    public func progress(signedIn: Bool) -> [PhoneRekeyLine] {
        var lines: [PhoneRekeyLine] = []
        let made = keys?.nextPublicKey != nil || keys?.previousPublicKey != nil || stage == .finished
        lines.append(PhoneRekeyLine(
            id: "key", title: "New Key", detail: made ? "Made on this phone." : "Making a new key…", done: made))
        if !signedIn {
            lines.append(PhoneRekeyLine(
                id: "account", title: "Your Account",
                detail: "Sign in to add the new key to your account.", done: false))
        } else {
            let confirmed = registered == .trusted || registered == .certified
            let detail: String
            if registered == nil {
                detail = "Adding it to your account…"
            } else if confirmed {
                detail = "Added to your account and confirmed."
            } else if needsScan {
                detail = "Added, not confirmed yet. Show this phone’s code and scan it from one of your "
                    + "confirmed devices."
            } else {
                detail = "Added, not confirmed yet."
            }
            lines.append(PhoneRekeyLine(id: "account", title: "Your Account", detail: detail, done: confirmed))
        }
        for runner in runners() {
            let move = moves[runner.id] ?? (keys?.runnersOnPrevious.contains(runner.id) == true ? .notReached : .waiting)
            lines.append(PhoneRekeyLine(id: "runner-\(runner.id)", title: runner.name, detail: move.detail, done: move == .moved))
        }
        if signedIn {
            lines.append(PhoneRekeyLine(
                id: "old", title: "Old Key",
                detail: oldRemoved ? "Removed from your account." : "Removed from your account once the runners have moved.",
                done: oldRemoved))
        }
        return lines
    }

    /// What the section says before a re-key, given whether the directory
    /// still lists this phone's key. A removed key can't come back, so the
    /// phone can't be listed to a team until it has a new one.
    public static func invitation(removed: Bool) -> String {
        removed
            ? "This phone was removed from your account, so your team admins can’t add it to their runners. "
                + "Make a new key to add it back."
            : "This phone signs in to your runners with its own key. Make a new one if this phone was removed "
                + "from your account, or if you think its key was copied."
    }

    /// Said beside Forget the Old Key, while runners still use it.
    public static let forgetWarning =
        "Runners that haven’t moved still let this phone in with its old key. If you forget it, add this "
        + "phone to them again from a device that’s already on them."

    /// The confirmation dialog's message.
    public static let confirmation =
        "Far Cooler makes a new key, adds it to your account and moves each of your runners to it. "
        + "Your old key stops working once that’s done. Keep Far Cooler open until it finishes."
}

/// "Make a New Key" in Settings, on a phone (ov-569). Nothing on a build
/// without key slots, which is every Mac.
struct PhoneRekeySection: View {
    @ObservedObject private var rekey = PhoneRekey.shared
    @ObservedObject private var account = Account.shared
    @Environment(\.colorScheme) private var scheme
    @State private var removed = false
    @State private var asking = false

    var body: some View {
        if rekey.isAvailable {
            Section {
                content
            } header: {
                Text("This Phone’s Key")
            }
            .confirmationDialog(
                "Make a New Key for This Phone?", isPresented: $asking, titleVisibility: .visible
            ) {
                Button("Make a New Key") { Task { await rekey.run() } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(PhoneRekey.confirmation)
            }
            .task { await loadRemoved() }
        }
    }

    @ViewBuilder
    private var content: some View {
        if rekey.stage == .notStarted, !rekey.isUnderway {
            if removed {
                Label {
                    Text(PhoneRekey.invitation(removed: true))
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Tint.attention(scheme))
                }
            } else {
                Text(PhoneRekey.invitation(removed: false))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Button("Make a New Key") { asking = true }
        } else {
            ForEach(rekey.progress(signedIn: account.isSignedIn)) { line in
                HStack(alignment: .firstTextBaseline) {
                    Image(systemName: line.done ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(line.title)
                        Text(line.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
            }
            if case .failed(let failure) = rekey.stage {
                Text(failure.message)
                    .font(.callout)
                    .foregroundStyle(Tint.failure)
            }
            if rekey.stage == .working {
                ProgressView()
            } else {
                if let code = PhoneRekey.codeView, rekey.needsScan {
                    NavigationLink("Show This Phone’s Code") { code() }
                }
                Button(rekey.stage == .notStarted || rekey.isFailed ? "Continue" : "Check Again") {
                    Task { await rekey.run() }
                }
                if !rekey.runnersLeft.isEmpty {
                    Text(PhoneRekey.forgetWarning)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button("Forget the Old Key", role: .destructive) { rekey.forgetOldKey() }
                }
            }
        }
    }

    private func loadRemoved() async {
        guard account.isSignedIn, let mine = rekey.keys?.currentPublicKey else { return }
        guard case .success(let rows) = await account.directoryClient.deviceKeys(includingRemoved: true) else {
            return
        }
        removed = rows.first { $0.isHeld(by: mine) }?.isRemoved ?? false
    }
}

extension PhoneRekey {
    /// The screen that shows this phone's code for a confirmed device to
    /// scan: the app's join screen. Set by the iPhone app.
    public static var codeView: (() -> AnyView)?

    var isFailed: Bool {
        if case .failed = stage { return true }
        return false
    }
}
