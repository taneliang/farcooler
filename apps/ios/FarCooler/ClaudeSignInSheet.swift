import SwiftUI

/// What the Claude section needs from a runner (ov-584): its name, who the
/// orchestrator's lanes run as, and the calls. Built by `RunnerSettingsModel`.
struct ClaudeSignInSupport {
    let runner: String
    let calls: ClaudeSignInModel.Calls
    /// Sign `handle` out; false when the runner didn't confirm it.
    let signOut: (String) async -> Bool
    /// The login changed on the runner: read the people again.
    let changed: () async -> Void
}

/// A member's own Claude login, above People (ov-584): sign in, sign in
/// again, sign out. Only on a member's own device, on a runner that signs a
/// member in; the Mac's section of the same name says the same words
/// (`ClaudeSignIn`).
struct ClaudeLoginSection: View {
    let handle: String
    let login: ClaudeLogin
    let support: ClaudeSignInSupport
    /// Bring the sheet up when the section first appears (ov-587), once:
    /// arrived from a Claude login line in Needs You.
    var startNow = false

    @State private var startedNow = false
    @State private var signingIn = false
    @State private var confirmingSignOut = false
    @State private var signingOut = false
    @State private var signOutFailed = false
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        Section {
            switch login.state {
            case .none:
                Text(ClaudeSignIn.notSignedInBody)
                    .accessibilityIdentifier("claude-login-state")
                Button(ClaudeSignIn.signInButton) { signingIn = true }
                    .accessibilityIdentifier("claude-sign-in")
                    .sheet(isPresented: $signingIn, onDismiss: signInEnded) {
                        ClaudeSignInSheet(handle: handle, support: support)
                    }
            case .signedIn:
                // One line, cut in the middle. At an accessibility size a line holds
                // too little of an email, so it wraps whole, breaking after
                // punctuation, and the Form scrolls.
                Text(
                    typeSize.isAccessibilitySize
                        ? ClaudeSignIn.signedInLine(email: ClaudeSignIn.breakable(login.email))
                        : ClaudeSignIn.signedInLine(email: login.email)
                )
                .lineLimit(typeSize.isAccessibilitySize ? nil : 1)
                .truncationMode(.middle)
                .accessibilityLabel(ClaudeSignIn.signedInLine(email: login.email))
                .accessibilityIdentifier("claude-login-state")
                signInAgain
                signOutButton
            case .stoppedWorking:
                Text(ClaudeSignIn.stoppedWorkingLine)
                    .accessibilityIdentifier("claude-login-state")
                signInAgain
                signOutButton
            }
            if signOutFailed {
                Label(ClaudeSignIn.signOutFailed, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("claude-sign-out-failed")
            }
        } header: {
            Text(ClaudeSignIn.sectionTitle)
        } footer: {
            Text(ClaudeSignIn.trustCopy(runner: support.runner))
        }
        .onAppear {
            if Self.opensSheet(startNow: startNow, alreadyStarted: startedNow) {
                startedNow = true
                signingIn = true
            }
        }
    }

    /// Whether the section brings its sheet up as it appears (ov-587): when
    /// it was opened from a Claude login line, and only the first time.
    static func opensSheet(startNow: Bool, alreadyStarted: Bool) -> Bool { startNow && !alreadyStarted }

    private var signInAgain: some View {
        Button(ClaudeSignIn.signInAgainButton) { signingIn = true }
            .accessibilityIdentifier("claude-sign-in-again")
            .sheet(isPresented: $signingIn, onDismiss: signInEnded) {
                ClaudeSignInSheet(handle: handle, support: support)
            }
    }

    private var signOutButton: some View {
        Button(ClaudeSignIn.signOutButton, role: .destructive) { confirmingSignOut = true }
            .disabled(signingOut)
            .accessibilityIdentifier("claude-sign-out")
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

    /// The login on the runner moved, whichever way the sheet ended.
    private func signInEnded() { Task { await support.changed() } }

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

/// "Sign In to Claude": a link from the runner, the sign-in page in Safari,
/// and the code it shows pasted back (ov-584).
struct ClaudeSignInSheet: View {
    let support: ClaudeSignInSupport
    @StateObject private var model: ClaudeSignInModel
    @Environment(\.dismiss) private var dismiss
    /// Opens in Safari, as the system does by default: the member's own
    /// claude.ai session is there, which an in-app browser would not have.
    @Environment(\.openURL) private var openURL
    @FocusState private var codeFocused: Bool
    @Environment(\.dynamicTypeSize) private var typeSize

    init(handle: String, support: ClaudeSignInSupport) {
        self.support = support
        _model = StateObject(
            wrappedValue: ClaudeSignInModel(handle: handle, runner: support.runner, calls: support.calls))
    }

    var body: some View {
        NavigationStack {
            Form {
                switch model.step {
                case .starting:
                    Section {
                        HStack(spacing: 12) {
                            ProgressView()
                            Text(ClaudeSignIn.starting(runner: support.runner))
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("claude-sign-in-starting")
                    }
                case .page:
                    pageSections
                case .finishing:
                    Section {
                        HStack(spacing: 12) {
                            ProgressView()
                            Text(ClaudeSignIn.finishing)
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("claude-sign-in-finishing")
                    }
                case .done(let email):
                    Section {
                        doneLine(email)
                    }
                case .failed(let failure):
                    Section {
                        Label(failure.message, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .accessibilityElement(children: .combine)
                            .accessibilityIdentifier("claude-sign-in-failure")
                        if let retry = failure.retry {
                            Button(retry.rawValue) { Task { await model.begin() } }
                                .accessibilityIdentifier("claude-sign-in-retry")
                        }
                    }
                }
            }
            .navigationTitle(ClaudeSignIn.sheetTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if case .done = model.step {} else {
                        Button(closeTitle) { dismiss() }
                            .disabled(model.step == .finishing)
                            .accessibilityIdentifier("claude-sign-in-cancel")
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    switch model.step {
                    case .done:
                        Button(ClaudeSignIn.doneButton) { dismiss() }
                            .accessibilityIdentifier("claude-sign-in-done")
                    case .page:
                        Button(ClaudeSignIn.finishButton) { Task { await model.finish() } }
                            .disabled(!model.canFinish)
                            .accessibilityIdentifier("claude-sign-in-finish")
                    default:
                        EmptyView()
                    }
                }
            }
        }
        .interactiveDismissDisabled(model.step == .finishing)
        .task { await model.beginIfNeeded() }
        // However the sheet goes (Cancel, a swipe, the runner's refusal), a
        // sign-in still under way on the runner ends with it.
        .onDisappear { Task { await model.close() } }
    }

    /// Cancel while there is something to cancel; Close for a failure with
    /// no move left but to leave.
    private var closeTitle: String {
        if case .failed(let failure) = model.step, failure.retry == nil { return ClaudeSignIn.closeButton }
        return ClaudeSignIn.cancelButton
    }

    /// "Signed in as", then the email on its own line, cut in the middle if it
    /// is long, never broken mid-word. One element: "Signed in as <email>."
    private func doneLine(_ email: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
            if email.isEmpty {
                Text(ClaudeSignIn.signedInAs(email))
            } else {
                VStack(alignment: .leading, spacing: 2) {
                    Text(ClaudeSignIn.signedInAsLead)
                    Text(typeSize.isAccessibilitySize ? ClaudeSignIn.breakable(email) : email)
                        .lineLimit(typeSize.isAccessibilitySize ? nil : 1)
                        .truncationMode(.middle)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(ClaudeSignIn.signedInAs(email))
        .accessibilityIdentifier("claude-sign-in-done-line")
    }

    @ViewBuilder
    private var pageSections: some View {
        Section {
            Text(ClaudeSignIn.openPageStep)
            Button(ClaudeSignIn.openPageButton) {
                if let page = model.pageToOpen() {
                    openURL(page) { accepted in
                        if !accepted { model.pageNotOpened() }
                    }
                    codeFocused = false
                }
            }
            .accessibilityIdentifier("claude-open-page")
            if let notice = model.openNotice {
                Label(notice, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("claude-open-notice")
            }
        }
        Section {
            Text(ClaudeSignIn.codeStep)
            // Wraps: a code is most of a line on its own, and a field that
            // scrolls sideways hides the end of what was pasted.
            TextField(ClaudeSignIn.codeLabel, text: $model.code, axis: .vertical)
                .lineLimit(1...4)
                .font(.body.monospaced())
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.asciiCapable)
                .textContentType(.oneTimeCode)
                .focused($codeFocused)
                .accessibilityIdentifier("claude-code-field")
            if let notice = model.codeNotice {
                Label(notice, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("claude-code-notice")
            }
        } footer: {
            Text(ClaudeSignIn.trustCopy(runner: support.runner))
        }
    }
}
