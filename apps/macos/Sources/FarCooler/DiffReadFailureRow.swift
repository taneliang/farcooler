import SwiftUI

/// A file whose changes could not be read (ov-155).
///
/// Said in the diff's own column, where the lines would have been, with the one
/// thing a person can do about it. The CLI's words are not drawn here: a
/// worktree-level failure puts them in the pane's error box, and a file that
/// failed alone has nothing a reader could act on beyond trying again.
struct DiffReadFailureRow: View {
    let font: Font
    let retry: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle")
                .foregroundStyle(.secondary)
            Text("Couldn’t read this file’s changes.")
                .font(font)
                .foregroundStyle(.secondary)
            Button("Try Again", action: retry)
                .buttonStyle(.link)
                .font(font)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .accessibilityElement(children: .combine)
    }
}
