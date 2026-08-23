package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The generated-file rule, and the mechanism that keeps it from drifting.
 *
 * Every case below is a port of `GeneratedFilesTests.swift` in
 * `apps/shared/AgentKit/Tests/AgentKitTests`, case for case, including the three
 * blind spots that file pins deliberately. That much is table stakes: it proves
 * this copy behaves like that one on the inputs somebody thought of.
 *
 * **What it does not prove is that the two LISTS are the same**, and that is the
 * failure that actually happens. `92058f4` hoisted this rule into AgentKit
 * precisely because two clients each held a copy and the copies drifted; Android
 * cannot import AgentKit, so it is a third copy and the same hazard is back. A
 * ported test suite is no defense at all — add `bun.lock` to the Swift set and
 * every test here still passes.
 *
 * So [theTwoListsAreTheSame] reads the Swift. It finds
 * `GeneratedFiles.swift` in this checkout, parses the two literals out of it,
 * and asserts set equality with the two in `GeneratedFile`. Adding a name on
 * either side without the other fails on the next `testInstrumentedUnitTest`.
 *
 * It fails loudly rather than skipping when the file cannot be found, which is
 * the deliberate half of the trade: a test that quietly passes when it cannot
 * check anything is the same as no test, and this is one repository — the
 * Android build already reads `../../scripts/version.sh` out of it.
 */
class GeneratedFilesTest {
    // ---- what it matches ----

    @Test
    fun `a whole name matches at any depth`() {
        assertTrue(GeneratedFile.isGenerated("Cargo.lock"))
        assertTrue(GeneratedFile.isGenerated("crates/daemon/Cargo.lock"))
        assertTrue(GeneratedFile.isGenerated("apps/ios/FarCooler.xcodeproj/project.pbxproj"))
        assertTrue(GeneratedFile.isGenerated("go.sum"))
        assertTrue(GeneratedFile.isGenerated("apps/macos/Package.resolved"))
    }

    @Test
    fun `a suffix matches the families whose stem varies`() {
        assertTrue(GeneratedFile.isGenerated("web/src/schema.generated.ts"))
        assertTrue(GeneratedFile.isGenerated("Sources/Api.generated.swift"))
        assertTrue(GeneratedFile.isGenerated("proto/farcooler.pb.go"))
        assertTrue(GeneratedFile.isGenerated("crates/proto/src/farcooler.pb.rs"))
        assertTrue(GeneratedFile.isGenerated("tools/farcooler_pb2.py"))
        assertTrue(GeneratedFile.isGenerated("lib/models.g.dart"))
    }

    /**
     * The conservatism the doc comment claims. A rule that folded these would be
     * demoting files people wrote, which is the failure it is written to avoid —
     * so each of these is a near miss on purpose.
     */
    @Test
    fun `hand-written files a looser rule would have caught`() {
        assertFalse(GeneratedFile.isGenerated("Cargo.toml"))
        assertFalse(GeneratedFile.isGenerated("package.json"))
        // A `.lock` suffix is not the rule; the whole names are.
        assertFalse(GeneratedFile.isGenerated("src/my-yarn.lock"))
        assertFalse(GeneratedFile.isGenerated("crates/core/src/lock.rs"))
        // `.generated.ts` is the suffix, not `generated` anywhere in the name.
        assertFalse(GeneratedFile.isGenerated("web/src/generated.ts"))
        assertFalse(GeneratedFile.isGenerated("vendor/leftpad/index.js"))
        assertFalse(GeneratedFile.isGenerated(""))
    }

    // ---- what only the host will get right ----

    /**
     * Not assertions that the behavior is correct — assertions that it is what it
     * is, so the day `linguist-generated` arrives on the wire these three fail and
     * name themselves as the reason the rule can be demoted.
     */
    @Test
    fun `the three blind spots a host rule would not have`() {
        // 1. A repository that marks its own generated files is telling us the
        //    answer, and nothing here reads `.gitattributes`.
        assertFalse(GeneratedFile.isGenerated("src/api.ts"))
        // 2. Only the last component is examined, so a directory of generated
        //    code reads as hand-written.
        assertFalse(GeneratedFile.isGenerated("src/generated/api.ts"))
        // 3. Matching is exact, because git paths are exact bytes.
        assertFalse(GeneratedFile.isGenerated("cargo.lock"))
        assertFalse(GeneratedFile.isGenerated("PACKAGE-LOCK.JSON"))
    }

    // ---- the order Next walks ----

    /**
     * The guarantee that makes this safe to drop into a screen that has movement
     * arithmetic around it: a comparison with nothing generated in it comes back
     * exactly as it went in.
     */
    @Test
    fun `nothing generated means the list is untouched`() {
        val files = listOf("a.swift", "b.rs", "c.kt", "Cargo.toml")
        assertEquals(files, GeneratedFile.reviewOrder(files) { it })
        assertTrue(GeneratedFile.reviewOrder(emptyList<String>()) { it }.isEmpty())
    }

    @Test
    fun `generated files go last and the daemon's order survives inside each group`() {
        val files = listOf(
            "Cargo.lock",
            "crates/daemon/src/a.rs",
            "apps/ios/FarCooler.xcodeproj/project.pbxproj",
            "crates/daemon/src/b.rs",
            "go.sum",
        )
        assertEquals(
            listOf(
                "crates/daemon/src/a.rs",
                "crates/daemon/src/b.rs",
                "Cargo.lock",
                "apps/ios/FarCooler.xcodeproj/project.pbxproj",
                "go.sum",
            ),
            GeneratedFile.reviewOrder(files) { it },
        )
    }

    @Test
    fun `everything generated is still every file in order`() {
        val files = listOf("go.sum", "Cargo.lock")
        assertEquals(files, GeneratedFile.reviewOrder(files) { it })
    }

    /**
     * Generic over the caller's row type, because there is no shared changed-file
     * type to be generic over — the accessor is the whole seam.
     */
    @Test
    fun `the path accessor is what is consulted, not the row`() {
        data class Row(val path: String, val label: String)

        val rows = listOf(Row("Cargo.lock", "first"), Row("src/main.rs", "second"))
        assertEquals(
            listOf("second", "first"),
            GeneratedFile.reviewOrder(rows) { it.path }.map { it.label },
        )
    }

    // ---- the drift ----

    /**
     * The two lists, read out of the Swift this one was copied from.
     *
     * Parsing rather than importing, because there is nothing to import. The
     * literals in `GeneratedFiles.swift` are one `Set<String>` and one
     * `[String]` of plain double-quoted strings with `//` comments between them,
     * which is a small enough grammar to read with a regex and a brace count —
     * and if that ever stops being true the parse fails loudly, which is the
     * right outcome for a file that has changed shape.
     */
    @Test
    fun theTwoListsAreTheSame() {
        val swift = agentKitSource()
        assertEquals(
            "GeneratedFile.GENERATED_NAMES has drifted from AgentKit's generatedNames. " +
                "Both lists decide what a lockfile is; two answers means the phone and the " +
                "Mac disagree about what a branch changed.",
            literals(swift, "generatedNames"),
            GeneratedFile.GENERATED_NAMES,
        )
        assertEquals(
            "GeneratedFile.GENERATED_SUFFIXES has drifted from AgentKit's generatedSuffixes.",
            literals(swift, "generatedSuffixes"),
            GeneratedFile.GENERATED_SUFFIXES.toSet(),
        )
    }

    /**
     * The literal that follows `private static let <name>` in [source], as the
     * set of double-quoted strings up to the closing bracket.
     */
    private fun literals(source: String, name: String): Set<String> {
        val start = source.indexOf("let $name")
        assertTrue("$name is not declared in GeneratedFiles.swift any more", start >= 0)
        // After the `=`, not after the name: the declarations are
        // `let generatedSuffixes: [String] = [ … ]`, and the first bracket is
        // the type annotation.
        val assign = source.indexOf('=', start)
        assertTrue("$name is no longer an assignment", assign >= 0)
        val open = source.indexOf('[', assign)
        assertTrue("$name is no longer a bracketed literal", open >= 0)
        var depth = 0
        var end = -1
        for (index in open until source.length) {
            when (source[index]) {
                '[' -> depth += 1
                ']' -> {
                    depth -= 1
                    if (depth == 0) { end = index; break }
                }
            }
        }
        assertTrue("$name's literal is unterminated", end > open)
        val body = source.substring(open + 1, end)
            // Comments inside the literal are prose about the entries, and this
            // repository's happens to contain an apostrophe and a filename.
            .lines().joinToString("\n") { it.substringBefore("//") }
        return Regex("\"([^\"]*)\"").findAll(body).map { it.groupValues[1] }.toSet()
    }

    /**
     * `apps/shared/AgentKit/Sources/AgentKit/GeneratedFiles.swift`, found by
     * walking up from wherever Gradle runs a unit test.
     */
    private fun agentKitSource(): String {
        val relative = "apps/shared/AgentKit/Sources/AgentKit/GeneratedFiles.swift"
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError(
            "Could not find $relative above ${System.getProperty("user.dir")}. This test is " +
                "the only thing holding GeneratedFile to AgentKit's list; if the Swift has " +
                "moved, point this at where it went rather than deleting the check."
        )
    }
}
