import Foundation

/// A crash's or hang's signature, read out of a MetricKit diagnostic (ov-797),
/// and the only thing about it that leaves the phone.
///
/// MetricKit hands the app each `MXCrashDiagnostic` and `MXHangDiagnostic` on
/// the launch after, as JSON (`jsonRepresentation()`). That JSON carries far
/// more than a signature: the exception's composed message and its format
/// arguments, which can hold anything the app was handling, every thread's
/// call tree, the termination reason, memory region details, the device and
/// region. This reads a short list of fields by name and nothing else:
///
/// - the Mach exception type and code, and the signal;
/// - an Objective-C exception's class name (`NSInvalidArgumentException`),
///   never its message, and only when it reads as a name in code;
/// - the faulting thread's top `topFrames` frames, each a binary's file name
///   and an offset into it, and the first frame in the app's own binary when
///   none of those is (an abort's top frames are all the system's);
/// - the app's version and build, and the OS.
///
/// What it sends is `wire`, which `CrashSignatureTests` pins key by key, and
/// the FFI decodes with `deny_unknown_fields`: a field added here that the
/// runner doesn't know is refused there, before it is sent.
public struct CrashSignature: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Sendable {
        case crash
        case hang
    }

    public struct Frame: Codable, Equatable, Sendable {
        /// The binary's file name, never a path.
        public var binary: String
        /// The offset into the binary's text segment.
        public var offset: UInt64
        /// A symbol, when the report had one. MetricKit's don't.
        public var symbol: String
        /// Where in the stack it was: 0 is the top.
        public var depth: Int
        /// In the app's own binary.
        public var own: Bool
    }

    public var kind: Kind
    public var exceptionType: Int?
    public var exceptionCode: Int64?
    public var signal: Int?
    public var exceptionName: String
    public var frames: [Frame]
    public var appVersion: String
    public var appBuild: String
    public var os: String

    enum CodingKeys: String, CodingKey {
        case kind
        case exceptionType = "exception_type"
        case exceptionCode = "exception_code"
        case signal
        case exceptionName = "exception_name"
        case frames
        case appVersion = "app_version"
        case appBuild = "app_build"
        case os
    }

    /// How many of the faulting thread's frames are sent, from the top.
    public static let topFrames = 10
    /// How deep the walk looks for the app's own frame.
    static let deepest = 128

    /// The signature in one diagnostic's JSON, or nil when it has no call
    /// stack to sign. `ownBinary` is the app's executable's file name.
    public static func read(diagnostic json: Data, kind: Kind, ownBinary: String) -> CrashSignature? {
        guard let root = try? JSONSerialization.jsonObject(with: json) as? [String: Any] else { return nil }
        let meta = root["diagnosticMetaData"] as? [String: Any] ?? [:]
        let tree = root["callStackTree"] as? [String: Any] ?? [:]
        let stacks = tree["callStacks"] as? [[String: Any]] ?? []
        // The thread the diagnostic is about; a hang's is the main thread,
        // which MetricKit lists first when it attributes none.
        guard let stack = stacks.first(where: { $0["threadAttributed"] as? Bool == true }) ?? stacks.first,
            let top = (stack["callStackRootFrames"] as? [[String: Any]])?.first
        else { return nil }

        var walked: [Frame] = []
        var frame: [String: Any]? = top
        while let f = frame, walked.count < deepest {
            guard let binary = f["binaryName"] as? String, let offset = number(f["offsetIntoBinaryTextSegment"]) else {
                break
            }
            let name = String(binary.split(separator: "/").last ?? "")
            walked.append(Frame(binary: binaryName(name), offset: UInt64(max(0, offset)), symbol: "", depth: walked.count, own: name == ownBinary))
            frame = (f["subFrames"] as? [[String: Any]])?.first
        }
        guard !walked.isEmpty else { return nil }
        var frames = Array(walked.prefix(topFrames))
        if !frames.contains(where: \.own), let own = walked.first(where: \.own) {
            frames.append(own)
        }

        let reason = meta["exceptionReason"] as? [String: Any]
        return CrashSignature(
            kind: kind,
            exceptionType: number(meta["exceptionType"]).map { Int($0) },
            exceptionCode: number(meta["exceptionCode"]),
            signal: number(meta["signal"]).map { Int($0) },
            exceptionName: className(reason?["exceptionName"] as? String),
            frames: frames,
            appVersion: versionText(meta["appVersion"] as? String ?? "", 64),
            appBuild: versionText(meta["appBuildVersion"] as? String ?? "", 64),
            os: versionText(meta["osVersion"] as? String ?? "", 128))
    }

    /// What `crash.report` is sent: these keys and no others.
    public var wire: [String: Any] {
        guard let data = try? JSONEncoder().encode(self),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return [:] }
        return object
    }

    /// An integer however JSONSerialization read it.
    private static func number(_ value: Any?) -> Int64? {
        (value as? NSNumber)?.int64Value
    }

    /// `text` in the runner's character set for `set`, at most `max` long:
    /// what isn't in it goes, so a report is never refused for a character,
    /// and nothing that could read as Markdown, a fence or a direction
    /// change reaches a card (`crash_reports::checked`).
    private static func kept(_ text: String, _ set: String, _ max: Int) -> String {
        let allowed = Set(set.unicodeScalars)
        let scalars = text.unicodeScalars.filter { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || allowed.contains($0)) }
        return String(String.UnicodeScalarView(scalars.prefix(max))).trimmingCharacters(in: .whitespaces)
    }

    /// A binary's file name: letters, digits, `.`, `_`, `+`, `-`, spaces.
    static func binaryName(_ text: String) -> String {
        let name = kept(text, "._+- ", 128)
        return name.isEmpty ? "unknown" : name
    }

    /// A version, build or OS: letters, digits, spaces, `.`, `_`, `(`, `)`, `,`, `-`.
    static func versionText(_ text: String, _ max: Int) -> String {
        kept(text, " ._(),-", max)
    }

    /// An exception's name when it reads as a class name, else nothing: a
    /// name never carries what the app was handling, and anything else might.
    private static func className(_ name: String?) -> String {
        guard let name, !name.isEmpty, name.count <= 128,
            name.unicodeScalars.allSatisfy({ $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "_.$".unicodeScalars.contains($0)) })
        else { return "" }
        return name
    }
}

/// The signatures waiting for the owner's runner (ov-797), kept on the phone
/// until one takes them, oldest first.
///
/// MetricKit delivers each diagnostic once, on a launch that may have no
/// runner connected, so a signature is kept until a runner that is the
/// owner's has it. Only signatures are kept, so nothing more could leave
/// later either.
public struct CrashOutbox: Codable, Equatable, Sendable {
    public private(set) var waiting: [CrashSignature] = []

    /// The most kept: a crash loop on a phone with no runner keeps the latest.
    public static let limit = 20

    public init(waiting: [CrashSignature] = []) { self.waiting = waiting }

    public mutating func add(_ signatures: [CrashSignature]) {
        waiting.append(contentsOf: signatures)
        if waiting.count > Self.limit { waiting.removeFirst(waiting.count - Self.limit) }
    }

    /// Take `signature` off, once a runner has it or refused it for good.
    public mutating func remove(_ signature: CrashSignature) {
        if let at = waiting.firstIndex(of: signature) { waiting.remove(at: at) }
    }

    /// Whether this phone may send a runner its crashes: the runner takes
    /// them, and it has said this phone is its owner's (`RunnerPeople.you`
    /// is "", read from the runner on this connection). Not a member's
    /// runner, a released device's, or one that hasn't said yet: a crash
    /// goes only to the owner's own runner.
    public static func mayReport(runnerTakesCrashes: Bool, you: String?) -> Bool {
        runnerTakesCrashes && you == ""
    }

    /// What a refusal of one signature means for it.
    public enum Refused: Equatable, Sendable {
        /// It never will be taken: let it go.
        case drop
        /// This runner won't take any (it says the phone isn't its owner's,
        /// or the phone's grant is too low): stop asking it until the app is
        /// next launched, and keep the signature for another runner.
        case notThisRunner
        /// Try it again on a later poll.
        case later
    }

    /// `word` is the refusal's machine word, nil when the phone refused it
    /// before sending.
    public static func refused(word: String?) -> Refused {
        switch word {
        case nil, "invalid-argument": .drop
        case "not-yours", "scope-denied", "capability-unsupported": .notThisRunner
        default: .later
        }
    }
}
