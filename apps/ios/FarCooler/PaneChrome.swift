import SwiftUI
import UIKit

// What a pane's surfaces are drawn with: the layout scale, the fills, the
// glass, and the approval card a held ask shows. They lived beside the chat
// pane, which is gone (ov-377); every other screen kept using them.

/// Putting the keyboard away without owning the field that raised it.
///
/// A field's text view is often a `UIViewRepresentable` several layers down,
/// so there is no `FocusState` up there to set false. Asking the responder chain is
/// the framework's own answer to "whatever is focused, stop being focused" and
/// does not require this view to know what that is.
@MainActor
enum KeyboardDismissal {
    static func now() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }
}

/// The steps this screen's layout is allowed to take.
///
/// Written down because it had stopped being a scale. The pane's files held 3,
/// 4, 5, 6, 7, 8, 9, 10, 11, 12 and 14 as gaps and insets, in a tree that
/// argues for every other number it uses — and with eleven values in play
/// nothing is a step above anything, it is just the number that looked right
/// the day the view was written. Four values, and the reason each exists.
///
/// The two surface constants are the phone's, not this screen's:
/// `TerminalKeyRow` already draws at 16 and 10, and a composer floating two
/// points inside a glass bar with corners curving at half the rate is what made
/// surfaces on one screen read as different objects.
enum PaneMetrics {
    /// Inside one control — a glyph and the word next to it.
    static let tight: CGFloat = 4
    /// Between two things in one group. The default; reach for another only
    /// with a reason.
    static let step: CGFloat = 8
    /// A transcript card's inner edge, and the gap between cards.
    static let card: CGFloat = 12
    /// A Radius.large (16) surface's inner edge. Larger than a card's because the
    /// corner is: text run to 12 points of a 16-point curve reads as text
    /// falling out of the rounding.
    static let edge: CGFloat = 16

    /// Every floating surface's corner, and the inset it sits at — the tab
    /// strip's pair and the terminal key row's, shipped there first.
    ///
    /// The inset is deliberately not a step on the scale above. It is where
    /// this platform's floating bars sit, and matching the strip 44 points
    /// below the composer matters more than matching the padding inside it.
    static let surfaceRadius: CGFloat = Radius.large
    static let surfaceInset: CGFloat = 10

    /// The smallest a thing you tap is allowed to be, which is the
    /// guideline's floor and not a preference.
    ///
    /// Almost nothing on this screen cleared it: the approve row was 28, the
    /// composer's chips 22, the queue's actions 16. The pattern throughout is
    /// the one the tab strip shipped — the visible control keeps the size it
    /// had, a 44-point frame goes around it, and a `contentShape` makes the
    /// band between the two live rather than merely occupied.
    static let target: CGFloat = 44

    /// The visible height of a composer chip, under a 44-point target.
    ///
    /// One number for both kinds of chip. The selectors were 3 points of
    /// vertical padding on `.caption` and the overflow was 4 points on a
    /// 15-point glyph, so two capsules sitting side by side in one row ended
    /// up two points different in height — close enough to look like a
    /// mistake and not close enough to look like anything else.
    static let chip: CGFloat = 26
}

/// The fills the transcript draws on, as one scale.
///
/// There were eight hand-mixed opacities across these files — 0.035, 0.04,
/// 0.07 twice, 0.08, 0.10, 0.12, 0.15 — beside `.quaternary` used semantically
/// a few hundred lines away, so half the file was on the platform's hierarchy
/// and half was on numbers. Worse than untidy: the user's bubble and every
/// tool card were the SAME 0.07, directly under a comment saying the bubble
/// has to read as a different speaker at a glance.
///
/// The neutral tiers are the platform's, so they follow the appearance the
/// theme puts this pane in. The tinted ones are not on that hierarchy — they
/// are a color that means something — so they stay explicit, and are named
/// here rather than mixed again at each call site.
enum TranscriptFill {
    /// The reader's own words. One full step above anything the agent draws,
    /// which is what "a different speaker" has to be to survive a glance.
    static let speaker = AnyShapeStyle(.tertiary)
    /// A container around the agent's output: a tool call, a subagent still
    /// working, a gap the transcript is reporting.
    static let container = AnyShapeStyle(.quaternary)
    /// A container that has finished, and a ground drawn inside another
    /// ground — the diff's own bed.
    static let recessed = AnyShapeStyle(.quinary)

    /// An agent is waiting on you. The one thing amber means in this app.
    static let attention = AnyShapeStyle(Color.orange.opacity(0.08))
    static let attentionEdge = AnyShapeStyle(Color.orange.opacity(0.35))
    /// The stronger edge, for the request drawn ON the call it is about.
    static let attentionRing = AnyShapeStyle(Color.orange.opacity(0.45))
    /// Something went wrong. Red, because amber is spoken for.
    static let alarm = AnyShapeStyle(Color.red.opacity(0.12))
}

/// One floating surface's glass, in one place.
///
/// Every floating thing in this app goes through here rather than reaching for
/// `glassEffect` itself, so there is exactly one opinion about what glass is
/// and a shape can never end up backed by a slightly different material than
/// the one beside it.
///
/// No fallback. This app's minimum is iOS 26, so the material-and-hairline
/// approximation that used to sit behind an availability check was dead code
/// pretending to be portability.
struct GlassSurface: ViewModifier {
    var radius: CGFloat = PaneMetrics.surfaceRadius
    /// Whether this surface is a thing you TOUCH, and should react like one.
    ///
    /// iOS 26's glass has a reaction built into it — it scales, bounces and
    /// lights up at the point of contact — and it is off unless a surface asks
    /// for it, because it is a statement about affordance rather than a
    /// finish: glass that flexes under a finger is glass the finger can do
    /// something to. A strip you drag says yes; a panel that merely floats
    /// over a transcript says no, and turning it on there would promise a
    /// control that is not there.
    ///
    /// Not for `Button`s. The standard button styles already carry the same
    /// reaction, and a button drawn on an interactive surface reacts twice.
    var interactive: Bool = false

    func body(content: Content) -> some View {
        content.glassEffect(  // style-exempt: the one place the composer's glass is drawn, with its interactive variant
            interactive ? .regular.interactive() : .regular, in: .rect(cornerRadius: radius))
    }
}

// MARK: - Approval

/// The answers to a permission request — the one place a pane asks
/// something of you rather than reporting something to you. Buttons are
/// full-width and tall on purpose: this is the card a thumb has to hit
/// correctly the first time, on a phone, possibly one-handed.
///
/// Ordered by what the question actually is. ACP hands back a flat list —
/// `allow_once`, `allow_always`, `reject_once`, … — and rendering it flat gave
/// three identical full-width buttons, two of them the same blue, with the
/// longest and loudest being a restatement of the command already shown above.
/// A stack of equal-weight options is not a decision; it is a menu.
///
/// So: the decision is one row, Allow prominent and Reject plain beside it, and
/// everything else — the "always" variants, which are a policy change rather
/// than an answer to this question — sits under it in small type.
///
/// Reject is NOT red. Red is for destructive; declining a command destroys
/// nothing, and spending the alarm color here leaves none for when it matters.
///
/// **The sentence above about full-width and tall was right, and the code
/// under it had drifted.** The decision row said `.controlSize(.small)` and
/// ended in a `Spacer`, so what shipped was two roughly 28-point pills bunched
/// against the left edge of the card — neither full-width nor tall, on the one
/// control in this app where hitting Reject instead of Allow means a command
/// ran that nobody allowed. `.large` now, each half of the row expands to
/// share the width, and 44 is the floor under both.
///
/// **And it had drifted a second time, in the half nobody could see.** That
/// fix put `.frame(maxWidth: .infinity)` on the buttons, which widened the
/// boxes and not the capsules — so what shipped was still two intrinsic pills,
/// merely centered in the halves instead of bunched at the left. Reasoning from
/// the code found neither round of this; a harness drawing the card is where
/// both were finally seen.
struct ApprovalControls: View {
    let options: [PermissionOption]
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.step) {
            HStack(spacing: PaneMetrics.step) {
                // `maxWidth` on the LABEL, not on the button.
                //
                // Seen at last, in the harness: the sentence above about
                // full-width was still not true. A frame around a `Button`
                // whose style draws its own capsule stretches the button's
                // box; the capsule inside it keeps its intrinsic width and
                // sits centered in the space, which is what shipped — two
                // ordinary pills floating in a row twice their width, on the
                // one card where a thumb has to hit the right half the first
                // time. Stretching the label is what stretches the capsule.
                if let allow {
                    Button { onChoose(allow.id) } label: {
                        Text(allow.name).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                }
                if let reject {
                    // NOT a second blue.
                    //
                    // `.bordered` paints its label in the tint, so Reject was
                    // accent text on a translucent gray capsule directly
                    // beside Allow's solid accent fill: two blues, one of them
                    // the hardest thing on the card to read, in the one place
                    // this app cannot afford a misread. Allow carries the
                    // color because it is the answer that lets something
                    // happen; Reject carries the same capsule and the ordinary
                    // label color, which is how a decision reads as two equal
                    // choices rather than as one recommendation and one link.
                    //
                    // Both said, in this order, on purpose. `.tint` is what
                    // reliably reaches a system button style; the foreground
                    // style is what lifts the label off the grayed tint to
                    // full label contrast. If the second one is ever
                    // overridden the button goes gray, not back to blue.
                    Button { onChoose(reject.id) } label: {
                        Text(reject.name).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .tint(.secondary)
                    .foregroundStyle(.primary)
                    .frame(maxWidth: .infinity, minHeight: PaneMetrics.target)
                }
            }
            .controlSize(.large)

            // Small type, full-size targets.
            //
            // These stay quiet deliberately — a policy change is not the
            // answer to the question being asked — but quiet is about weight,
            // not about whether a thumb can land on them. They were 16-point
            // rows of caption text. The band is 44 and the `contentShape` is
            // what makes it live rather than merely occupied, so the spacing
            // between two "always" options belongs to one of them.
            VStack(alignment: .leading, spacing: 0) {
                ForEach(secondary) { option in
                    Button(option.name) { onChoose(option.id) }
                        .buttonStyle(.plain)
                        // In the accent at body-adjacent size: caption text in
                        // gray read as a footnote under the buttons, not as
                        // something you could press. Allow still carries the
                        // one solid fill; these are quiet by weight.
                        .font(.subheadline)
                        .foregroundStyle(Color.accentColor)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(
                            maxWidth: .infinity, minHeight: PaneMetrics.target,
                            alignment: .leading)
                        .contentShape(.rect)
                }
            }
        }
    }

    /// The plain yes: `allow_once` if the adapter offers one, otherwise the
    /// first thing that allows at all.
    private var allow: PermissionOption? {
        options.first { $0.kind.lowercased().contains("once") && isAllow($0) }
            ?? options.first(where: isAllow)
    }

    private var reject: PermissionOption? {
        options.first { isReject($0) }
    }

    /// Everything that is not the straight yes or no — kept, because an adapter
    /// may offer options this client has never heard of and swallowing them
    /// would make an answer unreachable.
    private var secondary: [PermissionOption] {
        options.filter { $0.id != allow?.id && $0.id != reject?.id }
    }

    private func isAllow(_ option: PermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("allow") || kind.contains("accept")
    }

    private func isReject(_ option: PermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("reject") || kind.contains("deny")
    }
}

/// "Needs Your Approval" and the agent's answers, as one card, which
/// `TerminalPermissionBar` draws over a claude TUI pane.
struct ApprovalCard: View {
    let pending: PendingPermission
    /// Why the last answer to this ask did not land, if it did not.
    var failure: String? = nil
    /// An answer is out, so the buttons are off until it lands or fails.
    var sending: Bool = false
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.card) {
            // The one amber left in this file, with the ring on a gated tool
            // call: an agent is waiting on you, which is the only thing amber
            // says anywhere in this app.
            Label("Needs Your Approval", systemImage: "hand.raised.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)

            ApprovalControls(options: pending.options, onChoose: onChoose)
                .disabled(sending)

            if let failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("approval-failure")
            }
        }
        // Radius.large (16), the floating surfaces' corner.
        .padding(PaneMetrics.edge)
        .background(
            TranscriptFill.attention,
            in: .floating)
        .overlay(
            RoundedRectangle.floating
                .strokeBorder(TranscriptFill.attentionEdge))  // style-exempt: the attention card's edge (a state)
    }
}
