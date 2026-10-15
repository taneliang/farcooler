import SwiftUI

/// What a reply from a teammate's runner admin says, and the words this Mac
/// asks with before joining it (ov-543). The phone asks the same question in
/// the same words (`CeremonyStore.joinTitle` on iOS), and so does Android.
///
/// **`granted_by` is unauthenticated.** The reply proves nothing about who
/// made it, so the sentence says what "this code says" and never states it as
/// fact. The only thing the question can do is give a person the chance to say
/// no when they weren't adding this Mac with anyone just now.
extension CeremonyManifest {
    /// The email the reply says granted it, or nil when it names none (a
    /// same-account reply). Only another account's grant is asked about.
    var grantor: String? {
        let named = granted_by?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return named.isEmpty ? nil : named
    }
}

enum TeammateJoinCopy {
    /// A runner's label is one line in a sentence, cut in the middle past this
    /// many characters.
    static let labelLimit = 28

    /// The headline: one runner by name, or several by whose they are.
    static func title(runners: [CeremonyRunner]) -> String {
        if runners.count == 1, let only = runners.first {
            return "Join Your Teammate’s Runner “\(shortened(only.label))”?"
        }
        return "Join Your Teammate’s Runners?"
    }

    /// What joining means, under it.
    static func detail(from: String) -> String {
        "This code says \(from) added this device to their runners. Work there runs on the Claude account "
            + "signed in on each runner, so its usage and limits are shared. Join only if you were adding "
            + "this device with them just now."
    }

    /// `label` cut in the middle to at most `labelLimit` characters.
    static func shortened(_ label: String, limit: Int = labelLimit) -> String {
        guard label.count > limit, limit > 1 else { return label }
        let keep = limit - 1
        let head = keep - keep / 2
        return String(label.prefix(head)) + "…" + String(label.suffix(keep / 2))
    }
}

/// A reply from another account's runner admin, held until the person at this
/// Mac says yes. Nothing is added before that: these are someone else's
/// runners, on the Claude account signed in there.
struct TeammateJoinPrompt: View {
    let from: String
    let runners: [CeremonyRunner]
    let join: () -> Void
    let cancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            SheetHeader(title: TeammateJoinCopy.title(runners: runners))
                .accessibilityIdentifier("join-teammate-title")

            Text(TeammateJoinCopy.detail(from: from))
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if runners.count > 1 {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(runners) { runner in
                        // As its owner named it: one line, cut in the middle.
                        Text(runner.label)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }

            HStack {
                Spacer()
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("join-teammate-cancel")
                Button("Join", action: join)
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("join-teammate-join")
            }
        }
    }
}
