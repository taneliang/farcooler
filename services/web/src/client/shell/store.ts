/// What the React shell (shell.tsx) shows, and what its controls ask for
/// (ov-1510). The rest of the client writes here instead of into the shell's
/// DOM, and installs the handlers the shell's controls call: the shell renders
/// a view model, and holds no feature logic of its own.
///
/// A plain external store, read with `useSyncExternalStore`: every `set` makes
/// a new immutable snapshot and tells each subscriber once.

export type ShellScreen = 'status' | 'enroll' | 'compare' | 'join' | 'work'
export type ShellTab = 'terminals' | 'needs' | 'board' | 'plan'
export type ShellPage = ShellTab | 'workflow' | 'ticket'
export type ShellTool = 'orchestrator' | 'workflow' | 'commands'

export interface Command {
  label: string
  disabled: boolean
  /// `aria-keyshortcuts`, when the command has one.
  keys?: string
}

export interface Choice {
  id: string
  label: string
  disabled?: boolean
}

export interface ShellState {
  screen: ShellScreen
  /// The runner this page is connected to, and how that connection is.
  runner: { id: string; label: string; health: string; runtime: string }
  /// Every runner this browser holds, for switching. One runner: no switcher.
  runners: Choice[]
  /// The workspaces on the runner, each labeled with what waits in it.
  spaces: Choice[]
  space: string | null
  /// The workspace's repository, for the page header's breadcrumb.
  repository: string
  /// A change is being sent to the runner: the picker waits.
  busy: boolean
  page: ShellPage
  nav: Record<ShellTab, Command>
  tools: Record<ShellTool, Command>
  /// The view's one filled action: Start Work. Null where it isn't offered.
  startWork: string | null
  /// A recovery action is on screen, so it, not Start Work, is the filled
  /// one: Recover beside the terminal it brings back (Terminals only), or
  /// Reconnect beside a lost connection (every view).
  terminalLost: boolean
  linkLost: boolean
  /// Where each view lives, for links that open in a new tab.
  hrefs: Partial<Record<ShellTab, string>>
}

export interface ShellHandlers {
  showTab(tab: ShellTab): void
  pickSpace(id: string): void
  pickRunner(id: string): void
  addRunner(): void
  tool(tool: ShellTool): void
  startWork(): void
}

const idle = (label: string): Command => ({ label, disabled: false })

export const INITIAL: ShellState = {
  screen: 'status',
  runner: { id: '', label: '', health: '', runtime: '' },
  runners: [],
  spaces: [],
  space: null,
  repository: '',
  busy: false,
  page: 'terminals',
  nav: { terminals: idle('Terminals'), needs: idle('Needs You'), board: { label: 'Board', disabled: true }, plan: idle('Plan') },
  tools: { orchestrator: idle('Orchestrator'), workflow: idle('Workflow Runs'), commands: idle('Commands') },
  startWork: null,
  terminalLost: false,
  linkLost: false,
  hrefs: {},
}

const NOTHING: ShellHandlers = {
  showTab() {},
  pickSpace() {},
  pickRunner() {},
  addRunner() {},
  tool() {},
  startWork() {},
}

export class ShellStore {
  private state: ShellState = INITIAL
  private readonly listeners = new Set<() => void>()
  handlers: ShellHandlers = NOTHING

  readonly read = (): ShellState => this.state

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /// Change some of the state. A change that changes nothing tells no one,
  /// so a poll that found nothing new renders nothing.
  set(change: Partial<ShellState>): void {
    let changed = false
    for (const key of Object.keys(change) as (keyof ShellState)[]) {
      if (!same(this.state[key], change[key])) changed = true
    }
    if (!changed) return
    this.state = { ...this.state, ...change }
    for (const listener of [...this.listeners]) listener()
  }

  /// Change one nav item or tool, keeping the rest.
  command(kind: 'nav', id: ShellTab, change: Partial<Command>): void
  command(kind: 'tools', id: ShellTool, change: Partial<Command>): void
  command(kind: 'nav' | 'tools', id: string, change: Partial<Command>): void {
    const group = this.state[kind] as Record<string, Command>
    this.set({ [kind]: { ...group, [id]: { ...group[id], ...change } } })
  }

  /// Install the handlers for the shell's controls; any not given do nothing.
  handle(handlers: Partial<ShellHandlers>): void {
    this.handlers = { ...this.handlers, ...handlers }
  }

  /// Back to the start for a runner's workspaces: nothing to handle, and
  /// nothing but the screen and the runner's connection kept.
  reset(): void {
    this.handlers = NOTHING
    this.set({ ...INITIAL, screen: this.state.screen, runner: this.state.runner, linkLost: this.state.linkLost })
  }
}

function same(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b)
}

/// The page's one shell.
export const shell = new ShellStore()
