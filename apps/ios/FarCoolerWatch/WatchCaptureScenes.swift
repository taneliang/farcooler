import SwiftUI

#if DEBUG
    /// Screens for a watch simulator to photograph, chosen by launch environment.
    ///
    /// Nothing else can show the watch's team words (ov-739): they need a
    /// runner with members, a paired phone, and an ask waiting on someone
    /// else, and the preview canvas is not a screenshot a lane can attach.
    /// `xcrun simctl launch` with `SIMCTL_CHILD_FARCOOLER_WATCH_SCENE=<scene>`
    /// lands on one of them with a fixed fleet and no phone.
    ///
    /// Scenes: `list`, `answer`, `permission`. `FARCOOLER_WATCH_WHO` is the
    /// member's name ("Bob" if unset; empty for the owner), and
    /// `FARCOOLER_WATCH_CONFIRM=1` raises the "Answer for them?" alert at once,
    /// since `simctl` cannot tap. Not compiled into a release build.
    @MainActor
    enum WatchCaptureScenes {
        static var scene: String? { ProcessInfo.processInfo.environment["FARCOOLER_WATCH_SCENE"] }

        static var confirmAtOnce: Bool {
            ProcessInfo.processInfo.environment["FARCOOLER_WATCH_CONFIRM"] == "1"
        }

        private static var who: String {
            ProcessInfo.processInfo.environment["FARCOOLER_WATCH_WHO"] ?? "Bob"
        }

        private static func ask(
            _ id: String, _ question: String, terminal: String, concern: NeedsYouConcern?
        ) -> NeedsYouItem {
            NeedsYouItem(
                id: "ask:\(id)", kind: .ask, rank: 5, since: Date().addingTimeInterval(-240),
                workspaceName: "Billing",
                terminal: NeedsYouTerminal(
                    id: terminal, worktreeID: nil, label: "claude", role: nil, paneMode: "claude"),
                question: question, askID: id,
                actions: [
                    NeedsYouAction(id: "allow", title: "Allow", destructive: false, primary: true),
                    NeedsYouAction(id: "deny", title: "Deny", destructive: true, primary: false),
                ],
                runner: "studio", concern: concern)
        }

        static var snapshot: FleetSnapshot {
            let theirs = NeedsYouConcern(handle: "m", name: who, yours: false)
            let mine = NeedsYouConcern(handle: "me", name: "Me", yours: true)
            var base = PreviewFleet.snapshot
            base = FleetSnapshot(
                agents: base.agents, capturedAt: base.capturedAt, complete: true,
                reviewsWaiting: nil,
                needsYou: [
                    ask("1", "Run cargo test in the workspace root", terminal: "a", concern: theirs),
                    ask("2", "Allow touch x", terminal: "b", concern: mine),
                    ask("3", "Allow npm install", terminal: "c", concern: nil),
                ])
            return base
        }

        @ViewBuilder static func view() -> some View {
            let client = PreviewFleetClient(
                .live(snapshot), reply: { _ in .permission(PreviewFleet.claudePermission) })
            switch scene {
            case "answer":
                NavigationStack {
                    NeedsYouItemView(client: client, key: snapshot.needsYou![0].key)
                }
            case "permission":
                NavigationStack { PermissionView(client: client, terminal: "a") }
            default:
                FleetListView(client: client)
            }
        }
    }
#endif
