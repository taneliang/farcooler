#if canImport(SwiftUI)
import SwiftUI

private struct WorkflowPortKey: EnvironmentKey {
    static let defaultValue: WorkflowPort? = nil
}
extension EnvironmentValues {
    public var workflowPort: WorkflowPort? {
        get { self[WorkflowPortKey.self] }
        set { self[WorkflowPortKey.self] = newValue }
    }
}

// Attached to an existing workspace without replacing its terminal/navigation.
public struct WorkflowRunsButton: View {
    let host: String
    let workspace: String
    @State private var showing = false
    public init(host: String, workspace: String) { self.host = host; self.workspace = workspace }
    public var body: some View {
        Button("Workflow Runs") { showing = true }
            .sheet(isPresented: $showing) { WorkflowRunsView(host: host, workspace: workspace) }
    }
}
public struct WorkflowRunsView: View {
    @Environment(\.workflowPort) private var port
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model: WorkflowRunsModel
    @AppStorage private var selected: String
    public init(host: String, workspace: String) {
        _model = StateObject(wrappedValue: WorkflowRunsModel(host: host, workspace: workspace))
        _selected = AppStorage(wrappedValue: "", "workflow.run.\(host).\(workspace)")
    }
    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if !model.notice.isEmpty { Text(model.notice).accessibilityAddTraits(.updatesFrequently) }
                    ForEach(model.retained) { entry in
                        Button("\(entry.reinspect ? "Reinspect Unsubmitted Action" : "Retry Retained Action") · \(entry.item) · \(entry.label)") { model.clearInspection(); selected = entry.item; Task { await model.retry(entry, port: port) } }
                            .disabled(!model.ready || model.busy)
                    }
                    if !selected.isEmpty {
                        Button("All Runs") { model.clearInspection(); selected = "" }
                    }
                    if let run = model.presentation?.runs.first(where: { $0.item == selected }) {
                        card(run)
                    } else {
                        ForEach(model.presentation?.runs ?? []) { run in
                            Button { model.clearInspection(); selected = run.item } label: {
                                VStack(alignment: .leading) {
                                    Text(run.label); Text(run.item); Text(run.phase_label)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        if model.ready && model.presentation?.runs.isEmpty == true { Text("No workflow runs.") }
                        if model.ready && !selected.isEmpty { Text("This run is no longer available.") }
                    }
                    if !model.inspection.isEmpty { Text(model.inspection).textSelection(.enabled) }
                }
                .padding()
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .navigationTitle("Workflow Runs")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh") { Task { await model.refresh(port) } }.disabled(model.busy)
                }
            }
        }
        .frame(minWidth: 280, minHeight: 300)
        .task(id: port?.id) { model.disconnect(); await model.refresh(port) }
        .onDisappear { model.disconnect() }
        .onChange(of: selected) { _, _ in model.clearInspection() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await model.refresh(port) } }
            else { model.disconnect() }
        }
    }
    private func field(_ title: String, _ value: String?) -> some View {
        Group {
            if let value {
                VStack(alignment: .leading) {
                    Text(title).font(.caption).foregroundStyle(.secondary)
                    Text(value).textSelection(.enabled)
                }
            }
        }
    }
    private func card(_ run: WorkflowRun) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(run.label).font(.title2).accessibilityAddTraits(.isHeader)
            Text(run.phase_label)
            field("Item", run.item); field("Run", run.run)
            field("Definition", run.definition_pin); field("Policy", run.policy_pin)
            field("Candidate", run.candidate); field("Pending Effect", run.pending_effect)
            field("Iteration", String(run.iteration)); field("Attempt", String(run.attempt))
            field("Fence", String(run.fence)); field("Reworks Remaining", String(run.reworks_remaining))
            if let evidence = run.latest_evidence {
                field("Evidence Candidate", evidence.candidate); field("Evidence Manifest", evidence.manifest)
                field("Evidence Receipt", evidence.receipt); field("Evidence Policy", evidence.policy_pin)
            }
            if let stage = run.stage { Text(stage.label).font(.headline); Text(stage.progression_label) }
            if let gate = run.gate { Text(gate.label).font(.headline); Text(gate.text); field("Principal", gate.principal) }
            ForEach(Array(run.stages.enumerated()), id: \.offset) { _, stage in
                Text("\(stage.id) · \(stage.label) · \(stage.progression_label)")
            }
            ForEach(Array(run.reasons.enumerated()), id: \.offset) { _, reason in
                Text(reason.label).font(.headline); Text(reason.text)
            }
            ForEach(Array((run.rework_history ?? []).enumerated()), id: \.offset) { _, history in
                Text("\(history.principal) · \(history.iteration)")
                Text(history.reason).textSelection(.enabled)
            }
            ForEach(Array(run.actions.enumerated()), id: \.offset) { _, action in
                if let form = run.forms?.first(where: { $0.action == action.kind }) {
                    WorkflowReasonForm(form: form, enabled: model.ready && !model.busy && port != nil) { reason in
                        Task { await model.perform(item: run.item, action: action, port: port, reason: reason) }
                    }.id("\(run.item)/\(run.run)/\(action.guardValue)")
                } else {
                    Button(action.label) { Task { await model.perform(item: run.item, action: action, port: port) } }
                        .disabled(!model.ready || model.busy || port == nil)
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}
private struct WorkflowReasonForm: View {
    let form: WorkflowForm
    let enabled: Bool
    let submit: (String) -> Void
    @State private var draft = ""
    var body: some View {
        VStack(alignment: .leading) {
            Text(form.label).font(.headline)
            TextEditor(text: $draft).frame(minHeight: 100).accessibilityLabel(form.label).disabled(!enabled)
            Text(form.help)
            Button(form.submit_label) { submit(draft) }.disabled(!enabled)
        }
    }
}
#endif
