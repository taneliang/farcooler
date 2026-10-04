import SwiftUI

// The two small things a patch owes its reader beside its lines (ov-149): that
// it is not the whole patch, and the way to ask for the rest. Their own file
// because `ChangesView.swift` is past its size ceiling.

/// One sentence above a patch: that the daemon cut it off, or that it is a
/// merge shown against its first parent.
struct PatchNoticeRow: View {
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: "exclamationmark.triangle")
                .accessibilityHidden(true)
            Text(text)
                .accessibilityIdentifier("changes-file-notice")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }
}

/// The row that offers the lines `PatchBudget` held back.
struct ShowMoreLinesButton: View {
    let title: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: "chevron.up.chevron.down")
                .font(.footnote)
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .accessibilityIdentifier("changes-show-more-lines")
    }
}
