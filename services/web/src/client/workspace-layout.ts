import type { WorkTerminal, WorkView } from './work-screen'

export type Slot = 'main' | 'side'
export type Page = 'terminals' | 'needs' | 'plan' | 'ticket' | 'workflow'
export type Tab = Exclude<Page, 'ticket' | 'workflow'>
type WorkCommand = 'start_work' | 'rename_terminal' | 'close_terminal' | 'recover_terminal'
type Command = WorkCommand | 'commands' | 'shortcuts' | 'terminals' | 'needs' | 'plan' | 'workflow' |
  'orchestrator' | 'split' | 'close_split' | 'focus_other' | 'conversation' | 'raw_terminal' | 'close_page'
type Action = { id: Command; label: string; key: string | null; enabled: boolean }
export type PaneModel = {
  terminal: string | null
  title: string
  agent: boolean
  attach: boolean
  mode: 'conversation' | 'terminal'
  notice: string
}
type Intent = { kind: 'present' } | { kind: 'focus'; slot: Slot } | { kind: 'open'; id: string; slot?: Slot } |
  { kind: 'run'; command: Command; slot?: Slot } | { kind: 'bind'; command: Command; key: string } |
  { kind: 'key'; key: string; alt: boolean; shift: boolean; ctrl: boolean; meta: boolean; editing: boolean; terminal: boolean; modal: boolean }
type Effect = { kind: 'none' | 'commands' | 'shortcuts' | 'close_page' | 'refresh_pane' } |
  { kind: 'focus'; slot: Slot } | { kind: 'navigate'; page: Tab | 'workflow' } |
  { kind: 'work'; command: WorkCommand }
export type LayoutView = {
  layout: unknown
  bindings: unknown
  focus: Slot
  main: PaneModel
  side: PaneModel | null
  actions: Action[]
  effect: Effect
  handled: boolean
  problem: string
  close_label: string
  shortcut_prefix: string
  shortcut_help: string
}

export function readLocal(key: string): string {
  try { return localStorage.getItem(key) ?? '' } catch { return '' }
}

export function writeLocal(key: string, value: string): void {
  try {
    if (value) localStorage.setItem(key, value)
    else localStorage.removeItem(key)
  } catch {}
}

function readJson(key: string): unknown {
  try { return JSON.parse(readLocal(key)) } catch { return undefined }
}

const node = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T

export class WorkspaceLayout {
  private workspace = ''
  private context = {
    page: 'terminals' as Page, workspace: false, workspace_name: null as string | null,
    workspace_count: 0, missing_terminal: false, start_work: null as string | null, terminals: [] as WorkTerminal[], orchestrator: null as string | null,
  }
  private view: LayoutView | undefined
  private bindings: unknown
  private readonly dialog = node<HTMLDialogElement>('workspace-commands')
  private readonly disposeListeners = new AbortController()

  constructor(
    private readonly scope: string,
    private readonly runnerName: string,
    private readonly present: (request: string) => string,
    private readonly draw: (view: LayoutView, intent: Intent['kind']) => void,
    private readonly act: (effect: Effect) => void,
  ) {
    this.bindings = readJson(this.bindingKey)
    const options = { signal: this.disposeListeners.signal }
    document.addEventListener('keydown', event => this.key(event), options)
    for (const slot of ['main', 'side'] as const) {
      const element = node(`workspace-${slot}`)
      const focused = () => {
        if (this.view?.focus !== slot) this.dispatch({ kind: 'focus', slot })
      }
      element.addEventListener('pointerdown', focused, options)
      element.addEventListener('focusin', focused, options)
    }
  }

  private get bindingKey(): string { return `${this.scope}:shortcuts` }
  private get layoutKey(): string { return `${this.scope}:layout:${this.workspace}` }

  contextFor(workspace: string | null, page: Page, work: WorkView, orchestrator: string | null): void {
    const changed = this.workspace !== JSON.stringify(workspace)
    this.workspace = JSON.stringify(workspace)
    this.context = {
      workspace: work.workspace !== null, workspace_name: work.workspaces.find(space => space.id === workspace)?.name ?? null,
      workspace_count: work.workspaces.length, page, terminals: work.terminals, orchestrator,
      missing_terminal: work.missingTerminal, start_work: work.create || work.start ? work.labels.open : null,
    }
    this.dispatch({ kind: 'present' }, changed ? readJson(this.layoutKey) ?? null : this.view?.layout)
  }

  run(command: Command, slot?: Slot): void {
    this.dispatch({ kind: 'run', command, slot })
  }

  open(id: string, slot?: Slot): void { this.dispatch({ kind: 'open', id, slot }) }

  stop(): void {
    this.disposeListeners.abort()
    this.dialog.close()
  }

  private key(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.isComposing || event.repeat || !event.altKey || !event.shiftKey) return
    const target = event.target instanceof Element ? event.target : null
    const key = /^(?:Key|Digit)([A-Z0-9])$/.exec(event.code)?.[1] ?? event.key
    if (node('work').hidden) return
    this.dispatch({
      kind: 'key', key, alt: event.altKey, shift: event.shiftKey, ctrl: event.ctrlKey, meta: event.metaKey,
      editing: !!target?.closest('input,textarea,select,[contenteditable]'),
      terminal: !!target?.closest('.xterm'),
      modal: !!document.querySelector('dialog:modal'),
    }, this.view?.layout, event)
  }

  private dispatch(intent: Intent, layout = this.view?.layout, event?: KeyboardEvent): void {
    const context = { ...this.context, runner_name: this.runnerName }
    const view: LayoutView = JSON.parse(this.present(JSON.stringify({ layout, bindings: this.bindings, context, intent })))
    if (event && !view.handled) return
    event?.preventDefault()
    this.view = view
    this.bindings = view.bindings
    writeLocal(this.layoutKey, JSON.stringify(view.layout))
    if (intent.kind === 'bind') writeLocal(this.bindingKey, JSON.stringify(view.bindings))
    this.draw(view, intent.kind)
    if (view.effect.kind === 'commands' || view.effect.kind === 'shortcuts') this.palette(view.effect.kind === 'shortcuts')
    else this.act(view.effect)
  }

  private palette(settings: boolean): void {
    const dialog = this.dialog
    const view = this.view!
    const title = document.createElement('h2')
    title.textContent = view.actions.find(a => a.id === (settings ? 'shortcuts' : 'commands'))!.label
    const close = document.createElement('button')
    close.textContent = view.close_label
    close.onclick = () => dialog.close()
    dialog.setAttribute('aria-label', title.textContent)
    dialog.replaceChildren(title)
    if (settings) {
      const help = document.createElement('p')
      help.textContent = view.shortcut_help
      const problem = document.createElement('p')
      problem.setAttribute('role', 'status')
      dialog.append(help, problem)
      for (const action of view.actions) {
        const row = document.createElement('label')
        const input = document.createElement('input')
        input.setAttribute('aria-label', action.label)
        input.size = 3
        input.value = action.key?.toUpperCase() ?? ''
        input.onchange = () => {
          this.dispatch({ kind: 'bind', command: action.id, key: input.value })
          problem.textContent = this.view!.problem
          input.value = this.view!.actions.find(a => a.id === action.id)?.key?.toUpperCase() ?? ''
        }
        row.append(action.label, view.shortcut_prefix, input)
        dialog.append(row)
      }
    } else {
      for (const action of view.actions) {
        if (!action.enabled || action.id === 'commands') continue
        const button = document.createElement('button')
        button.textContent = action.label
        if (action.key) {
          const key = document.createElement('kbd')
          key.textContent = view.shortcut_prefix + action.key.toUpperCase()
          button.append(key)
        }
        button.onclick = () => {
          dialog.close()
          this.run(action.id)
        }
        dialog.append(button)
      }
    }
    dialog.append(close)
    if (!dialog.open) dialog.showModal()
  }
}
