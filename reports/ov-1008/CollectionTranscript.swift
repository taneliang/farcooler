import SwiftUI
import UIKit

/// Experiment only. Injected into a disposable copy by prepare.py; never in
/// the shipping source phase. SwiftUI still renders each production row.
struct SpikeCollectionTranscript: UIViewControllerRepresentable {
    let model: NativePaneModel
    let margin: CGFloat
    let items: [ConversationItem]

    func makeUIViewController(context: Context) -> Controller { Controller(model: model, margin: margin) }
    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.update(items)
    }

    final class Controller: UICollectionViewController {
        let model: NativePaneModel
        let margin: CGFloat
        var items: [ConversationItem] = []
        var data: UICollectionViewDiffableDataSource<Int, String>!
        var placed = false

        init(model: NativePaneModel, margin: CGFloat) {
            self.model = model
            self.margin = margin
            var config = UICollectionLayoutListConfiguration(appearance: .plain)
            config.showsSeparators = false
            config.backgroundColor = .clear
            super.init(collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        }
        required init?(coder: NSCoder) { fatalError("experiment has no storyboard") }

        override func viewDidLoad() {
            super.viewDidLoad()
            collectionView.backgroundColor = .clear
            collectionView.keyboardDismissMode = .interactive
            collectionView.accessibilityIdentifier = "native-transcript"
            collectionView.contentInset = UIEdgeInsets(top: Spacing.section, left: 0, bottom: Spacing.section, right: 0)
            let registration = UICollectionView.CellRegistration<UICollectionViewListCell, String> { [weak self] cell, _, id in
                guard let self, let item = items.first(where: { $0.id == id }) else { return }
                cell.backgroundConfiguration = .clear()
                cell.contentConfiguration = UIHostingConfiguration {
                    SpikeItem(item: item, model: self.model)
                        .id(id)
                        .environment(\.selectsRanges, true)
                        .environment(\.promptImages, self.model.promptImages)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, self.margin)
                        .padding(.bottom, Spacing.inset)
                }.margins(.all, 0)
            }
            data = UICollectionViewDiffableDataSource<Int, String>(collectionView: collectionView) { collection, index, id in
                collection.dequeueConfiguredReusableCell(using: registration, for: index, item: id)
            }
            update(model.store.items)
        }

        func update(_ next: [ConversationItem]) {
            guard isViewLoaded, next != items else { return }
            let old = Dictionary(uniqueKeysWithValues: items.map { ($0.id, $0) })
            items = next
            collectionView.accessibilityValue = "rows=\(model.store.ids.count) items=\(next.count)"
            var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
            snapshot.appendSections([0])
            snapshot.appendItems(next.map(\.id))
            snapshot.reconfigureItems(next.filter { old[$0.id] != nil && old[$0.id] != $0 }.map(\.id))
            data.apply(snapshot, animatingDifferences: false)
        }

        override func collectionView(_ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell,
                                     forItemAt indexPath: IndexPath) {
            if placed, indexPath.item == 0, model.store.moreBefore { model.loadOlder() }
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            if !placed, !items.isEmpty, collectionView.contentSize.height > collectionView.bounds.height {
                placed = true
                collectionView.setContentOffset(CGPoint(x: 0, y: max(0, collectionView.contentSize.height - collectionView.bounds.height)), animated: false)
            }
        }
    }
}

private struct SpikeItem: View {
    let item: ConversationItem
    let model: NativePaneModel

    var body: some View {
        switch item {
        case .row(let id):
            if let box = model.store.box(id) {
                NativeRowView(box: box, isLast: id == model.store.items.last?.id, showTerminal: {})
            }
        case .tools(let id, let rows):
            NativeToolGroupRow(id: id, boxes: rows.compactMap(model.store.box))
        }
    }
}

@MainActor
enum SpikeStats {
    static weak var store: AgentRowStore?
}
