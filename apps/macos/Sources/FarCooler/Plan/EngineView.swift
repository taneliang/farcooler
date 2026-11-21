import AgentKit
import SwiftUI

struct EngineView: View {
    let snapshot: EngineSnapshot
    let observedAt: Date
    let stale: Bool
    let focus: EngineFocus?
    let refresh: () -> Void
    let openTicket: (String) -> Void

    @Environment(\.dynamicTypeSize) private var textSize
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var selection: EngineFocus?
    @State private var disclosures: [String: Bool] = [:]

    private var textScale: CGFloat {
        // macOS keeps SwiftUI’s semantic fonts at their default point sizes.
        switch textSize {
        case .xSmall: 0.85
        case .small: 0.9
        case .medium: 0.95
        case .xLarge: 1.1
        case .xxLarge: 1.2
        case .xxxLarge: 1.3
        case .accessibility1: 1.45
        case .accessibility2: 1.65
        case .accessibility3: 1.85
        case .accessibility4: 2.05
        case .accessibility5: 2.25
        default: 1
        }
    }

    private func font(_ size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default) -> Font {
        .system(size: size * textScale, weight: weight, design: design)
    }

    private var selectedTicket: EngineSnapshot.Ticket? {
        guard case .ticket(let id) = selection else { return nil }
        return snapshot.stations.lazy.flatMap(\.tickets).first { $0.id == id }
            ?? snapshot.landings.lazy.flatMap(\.tickets).first { $0.id == id }
    }

    var body: some View {
        GeometryReader { geometry in
            let wide = geometry.size.width >= 760 && !textSize.isAccessibilitySize
            ScrollViewReader { scroll in
                ScrollView {
                    VStack(alignment: .leading, spacing: Spacing.inset) {
                        heading
                        capacity(wide: wide)
                        if wide {
                            HStack(alignment: .top, spacing: Spacing.inset) {
                                stations(select: { selection = $0 })
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                VStack(alignment: .leading, spacing: Spacing.inset) {
                                    inspector { closeInspector(in: scroll) }
                                    companions
                                }
                                .frame(width: geometry.size.width * 0.40)
                            }
                        } else {
                            inspector { closeInspector(in: scroll) }
                            stations(select: { selection = $0 })
                            companions
                        }
                    }
                    .padding(Spacing.inset)
                    .padding(.bottom, Spacing.section)
                }
                .scrollBounceBehavior(.basedOnSize)
                .scrollEdgeEffectStyle(.soft, for: .bottom)
                .onChange(of: focus, initial: true) { _, focus in selection = focus }
                .task(id: selection) {
                    guard let selection else { return }
                    // Wait for the newly inserted inspector to join the scroll view.
                    await Task.yield()
                    if !Task.isCancelled { scroll.scrollTo(selection.scrollID, anchor: .top) }
                }
                .onChange(of: snapshot.epoch) { _, _ in selection = focus }
            }
        }
        .font(font(13))
        .identified("engine-view")
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text("Engine").font(font(26, weight: .semibold)).accessibilityAddTraits(.isHeader)
            if stale {
                Text("Last updated \(observedAt.formatted(date: .abbreviated, time: .shortened)). The runner couldn’t be reached.")
                    .foregroundStyle(.quietInk)
                Button("Try Again", action: refresh)
            } else {
                Text("Updated at \(observedAt.formatted(date: .omitted, time: .shortened))")
                    .foregroundStyle(.quietInk)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func capacity(wide: Bool) -> some View {
        let count = wide ? 4 : textSize.isAccessibilitySize ? 1 : 2
        let columns = Array(repeating: GridItem(.flexible(), spacing: Spacing.inset, alignment: .topLeading), count: count)
        return card {
            LazyVGrid(columns: columns, alignment: .leading, spacing: Spacing.group) {
                ForEach(snapshot.capacity) { slot in
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        Text(slot.label).font(font(10)).foregroundStyle(.quietInk)
                        if let used = slot.used, let limit = slot.limit {
                            Text("\(used) of \(limit) used").monospacedDigit()
                            if let queued = slot.queued, queued > 0 {
                                Text("\(queued) queued here")
                                    .foregroundStyle(Tint.attention(scheme)).monospacedDigit()
                            }
                        } else {
                            Text("Unavailable").foregroundStyle(.quietInk)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private func stations(select: @escaping (EngineFocus) -> Void) -> some View {
        card {
            sectionTitle("Stations")
            if snapshot.stations.isEmpty {
                quiet("No workflow steps are available.")
            }
            ForEach(snapshot.stations) { station in
                let id = "engine-station-\(station.id)"
                disclosure(station.label, id: id, expanded: true, heading: true) {
                    VStack(alignment: .leading, spacing: Spacing.group) {
                        if station.tickets.isEmpty {
                            quiet("No tickets").frame(maxWidth: .infinity, alignment: .leading)
                        }
                        ForEach(station.tickets) { ticket in
                            ticketRow(ticket) { select(.ticket(ticket.id)) }
                        }
                    }
                }
                .id(id)
            }
        }
    }

    private func ticketRow(_ ticket: EngineSnapshot.Ticket, select: @escaping () -> Void) -> some View {
        let selected = selection == .ticket(ticket.id)
        return Button(action: select) {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                ticketHeading(ticket)
                if let budget = ticket.sendBacks {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .firstTextBaseline) {
                            status(ticket.status, tone: ticket.tone).fixedSize()
                            Spacer(minLength: Spacing.inset)
                            quiet(sendBackCount(budget)).fixedSize()
                        }
                        VStack(alignment: .leading, spacing: Spacing.tight) {
                            status(ticket.status, tone: ticket.tone)
                            quiet(sendBackCount(budget))
                        }
                    }
                } else {
                    status(ticket.status, tone: ticket.tone)
                }
                if let summary = ticket.summary { quiet(summary) }
            }
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(Spacing.group)
            .background {
                if selected { RoundedRectangle.control.fill(Fill.selection(active: true, contrast: contrast)) }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityHint("Opens run details")
        .id(ticket.id)
        .identified("engine-ticket-\(ticket.id)")
    }

    @ViewBuilder private func inspector(close: @escaping () -> Void) -> some View {
        if case .ticket = selection {
            card {
                ViewThatFits(in: .horizontal) {
                    HStack {
                        sectionTitle("Run Details").fixedSize()
                        Spacer()
                        closeButton(close).fixedSize()
                    }
                    VStack(alignment: .leading) {
                        sectionTitle("Run Details")
                        closeButton(close)
                    }
                }
                if let ticket = selectedTicket {
                    VStack(alignment: .leading, spacing: Spacing.group) {
                        ticketHeading(ticket)
                        status(ticket.status, tone: ticket.tone)
                        fact("Step", ticket.step)
                        if let agent = ticket.agent {
                            fact("Selected Agent Model", agent)
                        } else {
                            fact("Agent", "Unavailable")
                        }
                        fact("Review State", ticket.review)
                        if let reason = ticket.reason { fact("Current Reason", reason) }
                        if let failure = ticket.failure {
                            Text("Failure Details").font(font(13, weight: .semibold))
                            output(failure)
                        }
                        fact("Send-Backs", ticket.sendBacks.map(sendBackCount) ?? "Unavailable")
                        if let budget = ticket.sendBacks {
                            ForEach(budget.history) { back in
                                VStack(alignment: .leading, spacing: Spacing.tight) {
                                    Text("\(back.from) → \(back.to)").font(font(12, weight: .medium))
                                    output(back.reason)
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                        if let task = ticket.taskId {
                            Button("Open Ticket") { openTicket(task) }.buttonStyle(.link)
                        }
                    }
                    .textSelection(.enabled)
                } else {
                    quiet("This ticket is no longer in the current workflow.")
                }
            }
            .id(EngineFocus.ticket("").scrollID)
            .identified("engine-inspector")
        }
    }

    private func closeButton(_ close: @escaping () -> Void) -> some View {
        Button(action: close) { Label("Close Details", systemImage: "xmark") }
            .labelStyle(.iconOnly)
            .help("Close Details")
    }

    private func closeInspector(in scroll: ScrollViewProxy) {
        let id = selectedTicket?.id
        selection = nil
        if let id { scroll.scrollTo(id, anchor: .center) }
    }

    private var companions: some View {
        VStack(alignment: .leading, spacing: Spacing.inset) {
            card {
                sectionTitle("Landing Groups")
                if snapshot.landings.isEmpty { quiet("No landing groups") }
                ForEach(snapshot.landings) { group in
                    VStack(alignment: .leading, spacing: Spacing.group) {
                        Text(group.label).font(font(13, weight: .semibold))
                        status(group.status, tone: group.tone)
                        if group.sinceMs > 0 {
                            let date = Date(timeIntervalSince1970: Double(group.sinceMs) / 1_000)
                            quiet(date.formatted(date: .abbreviated, time: .shortened))
                        }
                        if group.tickets.isEmpty {
                            ForEach(group.members) { member in
                                quiet(member.title == member.id ? member.id : "\(member.id) · \(member.title)")
                            }
                        } else {
                            ForEach(group.tickets) { ticket in
                                ticketRow(ticket) { selection = .ticket(ticket.id) }
                            }
                        }
                        if let failure = group.failure { output(failure) }
                        githubCI(group.ci, group: group.id)
                    }
                    .padding(Spacing.group)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background {
                        if selection == .landing(group.id) {
                            RoundedRectangle.control.fill(Fill.selection(active: true, contrast: contrast))
                        }
                    }
                    .id(EngineFocus.landing(group.id).scrollID)
                }
                if case .landing(let id) = selection, !snapshot.landings.contains(where: { $0.id == id }) {
                    quiet("This landing group is no longer available.")
                }
            }
            card {
                sectionTitle("Agents")
                if snapshot.agents.isEmpty { quiet("No agent activity reported") }
                ForEach(snapshot.agents) { agent in
                    Button {
                        selection = .ticket(agent.ticket)
                    } label: {
                        VStack(alignment: .leading, spacing: Spacing.tight) {
                            Text("\(agent.role) · \(agent.ticket)").font(font(13, weight: .medium))
                            status(agent.status, tone: agent.tone)
                            if let model = agent.model { fact("Selected Model", model) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }

    private func githubCI(_ ci: EngineSnapshot.CI?, group: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            fact("GitHub CI", ci?.status ?? "Unavailable")
            if let ci {
                let date = Date(timeIntervalSince1970: Double(ci.fetchedMs) / 1_000)
                Text(ci.sha).font(font(10, design: .monospaced)).foregroundStyle(.quietInk)
                    .textSelection(.enabled)
                quiet("Observed \(date.formatted(date: .abbreviated, time: .shortened))")
                if !ci.jobs.isEmpty {
                    disclosure("Jobs", id: "engine-jobs-\(group)", expanded: false, heading: false) {
                        ForEach(Array(ci.jobs.enumerated()), id: \.offset) { _, job in
                            quiet("\(job.name) · \(job.status)")
                        }
                    }
                    .padding(.top, Spacing.tight)
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func ticketHeading(_ ticket: EngineSnapshot.Ticket) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(ticket.id).font(font(10, design: .monospaced)).foregroundStyle(.quietInk)
            if ticket.title != ticket.id { Text(ticket.title).font(font(13, weight: .semibold)) }
        }
    }

    private func sendBackCount(_ budget: EngineSnapshot.SendBacks) -> String {
        "\(budget.remaining) of \(budget.limit) send-backs left"
    }

    private func fact(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            Text(label).font(font(10, weight: .semibold)).foregroundStyle(.quietInk)
            Text(value)
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
    }

    private func status(_ label: String, tone: EngineSnapshot.Tone) -> some View {
        let tint: AnyShapeStyle = switch tone {
        case .normal: AnyShapeStyle(.quietInk)
        case .waiting: AnyShapeStyle(Tint.attention(scheme))
        case .failed: AnyShapeStyle(Tint.failure)
        }
        return Label(label, systemImage: "diamond.fill")
            .font(font(12, weight: .medium))
            .foregroundStyle(tint)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func quiet(_ text: String) -> some View {
        Text(text).font(font(12)).foregroundStyle(.quietInk).fixedSize(horizontal: false, vertical: true)
    }

    private func output(_ text: String) -> some View {
        Text(text)
            .font(font(12, design: .monospaced))
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(Spacing.group)
            .surface(.inset, in: .control)
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title).font(font(17, weight: .semibold)).accessibilityAddTraits(.isHeader)
    }

    private func disclosure<Content: View>(
        _ title: String, id: String, expanded: Bool, heading: Bool, @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        let binding = Binding(get: { disclosures[id] ?? expanded }, set: { disclosures[id] = $0 })
        var metrics = SectionMetrics.inline
        metrics.isHeading = heading
        return CollapsibleSection(id: id, metrics: metrics, isExpanded: binding, accessibilityLabel: title) { _ in
            Text(title)
                .font(font(heading ? 13 : 12, weight: heading ? .semibold : .regular))
                .fixedSize(horizontal: false, vertical: true)
        } accessory: {
            EmptyView()
        } content: {
            content()
        }
    }

    private func card(@ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: Spacing.inset, content: content)
            .padding(Spacing.inset)
            .frame(maxWidth: .infinity, alignment: .leading)
            .paneCard()
    }
}
