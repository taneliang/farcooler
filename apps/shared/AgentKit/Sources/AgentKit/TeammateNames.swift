import Foundation

/// What a teammate added by QR is called on a runner (ov-525, ruling R-65),
/// before anyone edits it: a handle from their email's local part, made
/// unique on that runner, and a name to read.
///
/// Suggestions only. The runner decides what's a handle (`valid_handle` in
/// `crates/store/src/people.rs`) and refuses anything else, so ``isValidHandle(_:)``
/// is the same rule spelled here only to say so before the person taps Add.
public enum TeammateNames {
    /// The handles the runner keeps for itself (`RESERVED` in `people.rs`).
    static let reserved: Set<String> = ["owner", "orchestrator", "rep", "runner", "manager", "user"]

    /// The runner's rule: 1 to 32 lowercase letters, digits or `-`, starting
    /// with a letter, and none it keeps for itself.
    public static func isValidHandle(_ handle: String) -> Bool {
        guard let first = handle.unicodeScalars.first, ("a"..."z").contains(first), handle.count <= 32 else {
            return false
        }
        let allowed = handle.unicodeScalars.allSatisfy { ("a"..."z").contains($0) || ("0"..."9").contains($0) || $0 == "-" }
        return allowed && !reserved.contains(handle)
    }

    /// `bob.smith@example.com` is `bob-smith`, or `bob-smith-2` when a member
    /// on this runner has that already.
    public static func handle(email: String, taken: Set<String>) -> String {
        let local = email.split(separator: "@").first.map(String.init) ?? email
        var base = ""
        for scalar in local.lowercased().unicodeScalars {
            let keep = ("a"..."z").contains(scalar) || ("0"..."9").contains(scalar)
            if keep {
                base.unicodeScalars.append(scalar)
            } else if !base.isEmpty, !base.hasSuffix("-") {
                base += "-"
            }
        }
        // It must start with a letter: digits in front are dropped, not kept.
        while let first = base.unicodeScalars.first, !("a"..."z").contains(first) { base.removeFirst() }
        base = String(base.prefix(32))
        while base.hasSuffix("-") { base.removeLast() }
        if base.isEmpty { base = "teammate" }
        return unique(base, taken: taken)
    }

    /// `chosen`, or with `-2`, `-3`… on the end until no member here has it.
    /// Cut so the suffix still fits in 32.
    public static func unique(_ chosen: String, taken: Set<String>) -> String {
        if !taken.contains(chosen), !reserved.contains(chosen) { return chosen }
        var n = 2
        while true {
            let suffix = "-\(n)"
            var stem = String(chosen.prefix(32 - suffix.count))
            while stem.hasSuffix("-") { stem.removeLast() }
            let candidate = stem + suffix
            if !taken.contains(candidate) { return candidate }
            n += 1
        }
    }

    /// `bob.smith@example.com` reads as "Bob Smith": the directory knows an
    /// email and no name, and this is a starting point the person edits.
    /// Within the runner's 64 characters.
    public static func name(email: String) -> String {
        let local = email.split(separator: "@").first.map(String.init) ?? email
        let words = local.split(whereSeparator: { ".-_+".contains($0) || $0.isNumber }).map { $0.capitalized }
        let name = words.isEmpty ? email : words.joined(separator: " ")
        return String(name.prefix(64))
    }
}
