import SwiftUI

/// "Some large files weren’t downloaded." at the top of a worktree's Changes,
/// with Try Again (ov-199).
///
/// What the notice says and when it shows is `LfsNotice`, in AgentKit, where
/// `swift test` holds it. Try Again asks the runner to download what it can
/// without touching anything the agent changed; the count comes back through
/// the fleet, so the card clears itself when the files arrive and stays when
/// they didn’t.
struct ChangesLfsNotice: View {
    let notice: LfsNotice
    /// Asks the runner to try again, and returns why it couldn’t be asked, or nil.
    let retry: @MainActor () async -> String?

    @State private var working = false
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Image(systemName: "arrow.down.circle").foregroundStyle(.secondary)
                Text(LfsNotice.title).font(.footnote).foregroundStyle(.primary)
                Spacer(minLength: 0)
            }
            Text(failure ?? LfsNotice.detail).font(.footnote).foregroundStyle(.secondary)
            if notice.canRetry {
                Button(working ? LfsNotice.retrying : LfsNotice.retry) {
                    working = true
                    failure = nil
                    Task {
                        failure = await retry()
                        working = false
                    }
                }
                .disabled(working)
                .accessibilityIdentifier("changes-lfs-retry")
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(ChangesSurface.card, in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("changes-lfs-notice")
    }
}

extension Connection {
    /// `worktree.hydrate_lfs`: the runner tries again to download a worktree’s
    /// large files. What it found comes back through the fleet’s next count;
    /// this says why the runner couldn’t be asked, or nil. A refusal says what
    /// `RunnerRefusal` says; anything else is `LfsNotice.unreachable`.
    func hydrateLfs(_ worktree: String) async -> String? {
        do {
            _ = try await core.call("worktree.hydrate_lfs", ["worktree": worktree])
            return nil
        } catch {
            return LfsNotice.failure(word: ClientCore.refusalWord(of: error))
        }
    }
}

extension ChangesView {
    /// What `ShellScreen` and the terminal’s Changes tab hand `ChangesView`:
    /// the worktree’s notice, with Try Again wired to its connection.
    static func lfs(_ worktree: Worktree?, _ connection: Connection) -> ChangesLfs? {
        guard let worktree,
            let notice = LfsNotice.make(pointers: worktree.lfsPointers, mayAct: connection.daemon?.mayAct ?? true)
        else { return nil }
        return ChangesLfs(notice: notice) { await connection.hydrateLfs(worktree.id) }
    }
}

/// A notice and the way to retry it, as one value `ChangesView` can be given.
struct ChangesLfs {
    let notice: LfsNotice
    let retry: @MainActor () async -> String?
}

extension ChangesLfs {
    /// `-lfs-pointers`: the Changes harness says two large files weren’t
    /// downloaded, and Try Again takes a moment and leaves them (the runner
    /// still lacks the objects). `-lfs-read-scope` takes the button away.
    static var harness: ChangesLfs? {
        let args = CommandLine.arguments
        guard args.contains("-lfs-pointers"),
            let notice = LfsNotice.make(pointers: 2, mayAct: !args.contains("-lfs-read-scope"))
        else { return nil }
        return ChangesLfs(notice: notice) {
            await Task.yield()
            do { try await Task.sleep(for: .seconds(2)) } catch { return false }
            return true
        }
    }
}
