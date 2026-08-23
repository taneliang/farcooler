import Foundation
import Testing

@testable import AgentKit

/// The generated-file rule, which has never had a test in either app.
///
/// It was written on the phone and read by one screen, so what it matched was
/// whatever the last person to add a name to the list believed. It decides two
/// visible things now — the headline counts on both clients, and the order Next
/// walks on the Mac — and it is a stopgap for a host rule that does not exist
/// yet, which makes its blind spots worth writing down rather than discovering.
struct GeneratedFilesTests {
    // MARK: - What it matches

    @Test("A whole name matches at any depth")
    func namesAtAnyDepth() {
        #expect(GeneratedFile.isGenerated("Cargo.lock"))
        #expect(GeneratedFile.isGenerated("crates/daemon/Cargo.lock"))
        #expect(GeneratedFile.isGenerated("apps/ios/FarCooler.xcodeproj/project.pbxproj"))
        #expect(GeneratedFile.isGenerated("go.sum"))
        #expect(GeneratedFile.isGenerated("apps/macos/Package.resolved"))
    }

    @Test("A suffix matches the families whose stem varies")
    func suffixes() {
        #expect(GeneratedFile.isGenerated("web/src/schema.generated.ts"))
        #expect(GeneratedFile.isGenerated("Sources/Api.generated.swift"))
        #expect(GeneratedFile.isGenerated("proto/farcooler.pb.go"))
        #expect(GeneratedFile.isGenerated("crates/proto/src/farcooler.pb.rs"))
        #expect(GeneratedFile.isGenerated("tools/farcooler_pb2.py"))
        #expect(GeneratedFile.isGenerated("lib/models.g.dart"))
    }

    /// The conservatism the doc comment claims. A rule that folded these would
    /// be demoting files people wrote, which is the failure it is written to
    /// avoid — so each of these is a near miss on purpose.
    @Test("Hand-written files a looser rule would have caught")
    func handWritten() {
        #expect(!GeneratedFile.isGenerated("Cargo.toml"))
        #expect(!GeneratedFile.isGenerated("package.json"))
        // A `.lock` suffix is not the rule; the whole names are.
        #expect(!GeneratedFile.isGenerated("src/my-yarn.lock"))
        #expect(!GeneratedFile.isGenerated("crates/core/src/lock.rs"))
        // `.generated.ts` is the suffix, not `generated` anywhere in the name.
        #expect(!GeneratedFile.isGenerated("web/src/generated.ts"))
        #expect(!GeneratedFile.isGenerated("vendor/leftpad/index.js"))
        #expect(!GeneratedFile.isGenerated(""))
    }

    // MARK: - What only the host will get right

    /// Not assertions that the behavior is correct — assertions that it is what
    /// it is, so the day `linguist-generated` arrives on the wire these three
    /// fail and name themselves as the reason the rule can be demoted.
    @Test("The three blind spots a host rule would not have")
    func blindSpots() {
        // 1. A repository that marks its own generated files is telling us the
        //    answer, and nothing here reads `.gitattributes`.
        #expect(!GeneratedFile.isGenerated("src/api.ts"))
        // 2. Only the last component is examined, so a directory of generated
        //    code reads as hand-written.
        #expect(!GeneratedFile.isGenerated("src/generated/api.ts"))
        // 3. Matching is exact, because git paths are exact bytes.
        #expect(!GeneratedFile.isGenerated("cargo.lock"))
        #expect(!GeneratedFile.isGenerated("PACKAGE-LOCK.JSON"))
    }

    // MARK: - The order Next walks

    /// The guarantee that makes this safe to drop into a pane that already
    /// had movement arithmetic around it: a comparison with nothing generated
    /// in it comes back exactly as it went in.
    @Test("Nothing generated means the list is untouched")
    func identity() {
        let files = ["a.swift", "b.rs", "c.kt", "Cargo.toml"]
        #expect(GeneratedFile.reviewOrder(files, path: { $0 }) == files)
        #expect(GeneratedFile.reviewOrder([String](), path: { $0 }).isEmpty)
    }

    @Test("Generated files go last, and the daemon's order survives inside each group")
    func partition() {
        let files = [
            "Cargo.lock",
            "crates/daemon/src/a.rs",
            "apps/ios/FarCooler.xcodeproj/project.pbxproj",
            "crates/daemon/src/b.rs",
            "go.sum",
        ]
        #expect(
            GeneratedFile.reviewOrder(files, path: { $0 }) == [
                "crates/daemon/src/a.rs",
                "crates/daemon/src/b.rs",
                "Cargo.lock",
                "apps/ios/FarCooler.xcodeproj/project.pbxproj",
                "go.sum",
            ])
    }

    @Test("Everything generated is still every file, in order")
    func allGenerated() {
        let files = ["go.sum", "Cargo.lock"]
        #expect(GeneratedFile.reviewOrder(files, path: { $0 }) == files)
    }

    /// Generic over the caller's row type, because there is no shared
    /// `ChangedFile` to be generic over — the accessor is the whole seam.
    @Test("The path accessor is what is consulted, not the row")
    func accessor() {
        struct Row: Equatable {
            let path: String
            let label: String
        }
        let rows = [
            Row(path: "Cargo.lock", label: "first"),
            Row(path: "src/main.rs", label: "second"),
        ]
        #expect(GeneratedFile.reviewOrder(rows, path: \.path).map(\.label) == ["second", "first"])
    }
}
