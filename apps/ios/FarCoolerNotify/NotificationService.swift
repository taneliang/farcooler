import UserNotifications
import WidgetKit

/// Keeping the widgets current when the app is not running.
///
/// A widget cannot fetch anything, and the app may not have run for hours. This
/// is the only code that touches the snapshot while the phone is in a pocket:
/// a push arrives, the one agent it is about is folded in, and the timelines
/// are reloaded.
///
/// Everything here is best-effort. The extension has roughly thirty seconds and
/// exactly one obligation — deliver the notification — so a snapshot that
/// cannot be written must not delay or drop the banner. That is why
/// `contentHandler` is called on every path, including the ones that give up.
///
/// It does NOT run for Live Activity pushes. Those are a different push type
/// and go straight to the card; see Task 7's note. The widget is therefore
/// refreshed on state CHANGES, which is exactly when what it says changes.
final class NotificationService: UNNotificationServiceExtension {
    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        // The banner is untouched. This extension exists to update a file, not
        // to rewrite what the notification says — the daemon composed that
        // sentence and it is already right.
        defer { contentHandler(request.content) }

        let info = request.content.userInfo
        guard
            let terminal = info["terminal"] as? String, !terminal.isEmpty,
            let status = info["status"] as? String, !status.isEmpty
        else { return }

        let label = info["label"] as? String ?? ""
        // How the turn ENDED, which `status` cannot say: a turn that died
        // arrives as `done` exactly as a turn that finished does, because both
        // are over and the relay treats them the same. See
        // `push::Notification::failed` for why it is a field rather than a
        // fourth status.
        //
        // Absent for a runner or a relay built before the field, and absent
        // reads as `false` — the behavior this extension always had, which is
        // the right way for an optional field to be missing.
        let failed = info["failed"] as? Bool ?? false
        let now = Date()
        // Nothing on disk yet means nothing has ever polled, and a snapshot
        // built from here is partial by definition — `FleetSnapshot.empty`
        // carries `complete == false`, and `merging` keeps it false.
        let existing = SnapshotStore.read() ?? .empty
        let previous = existing.agents.first { $0.id == terminal }

        // The push carries a status, a name, and how the turn ended. Every
        // other field is kept from what the app last wrote rather than blanked:
        // a card that lost its runner name because a push did not repeat it
        // would be a row that got worse when news arrived.
        let agent = FleetSnapshot.Agent(
            id: terminal,
            label: label.isEmpty ? (previous?.label ?? "") : label,
            machine: previous?.machine ?? "",
            status: status,
            glyph: glyph(for: status, failed: failed),
            headline: previous?.headline ?? "",
            line: request.content.body,
            feed: previous?.feed ?? [],
            rank: rank(for: status),
            // From the push, not from what was on disk. `previous?.turnFailed`
            // is the previous TURN's outcome, and carrying it forward is wrong
            // in both directions: a turn that died reported as fine because the
            // last one was, and a turn that finished reported as failed because
            // the last one died. A `done` push knows which this one is.
            turnFailed: status == "done" ? failed : (previous?.turnFailed ?? false),
            activityChangedAt: now)

        SnapshotStore.write(existing.merging(agent, at: now))
        WidgetCenter.shared.reloadAllTimelines()
    }

    /// Where a pushed agent sorts, in the host's own scale.
    ///
    /// An APPROXIMATION, and the one place in this feature that computes a rung
    /// the daemon has already computed. The rule everywhere else is that a
    /// client never re-derives the ladder, because two derivations are two
    /// answers to "which agent matters most". This extension is the exception it
    /// was written for: `farcooler_core::feed::rank` takes a terminal — its tier
    /// and how long that tier has been true — and all that arrives here is a
    /// status word. The real rank replaces this one the moment the app next
    /// polls.
    ///
    /// So it reproduces the host's arithmetic rather than inventing a scale of
    /// its own, and a pushed agent sorts against a polled one: tiers a whole
    /// `TIER_SPAN` apart in the order `feed::tier` sets — blocked, then done,
    /// then working, then everything else — with age SUBTRACTED inside the tier
    /// so the oldest, most-stuck state sorts first. A push is news that arrived
    /// this second, so its age is zero, which puts it last within its own tier
    /// and never above an agent that has genuinely been waiting.
    ///
    /// `rank: 0` stood here, and 0 is the top of the whole scale: a `done` push
    /// outranked a live `blocked` agent on all six widget families until the
    /// next poll, inverting the exact ordering the ladder exists to provide.
    private func rank(for status: String) -> UInt32 {
        // `TIER_SPAN` in crates/core/src/feed.rs. Written out because there is
        // no wire field carrying it; if it ever changes there, a pushed agent
        // lands in the wrong tier and the fix is here.
        let span: UInt32 = 100_000_000
        let tier: UInt32 =
            switch status {
            case "blocked": 0
            case "done": 1
            case "working": 2
            default: 3
            }
        return tier * span + (span - 1)
    }

    /// The same marks `farcooler_core::feed::glyph` uses.
    ///
    /// Duplicated here rather than derived, because this extension never sees a
    /// terminal — only a status word. Kept to the same five characters, `● ? ✓
    /// ✗ ·`, so a widget refreshed by a push and one refreshed by the app do
    /// not draw the same agent differently.
    ///
    /// `failed` is the fifth of those five, and it needs its own argument
    /// because a turn that died and a turn that finished arrive with the SAME
    /// status: `"done"` is about whether the agent is still going, not about
    /// how it stopped. Drawn from the status alone, `✗` was unreachable — so
    /// `accessoryCircular`, which draws only the glyph and nothing else, put a
    /// `✓` on an agent that had died and left it there until the app next
    /// polled. A widget asserting success for a dead agent is worse than a
    /// widget with nothing on it.
    private func glyph(for status: String, failed: Bool) -> String {
        switch status {
        case "blocked": "?"
        // The same mark a failed command gets on the Mac, for the reading
        // `feed::glyph` gives it: something ended, and it ended badly.
        case "done": failed ? "✗" : "✓"
        case "working": "●"
        default: "·"
        }
    }
}
