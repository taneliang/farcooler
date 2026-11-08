import Foundation
import Testing
@testable import AgentKit

/// Watching an engine agent: the picture arrives, the end is told apart, and nothing goes back (ov-976).
@MainActor
struct AgentWatchTests {
    final class Recorder: AgentWatchTransport, @unchecked Sendable {
        var calls: [String] = []
        var chunk: (@Sendable ([UInt8]) -> Void)?
        var end: (@Sendable (String?) -> Void)?
        var opens = true
        func startWatch(workspace: String, item: String, onChunk: @escaping @Sendable ([UInt8]) -> Void,
                        onEnd: @escaping @Sendable (String?) -> Void) async -> String? {
            calls.append("start \(workspace) \(item)"); chunk = onChunk; end = onEnd
            return opens ? "watch:\(workspace)/\(item)" : nil
        }
        func stopWatch(_ name: String) async { calls.append("stop \(name)") }
    }
    final class Screen: WatchScreen {
        let columns: Int, rows: Int
        var fed: [UInt8] = []
        init(_ c: Int, _ r: Int) { columns = c; rows = r }
        func feed(_ bytes: [UInt8]) { fed += bytes }
    }
    private func marker(_ c: Int, _ r: Int) -> [UInt8] { Array("\u{1b}P>farcooler-size;\(c);\(r)\u{1b}\\".utf8) }
    private func settle() async { for _ in 0..<20 { await Task.yield() } ; try? await Task.sleep(nanoseconds: 50_000_000) }

    @Test func theMarkerSizesTheScreenAndThePictureFollows() async {
        let rec = Recorder(); var made: Screen?
        let model = AgentWatchModel(workspace: "w", item: "ov-9", transport: rec) { c, r in let s = Screen(c, r); made = s; return s }
        model.start(); await settle()
        let bytes = marker(120, 40) + Array("hello".utf8)
        rec.chunk!(Array(bytes[0..<10])); await settle()
        #expect(model.phase == .connecting)
        rec.chunk!(Array(bytes[10...])); await settle()
        #expect(made?.columns == 120 && made?.rows == 40)
        #expect(made?.fed == Array("hello".utf8))
        #expect(model.phase == .live)
    }

    @Test func finishedLostAndNotRunningAreToldApart() async {
        func run(_ why: String?, picture: Bool) async -> AgentWatchModel.Phase {
            let rec = Recorder()
            let model = AgentWatchModel(workspace: "w", item: "i", transport: rec) { Screen($0, $1) }
            model.start(); await settle()
            if picture { rec.chunk!(marker(80, 24)); await settle() }
            rec.end!(why); await settle()
            return model.phase
        }
        #expect(await run(nil, picture: true) == .finished)
        #expect(await run("lost", picture: true) == .lost)
        #expect(await run(nil, picture: false) == .unavailable(gotPicture: false))
        #expect(AgentWatchModel.status(.unavailable(gotPicture: false)) == "This agent isn’t running right now.")
        #expect(AgentWatchModel.status(.finished) == "Finished")
        #expect(AgentWatchModel.status(.live) == nil)
    }

    @Test func noInputReachesTheAgent() async {
        let rec = Recorder()
        let model = AgentWatchModel(workspace: "w", item: "ov-9", transport: rec) { Screen($0, $1) }
        model.start(); await settle()
        rec.chunk!(marker(80, 24) + Array("\u{1b}[6n".utf8)); await settle()   // a program asking for the cursor position
        model.stop(); await settle()
        #expect(rec.calls == ["start w ov-9", "stop watch:w/ov-9"])
    }

    @Test func aRunnerThatCannotOpenItSaysSo() async {
        let rec = Recorder(); rec.opens = false
        let model = AgentWatchModel(workspace: "w", item: "i", transport: rec) { Screen($0, $1) }
        model.start(); await settle()
        #expect(model.phase == .unavailable(gotPicture: false))
    }

    /// The view and the model hold no way to type: the words for every input path are absent from both files.
    @Test func theWatchFilesMentionNoInputPath() throws {
        let here = URL(fileURLWithPath: #filePath)
        let root = (0..<6).reduce(here) { url, _ in url.deletingLastPathComponent() }
        let files = ["apps/shared/AgentKit/Sources/AgentKit/AgentWatch.swift", "apps/ios/FarCooler/AgentWatchView.swift"]
        for file in files {
            let text = try String(contentsOf: root.appendingPathComponent(file), encoding: .utf8)
                .split(separator: "\n").filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }.joined(separator: "\n")
            for word in ["farcooler_client_call", "core.call", ".call(", "terminal.write", "terminal.resize", "terminal.tell", "onInput", "TextField", "UIKeyInput", "becomeFirstResponder", "paste", "sendKeys", "takeWrites", "encode(key", "encode(paste", "DragGesture", "onTapGesture", "LongPressGesture"] {
                #expect(!text.contains(word), "\(file) mentions \(word)")
            }
        }
    }
}
