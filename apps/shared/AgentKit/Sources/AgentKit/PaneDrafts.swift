import Foundation

/// What somebody had typed into a pane and not sent yet.
///
/// **The one irreversible thing a pane's teardown costs.** A pane is a tmux
/// session on the runner, so nothing there ends when the app stops drawing it:
/// the scrollback is refetchable, the scroll position is a nuisance, and the
/// transcript is on disk. The half-written message in the composer is the only
/// thing that exists nowhere but this phone, and the only thing that cannot be
/// got back.
///
/// So it is written down. Every mount of a composer reads its pane's draft on
/// the way in and records it as it is typed, which makes an unmounted pane —
/// whatever unmounted it, and a relaunch counts — cost nothing that was not
/// already costless.
///
/// `UserDefaults` and not the App Group container, for `RunnerDirectoryStore`'s
/// reason: nothing outside the app reads this. A widget has no composer, and a
/// notification extension has no keyboard.
struct PaneDraft: Codable, Sendable, Equatable {
    var text: String
    /// When it was last typed into, which is what both bounds below are
    /// measured against. Part of the value rather than derived, because
    /// `UserDefaults` has no modification time per key.
    var savedAt: Date

    init(text: String, savedAt: Date) {
        self.text = text
        self.savedAt = savedAt
    }
}

/// Every pane's unsent draft, on disk, keyed by the pane it belongs to.
///
/// **Bounded by age and by count, and not by the fleet.** The obvious pruning
/// rule — drop every draft whose terminal is no longer in the fleet — is the
/// one rule this must not use: the app holds a connection per runner and a
/// runner that is merely asleep reports no terminals at all, so a fleet-shaped
/// prune would delete the drafts of every pane on it. Age and count need no
/// knowledge of who is awake, which is the only kind of knowledge this has.
///
/// One key holding one dictionary, written whole — `RunnerDirectoryStore`'s
/// shape, for its reason: a key per pane cannot be enumerated without already
/// knowing every pane id, which is exactly what pruning needs to ask.
enum PaneDraftStore {
    /// Spelled out, because it names a slot on disk that installs already have.
    private static let key = "paneDrafts"

    /// How long an untouched draft is kept.
    ///
    /// A month, because the case this exists for is "I started writing this,
    /// got interrupted, and came back" — and on a phone that interruption is
    /// measured in days rather than minutes. Shorter would throw away the
    /// thing it was built to keep; unbounded would be a preferences file that
    /// only ever grows.
    static let keepFor: TimeInterval = 60 * 60 * 24 * 30

    /// How many drafts are kept at once, newest first.
    ///
    /// A second bound rather than a redundant one: age bounds a fleet that is
    /// mostly quiet, and this bounds a fleet that is not. Two hundred is far
    /// more panes than anyone has open and still a file measured in kilobytes.
    static let keepAtMost = 200

    static func read(from defaults: UserDefaults = .standard) -> [String: PaneDraft] {
        guard let data = defaults.data(forKey: key),
            let decoded = try? decoder.decode([String: PaneDraft].self, from: data)
        else { return [:] }
        return decoded
    }

    /// What is in this pane's composer, or nil for a pane nothing was typed
    /// into.
    ///
    /// Expired drafts are not returned even while they are still on disk:
    /// pruning happens on write, so a phone that was not opened for two months
    /// would otherwise hand back a draft this store has already promised to
    /// forget. `now` is an argument so that promise is a pure function of its
    /// inputs.
    static func draft(
        forPane pane: String, from defaults: UserDefaults = .standard, now: Date = Date()
    ) -> String? {
        guard let draft = read(from: defaults)[pane] else { return nil }
        guard now.timeIntervalSince(draft.savedAt) <= keepFor else { return nil }
        return draft.text
    }

    /// Record what is in a pane's composer.
    ///
    /// An empty draft REMOVES the entry rather than storing an empty string.
    /// Sending a message clears the field, and a row per pane anybody has ever
    /// sent from would be the unbounded file the two bounds above exist to
    /// prevent — while also making "nothing typed" and "typed and then erased"
    /// two states that read the same and cost differently.
    static func record(
        _ text: String, forPane pane: String, in defaults: UserDefaults = .standard,
        now: Date = Date()
    ) {
        var all = read(from: defaults)
        if text.isEmpty {
            all[pane] = nil
        } else {
            all[pane] = PaneDraft(text: text, savedAt: now)
        }
        write(prune(all, now: now), to: defaults)
    }

    /// Forget one pane's draft — the pane went away for good, or its message
    /// was sent.
    static func clear(pane: String, in defaults: UserDefaults = .standard) {
        var all = read(from: defaults)
        guard all.removeValue(forKey: pane) != nil else { return }
        write(all, to: defaults)
    }

    /// Both bounds, applied together.
    ///
    /// Age first and then count, because the count is a cap on what is worth
    /// keeping rather than on what has been written: dropping the expired
    /// entries first means a fleet with three hundred long-dead panes and ten
    /// live ones keeps all ten.
    static func prune(_ all: [String: PaneDraft], now: Date) -> [String: PaneDraft] {
        let fresh = all.filter { now.timeIntervalSince($0.value.savedAt) <= keepFor }
        guard fresh.count > keepAtMost else { return fresh }
        let newest = fresh.sorted { $0.value.savedAt > $1.value.savedAt }.prefix(keepAtMost)
        return Dictionary(uniqueKeysWithValues: newest.map { ($0.key, $0.value) })
    }

    private static func write(_ all: [String: PaneDraft], to defaults: UserDefaults) {
        guard let data = try? encoder.encode(all) else { return }
        defaults.set(data, forKey: key)
    }

    /// Dates as seconds since 1970 on both sides, for `SnapshotStore`'s
    /// reason: a build that encoded them one way and decoded them another
    /// would silently read every draft as absent.
    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }()
}
