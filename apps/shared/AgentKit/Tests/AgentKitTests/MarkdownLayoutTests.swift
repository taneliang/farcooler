#if os(macOS)
import AppKit
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
    #expect(MarkdownTypeScale.h3 > NSFont.preferredFont(forTextStyle: .body).pointSize)
}

@MainActor
private func renderedHeight<Content: View>(_ content: Content, width: CGFloat) -> CGFloat {
    let renderer = ImageRenderer(content: content.frame(width: width))
    renderer.proposedSize = ProposedViewSize(width: width, height: nil)
    renderer.scale = 1
    return renderer.cgImage.map { CGFloat($0.height) } ?? 0
}
#endif
