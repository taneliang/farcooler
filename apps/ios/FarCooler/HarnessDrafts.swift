import SwiftUI

/// The UI harnesses' clean start (ov-672): every draft the app keeps across
/// launches, for every pane, is gone before the first frame, so a test never
/// meets what an earlier test (or an earlier run on the same simulator) left
/// in the container. CI's fresh simulators hide the difference; a local run
/// does not.
///
/// One call clears them all, where the harnesses used to clear the panes they
/// knew by name and miss the next one that learned to keep something:
/// - the composers' draft text (`NativeDraftStore`, `PaneDraftStore`) and the
///   new-worktree description (`newworktree.draft`);
/// - the draft photos on disk (`DraftImages`) and the sent messages' image
///   cache (`PromptImageDisk`);
/// - the agent-row caches (`AgentRowCache`), which a run with other flags
///   would otherwise read as its own page.
///
/// `-native-keep-photos` (ov-622) is the opt-out for a test that relaunches
/// to see a draft survive: the text, the draft photos and the image cache
/// stay. The row caches go regardless, as they always did.
enum HarnessDrafts {
    static let keepFlag = "-native-keep-photos"

    /// A pane no harness screen opens, which `-harness-seed-stores` fills so a
    /// test can tell a reset that clears every pane from one that clears the
    /// panes it knows.
    static let secondPane = "harness-second-pane"

    private static var keeps: Bool { CommandLine.arguments.contains(keepFlag) }

    /// Once per launch, before anything restores a draft.
    static func resetAll(defaults: UserDefaults = .standard) {
        if !keeps {
            NativeDraftStore.removeAll(in: defaults)
            PaneDraftStore.clearAll(in: defaults)
            defaults.removeObject(forKey: "newworktree.draft")
            DraftImages.shared.dropEverything()
            PromptImageDisk.shared.files.removeAll()
        }
        AgentRowCache.shared.removeAll()
        if CommandLine.arguments.contains("-harness-seed-stores") { seedSecondPane(defaults) }
    }

    /// A draft, a photo, a sent image and a row cache in `secondPane`.
    private static func seedSecondPane(_ defaults: UserDefaults) {
        NativeDraftStore.write("seeded words", for: secondPane, in: defaults)
        PaneDraftStore.record("seeded words", forPane: secondPane, in: defaults)
        defaults.set("seeded words", forKey: "newworktree.draft")
        DraftImages.shared.keep(OutgoingImage(mime: "image/png", data: Data([0x89, 0x50, 0x4E, 0x47])), terminal: secondPane)
        _ = PromptImageDisk.shared.keep(
            pane: secondPane, row: "turn:1", index: 0, mime: "image/png", original: Data([1, 2, 3]), thumbnail: Data([4, 5, 6]))
        AgentRowCache.shared.keep(AgentRowSnapshot(epoch: 1, rev: 1, moreBefore: false, rows: []), for: "phone-\(secondPane)")
        AgentRowCache.shared.flush()
    }

    /// What is kept for `secondPane`, as a line a test reads.
    static func report(defaults: UserDefaults = .standard) async -> String {
        let photos = await DraftImages.shared.restored(terminal: secondPane).count
        AgentRowCache.shared.flush()
        let parts = [
            "text=\(NativeDraftStore.read(secondPane, in: defaults).isEmpty ? 0 : 1)",
            "pane=\(PaneDraftStore.draft(forPane: secondPane, from: defaults) == nil ? 0 : 1)",
            "worktree=\(defaults.string(forKey: "newworktree.draft") == nil ? 0 : 1)",
            "photos=\(photos)",
            "sent=\(PromptImageDisk.shared.kept(pane: secondPane).count)",
            "rows=\(AgentRowCache.shared.onDisk("phone-\(secondPane)") == nil ? 0 : 1)",
        ]
        return parts.joined(separator: " ")
    }
}

/// `HarnessDrafts.report` as an element the UI tests read (`harness-stores`).
struct HarnessStoresProbe: View {
    @State private var line = ""

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a 1 pt element the UI tests read, nothing drawn
            .allowsHitTesting(false)
            .accessibilityElement()
            .accessibilityIdentifier("harness-stores")
            .accessibilityValue(line)
            .task { line = await HarnessDrafts.report() }
    }
}
