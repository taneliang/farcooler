import Foundation

/// A key's fingerprint as two people compare it in person: the device being
/// added shows it under its code, and the device adding it shows it on the
/// confirmation (ov-525).
///
/// One spelling on every platform, so the two screens can be compared at a
/// glance: `SHA256:` and the first and last eight characters, about 96 bits.
/// It used to be four and three, about 42 bits, which a patient attacker can
/// grind a key to match (review F10). Android spells it the same way, in
/// `KeyFingerprint.kt`.
public enum KeyFingerprint {
    public static func short(_ fingerprint: String) -> String {
        let prefix = "SHA256:"
        guard fingerprint.hasPrefix(prefix) else { return fingerprint }
        let body = fingerprint.dropFirst(prefix.count)
        guard body.count > 20 else { return fingerprint }
        return "\(prefix)\(body.prefix(8))…\(body.suffix(8))"
    }
}
