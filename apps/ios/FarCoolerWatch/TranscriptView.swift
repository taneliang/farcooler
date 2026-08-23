import SwiftUI

/// What the agent actually said, on a wrist.
///
/// The screen the owner asked for, in their own words: *"often I get up to go
/// to the bathroom and an agent completes answering my question and I want to
/// see what it said."* Everything below follows from that sentence.
///
/// **It is not the fleet's summary, and that is the whole point.** The fleet
/// row and the screen behind this one draw `headline`, `line` and `feed` —
/// three strings the host composed and truncated to fit a widget, which say
/// what the agent is DOING. None of them is a word the agent wrote. This asks
/// the phone for the words.
///
/// **The phone decides how much fits.** `WatchTranscript` carries the reasoning
/// and the numbers; what matters here is that this screen never truncates and
/// never pages. It shows what arrived, and when `complete` is false it says
/// there is more and where the more is.
///
/// **Nothing renders the Markdown.** Agents write fenced code, tables, headings
/// and links, and AgentKit has a `MarkdownView` that draws all of it — for a
/// phone and a Mac. On a 45mm screen a fenced block is a horizontal scroll
/// nobody can steer with a crown, a table is unreadable at any width, and a
/// link goes somewhere this device cannot follow. Plain text with the wrapping
/// left to SwiftUI degrades to a stray backtick; a half-working renderer
/// degrades to a layout somebody has to fight. The stray backtick is the better
/// failure.
///
/// **One round trip, taken when the screen opens.** Nothing polls, for
/// `PermissionView`'s reason: the phone answers this by replaying the agent's
/// stream off the runner, and re-asking on a timer would spend somebody's
/// battery re-reading a conversation to no end. Pulling down asks again, which
/// is what a person will try; the second ask is a delta on the phone's cached
/// fold, so it costs the runner almost nothing.
struct TranscriptView<Client: FleetClient>: View {
    @ObservedObject var client: Client
    let terminal: String

    @State private var phase = Phase.reading

    /// The three things this can be showing, and "nothing said yet" is
    /// deliberately not a fourth.
    ///
    /// An agent that has written nothing is a `.read` with no entries, drawn as
    /// a sentence inside the same list. It is a fact the phone established, the
    /// same way `.permission(nil)` is — and it must not borrow `.failed`'s red,
    /// which would report a silent agent as a broken link.
    private enum Phase {
        case reading
        case read(WatchTranscript)
        case failed(String)
    }

    private var agent: FleetSnapshot.Agent? {
        client.state.snapshot?.agents.first { $0.id == terminal }
    }

    var body: some View {
        Group {
            switch phase {
            case .reading:
                reading
            case let .read(transcript):
                read(transcript)
            case let .failed(reason):
                failed(reason)
            }
        }
        // The agent's own name, not "Transcript". A watch title is four or five
        // characters of usable width beside the time, and which agent this is
        // is the one thing the words below cannot say for themselves.
        .navigationTitle(agent?.label ?? "Said")
        .task { await read() }
    }

    // MARK: - The three states

    private var reading: some View {
        VStack(spacing: 10) {
            // `fixedSize` for `PermissionView.asking`'s reason: a bare
            // `ProgressView` claims every point it is offered and strands the
            // sentence at the bottom of the screen, where it reads as a second
            // unrelated thing rather than as this spinner's label.
            ProgressView()
                .fixedSize()
            // Named, because this can take the ten seconds
            // `WatchLinkHost.replayBudget` allows — the phone has to wake, reach
            // the runner and replay the stream — and a silent spinner for ten
            // seconds is indistinguishable from a screen that has hung.
            Text("Reading from your iPhone…")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    @ViewBuilder private func read(_ transcript: WatchTranscript) -> some View {
        List {
            if transcript.entries.isEmpty {
                // Established, not assumed. The phone replayed the conversation
                // and there was nothing in it — a turn that has not produced a
                // word yet, or one that only ran tools.
                VStack(alignment: .leading, spacing: 4) {
                    Text("Nothing Said Yet")
                        .font(.headline)
                    Text("This agent hasn’t written anything this watch can show.")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            // At the TOP, where the missing words would have been. This list is
            // oldest-first and opens at the bottom, so a reader who scrolls up
            // looking for the beginning arrives here and is told why there
            // isn't one — which is the moment the sentence is worth reading. At
            // the bottom it would sit under the newest message, where it reads
            // as a comment on that message instead.
            if !transcript.complete { Elided() }
            ForEach(Array(transcript.entries.enumerated()), id: \.offset) { _, entry in
                Message(entry: entry)
            }
        }
        // The newest message on screen with no scrolling, and the conversation
        // still in the order it happened. Those two are only compatible this
        // way round: reversing the array would put the answer above the question
        // it answered and every reply above what it replied to.
        .defaultScrollAnchor(.bottom)
        // Asking again WITHOUT blanking the screen first. A refresh that
        // dropped back to the spinner would take away the words somebody is in
        // the middle of reading in order to fetch the words they are in the
        // middle of reading — and if the phone has since walked out of range it
        // would take them away for good and leave an error in their place.
        .refreshable { await read(fromScratch: false) }
    }

    /// We could not read it, which is not the same as there being nothing to
    /// read.
    ///
    /// `PermissionView.failed`'s shape, and deliberately so — two screens that
    /// failed to reach the same phone in the same way should not look like two
    /// different problems. Red, a mark, the phone's own sentence, and a retry.
    private func failed(_ reason: String) -> some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Label("Couldn’t Read It", systemImage: "exclamationmark.triangle.fill")
                        .font(.headline)
                        .foregroundStyle(.red)
                    // The phone's sentence, unedited. `WatchLinkHost.reason`
                    // already turned whatever went wrong into words for a
                    // person; rewording it here would be a second vocabulary
                    // for one failure.
                    Text(reason)
                        .font(.caption2)
                }
            }
            Section {
                Button("Try Again") { Task { await read() } }
                    .disabled(!client.state.canAct)
            }
        }
    }

    // MARK: - Asking

    /// `fromScratch` is what separates opening this screen from refreshing it.
    /// Opening has nothing to show and says so with a spinner; refreshing has
    /// the previous answer on screen and keeps it there until a better one
    /// arrives. A failure on a refresh still replaces it, because a stale
    /// transcript with a silent failure behind it is a screen quietly showing
    /// the past as the present — which is the mistake the whole staleness rule
    /// on the screens behind this one exists to prevent.
    private func read(fromScratch: Bool = true) async {
        if fromScratch { phase = .reading }
        // `attempt`, never `send`: see `FleetClient.attempt`. It refuses an
        // unreachable phone with the transport's own sentence, so this screen
        // renders that as the failure it is rather than sending into nothing.
        switch await client.attempt(.transcript(terminal: terminal)) {
        case let .transcript(transcript):
            phase = .read(transcript)
        case let .failed(reason):
            phase = .failed(reason)
        case .sent, .permission:
            // A phone answering a question with an action's receipt, or with a
            // permission, is a phone speaking a dialect this build does not
            // know — a version skew. Emphatically not an empty transcript:
            // nothing was established about what this agent said.
            phase = .failed(WatchLinkClient.unreadableReply)
        }
    }
}

/// One message, marked with who said it.
///
/// A label above the words rather than a bubble or an alignment. Bubbles need
/// horizontal room to mean anything and a watch has none to give — a
/// right-aligned message on a 45mm screen is the same width as a left-aligned
/// one and reads as a layout bug. A word is unambiguous at any width.
private struct Message: View {
    let entry: WatchTranscriptEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            // Only on the wearer's own messages. The agent's words are the
            // reason this screen exists and they are the default; labeling every
            // one of them "Agent" would spend a line of a small screen on the
            // thing that needs no saying, and by their absence the labels then
            // mark exactly the turns that are not the agent's.
            if entry.isYours {
                Text("You")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            Text(entry.text)
                .font(entry.isYours ? .caption2 : .caption)
                // Never truncated. The list scrolls, so vertical space is what
                // this screen has and what it should spend — the same argument
                // `OptionButton` makes for a permission's options, and it is
                // stronger here: an answer cut off at three lines is precisely
                // the "truncated message that's basically useless" this screen
                // was built to replace.
                .foregroundStyle(entry.isYours ? .secondary : .primary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// That this is not the whole conversation.
///
/// One sentence for all three ways it can be true — the message count was
/// capped, a long message was clipped, or the phone's own copy has a gap in it
/// — because the action is the same for all three and a wrist is the wrong
/// place to explain the difference. See `WatchTranscript.complete`.
///
/// It names the phone rather than saying "truncated", because "there is more"
/// with nowhere to get it is a dead end, and the phone is where the rest
/// genuinely is.
private struct Elided: View {
    var body: some View {
        Text("There’s more of this conversation on your iPhone.")
            .font(.caption2)
            .foregroundStyle(.tertiary)
    }
}

#if DEBUG
    /// A conversation of the shape this screen is for: a question, and the
    /// answer somebody walked away before reading.
    ///
    /// `nonisolated`, like `PreviewFleet`'s permissions and for the identical
    /// reason: a `#Preview`'s body is a `@Sendable` closure, and a main-actor
    /// property read from inside one is an error in the Swift 6 language mode.
    private enum PreviewTranscript {
        nonisolated static let said = WatchTranscript(
            entries: [
                WatchTranscriptEntry(
                    role: "User", text: "Why is the watch showing so many last-seen headers?"),
                WatchTranscriptEntry(
                    role: "Agent",
                    text: "Because `confidence` measured age from `activityChangedAt`, which "
                        + "is when the agent's state BEGAN rather than when we last heard "
                        + "about it. An agent that has been working for ten minutes reads as "
                        + "stale on a snapshot polled a second ago. I've added a per-agent "
                        + "`observedAt`, stamped by the writer on every poll and by `merging` "
                        + "for the one agent a push is about."),
            ],
            complete: true)

        nonisolated static let clipped = WatchTranscript(
            entries: [
                WatchTranscriptEntry(role: "Agent", text: "…and that is the last of the four."),
            ],
            complete: false)
    }

    #Preview("Transcript") {
        NavigationStack {
            TranscriptView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .transcript(PreviewTranscript.said) }),
                terminal: "b")
        }
    }

    #Preview("Transcript, more on the phone") {
        NavigationStack {
            TranscriptView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .transcript(PreviewTranscript.clipped) }),
                terminal: "b")
        }
    }

    #Preview("Transcript, nothing said") {
        NavigationStack {
            TranscriptView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .transcript(WatchTranscript(entries: [], complete: true)) }),
                terminal: "b")
        }
    }

    /// The ten-second wait, held still. `latency` is the only way to look at
    /// this state — with a zero-latency client it exists for one frame.
    #Preview("Transcript, reading") {
        NavigationStack {
            TranscriptView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    latency: .seconds(60),
                    reply: { _ in .transcript(PreviewTranscript.said) }),
                terminal: "b")
        }
    }

    #Preview("Transcript, couldn’t read") {
        NavigationStack {
            TranscriptView(
                client: PreviewFleetClient(
                    .live(PreviewFleet.snapshot),
                    reply: { _ in .failed("Couldn’t read that agent in time. Open it on your iPhone.") }),
                terminal: "b")
        }
    }
#endif
