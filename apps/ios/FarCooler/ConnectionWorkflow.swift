import Foundation

extension Connection {
    /// Build once per host, retaining no authenticated Session in the adapter.
    func makeWorkflowAdapter() -> WorkflowSessionAdapter {
        WorkflowSessionAdapter(root: WorkflowSessionAdapter.journalRoot()) { [weak self] requested in
            guard let self, self.isAnswering, let host = self.hostId,
                  let actualHost = self.daemon?.runnerId, !actualHost.isEmpty,
                  host.uuidString.caseInsensitiveCompare(requested) == .orderedSame else { return nil }
            let account = Account.shared.userId.isEmpty ? self.ownClientId ?? "" : Account.shared.userId
            return WorkflowSessionAdapter.Current(account: account, host: actualHost,
                connection: self.workflowConnectionID) { [weak self] method, raw in
                    guard let self, self.isAnswering, self.hostId == host else { throw WorkflowDisplayError.pending }
                    let args = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
                    return String(decoding: try await self.core.call(method, args), as: UTF8.self)
                }
        }
    }
}
