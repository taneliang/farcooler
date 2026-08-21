#if os(iOS)
    import AppIntents
    import Foundation
    import UIKit

    /// "iPhone" or "iPad" — the object in the reader’s hand, named, because
    /// "device" is what a protocol calls it.
    ///
    /// In AgentKit rather than in the app because this file is compiled into
    /// TWO binaries and both write sentences about the device: the app, through
    /// `WatchLinkHost`, and the widget extension, through the fallback in
    /// `AnswerPermissionDelivery` below, which is the one that runs when a card
    /// is tapped somewhere that cannot deliver it. The extension can see
    /// nothing under `apps/ios/FarCooler/`.
    ///
    /// **`AddView.deviceKind` forwards to this rather than checking the idiom
    /// again.** One implementation, so the add flow and a lock screen card
    /// cannot come to call the same object two things. That is the bug class
    /// the three copies of "Its last turn didn’t finish" were unified for:
    /// one person is on the receiving end of both surfaces, and a disagreement
    /// between them is one you only see by being there.
    ///
    /// `@MainActor` because `UIDevice` is — the SDK marks the class
    /// `NS_SWIFT_UI_ACTOR`, so this is not a choice made here. Every sentence
    /// that needs it is written on the main actor already, with one exception
    /// named where it is: `WatchLinkHost`’s `WCSessionDelegate` methods, which
    /// are `nonisolated` and speak only to a watch.
    @MainActor
    public enum DeviceKind {
        public static var current: String {
            UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        }
    }

    /// Answering one permission with one of the answers the agent offered,
    /// from a surface that cannot reach a runner.
    ///
    /// The first `AppIntent` in this codebase, and it exists because the Live
    /// Activity fires **precisely when an agent blocks** and until now could
    /// only offer a link into the app. Unblocking an agent is the highest-value
    /// thing any glance surface could do and the one closest to it could not.
    ///
    /// **It carries an option, never a verb.** The parameters are the
    /// permission's own id and the id of one of ITS options, because the daemon
    /// declines to invent an Allow/Deny vocabulary and so does everything
    /// downstream of it — see `WatchPermission.options` and `ApprovalControls`.
    /// `optionName` rides along for one reason and it is not display on this
    /// side: the surface has to be able to say WHICH of several buttons landed,
    /// minutes later, when the permission it was about is gone and its words
    /// with it. `GlanceAnswer.optionName` is where it ends up.
    ///
    /// **It runs in the app's process**, which is what `LiveActivityIntent`
    /// buys over a bare `AppIntent`: the widget extension has no Rust core, no
    /// SSH identity and no connection, so an intent performed over there could
    /// only ever fail. In the app it goes through `Connection.core` and the
    /// same `terminal.agent_answer` call `AgentStream.answer` makes for the
    /// phone's own chat pane, so a card and the app cannot answer one agent
    /// differently. `WatchLinkHost` is the seam, for the reason its own doc
    /// comment gives about the watch: a second client that spoke to the daemon
    /// on its own would be a second place for "which option did they pick" to
    /// be decided.
    ///
    /// **It never retries and never reports a success it cannot confirm.**
    /// Both rules are enforced below the intent — see
    /// `WatchLinkHost.answerFromGlance` — because both are about what happened
    /// on the wire and this type has no view of that. What this type does is
    /// make sure exactly one attempt is made per tap.
    ///
    /// **Not discoverable.** `isDiscoverable` is false so this never appears in
    /// Shortcuts, Spotlight or Siri. Its three parameters are opaque ids that
    /// exist for a few minutes inside one agent's conversation; a person
    /// building a shortcut has nothing to put in them, and an action that
    /// cannot be filled in usefully is an action that should not be offered.
    public struct AnswerPermissionIntent: LiveActivityIntent {
        public static let title: LocalizedStringResource = "Answer an Agent"
        public static let description = IntentDescription(
            "Answers what an agent is waiting on, using one of the answers it offered.")
        public static let isDiscoverable = false
        /// Deliberately false: the whole point of the card's buttons is
        /// answering **without** opening the app, and on a locked phone opening
        /// it means unlocking first. What happens when the app cannot reach the
        /// runner from the background is reported on the card, not papered over
        /// by bringing the app forward.
        public static let openAppWhenRun = false

        @Parameter(title: "Agent") public var terminal: String
        @Parameter(title: "Permission") public var request: String
        @Parameter(title: "Answer") public var option: String
        @Parameter(title: "Answer Name") public var optionName: String

        public init() {}

        public init(terminal: String, request: String, option: String, optionName: String) {
            self.terminal = terminal
            self.request = request
            self.option = option
            self.optionName = optionName
        }

        /// One attempt, and whatever came of it is written where the surface
        /// can read it.
        ///
        /// Nothing is returned to the caller and nothing is thrown. A widget
        /// button has no dialog and no error presentation — a thrown error from
        /// here reaches a system log and nobody else — so the outcome travels
        /// the way everything else this extension knows travels: through the
        /// App Group, into `GlanceAnswer`, onto the card. Throwing as WELL
        /// would give iOS a failure to report in its own words on top of ours,
        /// which is two accounts of one event.
        @MainActor
        public func perform() async throws -> some IntentResult {
            await AnswerPermissionDelivery.deliver(self)
            return .result()
        }
    }

    /// How an intent performed in the app's process reaches the code that holds
    /// the connection.
    ///
    /// A hook rather than a direct call, because this file is compiled into TWO
    /// binaries: the app, which has `WatchLinkHost` and a `Connection`, and the
    /// widget extension, which has neither and must still be able to construct
    /// the intent to put it on a button. The extension cannot see anything
    /// under `apps/ios/FarCooler/`, so the app installs itself here instead.
    ///
    /// **Installed from `PushDelegate.application(_:didFinishLaunchingWithOptions:)`**,
    /// beside `WatchLinkHost.start()` and for the identical reason that call
    /// site was chosen: iOS launches this app into the background to perform an
    /// intent, a background launch may never build a scene at all, and a
    /// SwiftUI `.task` is therefore a hook that does not run in exactly the
    /// case this feature exists for.
    ///
    /// The default refuses in words rather than doing nothing. An uninstalled
    /// hook means the intent is running somewhere that cannot deliver it, and a
    /// tap that silently evaporates is the failure this whole task is written
    /// against.
    @MainActor
    public enum AnswerPermissionDelivery {
        /// Set by the app at launch. Nil everywhere else.
        public static var handler: ((AnswerPermissionIntent) async -> Void)?

        static func deliver(_ intent: AnswerPermissionIntent) async {
            guard let handler else {
                GlancePermissionStore.update {
                    // Claimed and settled in one step, so the card has
                    // something to show. `nothingSent` is the truthful outcome:
                    // this process never had anything to send it with.
                    $0.claiming(
                        terminal: intent.terminal, request: intent.request,
                        option: intent.option, optionName: intent.optionName, at: Date()
                    )?
                    .settling(
                        terminal: intent.terminal, request: intent.request,
                        outcome: .nothingSent,
                        message:
                            "Your \(DeviceKind.current) couldn’t take that, so nothing was sent.",
                        at: Date())
                }
                return
            }
            await handler(intent)
        }
    }
#endif
