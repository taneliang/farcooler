import Foundation
import Testing

/// No `@State` is given a value twice: once where it is declared and again in
/// an `init`.
///
/// Xcode 27 replaced the `@State` property wrapper with a macro, and Apple's
/// note for it (macOS 27 release notes, SwiftUI, 105893279) says that a state
/// with an initial value at its declaration AND an assignment in an
/// initializer has the initializer's value discarded, and that some such
/// cases no longer compile. Measured with Xcode 27.0 (27A266a), the
/// `_name = State(initialValue:)` form still wins today, for internal and
/// private state alike, and only `self.name = value` is dropped. So nothing
/// here is broken yet. Apple's own advice is still to leave the declaration
/// bare when an init supplies the value, and a declaration that says `= 0`
/// while the view opens on 1 misleads whoever reads it.
///
/// A source scan, because the views that do this live in the app targets,
/// which `swift test` cannot build.
struct StateInitializerTests {
    @Test("An @State assigned in an init has no initial value where it is declared")
    func noStateIsInitializedTwice() throws {
        let root = try #require(repositoryRoot(), "cannot find the repository from \(#filePath)")
        var scanned = 0
        var offenders: [String] = []
        let here = URL(fileURLWithPath: #filePath).standardizedFileURL
        for file in swiftFiles(under: root.appendingPathComponent("apps")) {
            // This file's own fixtures are the shape on purpose.
            if file.standardizedFileURL == here { continue }
            guard let source = try? String(contentsOf: file, encoding: .utf8) else { continue }
            scanned += 1
            let path = file.path.replacingOccurrences(of: root.path + "/", with: "")
            offenders += Self.doublyInitialized(in: source).map { "\(path): \($0)" }
        }
        // Proof the scan read the apps, not an empty directory.
        #expect(scanned > 100)
        #expect(offenders.isEmpty, "declare these without `= …`: \(offenders)")
    }

    /// The scanner itself, on the shape it exists to catch and its near misses.
    @Test("The scan finds the doubled shape and nothing else")
    func scannerFindsTheShape() {
        let doubled = """
            struct A: View {
                @State var reveal: CGFloat = 0
                @State private var name = "x"
                init(open: Bool) {
                    _reveal = State(initialValue: open ? 1 : 0)
                    self.name = "y"
                }
            }
            """
        #expect(Self.doublyInitialized(in: doubled) == ["reveal", "name"])

        let bare = """
            struct B: View {
                @State var reveal: CGFloat
                @State private var other = 0
                init(open: Bool) {
                    _reveal = State(initialValue: open ? 1 : 0)
                    let other = 3
                    _ = other == 3
                }
                func later() { other = 2 }
            }
            """
        #expect(Self.doublyInitialized(in: bare).isEmpty)
    }

    // MARK: - The scan

    /// Names of `@State` properties that have an initial value at their
    /// declaration and are also assigned inside some `init` body in `source`.
    static func doublyInitialized(in source: String) -> [String] {
        let text = source as NSString
        let declared = Set(
            captures(
                #"@State\s+(?:(?:private|fileprivate|internal|public)(?:\(set\))?\s+)*var\s+(\w+)\s*(?::[^=\n{]*)?="#,
                in: source))
        guard !declared.isEmpty else { return [] }

        var found: [String] = []
        let initializers = try! NSRegularExpression(pattern: #"(?<![\w.])init\s*\("#)
        for initializer in initializers.matches(in: source, range: NSRange(location: 0, length: text.length)) {
            let paren = initializer.range.location + initializer.range.length - 1
            guard let close = matching(text, from: paren, open: "(", close: ")") else { continue }
            let rest = text.range(of: "{", range: NSRange(location: close, length: text.length - close))
            guard rest.location != NSNotFound,
                let end = matching(text, from: rest.location, open: "{", close: "}")
            else { continue }
            let body = text.substring(with: NSRange(location: rest.location, length: end - rest.location + 1))
            // An assignment, not a comparison, and not a local of the same name.
            let assigned = captures(#"(?<![\w.$])(?<!let )(?<!var )(?:self\.)?_?(\w+)\s*=(?!=)"#, in: body)
            for name in assigned where declared.contains(name) && !found.contains(name) {
                found.append(name)
            }
        }
        return found
    }

    private static func captures(_ pattern: String, in source: String) -> [String] {
        let expression = try! NSRegularExpression(pattern: pattern)
        let text = source as NSString
        return expression.matches(in: source, range: NSRange(location: 0, length: text.length))
            .map { text.substring(with: $0.range(at: 1)) }
    }

    private static func matching(_ text: NSString, from start: Int, open: Character, close: Character) -> Int? {
        let open = open.utf16.first!, close = close.utf16.first!
        var depth = 0
        for index in start..<text.length {
            let unit = text.character(at: index)
            if unit == open { depth += 1 }
            if unit == close {
                depth -= 1
                if depth == 0 { return index }
            }
        }
        return nil
    }

    private func repositoryRoot() -> URL? {
        var directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        while directory.path != "/" {
            let marker = directory.appendingPathComponent("apps/ios/generate-project.py")
            if FileManager.default.fileExists(atPath: marker.path) { return directory }
            directory = directory.deletingLastPathComponent()
        }
        return nil
    }

    private func swiftFiles(under directory: URL) -> [URL] {
        let walker = FileManager.default.enumerator(
            at: directory, includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles])
        var files: [URL] = []
        while let url = walker?.nextObject() as? URL {
            // Build output: DerivedData copies and SwiftPM checkouts.
            if url.lastPathComponent == "build" || url.lastPathComponent == ".build" {
                walker?.skipDescendants()
                continue
            }
            if url.pathExtension == "swift" { files.append(url) }
        }
        return files
    }
}
