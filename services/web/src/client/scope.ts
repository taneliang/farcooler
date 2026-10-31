/// One workspace at a time (ov-871): the Mac and the phones organize a runner
/// by workspace, and so does the page. Terminals, Needs You and the plan show
/// the workspace the person picked and nothing from the others.
///
/// A runner's workspaces and each terminal's and item's workspace are the
/// runner's own words (`workspace.list`, `terminal.list`, `needs_you.list`).
/// What is here is only what a page does with them: pick which workspace is
/// showing, keep the terminals and items that belong to it, and count what
/// waits in each so the picker can say so. Nothing here touches the page or
/// wasm, so a test runs it with lists written by hand.

import { type BoardView, countOf, type Item } from './needs'

export interface Space {
  id: string
  name: string
  isMain: boolean
}

/// A terminal as the page lists it.
export interface Listed {
  id: string
  title: string
  state: string
  agent: boolean
  /// The workspace whose work it is doing; null when the runner says none.
  workspace: string | null
}

export function parseTerminals(json: string): Listed[] {
  let raw: any
  try {
    raw = JSON.parse(json)
  } catch {
    return []
  }
  if (!Array.isArray(raw)) return []
  return raw
    .filter(t => t && typeof t.id === 'string')
    .map(t => ({
      id: t.id,
      title: typeof t.title === 'string' ? t.title : '',
      state: typeof t.state === 'string' ? t.state : '',
      agent: t.agent === true,
      workspace: typeof t.workspace === 'string' && t.workspace ? t.workspace : null,
    }))
}

/// The workspace a terminal or item with `workspace` belongs to: its own, and
/// the main workspace's when it has none or names one that is gone, as the
/// apps draw an unclaimed worktree under the main workspace.
export function ownerOf(workspace: string | null, spaces: Space[]): string | null {
  if (workspace && spaces.some(s => s.id === workspace)) return workspace
  return (spaces.find(s => s.isMain) ?? spaces[0])?.id ?? null
}

/// Which workspace is showing: the one the person last picked on this
/// runner, else the main one, else the first.
export function pickSpace(spaces: Space[], saved: string | null): string | null {
  return spaces.find(s => s.id === saved)?.id ?? (spaces.find(s => s.isMain) ?? spaces[0])?.id ?? null
}

/// The terminals in `chosen`, in the runner's order. With no workspaces (an
/// older runner) every terminal is shown.
export function terminalsIn(terminals: Listed[], spaces: Space[], chosen: string | null): Listed[] {
  if (spaces.length === 0 || chosen === null) return terminals
  return terminals.filter(t => ownerOf(t.workspace, spaces) === chosen)
}

/// The terminal to open when a workspace comes to the front: the first one
/// running, else the first.
export function firstToOpen(terminals: Listed[]): Listed | undefined {
  return terminals.find(t => t.state === 'running') ?? terminals[0]
}

/// Needs You for the workspace in front. Items on the runner in front belong
/// to a workspace and are kept only if it is `chosen`; items on the other
/// runners (whose workspaces aren't the picker's) are all kept, marked
/// `elsewhere` so they're listed apart. With no workspaces nothing is cut.
export function scopeBoard(view: BoardView, spaces: Space[], chosen: string | null, front: string): BoardView {
  if (spaces.length === 0 || chosen === null) return view
  const keep = new Set<string>()
  const items: Item[] = []
  for (const item of view.items) {
    if (item.runner !== undefined && item.runner !== front) {
      items.push({ ...item, elsewhere: true })
      keep.add(item.id)
    } else if (ownerOf(item.workspaceId, spaces) === chosen) {
      items.push(item)
      keep.add(item.id)
    }
  }
  return {
    ...view,
    items,
    busy: new Set([...view.busy].filter(id => keep.has(id))),
    notices: new Map([...view.notices].filter(([id]) => keep.has(id))),
  }
}

/// How many items wait in each workspace of the runner in front.
export function waitingIn(view: BoardView, spaces: Space[], front: string): Map<string, number> {
  const counts = new Map<string, number>()
  for (const space of spaces) counts.set(space.id, 0)
  const mine = view.items.filter(i => i.runner === undefined || i.runner === front)
  for (const space of spaces) counts.set(space.id, countOf(mine.filter(i => ownerOf(i.workspaceId, spaces) === space.id)))
  return counts
}

/// A workspace's line in the picker: its name, and how many wait in it.
export function optionLabel(space: Space, waiting: number): string {
  return waiting > 0 ? `${space.name} (${waiting})` : space.name
}

/// The line over an empty terminal list.
export function noTerminals(space: Space | undefined, runner: string, many: boolean): string {
  return many && space
    ? `${space.name} has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.`
    : `${runner} has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.`
}
