import FarCoolerVT
import SwiftUI

/// The emulator behind a watch. Bytes go in; the program's replies are never read.
@MainActor
final class VTWatchScreen: WatchScreen {
    private let core: VTCore
    init(columns: Int, rows: Int) { core = VTCore(columns: columns, rows: rows) }
    func feed(_ bytes: [UInt8]) { core.feed(bytes) }
    func grid() -> TerminalGrid? { core.withSnapshot { TerminalGrid(snapshot: $0) } }
}

/// The phone's door onto `workflow.watch`: start and stop, and nothing that types.
struct ClientCoreWatch: AgentWatchTransport {
    let core: ClientCore
    func startWatch(workspace: String, item: String, onChunk: @escaping @Sendable ([UInt8]) -> Void,
                    onEnd: @escaping @Sendable (String?) -> Void) async -> String? {
        await core.startWatch(workspace: workspace, item: item, onChunk: onChunk, onEnd: onEnd)
    }
    func stopWatch(_ name: String) async { await core.stopStream(name) }
}

/// A running engine agent's terminal, to look at and nothing more (ov-976).
///
/// A drawing and a Close button. No keyboard, no gestures, no size report: the picture scales to fit
/// the width and that is all it does.
struct AgentWatchView: View {
    let workspace: String
    let item: String
    @StateObject private var model: AgentWatchModel
    @Environment(\.dismiss) private var dismiss

    init(workspace: String, item: String, transport: any AgentWatchTransport) {
        self.workspace = workspace; self.item = item
        _model = StateObject(wrappedValue: AgentWatchModel(workspace: workspace, item: item, transport: transport) {
            VTWatchScreen(columns: $0, rows: $1)
        })
    }

    var body: some View {
        NavigationStack {
            Group {
                if let screen = model.screen as? VTWatchScreen, let grid = screen.grid() {
                    Canvas { context, size in
                        TerminalRenderer(fontChoice: .iosevka, fontSize: 13).draw(grid: grid, into: context, size: size)
                    }
                    .id(model.revision)
                    .accessibilityElement()
                    .accessibilityIdentifier("agent-watch-surface")
                    .accessibilityLabel("Agent terminal for \(item), read-only")
                } else {
                    Text(AgentWatchModel.status(model.phase) ?? "Connecting…")
                        .foregroundStyle(.secondary).padding()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .background(TerminalPalette.background)
            .navigationTitle("Watching \(item)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Text(["Read-only", model.screen == nil ? nil : AgentWatchModel.status(model.phase)].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .onAppear { model.start() }
        .onDisappear { model.stop() }
    }
}
