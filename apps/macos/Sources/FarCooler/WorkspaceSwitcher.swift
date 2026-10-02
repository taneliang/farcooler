import AgentKit
import SwiftUI

/// The workspace switcher in the title bar (ov-86): the workspace you're
/// in and its repository, "Billing ▾ · shop", and a popover of every
/// workspace grouped by repository, each with what's waiting in it and its
/// ⌘-number, with New Workspace…, Needs You and the runners at its foot.
///
/// It's what the sidebar was for, so the sidebar can stay hidden: the
/// repositories are its groups, the workspaces its rows.
struct WorkspaceSwitcher: View {
    let groups: [WorkspaceNumbers.Group]
    /// The workspace on screen, and its repository's name, or nil.
    let current: (host: String, workspace: String)?
    let title: String
    let repository: String
    /// What's waiting in a workspace, for its dot and count.
    let waiting: (WorkspaceNumbers.Place) -> Int
    /// Whether a group names its runner: only on a fleet of more than one.
    let showsHosts: Bool
    let needsYou: Int
    let onGo: (ContentView.Selection) -> Void
    let onNeedsYou: () -> Void
    /// New Workspace…, or nil where no runner has workspaces.
    let onNewWorkspace: (() -> Void)?
    let onRunners: () -> Void

    @State private var open = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button {
            open.toggle()
        } label: {
            HStack(spacing: 4) {
                Image(systemName: WorkspaceRow.glyph)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.secondary)
                if !repository.isEmpty {
                    Text("· \(repository)")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Switch Workspace")
        .accessibilityLabel("Workspace, \(title)")
        .accessibilityIdentifier("workspace-switcher")
        .popover(isPresented: $open, arrowEdge: .bottom) {
            list
                .frame(width: 300)
        }
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
                        Text(showsHosts && !group.host.isEmpty ? "\(group.repository) · \(group.host)" : group.repository)
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 12)
                            .padding(.top, 8)
                            .padding(.bottom, 2)
                        ForEach(group.places, id: \.workspace.id) { place in
                            row(place)
                        }
                    }
                    if groups.isEmpty {
                        Text("No workspaces yet")
                            .foregroundStyle(.secondary)
                            .padding(12)
                    }
                }
                .padding(.vertical, 6)
            }
            .frame(maxHeight: 420)
            .fixedSize(horizontal: false, vertical: true)
            Divider()
            VStack(alignment: .leading, spacing: 0) {
                footer("Needs You", systemImage: needsYou > 0 ? "tray.full" : "tray", count: needsYou) {
                    open = false
                    onNeedsYou()
                }
                if let onNewWorkspace {
                    footer("New Workspace…", systemImage: "plus") {
                        open = false
                        onNewWorkspace()
                    }
                }
                footer("Runners and Devices…", systemImage: "server.rack") {
                    open = false
                    onRunners()
                }
            }
            .padding(.vertical, 6)
        }
    }

    private func row(_ place: WorkspaceNumbers.Place) -> some View {
        let here = current.map { $0.host == place.host && $0.workspace == place.workspace.id } ?? false
        let count = waiting(place)
        return Button {
            open = false
            onGo(place.selection)
        } label: {
            HStack(spacing: 8) {
                Circle()
                    .fill(count > 0 ? GlancePalette.amber(scheme) : Color.clear)
                    .frame(width: 7, height: 7)
                Text(place.name)
                    .fontWeight(here ? .semibold : .regular)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if count > 0 {
                    Text("\(count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(GlancePalette.amber(scheme))
                }
                if let number = place.number {
                    Text("⌘\(number)")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                        .monospacedDigit()
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 24)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(here ? Color.accentColor.opacity(0.15) : .clear)
                    .padding(.horizontal, 4))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(count > 0 ? "\(place.name), \(count) waiting" : place.name)
        .accessibilityAddTraits(here ? .isSelected : [])
    }

    private func footer(
        _ title: String, systemImage: String, count: Int = 0, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Image(systemName: systemImage)
                    .frame(width: 16)
                    .foregroundStyle(count > 0 ? GlancePalette.amber(scheme) : .secondary)
                Text(title)
                Spacer(minLength: 8)
                if count > 0 {
                    Text("\(count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(GlancePalette.amber(scheme))
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 24)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// Needs You in the title bar (ov-86): the tray, with its count in amber
/// while anything is waiting. The sidebar's Needs You row, for a window
/// without the sidebar.
struct NeedsYouToolbarButton: View {
    let count: Int
    let selected: Bool
    let onSelect: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: onSelect) {
            HStack(spacing: 3) {
                Image(systemName: count > 0 ? "tray.full" : "tray")
                    .symbolVariant(selected ? .fill : .none)
                if count > 0 {
                    Text("\(count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                }
            }
            .foregroundStyle(count > 0 ? GlancePalette.amber(scheme) : Color.secondary)
        }
        .help(count == 0 ? "Needs You" : count == 1 ? "Needs You: 1 item" : "Needs You: \(count) items")
        .accessibilityLabel(count == 1 ? "Needs You, 1 item" : "Needs You, \(count) items")
        .accessibilityIdentifier("toolbar-needs-you")
    }
}
