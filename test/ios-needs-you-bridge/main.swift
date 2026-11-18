import Foundation
let terminal = NeedsYouTerminal(id: "t", worktreeID: nil, label: "agent", role: "agent", paneMode: "terminal")
let item = NeedsYouItem(id: "ask", kind: .ask, rank: 1, since: nil, terminal: terminal, question: "?", runner: "host", concern: NeedsYouConcern(handle: "bob", name: "Bob", yours: false))
precondition(PlanViewPresenter.linked != nil)
precondition(item.isOthers)
precondition(NeedsYou.othersItem(askingOn: "t", in: [item]) != nil)
let merged = NeedsYou.merge(["host": [item, item]])
precondition(merged.count == 1 && merged.countedTotal == 1 && merged[0].isOthers)
print("bridge, ownership and duplicate recovery passed")
