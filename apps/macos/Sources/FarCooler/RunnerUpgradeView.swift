import AgentKit
import SwiftUI

/// The recovery screen (ov-984): what this Mac's window shows while its runner
/// is mid-upgrade, with the one button that finishes it.
///
/// Purpose and outcome first, as onboarding copy does: why the runner is
/// waiting, and what pressing the button gets you. Then each step in plain
/// words while it runs, and on a failure, what happened and Recover.
struct RunnerUpgradeView: View {
    @ObservedObject var upgrade: RunnerUpgrade
    var refresh: @MainActor () async -> Void

    /// The buttons a phase offers.
    enum Action: Equatable {
        case finish
        case recover
    }

    nonisolated static func actions(for phase: RunnerUpgrade.Phase) -> [Action] {
        switch phase {
        case .ready: [.finish]
        case .failed: [.recover]
        case .confirming, .running, .recovering, .done: []
        }
    }

    static let title = "Finish Upgrading Far Cooler"
    static let lede =
        "This version keeps your work in a new format. Finishing moves your data over, keeps a backup, "
        + "and opens your work again."
    static let promptNote = "It takes a moment, and asks for Touch ID or your password first."

    var body: some View {
        ContentUnavailableView {
            EmptyStateTitle(heading, symbol: symbol)
        } description: {
            description
        } actions: {
            actions
        }
        .accessibilityIdentifier("runner-upgrade")
    }

    private var heading: String {
        switch upgrade.phase {
        case .ready, .confirming: Self.title
        case .running: "Upgrading Far Cooler…"
        case .recovering: "Recovering…"
        case .failed: "The Upgrade Didn’t Finish"
        case .done: "Upgrade Finished"
        }
    }

    private var symbol: String {
        switch upgrade.phase {
        case .failed: "exclamationmark.triangle"
        case .done: "checkmark.circle"
        default: "arrow.up.circle"
        }
    }

    @ViewBuilder private var description: some View {
        switch upgrade.phase {
        case .ready(let notice):
            VStack(spacing: 8) {
                Text(Self.lede)
                Text(notice ?? Self.promptNote)
            }
        case .confirming:
            Text(Self.lede)
        case .running(let current):
            steps(current: current)
        case .recovering:
            Text("Putting your data back as it was before the upgrade.")
        case .failed(let failed, let detail):
            VStack(spacing: 8) {
                Text(Self.sentence(for: failed))
                Text(RunnerUpgrade.recoverCopy(for: failed))
                // The runner's own words in a box, never the app's voice:
                // under the sentences, since the actions row lays out across.
                if !detail.isEmpty {
                    DetailBox(text: detail)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: 420)
                        .padding(.top, 4)
                }
            }
        case .done:
            Text("Opening your work…")
        }
    }

    /// The failure's sentence: the step's, or Recover's own.
    static func sentence(for failed: RunnerUpgrade.Failed) -> String {
        switch failed {
        case .step(let step): step.failure
        case .recover: "Far Cooler couldn’t put your data back."
        }
    }

    /// Each step of this run: done, running, or still to come.
    private func steps(current: RunnerUpgrade.Step) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(upgrade.steps, id: \.self) { step in
                HStack(spacing: 8) {
                    Group {
                        if step.rawValue < current.rawValue {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                        } else if step == current {
                            ProgressView().controlSize(.small)
                        } else {
                            Image(systemName: "circle").foregroundStyle(.tertiary)
                        }
                    }
                    .frame(width: 16, height: 16)
                    Text(step.title)
                        .foregroundStyle(step.rawValue > current.rawValue ? .secondary : .primary)
                }
                .accessibilityElement(children: .combine)
            }
        }
        // Its own width, so the list sits centered under the title.
        .fixedSize()
    }

    @ViewBuilder private var actions: some View {
        ForEach(Self.actions(for: upgrade.phase), id: \.self) { action in
            switch action {
            case .finish:
                Button("Finish Upgrade") { Task { await upgrade.finish(refresh: refresh) } }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            case .recover:
                Button("Recover") { Task { await upgrade.recover(refresh: refresh) } }
                    .buttonStyle(.borderedProminent)
            }
        }
    }
}
