import Foundation

/// What an iPhone or iPad says it is at pairing (ov-989), for the Mac's approve list.
///
/// The system gives a hardware identifier ("iPhone16,1"), not a name. The names below are the ones Apple
/// published for those identifiers; one this table doesn't know is said as the identifier itself, which is
/// true and still tells the owner which phone, where a guessed name could be wrong.
public enum PhoneModel {
    private static let names: [String: String] = [
        "iPhone14,7": "iPhone 14", "iPhone14,8": "iPhone 14 Plus", "iPhone15,2": "iPhone 14 Pro",
        "iPhone15,3": "iPhone 14 Pro Max", "iPhone15,4": "iPhone 15", "iPhone15,5": "iPhone 15 Plus",
        "iPhone16,1": "iPhone 15 Pro", "iPhone16,2": "iPhone 15 Pro Max", "iPhone17,1": "iPhone 16 Pro",
        "iPhone17,2": "iPhone 16 Pro Max", "iPhone17,3": "iPhone 16", "iPhone17,4": "iPhone 16 Plus",
        "iPhone17,5": "iPhone 16e",
    ]

    /// The name for a hardware `identifier`, or the identifier when it isn't known.
    public static func name(forIdentifier identifier: String) -> String {
        names[identifier] ?? identifier
    }

    /// This device: the Simulator says the model it stands in for.
    public static func current() -> String {
        if let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] {
            return name(forIdentifier: simulated)
        }
        var system = utsname()
        uname(&system)
        let identifier = withUnsafeBytes(of: &system.machine) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }
        return name(forIdentifier: identifier)
    }
}
