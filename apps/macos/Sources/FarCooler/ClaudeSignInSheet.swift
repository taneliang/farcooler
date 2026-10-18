import AgentKit
import AppKit
import SwiftUI

/// What the Claude section needs from a runner (ov-584): its name, who the
/// orchestrator's lanes run as, and the calls. Built by `RunnerSettingsStore`.
struct ClaudeSignInSupport {
    let runner: String
    let calls: ClaudeSignInModel.Calls
    /// Sign `handle` out; false when the runner didn't confirm it.
    let signOut: (String) async -> Bool
    /// The login changed on the runner: read the people again.
    let changed: () async -> Void
}

/// A member's own Claude login, above People (ov-584): sign in, sign in
/// again, sign out. Only on a member's own Mac, on a runner that signs a
/// member in. The iPhone's section of the same name says the same words
/// (`ClaudeSignIn`).
struct ClaudeLoginSection: View {
    let handle: String
    let login: ClaudeLogin
    let support: ClaudeSignInSupport

    /// The sign-in under way, while its sheet is up.
    @State private var signIn: ClaudeSignInModel?
    @State private var confirmingSignOut = false
    @State private var signingOut = false
    @State private var signOutFailed = false

    var body: some View {
        Section {
            switch login.state {
            case .none:
                Text(ClaudeSignIn.notSignedInBody)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("claude-login-state")
                HStack {
                    Spacer()
                    Button(ClaudeSignIn.signInButton) { startSigningIn() }
                        .identified("claude-sign-in")
                }
            case .signedIn, .stoppedWorking:
                HStack(spacing: 8) {
                    Text(
                        login.state == .signedIn
                            ? ClaudeSignIn.signedInLine(email: login.email) : ClaudeSignIn.stoppedWorkingLine
                    )
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .identified("claude-login-state")
                    Spacer(minLength: 8)
                    Button(ClaudeSignIn.signInAgainButton) { startSigningIn() }
                        .identified("claude-sign-in-again")
                    Button(ClaudeSignIn.signOutButton) { confirmingSignOut = true }
                        .disabled(signingOut)
                        .identified("claude-sign-out")
                }
            }
            if signOutFailed {
                Label(ClaudeSignIn.signOutFailed, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("claude-sign-out-failed")
            }
        } header: {
            Text(ClaudeSignIn.sectionTitle)
        } footer: {
            Text(ClaudeSignIn.trustCopy(runner: support.runner))
                .fixedSize(horizontal: false, vertical: true)
        }
        .sheet(
            isPresented: Binding(get: { signIn != nil }, set: { if !$0 { signIn = nil } })
        ) {
            // The login on the runner moved, whichever way the sheet ended.
            Task { await support.changed() }
        } content: {
            if let signIn {
                ClaudeSignInSheet(model: signIn, support: support)
            }
        }
        .confirmationDialog(
            ClaudeSignIn.signOutTitle(runner: support.runner), isPresented: $confirmingSignOut,
            titleVisibility: .visible
        ) {
            Button(ClaudeSignIn.signOutButton, role: .destructive) { signOut() }
            Button(ClaudeSignIn.cancelButton, role: .cancel) {}
        } message: {
            Text(ClaudeSignIn.signOutMessage)
        }
    }

    private func startSigningIn() {
        signIn = ClaudeSignInModel(handle: handle, runner: support.runner, calls: support.calls)
    }

    private func signOut() {
        signingOut = true
        signOutFailed = false
        Task {
            let done = await support.signOut(handle)
            signingOut = false
            signOutFailed = !done
            await support.changed()
        }
    }
}

/// "Sign In to Claude": a link from the runner, the sign-in page in the
/// default browser, and the code it shows pasted back (ov-584).
struct ClaudeSignInSheet: View {
    static let width: CGFloat = 440

    @ObservedObject var model: ClaudeSignInModel
    let support: ClaudeSignInSupport
    @Environment(\.dismiss) private var dismiss
    /// The default browser, where the member's own claude.ai session is.
    var openPage: (URL) -> Bool = { NSWorkspace.shared.open($0) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(ClaudeSignIn.sheetTitle)
                .font(.headline)
            // Scrolls, so a long error or a large text size never pushes the
            // buttons out of the sheet.
            ScrollView {
                content
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .scrollBounceBehavior(.basedOnSize)
            buttons
        }
        .padding(20)
        .frame(width: Self.width, height: 420)
        .interactiveDismissDisabled(model.step == .finishing)
        .task { await model.beginIfNeeded() }
        // However the sheet goes, a sign-in still under way on the runner
        // ends with it.
        .onDisappear { Task { await model.close() } }
    }

    @ViewBuilder
    private var content: some View {
        switch model.step {
        case .starting:
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text(ClaudeSignIn.starting(runner: support.runner))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .identified("claude-sign-in-starting")
        case .page:
            VStack(alignment: .leading, spacing: 12) {
                Text(ClaudeSignIn.openPageStep)
                    .fixedSize(horizontal: false, vertical: true)
                Button(ClaudeSignIn.openPageButton) {
                    if let page = model.pageToOpen(), !openPage(page) { model.pageNotOpened() }
                }
                .identified("claude-open-page")
                if let notice = model.openNotice {
                    Label(notice, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                        .identified("claude-open-notice")
                }
                Text(ClaudeSignIn.codeStep)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 4)
                // Wraps: a code is most of a line on its own, and a field that
                // scrolls sideways hides the end of what was pasted.
                TextField(ClaudeSignIn.codeLabel, text: $model.code, axis: .vertical)
                    .lineLimit(1...4)
                    .font(.body.monospaced())
                    .textFieldStyle(.roundedBorder)
                    .autocorrectionDisabled()
                    .identified("claude-code-field")
                if let notice = model.codeNotice {
                    Label(notice, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                        .identified("claude-code-notice")
                }
                Text(ClaudeSignIn.trustCopy(runner: support.runner))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        case .finishing:
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text(ClaudeSignIn.finishing)
            }
            .identified("claude-sign-in-finishing")
        case .done(let email):
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: "checkmark.circle.fill")
                    if email.isEmpty {
                        Text(ClaudeSignIn.signedInAs(email))
                    } else {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(ClaudeSignIn.signedInAsLead)
                            Text(email)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(ClaudeSignIn.signedInAs(email))
                .identified("claude-sign-in-done-line")
            }
        case .failed(let failure):
            VStack(alignment: .leading, spacing: 12) {
                Label(failure.message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("claude-sign-in-failure")
            }
        }
    }

    /// Sign In is the page step's alone; a failure's move (Try Again, Start
    /// Again) is the default, and a failure with none is closed.
    private var buttons: some View {
        HStack {
            Spacer()
            switch model.step {
            case .done:
                Button(ClaudeSignIn.doneButton) { dismiss() }
                    .keyboardShortcut(.defaultAction)
                    .identified("claude-sign-in-done")
            case .page:
                cancel(ClaudeSignIn.cancelButton)
                Button(ClaudeSignIn.finishButton) { Task { await model.finish() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canFinish)
                    .identified("claude-sign-in-finish")
            case .failed(let failure):
                if let retry = failure.retry {
                    cancel(ClaudeSignIn.cancelButton)
                    Button(retry.rawValue) { Task { await model.begin() } }
                        .keyboardShortcut(.defaultAction)
                        .identified("claude-sign-in-retry")
                } else {
                    close
                }
            case .starting, .finishing:
                cancel(ClaudeSignIn.cancelButton)
                    .disabled(model.step == .finishing)
            }
        }
    }

    /// Close for a failure with no move left: Return and Esc both close the
    /// sheet. A button takes one shortcut, so Esc is a hidden twin.
    private var close: some View {
        Button(ClaudeSignIn.closeButton) { dismiss() }
            .keyboardShortcut(.defaultAction)
            .identified("claude-sign-in-cancel")
            .background {
                Button("") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .opacity(0)
                    .accessibilityHidden(true)
            }
    }

    private func cancel(_ title: String) -> some View {
        Button(title) { dismiss() }
            .keyboardShortcut(.cancelAction)
            .identified("claude-sign-in-cancel")
    }
}
