// Both platforms, deliberately.
//
// These measure rendered geometry, and the two platforms do not share a body
// font — 13 points against 17 — so a number that holds on a Mac is not evidence
// about a phone. `paragraphsMatchTheirOldSpacing` in particular pins a constant
// whose whole job is to measure the same on both.
#if os(macOS) || os(iOS)
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
import SwiftUI
import Testing
@testable import AgentKit

@MainActor
@Test func wrappedAgentMessageReservesEveryRenderedLine() {
    let table = """
        | Use Case | Choose |
        | --- | --- |
        | Structured, relational data | SQL |
        | User profiles, orders | SQL |
        | Real-time analytics, unstructured logs | NoSQL |
        | Consistency critical (payments) | SQL |
        | High write throughput, flexible schema | NoSQL |
        """
    let conclusion = "**Safe bet:** Start with SQL (PostgreSQL). Switch to NoSQL only when SQL limitations are proven."

    for rowWidth in [260.0, 320.0, 360.0] {
        let height = renderedHeight(
            AgentReplyText(text: table + "\n\n" + conclusion, trailingClearance: 32),
            width: rowWidth)
        let tableHeight = renderedHeight(
            AgentReplyText(text: table, trailingClearance: 32),
            width: rowWidth)
        let paragraphHeight = renderedHeight(
            Text(Markdown.inline(conclusion))
                .font(.body)
                .lineSpacing(3)
                .fixedSize(horizontal: false, vertical: true),
            width: rowWidth - 32)

        // The message must reserve the complete independently rendered
        // paragraph plus its semantic 16pt table-to-paragraph gap. Testing a
        // width range catches the bug where another wrap appears only on a
        // phone or a narrower desktop pane.
        #expect(height - tableHeight >= paragraphHeight + 15)
    }
}

@MainActor
@Test func wrappedTableCellIncreasesTheTableRowHeight() {
    let short = """
        | Use Case | Choose |
        | --- | --- |
        | Structured data | SQL |
        | User profiles | SQL |
        | Analytics | NoSQL |
        | Payments | SQL |
        | High throughput | NoSQL |
        """
    let wrapped = """
        | Use Case | Choose |
        | --- | --- |
        | Structured, relational data | SQL |
        | User profiles, orders | SQL |
        | Real-time analytics, unstructured logs | NoSQL |
        | Consistency critical (payments) | SQL |
        | High write throughput, flexible schema | NoSQL |
        """

    let shortHeight = renderedHeight(AgentReplyText(text: short, trailingClearance: 32), width: 360)
    let wrappedHeight = renderedHeight(AgentReplyText(text: wrapped, trailingClearance: 32), width: 360)

    // Two first-column cells wrap at this width. Together their rows must
    // become at least two body lines taller or the paragraph after the table
    // is placed over the final cell.
    #expect(wrappedHeight - shortHeight >= 30)
}

/// The merge that made a selection span two paragraphs must not have moved
/// them.
///
/// The reference here is the OLD drawing, written out longhand: one `Text` per
/// paragraph in a `VStack`, with the 16-point top pad
/// `MarkdownBlockSpacing.gap(after: .paragraph, before: .paragraph)` gives. If
/// `MarkdownText` ever stops measuring what that measures, this fails.
///
/// One point of tolerance, and it is not slack — it is the measured cost. The
/// separator inside a merged `Text` is a blank line, and a line's height
/// quantizes, so 16.0 is not exactly reachable: across the widths and styles
/// below the merged drawing lands within half a point of the old one per
/// paragraph boundary. Two boundaries, so one point.
@MainActor
@Test func paragraphsMatchTheirOldSpacing() {
    let paragraphs = [
        "The first paragraph runs long enough to wrap at least twice at this width so the measurement is not degenerate.",
        "A second one, shorter.",
        "And a third that also wraps because it keeps going past the end of one line without stopping.",
    ]

    for count in [2, 3] {
        for width in [240.0, 320.0, 360.0, 420.0, 680.0] as [CGFloat] {
            for secondary in [false, true] {
                let taken = Array(paragraphs.prefix(count))
                let merged = renderedHeight(
                    MarkdownText(text: taken.joined(separator: "\n\n"), secondary: secondary),
                    width: width)
                let stacked = renderedHeight(
                    oldParagraphStack(taken, secondary: secondary), width: width)
                #expect(
                    abs(merged - stacked) <= 1,
                    "\(count) paragraphs at \(width)pt, secondary \(secondary): merged \(merged) vs stacked \(stacked)")
            }
        }
    }
}

/// The negative control for the test above, as a test of its own: with no
/// separator at all the paragraphs would run together, and a tolerance loose
/// enough to hide that would be a tolerance that proves nothing.
@MainActor
@Test func aMissingParagraphGapWouldBeVisible() {
    let taken = [
        "The first paragraph runs long enough to wrap at least twice at this width so the measurement is not degenerate.",
        "A second one, shorter.",
    ]
    let stacked = renderedHeight(oldParagraphStack(taken, secondary: false), width: 320)
    let runTogether = renderedHeight(
        Text(taken.joined(separator: "\n"))
            .font(.body)
            .fixedSize(horizontal: false, vertical: true)
            .lineSpacing(3)
            .frame(maxWidth: .infinity, alignment: .leading),
        width: 320)
    #expect(stacked - runTogether > 1)
}

/// The drawing as it was before paragraphs were merged.
@MainActor
private func oldParagraphStack(_ paragraphs: [String], secondary: Bool) -> some View {
    VStack(alignment: .leading, spacing: 0) {
        ForEach(paragraphs.indices, id: \.self) { index in
            Text(Markdown.inline(paragraphs[index]))
                .fixedSize(horizontal: false, vertical: true)
                .lineSpacing(3)
                .padding(
                    .top,
                    index == 0
                        ? 0
                        : MarkdownBlockSpacing.gap(after: .paragraph, before: .paragraph))
        }
    }
    .font(secondary ? .caption : .body)
    .foregroundStyle(secondary ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
    .frame(maxWidth: .infinity, alignment: .leading)
}

@Test func markdownSpacingReflectsReadingRelationships() {
    let withinList = MarkdownBlockSpacing.gap(after: .listItem, before: .listItem)
    let headingToBody = MarkdownBlockSpacing.gap(after: .heading(level: 2), before: .paragraph)
    let paragraphToSection = MarkdownBlockSpacing.gap(after: .paragraph, before: .heading(level: 2))
    let betweenParagraphs = MarkdownBlockSpacing.gap(after: .paragraph, before: .paragraph)

    #expect(withinList == 8)
    #expect(headingToBody == 8)
    #expect(betweenParagraphs == 16)
    #expect(paragraphToSection == 20)
    #expect(paragraphToSection > headingToBody)
}

@Test func everyHeadingLevelHasARealSizeStep() {
    #expect(MarkdownTypeScale.h1 > MarkdownTypeScale.h2)
    #expect(MarkdownTypeScale.h2 > MarkdownTypeScale.h3)
    #if canImport(AppKit)
    #expect(MarkdownTypeScale.h3 > NSFont.preferredFont(forTextStyle: .body).pointSize)
    #else
    #expect(MarkdownTypeScale.h3 > UIFont.preferredFont(forTextStyle: .body).pointSize)
    #endif
}

@MainActor
private func renderedHeight<Content: View>(_ content: Content, width: CGFloat) -> CGFloat {
    let renderer = ImageRenderer(content: content.frame(width: width))
    renderer.proposedSize = ProposedViewSize(width: width, height: nil)
    renderer.scale = 1
    return renderer.cgImage.map { CGFloat($0.height) } ?? 0
}
#endif
