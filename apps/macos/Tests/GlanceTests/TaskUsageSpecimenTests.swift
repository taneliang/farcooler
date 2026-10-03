import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A task's Usage section in each state it can be in (ov-195), rendered in
/// both appearances, written where `FARCOOLER_GLANCE_OUT` says. Not an
/// assertion beyond "it drew" and "it says so".
@MainActor
struct TaskUsageSpecimenTests {
    /// bil-7's spend as the iOS harness has it: a claude turn that stated
    /// only part, and codex nobody can price.
    static let spend = """
        {"task":"t","price_table":"2026-09-25",
         "totals":{"turns":12,"turns_partial":1,"active_ms":10200000,"input_tokens":251000,"output_tokens":27400,
                   "cache_read_tokens":1084000,"cache_write_tokens":62000,"cost_reported_micros":2870000,
                   "unpriced_tokens":283000},
         "by_harness_model":[
           {"harness":"claude","model":"claude-opus-5","totals":{"turns":9,"turns_partial":1,"active_ms":7800000,
             "input_tokens":41000,"output_tokens":18400,"cache_read_tokens":1020000,"cache_write_tokens":62000,
             "cost_reported_micros":2870000}},
           {"harness":"codex","model":"gpt-5.5","totals":{"turns":3,"active_ms":2400000,"input_tokens":210000,
             "output_tokens":9000,"cache_read_tokens":64000,"unpriced_tokens":283000}}]}
        """

    @Test("Write the task usage sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let usage = try TaskUsage.decode(Data(Self.spend.utf8))
        #expect(TaskUsageFormat.detail(usage.rows[0], locale: Locale(identifier: "en_US"))
            == "1.1M tokens · $2.87 partly not reported")
        let key = "task.usage.breakdown.open"
        let was = UserDefaults.standard.object(forKey: key)
        UserDefaults.standard.set(true, forKey: key)
        defer { UserDefaults.standard.set(was, forKey: key) }
        for dark in [false, true] {
            let host = NSHostingView(
                rootView: Specimen(usage: usage)
                    .background(dark ? Color(white: 0.12) : Color.white))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("task-usage-\(dark ? "dark" : "light").png"))
        }
    }
}

private struct Specimen: View {
    let usage: TaskUsage

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            caption("Spend, with its breakdown open")
            TaskUsageView(state: .loaded(usage))
            caption("An older runner")
            TaskUsageView(state: .needsUpdate)
            caption("A read that didn’t come back")
            TaskUsageView(state: .failed)
            caption("Nothing yet")
            TaskUsageView(state: .loaded(try! TaskUsage.decode(Data(
                #"{"task":"t","price_table":"","totals":{},"by_harness_model":[]}"#.utf8))))
            caption("Reading")
            TaskUsageView(state: .loading)
        }
        .padding(20)
        .frame(width: 520, alignment: .leading)
    }

    private func caption(_ text: String) -> some View {
        Text(text.uppercased())
            .font(.system(size: 9, weight: .semibold))
            .foregroundStyle(.tertiary)
    }
}
