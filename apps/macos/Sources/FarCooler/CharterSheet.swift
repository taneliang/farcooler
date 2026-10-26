import AgentKit
import SwiftUI

/// Show Charter for a runner on another machine (ov-755): the charter's text
/// in a sheet, read only. The charter is the team's rules, and every member
/// reads it; this Mac can't open the file, so the runner sends the text.
struct CharterSheetRequest: Identifiable {
    let host: String
    let workspace: WorkspaceSummary
    var id: String { "\(host)|\(workspace.id)" }
}

struct CharterSheet: View {
    let workspace: WorkspaceSummary
    /// Asks the runner for the text.
    let load: () async -> CharterReading
    @State private var reading: CharterReading?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("\(workspace.name) Charter").font(.headline)
            content
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(minWidth: 320, idealWidth: 560, maxWidth: .infinity, minHeight: 240, idealHeight: 520, maxHeight: .infinity)
        // Escape closes it, as Done does.
        .onExitCommand { dismiss() }
        .task { reading = await load() }
    }

    @ViewBuilder
    private var content: some View {
        switch reading {
        case nil:
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
        case .text(let text)?:
            Self.page(text)
        case .missing?:
            Self.note("No charter yet. The orchestrator writes one when the lead first talks to it.")
        case .failed(let why)?:
            Self.note(why)
        }
    }

    /// The charter, scrolling when it is long, and selectable.
    static func page(_ text: String) -> some View {
        ScrollView {
            Text(text)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
        }
        .surface(.inset, in: .control)
        .accessibilityIdentifier("charter-text")
    }

    static func note(_ sentence: String) -> some View {
        Text(sentence)
            .font(.callout)
            .foregroundStyle(.quietInk)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
