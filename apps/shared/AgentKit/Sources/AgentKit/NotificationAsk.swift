import Foundation

/// When the phone asks for notification permission, and what it says first.
///
/// It used to ask at launch, so on a first run the system's alert covered
/// "Connect a Runner" before the person had seen what the app was, and a
/// refusal at that moment is permanent. The reason for asking early still
/// holds: a prompt that only appears once the person is looking at the app has
/// missed the agent that finished while they weren't. So the ask moves to the
/// first runner being added, which is still before they look away, with one
/// line saying what the permission is for.
enum NotificationAsk {
    /// Whether to ask at launch: only a phone that already has a runner. That is
    /// a returning launch, where the system either has an answer already or the
    /// person has been through the explainer before.
    static func asksAtLaunch(hasRunners: Bool) -> Bool { hasRunners }

    /// Whether the runner list going from `before` to `after` is the first
    /// runner arriving, which is when the explainer is shown.
    static func explainsAfterFirstRunner(hadRunners before: Bool, hasRunners after: Bool) -> Bool {
        !before && after
    }

    static let title = "Get Notified When an Agent Needs You"
    /// No promise about a closed app: a push needs this device signed in and
    /// the runner paired with the relay, and a first run has neither (ov-205).
    static let message =
        "Your agents keep working without you. Far Cooler can tell you when one has a question or finishes, "
        + "so you don’t have to keep checking."
    static let allow = "Allow Notifications"
    static let decline = "Not Now"
}
