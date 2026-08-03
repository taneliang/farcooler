import SwiftUI

/// The sign-in row, on both platforms.
///
/// Shared because it is the same three states — signed out, working, signed in
/// — and because the explanation underneath is the part that matters most: this
/// is the one place Far Cooler asks for an account, and someone reading it should
/// find out immediately that it buys notifications and nothing else. The fleet
/// works signed out. It always will.
public struct AccountSection: View {
    @ObservedObject private var account = Account.shared

    public init() {}

    public var body: some View {
        Section {
            if account.isSignedIn {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(account.email.isEmpty ? "Signed in" : account.email)
                        Text("Notifications can reach this device.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(AppVersion.display)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Sign out") { Task { await account.signOut() } }
                }
            } else if account.signingIn {
                HStack {
                    Text("Signing in…")
                    Spacer()
                    ProgressView().controlSize(.small)
                }
            } else {
                Button("Sign in") {
                    Task {
                        await account.signIn()
                        // Pairs a token that arrived before the account did.
                        // Registration and sign-in happen in either order.
                        await AccountSection.afterSignIn?()
                    }
                }
            }

            if let error = account.lastError {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
        } header: {
            Text("Account")
        } footer: {
            Text(
                "An account exists for one reason: so a machine can wake this device "
                    + "when an agent needs you. Everything else in Far Cooler works signed "
                    + "out, over your own SSH, with nothing in the middle."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    /// What to do once someone is signed in.
    ///
    /// A closure rather than a direct call because the two apps register
    /// different things — a phone has an APNs token, a Mac has its own — and
    /// this package should not know about either.
    public static var afterSignIn: (@MainActor () async -> Void)?
}
