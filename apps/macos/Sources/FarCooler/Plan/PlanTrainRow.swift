import AgentKit
import SwiftUI

// A train heading its lanes in Now (ov-309): "Train 72" and what it carries
// (ov-462), its slug second, where it stands, its integrating agent's state
// and spend (ov-461), and its CI as the runner last read it, "Red · c85bf83d · CI Failed · 1 of 3 jobs
// failed". Red, or a failed run, is the one thing drawn in amber, with its
// word. A click opens the CI run on GitHub, the one place it goes.

/// A train's row at the head of its group.
struct PlanTrainRow: View {
    let train: PlanTrain
    let ci: PlanCIRead?
    /// The runner's clock, for how old a stale read is.
    var now: Int64 = 0
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL
    @State private var hovering = false

    private var words: String { PlanWords.train(train, ci: ci, now: now) }
    private var attention: Bool { PlanWords.trainNeedsAttention(train, ci: ci) }
    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }

    var body: some View {
        Button {
            if let run { openURL(run) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Image(systemName: "train.side.front.car")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                    .glyphColumn()
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                        Text(train.heading)
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold))
                            .lineLimit(1)
                            .layoutPriority(1)
                        Spacer(minLength: 0)
                        if let slug = train.slug {
                            Text(slug)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                    }
                    if let carries = train.carries {
                        Text(carries)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                            .identified("plan-train-\(train.name)-carries")
                    }
                    Text(words)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: attention ? .medium : .regular))
                        .foregroundStyle(attention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .identified("plan-train-\(train.name)-words")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .navigatorRow(selected: false, keyed: false, leading: 0)
            .background {
                if hovering && run != nil { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
            }
        }
        .buttonStyle(.plain)
        .disabled(run == nil)
        .onHover { hovering = $0 }
        .help(run.map { "Open CI on \($0.host() ?? "GitHub")" } ?? "")
        .accessibilityElement(children: .combine)
        .accessibilityLabel([train.heading, train.carries, words].compactMap { $0 }.joined(separator: ", "))
        .accessibilityAddTraits(run == nil ? [] : .isLink)
        .identified("plan-train-\(train.name)")
    }
}

extension PlanChanges {
    /// Now's rows as one list: each train, then its lanes, then the lanes on
    /// none, so a train that moves or turns red washes as a lane does.
    static func now(_ model: PlanModel, statuses: [String: TaskStatus]) -> [ListChangeRow] {
        model.nowGroups.flatMap { group -> [ListChangeRow] in
            let head = group.train.map { train in
                [ListChangeRow(id: train.id, signature: PlanWords.train(train, ci: model.ci(of: train), now: model.nowMs))]
            } ?? []
            return head + lanes(group.lanes, model, statuses: statuses)
        }
    }
}
