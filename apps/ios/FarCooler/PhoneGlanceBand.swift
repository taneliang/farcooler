import SwiftUI

// The Plan's glance band on a phone (ov-503, design 2 and 7): what is moving
// and what is next, in two lines, and what needs the owner as the one amber
// mark. The words are AgentKit's `PlanGlanceSentence` (carried by
// `PlanStrip.band`), so the Mac, the phones and the watch read one sentence.
//
//   ● 2 need you
//   NOW   Train 86 waiting for CI · 2 building · 3 in review
//   NEXT  Reps: each person's asks reach the orchestrator, then Onboarding…
//
// Over the orchestrator it is the strip (`WorkspaceCards`): the same two
// lines, one line each, with the count as a badge.

struct PhoneGlanceBand: View {
    let band: PlanGlanceSentence
    let needsYou: Int
    /// The lines the Now and Next sentences may take: the strip over the
    /// orchestrator gives each one; the Plan's list, three.
    var lines = 3
    /// Say Now only: the Plan's sheet at its medium detent, where the Next
    /// line would take the room the first train needs (ov-515).
    var nowOnly = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dynamicTypeSize) private var type
    /// "NEXT" in small capitals, scaled with the type around it.
    @ScaledMetric(relativeTo: .caption) private var labelWidth: CGFloat = 40

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if needsYou > 0 { pill }
            line("Now", band.nowLineUnbroken, quiet: band.now == nil, id: "plan-band-now")
            if !nowOnly { line("Next", band.nextLine, quiet: band.next == nil, id: "plan-band-next") }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(band.spoken(needsYou: needsYou, nowOnly: nowOnly))
        .accessibilityIdentifier("plan-band")
    }

    func line(_ label: String, _ words: String, quiet: Bool, id: String) -> some View {
        // At the accessibility sizes the label sits over its words: beside
        // them, at that size, it left the words a column a letter wide (ov-685).
        let layout = type.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2))
            : AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: 8))
        return layout {
            Text(label.uppercased())
                .font(.caption.weight(.semibold))
                .kerning(0.4)
                .foregroundStyle(.quietInk)
                .frame(minWidth: type.isAccessibilitySize ? nil : labelWidth, alignment: .leading)
            Text(words)
                .font(.subheadline.weight(quiet ? .regular : .medium))
                .foregroundStyle(quiet ? AnyShapeStyle(Color.quietInk) : AnyShapeStyle(.primary))
                .lineLimit(lines)
                .truncationMode(.tail)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(id)
    }

    /// "● 2 need you": the one colored mark, with its words. The dot carries
    /// the color; the words stay the text's own (review 17).
    private var pill: some View {
        HStack(spacing: 6) {
            Circle().fill(Tint.attention(scheme)).frame(width: 7, height: 7).accessibilityHidden(true)
            Text(PlanGlanceSentence.needsYouWords(needsYou)).font(.subheadline.weight(.semibold))
        }
        .accessibilityIdentifier("plan-band-needs-you")
    }
}
