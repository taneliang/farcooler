import SwiftUI

// The queue's row, moved out of `AgentView.swift` (ov-171) to keep that file
// inside its size ceiling. The move changed nothing; the same change then
// added `controls`, which dims the three actions on a runner that can't
// change a queued message and says why.

/// A message written but not yet sent. See the Mac's `QueuedRow`.
struct QueuedRow: View {
    let queued: QueuedPrompt
    let onEdit: (String) -> Void
    let onCancel: () -> Void
    /// Send this one into the turn already running.
    ///
    /// The queue's whole point is that a message you can still see and still
    /// edit beats one already gone — so waiting is the default. But a message
    /// written mid-turn is very often a correction, and a correction is worth
    /// nothing once the wrong thing has been done. This is the escape hatch:
    /// you looked at what you wrote and decided it should interrupt.
    let onSteer: () -> Void
    /// Whether there is an agent on the other end — see `AgentView.hasAgent`.
    ///
    /// All THREE of this row's actions need it, which was checked in the Rust
    /// rather than assumed. `terminal.agent_steer_queued`,
    /// `terminal.agent_edit_queued` and `terminal.agent_cancel_queued` are one
    /// `svc.agents().send(…)` each in `crates/daemon/src/rpc.rs`, and that call
    /// looks the terminal up in the supervisor's writer table and returns
    /// having done nothing when it is not there — the same floor
    /// `terminal.agent_prompt` drops a message onto.
    ///
    /// Remove is the one worth naming, because a queue you can still empty
    /// would be a fair thing to leave live and it is not one. The queue is not
    /// this app's; it lives in the shim, beside the backend, and reaches a
    /// client only as `promptQueue` — which `Transcript` applies WHOLESALE,
    /// never editing the list itself. So with the shim gone the row is a
    /// picture of a queue that no longer exists anywhere: Remove would send
    /// into the same floor, no `promptQueue` would ever answer, and the row it
    /// was meant to delete would still be sitting there.
    let hasAgent: Bool
    /// Whether this runner can change a queued message at all (ov-171).
    ///
    /// Dimmed with a reason, never hidden: `DaemonBuild.can` states the rule,
    /// because the same app showing different controls on two runners with
    /// nothing said about why reads as a bug. (`hasAgent` is the opposite
    /// case and takes the actions away: there the pane itself is gone.)
    let controls: QueueControls

    @State private var editing = false
    @State private var draft = ""

    /// Editing needs somewhere to send the result, so a pane that loses its
    /// agent mid-edit leaves the field rather than keeping a `Save` that
    /// saves nothing. Read instead of `editing` everywhere, because `@State`
    /// set before the session went is still `true`.
    private var isEditing: Bool { editing && hasAgent && controls.isAvailable }

    var body: some View {
        HStack(alignment: .top, spacing: PaneMetrics.step) {
            Spacer(minLength: 40)
            VStack(alignment: .trailing, spacing: PaneMetrics.tight) {
                // `.body` in all three arms. A queued message is the reader's
                // own words waiting to be sent, so it is set at the size the
                // transcript sets them — and this row used to hold `.callout`
                // and `.body` in the SAME row, one step apart, depending on
                // whether what you queued had any words in it.
                if isEditing {
                    TextField("", text: $draft, axis: .vertical)
                        .textFieldStyle(.plain)
                        .font(.body)
                        .frame(minWidth: 140, minHeight: PaneMetrics.target)
                        .onSubmit(commit)
                } else if queued.text.isEmpty && queued.imageCount > 0 {
                    // An image with no words is still a message. Without this
                    // the bubble was empty and read as a dropped attachment.
                    Label(
                        queued.imageCount == 1 ? "1 image" : "\(queued.imageCount) images",
                        systemImage: "photo")
                        .font(.body)
                } else {
                    Text(queued.text).font(.body)
                }

                // A label and three actions, told apart.
                //
                // All four used to be `.caption` in `.secondary` with the
                // buttons set `.plain`, so "Queued", "Send Now", "Edit" and
                // "Remove" were one line of identical gray words — three of
                // which do something, with nothing saying which three.
                //
                // The correction to THAT was a full accent on all three, and
                // it is what the owner photographed: "Send Now", "Edit" and
                // "Remove" side by side in bright blue on a dark card, which
                // is the same failure with the contrast turned up. Three
                // equally loud words say nothing about which one you want,
                // and `FleetList` already states the rule this row broke —
                // accent is spent per screen, not per control.
                //
                // So the weight carries "these do something" — it is what
                // separates them from the `.caption` regular "Queued" beside
                // them — and the color carries "this is the one worth
                // finding", which is `onSteer` and only `onSteer`. Editing a
                // draft and dropping a draft are both things you came here
                // already meaning to do; interrupting a running turn is the
                // one this card exists to offer. See `QueuedActionStyle`.
                //
                // And all three GO when the pane's agent does. Not grayed:
                // graying is what the composer does with Send, and it earns it
                // — the field beside it still holds your draft, so a dead
                // button is the thing keeping your words on screen. Nothing
                // here is holding anything. Three unreadable words in a row
                // are the "gray on gray" complaint in miniature, and they
                // would be saying, at their most legible, exactly what the
                // notice a few points below already says.
                //
                // What replaces them is one true word. "Queued" is a promise
                // about the future — this goes next — and on a pane with no
                // shim it is a promise nothing can keep, so the label says
                // what actually happened to the message instead. See
                // `hasAgent` for why none of the three would have worked.
                // On one line when it fits, stacked when it doesn't: at the
                // larger Dynamic Type sizes the row broke mid-word ("Queu/ed",
                // "Re-/move"), and a label that wraps is a button nobody can
                // read at a glance.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: PaneMetrics.card) { queuedActions(stacked: false) }
                    VStack(alignment: .leading, spacing: 0) { queuedActions(stacked: true) }
                }
                .buttonStyle(QueuedActionStyle())

                if case let .unavailable(sentence) = controls {
                    Text(sentence)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("agent-queue-unavailable")
                }
            }
            .padding(.horizontal, PaneMetrics.edge)
            .padding(.vertical, PaneMetrics.card)
            // Radius.large (16), the corner every surface in `composerStack` draws.
            // The dashed edge is what says "not sent yet"; the rounding was
            // never carrying that and only made this bubble a different object
            // from the composer it is attached to.
            .background {
                RoundedRectangle.floating
                    .fill(.regularMaterial)
                    .overlay {
                        RoundedRectangle.floating
                            .strokeBorder(
                                Color.secondary.opacity(0.4),
                                style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                    }
            }
        }
    }

    /// The state word and the actions, one line each: `stacked` only says
    /// whether the caller may let them be as wide as the card.
    @ViewBuilder
    private func queuedActions(stacked: Bool) -> some View {
        Text(hasAgent ? "Queued" : "Not sent")
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .fixedSize(horizontal: !stacked, vertical: true)
            .accessibilityIdentifier("agent-queued-state")
        if hasAgent {
            Button("Send Now", action: onSteer)
                .disabled(!controls.isAvailable)
                .buttonStyle(QueuedActionStyle(prominent: true))
                .lineLimit(1)
                .fixedSize(horizontal: !stacked, vertical: true)
            Button(isEditing ? "Save" : "Edit") {
                if isEditing {
                    commit()
                } else {
                    draft = queued.text
                    editing = true
                }
            }
            .disabled(!controls.isAvailable)
            .lineLimit(1)
            .fixedSize(horizontal: !stacked, vertical: true)
            Button("Remove", action: onCancel)
                .disabled(!controls.isAvailable)
                .lineLimit(1)
                .fixedSize(horizontal: !stacked, vertical: true)
        }
    }

    private func commit() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        editing = false
        // `hasAgent` asked again here rather than only where the button is
        // drawn, for the reason the composer's `send()` re-asks its own
        // condition: a hardware Return reaches `onSubmit` without passing a
        // button at all.
        guard hasAgent, controls.isAvailable, !trimmed.isEmpty, trimmed != queued.text else { return }
        onEdit(trimmed)
    }
}

/// One of the queue's actions: a word you can tap, drawn as one.
///
/// Semibold, which is what this file already uses for an action standing in
/// prose — the composer's "Retry" — rather than a fifth gray word in a row of
/// gray words. The 44-point band is the hit target a 16-point caption never
/// had, and the `contentShape` is what makes it live: padding around a
/// `Button`'s label is layout only.
///
/// **Weight says "control"; color says "this one".** This style used to paint
/// every label `.tint`, which on a queued card is three accent words in a row
/// — the thing the owner photographed. `prominent` is now what spends the
/// accent, and exactly one of the three asks for it.
///
/// `Color.accentColor` and `Color.secondary` rather than `.tint` and a
/// hierarchical style, for the reason `FleetList` gives about its own `+`: a
/// hierarchical style resolves against whatever foreground is in force, so
/// `.secondary` under a tinted control is a paler accent and not gray at all.
/// A custom `ButtonStyle` does not tint its label, so both of these reach —
/// and if a future edit puts a tint back over this row, it goes gray rather
/// than blue, which is the safe direction to fail in.
struct QueuedActionStyle: ButtonStyle {
    /// Whether this is the one action on the card worth finding by color.
    var prominent = false
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.caption.weight(.semibold))
            .foregroundStyle(prominent ? Color.accentColor : Color.secondary)
            // The press, said by the label rather than by a fill: there is no
            // fill here to press, and a word that does nothing under a thumb
            // is a word nobody is sure they hit.
            .opacity(configuration.isPressed ? 0.55 : (isEnabled ? 1 : 0.4))
            .frame(minHeight: PaneMetrics.target)
            .contentShape(.rect)
    }
}
