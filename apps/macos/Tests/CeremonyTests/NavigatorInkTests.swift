import Foundation
import Testing

/// The navigator's text is on the frosted plane, where the system's
/// secondary and tertiary labels fall under 4.5:1 (ov-289). Every navigator
/// view takes `SidebarInk.secondary` instead, and this reads their sources:
/// `secondaryInkReadsOnThePlane` measures the ink, and this holds the views
/// to it, so a new row drawn in plain `.secondary` goes red here.
struct NavigatorInkTests {
    private static var sources: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler")
    }

    /// The files drawn only in the navigator, whole.
    static let navigatorFiles = [
        "Navigator.swift", "NavigatorSplit.swift", "NavigatorFiltering.swift", "NavigatorVisibility.swift",
        "SidebarViews.swift", "SidebarLayout.swift", "TaskListSection.swift",
        "ProjectTerminalsSection.swift", "BoardWorktreesSection.swift",
        "Components/CompactTaskRow.swift", "Components/CollapsibleSection.swift",
        "Components/SectionHeaderHover.swift",
    ]

    /// TaskBoard.swift also holds the main area's card; only the navigator's
    /// list, from `TaskBoardView` up to `TaskCard`, is on the plane.
    static let navigatorSpans = [("TaskBoard.swift", "struct TaskBoardView", "struct TaskCard")]

    /// A system hierarchical ink used as a style: `.secondary`,
    /// `Color.tertiary` and the like, but not `SidebarInk.secondary` or
    /// `PaneText.secondary` (an identifier before the dot).
    static let plainInk = try! NSRegularExpression(
        pattern: #"(?<![A-Za-z0-9_])(Color|HierarchicalShapeStyle)?\.(secondary|tertiary|quaternary)\b"#)

    /// The plain-ink sites in `text`, as "file:line: code", comments left out.
    static func offenders(in text: String, file: String, firstLine: Int = 1) -> [String] {
        text.components(separatedBy: "\n").enumerated().compactMap { index, line in
            let code = line.range(of: "//").map { String(line[..<$0.lowerBound]) } ?? line
            let range = NSRange(code.startIndex..., in: code)
            guard plainInk.firstMatch(in: code, range: range) != nil else { return nil }
            return "\(file):\(firstLine + index): \(code.trimmingCharacters(in: .whitespaces))"
        }
    }

    @Test("No navigator text uses the system's plain secondary or tertiary ink")
    func navigatorTextUsesSidebarInk() throws {
        var found: [String] = []
        for file in Self.navigatorFiles {
            let text = try String(contentsOf: Self.sources.appendingPathComponent(file), encoding: .utf8)
            found += Self.offenders(in: text, file: file)
        }
        for (file, start, end) in Self.navigatorSpans {
            let text = try String(contentsOf: Self.sources.appendingPathComponent(file), encoding: .utf8)
            let from = try #require(text.range(of: start), "\(start) is gone from \(file)")
            let to = try #require(text.range(of: end, range: from.upperBound..<text.endIndex), "\(end) is gone from \(file)")
            let before = text[..<from.lowerBound].components(separatedBy: "\n").count
            found += Self.offenders(in: String(text[from.lowerBound..<to.lowerBound]), file: file, firstLine: before)
        }
        #expect(found.isEmpty, "Navigator text on plain .secondary or .tertiary:\n\(found.joined(separator: "\n"))")
    }

    /// The matcher itself: it must see each spelling, or the test above
    /// passes on nothing.
    @Test("The matcher sees each spelling of the plain ink and passes SidebarInk")
    func theMatcherSeesThePlainInk() {
        #expect(Self.offenders(in: ".foregroundStyle(.secondary)", file: "f").count == 1)
        #expect(Self.offenders(in: ".foregroundStyle(on ? Color.tertiary : .primary)", file: "f").count == 1)
        #expect(Self.offenders(in: ".foregroundStyle(SidebarInk.secondary)", file: "f").isEmpty)
        #expect(Self.offenders(in: ".font(.system(size: WorkspaceStyle.PaneText.secondary))", file: "f").isEmpty)
        #expect(Self.offenders(in: "// was .secondary", file: "f").isEmpty)
    }
}
