import type { Item } from './needs'
import type { NeedsYouAggregateView } from './generated/wire-types'

let present: ((request: string) => string) | undefined

// The app installs wasm's existing view-model bridge; tests install the same Rust entry point.
export function useNeedsYouPresenter(bridge: (request: string) => string): void { present = bridge }

export function aggregate(items: Item[]): { view: NeedsYouAggregateView; items: Item[]; yours: Item[]; others: Item[] } {
  const runners = new Map<string, Item[]>()
  for (const item of items) {
    const runner = item.runner ?? ''
    if (!runners.has(runner)) runners.set(runner, [])
    runners.get(runner)!.push(item)
  }
  const request = { schema: 1, needs_you: [...runners].map(([runner_id, rows]) => ({ runner_id, items: rows.map(i => ({
    ...i.wire, id: i.rawItemId ?? i.id, kind: i.rawKind ?? i.kind, rank: i.rank,
    workspace_id: i.workspaceId, repository_id: i.repositoryId, concern: i.concern,
  })) })) }
  let view: NeedsYouAggregateView
  try {
    if (!present) throw new Error('needs_you_bridge')
    const answer = JSON.parse(present(JSON.stringify(request)))
    if (!answer.needs_you) throw new Error(`needs_you_invalid:${answer.error}`)
    view = answer.needs_you
  } catch (error) {
    console.error('Needs You presenter rejected the list', error)
    const seen = new Set<string>()
    const unique = items.filter(item => {
      const key = JSON.stringify([item.runner ?? '', item.rawItemId ?? item.id])
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    if (unique.length < items.length) return aggregate(unique)
    view = { schema: 1, rows: [], yours: [], others: [], split: false, counted_total: 0,
      runner_counts: [], workspace_counts: [], unclaimed_counts: [] }
  }
  const resolve = (key: { runner_id: string; item_id: string }): Item =>
    runners.get(key.runner_id)!.find(i => (i.rawItemId ?? i.id) === key.item_id)!
  return { view, items: view.rows.map(r => resolve(r.key)), yours: view.yours.map(resolve), others: view.others.map(resolve) }
}
