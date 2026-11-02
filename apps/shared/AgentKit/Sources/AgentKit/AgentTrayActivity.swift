import Foundation

public enum AgentTray {
    public enum Activity: Equatable, Sendable {
        case none
        case tool(String)
        case thinking

        public init(_ row: AgentRow) {
            switch row.kind {
            case .tool(let tool) where tool.status == .running:
                self = .tool(tool.summary.isEmpty ? tool.name : "\(tool.name) \(tool.summary)")
            case .thinking(let thinking) where thinking.endedMs == nil:
                self = .thinking
            default:
                self = .none
            }
        }
    }
}
