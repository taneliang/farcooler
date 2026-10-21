#if DEBUG
import Foundation

// `-phone-many-repos` (ov-676): the canned runner holds three more repositories
// with long names, each with its main checkout, an idle orchestrator named for
// its work, and one with a second workspace, so All Agents has to tell four
// repositories' "Main" apart.
enum HarnessManyRepos {
    static var on: Bool { CommandLine.arguments.contains("-phone-many-repos") }

    private static let names = [
        "farcooler-relay-and-push-notification-service",
        "customer-billing-and-invoicing-platform",
        "next-js-storefront",
    ]
    private static let titles = [
        "Minimize CI costs across every runner and every branch",
        "Project push to main once the review lands",
        "Next.js migration of the checkout and cart pages",
    ]

    private static func id(_ kind: Int, _ n: Int) -> String {
        "0198f2c0-0000-7000-8000-00000000\(kind)0\(n)\(n)"
    }

    static var repositories: [Repository] {
        guard on else { return [] }
        return names.indices.map { Repository(id: id(1, $0), short: "a10\($0)", displayName: names[$0], remote: "") }
    }

    static var worktrees: [Worktree] {
        guard on else { return [] }
        return names.indices.map { n in
            Worktree(
                id: id(3, n), short: "c10\(n)", repository: id(1, n), task: names[n], branch: "main", state: "ready",
                terminals: [
                    Terminal(
                        id: id(4, n), short: "d10\(n)", title: titles[n], preset: "claude", runningAgent: "claude",
                        state: "running", activity: "idle", line: String("\(titles[n]) idle".prefix(17)) + "…", epoch: 1,
                        paneMode: "terminal", workspace: id(2, n), role: "orchestrator")
                ],
                isMainCheckout: true, workspace: id(2, n), openTasks: [])
        }
    }

    static var workspaces: [WorkspaceSummary] {
        guard on else { return [] }
        return names.indices.map { n in
            WorkspaceSummary(
                id: id(2, n), name: "Main", taskPrefix: "m\(n)", isMain: true, ordinal: 0, repository: id(1, n),
                orchestrator: id(4, n))
        }
    }
}
#endif
