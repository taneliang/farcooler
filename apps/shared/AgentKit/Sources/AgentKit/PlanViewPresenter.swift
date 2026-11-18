import Foundation

public enum PlanViewPresenter {
    /// A request in, the answer out; nil when there is nothing to call.
    public typealias Present = @Sendable (String) -> String?

    final class State: @unchecked Sendable {
        let lock = NSLock()
        var present: Present?
        /// The last few answers, so a view body that asks again doesn't cross
        /// the bridge for the same plan.
        var recent: [(key: Any, view: WirePlanView?)] = []
    }

    static let state = State()
    static let kept = 8

    /// Install the app's call to the bridge. Each app does this at launch.
    public static func install(_ present: @escaping Present) {
        state.lock.lock()
        state.present = present
        state.recent = []
        state.lock.unlock()
    }

    /// Any request to the same bridge, for the shared phrases (`SharedWords`).
    static func ask(_ request: String) -> String? {
        state.lock.lock()
        let present = state.present ?? linked
        state.lock.unlock()
        return present?(request)
    }

    /// The bridge's C signature.
    public typealias Bridge = @convention(c) (UnsafePointer<CChar>?, UnsafeMutablePointer<UInt8>?, Int) -> Int

    /// Call a bridge that answers into a caller's buffer, as every
    /// `farcooler_client_*` answer does: once to learn the length when the
    /// first buffer is short, then again.
    public static func call(_ bridge: Bridge, _ request: String) -> String? {
        request.withCString { input -> String? in
            var capacity = 64 * 1024
            for _ in 0..<2 {
                var buffer = [UInt8](repeating: 0, count: capacity)
                let needed = buffer.withUnsafeMutableBufferPointer { bridge(input, $0.baseAddress, capacity) }
                if needed == 0 { return nil }
                if needed <= capacity { return String(decoding: buffer[0..<needed], as: UTF8.self) }
                capacity = needed
            }
            return nil
        }
    }

    /// The bridge, looked up in the running image: there when whatever runs
    /// this links the Rust library (AgentKit's tests do), else nil.
    static let linked: Present? = {
        // RTLD_DEFAULT: every image loaded.
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "farcooler_client_plan_view") else { return nil }
        let bridge = unsafeBitCast(symbol, to: Bridge.self)
        return { call(bridge, $0) }
    }()
}

