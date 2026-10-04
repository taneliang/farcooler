import AgentKit
import SwiftUI

/// "Some large files weren’t downloaded." above a worktree's changes, with
/// Try Again (ov-199).
///
/// Its own view, observing the client, so a fleet refresh redraws this one row
/// and not the diff beneath it. What it says and when it shows is `LfsNotice`
/// in AgentKit; the count is the runner's, read from the live worktree because
/// the pane's own copy is as it was when the pane opened. Neutral, not orange:
/// nothing here is wrong with the diff, only with what the runner could fetch.
struct ChangesLfsNotice: View {
    @ObservedObject var client: DaemonClient
    let worktree: String

    @State private var working = false
    @State private var failure: String?

    private var notice: LfsNotice? {
        LfsNotice.make(pointers: client.fleet.worktrees.first { $0.id == worktree }?.lfsPointers, mayAct: true)
    }

    var body: some View {
        if let notice {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: "arrow.down.circle").foregroundStyle(.secondary)
                    Text(LfsNotice.title).font(.system(size: WorkspaceStyle.PaneText.secondary))
                    Spacer(minLength: 8)
                    if notice.canRetry {
                        Button(working ? LfsNotice.retrying : LfsNotice.retry) { retry() }
                            .disabled(working)
                            .controlSize(.small)
                            .accessibilityIdentifier("changes-lfs-retry")
                    }
                }
                Text(failure ?? LfsNotice.detail)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("changes-lfs-notice")
        }
    }

    private func retry() {
        working = true
        failure = nil
        Task {
            let message = await client.hydrateLfs(worktree)
            if message != nil { failure = LfsNotice.unreachable }
            working = false
        }
    }
}
