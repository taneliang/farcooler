/// Browser navigation and rendering for the shared Rust work view.
import { type Space } from './scope'
import { shell } from './shell/store'
import { optionLabel } from './scope'

interface Choice { id: string; label: string }
interface Prompt { label: string; prompt: string }
interface Preset extends Choice { unavailable: boolean; models: Choice[] }

export interface WorkTerminal {
  id: string
  title: string
  sidebarTitle: string
  sidebarBadge: string | null
  live: boolean
  lost: boolean
  agent: boolean
  state: string
  access: string
  attach: boolean
  attachNote: string
  rename: Prompt | null
  close: Prompt | null
  recover: {
    action: 'restart' | 'restart_orchestrator' | 'reattach'
    label: string
    sidebarLabel?: string
    sidebarName?: string
    confirm?: string
    /** Why it's off here. */
    disabled?: string | null
  } | null
}

export interface WorkView {
  workspaces: Space[]
  repositories: Choice[]
  worktrees: Choice[]
  workspace: string | null
  orchestrator: string | null
  repository: string | null
  worktree: string | null
  terminal: string | null
  terminals: WorkTerminal[]
  notice: string
  missingTerminal: boolean
  labels: {
    open: string
    link: string
    repository: string
    worktree: string
    close: string
    refresh: string
    emptyTree: string
    review?: string | null
  }
  create: { open: string; submit: string; name: string; branch: string; disabled: string | null } | null
  start: { preset: string; model: string; submit: string; presets: Preset[]; disabled: string | null } | null
}

interface Selection {
  workspace?: string | null
  preferredWorkspace?: string | null
  repository?: string | null
  worktree?: string | null
  terminal?: string | null
  streamEnded?: boolean
  viewing?: Viewing
}

interface Viewing { terminal: string; reason: string }

type Intent =
  | { action: 'create'; repository: string | null; workspace: string | null; name: string; branch: string }
  | { action: 'start'; worktree: string | null; preset: string; model: string }
  | { action: 'rename'; terminal: string; name: string }
  | { action: 'close' | 'restart' | 'restart_orchestrator'; terminal: string }

interface WorkRunner {
  work_inventory(): Promise<string>
  work_intent(intent: string): Promise<string | undefined>
}
export type WorkError = Error & { code?: string; what?: string }
interface Options {
  runner: string
  current: () => WorkRunner | null
  project: (inventory: string, selection: string) => string | Promise<string>
  select: (view: WorkView, terminal: WorkTerminal | undefined, open: boolean, replace?: string, historyMode?: 'pushState' | 'replaceState') => void
  review?: (worktree: string) => void
  started: () => void
  note: (words: string) => void
  prefer: () => string | null
  inventory?: string
  navigation?: { workspace(id: string | null): void; terminal(id: string): void }
  failure: (error: WorkError) => string | Promise<string>
}

const get = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T
const param = (name: string) => new URLSearchParams(location.search).get(name)
export const linkedRunner = () => param('runner')

function fromUrl(): Selection {
  return { workspace: param('workspace'), terminal: param('terminal'), worktree: param('worktree') }
}

export function destination(runner: string, workspace: string | null, terminal?: string, worktree?: string): URL {
  const url = new URL(location.href)
  url.hash = ''
  url.searchParams.set('runner', runner)
  updateParam(url, 'workspace', workspace)
  updateParam(url, 'terminal', terminal)
  updateParam(url, 'worktree', worktree)
  url.searchParams.delete('view')
  url.searchParams.delete('ticket')
  return url
}

function updateParam(url: URL, key: string, value?: string | null): void {
  if (value) url.searchParams.set(key, value)
  else url.searchParams.delete(key)
}

function follow(event: MouseEvent, run: () => void): void {
  if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
  event.preventDefault()
  run()
}

function choices(select: HTMLSelectElement, items: (Choice & { unavailable?: boolean })[], chosen?: string | null): void {
  const signature = JSON.stringify(items)
  if (select.dataset.choices !== signature) {
    select.replaceChildren(...items.map(({ id, label, unavailable }) => {
      const option = new Option(label, id)
      option.disabled = !!unavailable
      return option
    }))
    select.dataset.choices = signature
  }
  if (chosen !== undefined) select.value = chosen ?? ''
}

function action(button: HTMLButtonElement, label?: string, run?: () => void): void {
  button.hidden = !label
  button.textContent = label ?? ''
  button.onclick = run ?? null
}

export function mountWork(options: Options) {
  const list = get('terminal-list')
  const liveList = get('live-terminals')
  const lostList = get('lost-terminals')
  const unavailable = get<HTMLDetailsElement>('unavailable-terminals')
  const unavailableList = get('unavailable-terminal-list')
  const dialog = get<HTMLDialogElement>('start-work')
  const form = get<HTMLFormElement>('work-create')
  const repo = get<HTMLSelectElement>('work-repository')
  const tree = get<HTMLSelectElement>('work-worktree')
  const preset = get<HTMLSelectElement>('work-preset')
  const model = get<HTMLSelectElement>('work-model')
  const workspaceLink = get<HTMLAnchorElement>('workspace-link')
  const workNote = get('work-note')
  const start = get<HTMLButtonElement>('work-start')
  const newTree = get<HTMLButtonElement>('work-new')
  const create = get<HTMLButtonElement>('work-create-submit')
  const refreshButton = get<HTMLButtonElement>('work-refresh')
  const rename = get<HTMLButtonElement>('terminal-rename')
  const close = get<HTMLButtonElement>('terminal-close')
  const recover = get<HTMLButtonElement>('terminal-recover')
  const manage = get('terminal-manage')
  const mutationControls = [repo, tree, preset, model, create, start, newTree, refreshButton, rename, close, recover]
  let inventory = options.inventory
  let selection: Selection = { preferredWorkspace: options.prefer(), ...fromUrl() }
  let view: WorkView | undefined
  let openTerminal: string | null = null
  let drawId = 0
  let readId = 0
  let navigationId = 0
  let busy = false
  let disposed = false
  let drawnWorkspace: string | null | undefined
  let drawnTerminal: string | null = null
  let replaced: string | undefined
  const ended = new Set<string>()
  let viewing: Viewing | undefined

  // Not twice: the dialog's disabled controls may say the same sentence.
  const noteShows = (words: string) => !!words && dialog.open && words !== get('work-reason').textContent
  const notify = (words: string) => {
    workNote.textContent = words
    workNote.hidden = !noteShows(words)
    options.note(words)
  }

  // A control the view turned off says so in `data-off`, and stays off
  // when a command in flight finishes.
  const off = (control: HTMLElement) => control.dataset.off === 'true'
  const turn = (control: HTMLButtonElement | HTMLSelectElement | HTMLInputElement, reason?: string | null) => {
    control.dataset.off = String(!!reason)
    control.disabled = busy || !!reason
  }

  function lock(on: boolean): void {
    busy = on
    for (const control of mutationControls) control.disabled = on || off(control)
    for (const button of lostList.querySelectorAll('button')) button.disabled = on || off(button)
    shell.set({ busy: on })
    dialog.setAttribute('aria-busy', String(on))
  }

  function why(id: string, reason?: string | null): void {
    const line = get(id)
    line.textContent = reason ?? ''
    line.hidden = !reason
  }

  /** The chosen agent's models; a shell has none. */
  function drawModels(): void {
    const models = view?.start?.presets.find(p => p.id === preset.value)?.models ?? []
    get('work-model-row').hidden = models.length === 0
    choices(model, models)
  }

  function navigate(next: Selection): void {
    if (busy) return
    selection = next
    void project(true, 'pushState')
  }

  async function chooseTerminal(id: string): Promise<boolean> {
    if (busy || !view?.terminals.some(t => t.id === id)) return false
    selection = { workspace: view.workspace, terminal: id }
    try {
      return (await project(true, 'pushState'))?.id === id
    } catch (e) {
      if (!disposed) notify(await options.failure(e as WorkError))
      return false
    }
  }

  async function project(open: boolean, historyMode?: 'pushState' | 'replaceState', replace?: string): Promise<WorkTerminal | undefined> {
    if (!inventory) return
    const revision = ++drawId
    const next = JSON.parse(await options.project(inventory, JSON.stringify({ ...selection, viewing }))) as WorkView
    if (disposed || revision !== drawId) return
    view = next
    const terminal = next.terminals.find(t => t.id === next.terminal)
    const switchPane = open || openTerminal !== next.terminal
    openTerminal = next.terminal
    draw(next, terminal)
    if (open && !options.navigation && selection.worktree && next.worktree === selection.worktree && !dialog.open) dialog.showModal()
    if (historyMode && !options.navigation) {
      const workspace = next.workspace ?? selection.workspace ?? null
      selection.workspace = workspace
      const url = destination(options.runner, workspace, selection.terminal ?? undefined, selection.worktree ?? undefined)
      if (url.href !== location.href) history[historyMode](null, '', url)
    }
    options.select(next, terminal, options.navigation ? open : switchPane, replace, historyMode)
    return terminal
  }

  function draw(view: WorkView, terminal: WorkTerminal | undefined): void {
    if (!options.navigation) {
      shell.set({ spaces: view.workspaces.map(space => ({ id: space.id, label: optionLabel(space, 0) })), space: view.workspace })
    }
    workspaceLink.hidden = !view.workspace
    workspaceLink.textContent = view.labels.link
    workspaceLink.href = destination(options.runner, view.workspace).href
    workspaceLink.onclick = e => follow(e, () => {
      if (options.navigation) options.navigation.workspace(view.workspace)
      else navigate({ workspace: view.workspace })
    })
    choices(repo, view.repositories, view.repository)
    choices(tree, view.worktrees.length ? view.worktrees : [{ id: '', label: view.labels.emptyTree }], view.worktree ?? '')
    get('work-title').textContent = view.labels.open
    get('work-repository-label').textContent = view.labels.repository
    get('work-worktree-label').textContent = view.labels.worktree
    // The page header's filled action (shell.tsx).
    shell.set({ startWork: view.labels.open })
    shell.handle({
      startWork: () => {
        if (disposed || dialog.open) return
        dialog.showModal()
        workNote.hidden = !noteShows(workNote.textContent ?? '')
      },
    })
    action(get('work-review'), options.review ? view.labels.review ?? undefined : undefined, () => {
      if (view.worktree) options.review?.(view.worktree)
    })
    action(get('work-done'), view.labels.close, () => dialog.close())
    action(refreshButton, view.labels.refresh, () => void refresh())
    action(newTree, view.create?.open, () => {
      form.hidden = !form.hidden
      if (!form.hidden) get('work-name').focus()
    })
    if (!view.create || view.create.disabled) form.hidden = true
    turn(newTree, view.create?.disabled)
    turn(create, view.create?.disabled)
    why('work-reason', view.start?.disabled ?? view.create?.disabled)
    if (view.create) {
      get('work-name-label').textContent = view.create.name
      get('work-branch-label').textContent = view.create.branch
      create.textContent = view.create.submit
    }
    get('work-start-fields').hidden = !view.start
    if (view.start) {
      get('work-preset-label').textContent = view.start.preset
      get('work-model-label').textContent = view.start.model
      choices(preset, view.start.presets)
      drawModels()
      for (const control of [preset, model, start]) turn(control, view.start.disabled)
      action(start, view.start.submit, () => {
        void send({ action: 'start', worktree: view.worktree, preset: preset.value, model: model.value })
      })
    }
    const focused = list.contains(document.activeElement) ? document.activeElement as HTMLElement : null
    const liveLinks: HTMLAnchorElement[] = []
    const lostRows: HTMLDivElement[] = []
    const unavailableLinks: HTMLAnchorElement[] = []
    for (const t of view.terminals) {
      const link = document.createElement('a')
      link.className = 'terminal-choice'
      link.title = `${t.title} · ${t.state}`
      if (t.sidebarBadge) {
        const badge = document.createElement('span')
        badge.className = 'terminal-badge'
        badge.textContent = t.sidebarBadge
        link.append(badge)
      }
      if (t.sidebarTitle) {
        const name = document.createElement('span')
        name.textContent = t.sidebarTitle
        link.append(name)
      }
      link.href = destination(options.runner, view.workspace, t.id).href
      link.dataset.terminal = t.id
      if (t.agent) link.dataset.agent = 'true'
      if (t.id === view.terminal) link.setAttribute('aria-current', 'true')
      link.onclick = e => follow(e, () => {
        if (busy) return
        if (options.navigation) options.navigation.terminal(t.id)
        else void chooseTerminal(t.id)
      })
      if (t.live) {
        liveLinks.push(link)
      } else if (t.lost) {
        link.setAttribute('aria-label', t.title)
        link.setAttribute('aria-description', t.state)
        const row = document.createElement('div')
        row.className = 'lost-terminal'
        row.append(link)
        if (t.recover?.action === 'restart') {
          const button = document.createElement('button')
          button.type = 'button'
          button.className = 'quiet'
          button.textContent = t.recover.sidebarLabel ?? ''
          button.setAttribute('aria-label', t.recover.sidebarName ?? '')
          button.dataset.terminal = t.id
          turn(button, t.recover.disabled)
          if (t.recover.disabled) {
            button.title = t.recover.disabled
            button.setAttribute('aria-description', t.recover.disabled)
          }
          button.onclick = () => void send({ action: 'restart', terminal: t.id })
          row.append(button)
        }
        lostRows.push(row)
      } else {
        unavailableLinks.push(link)
      }
    }
    liveList.replaceChildren(...liveLinks)
    liveList.hidden = liveLinks.length === 0
    lostList.replaceChildren(...lostRows)
    lostList.hidden = lostRows.length === 0
    unavailableList.replaceChildren(...unavailableLinks)
    unavailable.hidden = unavailableLinks.length === 0
    if (drawnWorkspace !== view.workspace || unavailable.hidden) unavailable.open = false
    if (terminal && !terminal.live && !terminal.lost && (drawnWorkspace !== view.workspace || drawnTerminal !== terminal.id)) {
      unavailable.open = true
    }
    drawnWorkspace = view.workspace
    drawnTerminal = view.terminal
    if (focused?.dataset.terminal) {
      [...list.querySelectorAll<HTMLElement>('[data-terminal]')].find(control =>
        control.tagName === focused.tagName && control.dataset.terminal === focused.dataset.terminal)?.focus()
    }
    manage.hidden = !terminal
    get('terminal-name').textContent = terminal?.title ?? ''
    const access = get('terminal-access')
    access.textContent = terminal?.access ?? ''
    access.hidden = !terminal?.access
    get('terminal-state').textContent = terminal?.state ?? ''
    action(rename, terminal?.rename?.label, () => {
      if (!terminal?.rename) return
      const name = window.prompt(terminal.rename.prompt, terminal.title)
      if (name !== null) void send({ action: 'rename', terminal: terminal.id, name })
    })
    action(close, terminal?.close?.label, () => {
      if (terminal?.close && window.confirm(terminal.close.prompt)) void send({ action: 'close', terminal: terminal.id })
    })
    action(recover, terminal?.recover?.label, () => {
      const offer = terminal?.recover
      if (!offer) return
      if (offer.action === 'reattach') void refresh(true)
      else if (!offer.confirm || window.confirm(offer.confirm)) void send({ action: offer.action, terminal: terminal.id })
    })
    turn(recover, terminal?.recover?.disabled)
    // Restarting a running orchestrator interrupts it, and a button that's off
    // does nothing: neither is the view's filled action, nor a warning.
    const interrupts = terminal?.recover?.action === 'restart_orchestrator' && terminal.live
    const loud = !!terminal?.recover && !interrupts && !terminal.recover.disabled
    recover.classList.toggle('primary', loud)
    recover.classList.toggle('quiet', !loud)
    get('terminal-recovery').classList.toggle('calm', interrupts)
    shell.set({ terminalLost: loud })
    why('terminal-reason', terminal?.recover?.disabled)
    // The pane under the bar may be saying it already; the button still reads it.
    if (terminal && terminal.recover?.disabled === terminal.attachNote) get('terminal-reason').hidden = true
    // The pane renders Rust's empty and attachment notices in the workspace.
    const paneNotice = options.navigation && !view.missingTerminal && view.workspace && terminal?.recover?.action !== 'reattach'
    notify(paneNotice ? '' : view.notice || terminal?.attachNote || '')
  }

  async function refresh(open = false, historyMode?: 'pushState' | 'replaceState'): Promise<void> {
    const runner = options.current()
    if (!runner || disposed) return
    const revision = ++readId
    try {
      const data = await runner.work_inventory()
      if (disposed || revision !== readId || runner !== options.current()) return
      inventory = data
      if (open) {
        if (selection.terminal) ended.delete(selection.terminal)
        selection.streamEnded = false
      }
      const old = replaced
      await project(open, historyMode, old)
      if (replaced === old) replaced = undefined
    } catch (e) {
      if (!disposed && revision === readId) notify(await options.failure(e as WorkError))
    }
  }

  async function send(intent: Intent): Promise<void> {
    const runner = options.current()
    if (!runner || busy || disposed) return
    const navigation = navigationId
    const destination = { workspace: view?.workspace, worktree: view?.worktree }
    const restarts = intent.action === 'restart' || intent.action === 'restart_orchestrator'
    const pushHistory = intent.action === 'start' || (restarts && intent.terminal !== view?.terminal)
    lock(true)
    notify('')
    try {
      const id = await runner.work_intent(JSON.stringify(intent))
      if (disposed) return
      if (intent.action === 'close' || restarts) replaced = intent.terminal
      if (navigation !== navigationId) {
        await refresh()
        return
      }
      switch (intent.action) {
        case 'create':
          selection = { workspace: intent.workspace, repository: intent.repository, worktree: id }
          form.reset()
          form.hidden = true
          break
        case 'start':
        case 'restart':
        case 'restart_orchestrator':
          if (intent.action === 'start') options.started()
          selection = { workspace: destination.workspace, terminal: id }
          if (dialog.open) dialog.close()
          break
        case 'close':
          selection = destination
          break
      }
      const opensTerminal = intent.action === 'start' || restarts
      await refresh(opensTerminal, pushHistory ? 'pushState' : 'replaceState')
    } catch (e) {
      if (!disposed) notify(await options.failure(e as WorkError))
    } finally {
      if (!disposed) lock(false)
    }
  }

  preset.onchange = () => drawModels()
  if (!options.navigation) shell.handle({ pickSpace: workspace => navigate({ workspace }) })
  repo.onchange = () => navigate({ repository: repo.value })
  tree.onchange = () => {
    selection = { ...selection, worktree: tree.value }
    void project(false)
  }
  form.onsubmit = e => {
    e.preventDefault()
    if (!view?.create) return
    void send({
      action: 'create', repository: view.repository, workspace: view.workspace,
      name: get<HTMLInputElement>('work-name').value, branch: get<HTMLInputElement>('work-branch').value,
    })
  }
  function pop(): void {
    if (linkedRunner() && linkedRunner() !== options.runner) return location.reload()
    if (dialog.open) dialog.close()
    selection = { ...fromUrl(), preferredWorkspace: options.prefer() }
    void project(true)
  }
  if (!options.navigation) window.addEventListener('popstate', pop)

  return {
    refresh,
    chooseTerminal,
    /** Whether this browser watches `id` without being its writer. */
    viewing(id: string, reason: string | null) {
      const next = reason ? { terminal: id, reason } : undefined
      if (!next && viewing?.terminal !== id) return
      if (JSON.stringify(next) === JSON.stringify(viewing)) return
      viewing = next
      if (view?.terminal === id) void project(false)
    },
    async show(workspace: string | null, terminal?: string, worktree?: string) {
      navigationId++
      if (selection.worktree && selection.worktree !== worktree && dialog.open) dialog.close()
      if (terminal) ended.delete(terminal)
      selection = { workspace, terminal, worktree }
      await project(false)
      if (!disposed && selection.worktree === worktree && worktree && view?.worktree === worktree && !dialog.open) {
        dialog.showModal()
      }
    },
    focusTerminal(id: string | null) {
      if (!view || (selection.terminal ?? view.terminal) === id) return
      selection = { ...selection, workspace: view.workspace, terminal: id, streamEnded: id !== null && ended.has(id) }
      void project(false)
    },
    hasTerminal: (id: string) => !!view?.terminals.some(t => t.id === id),
    streamEnded(id: string) {
      ended.add(id)
      if (id !== view?.terminal) return
      selection = { ...selection, terminal: id, streamEnded: true }
      void refresh()
    },
    dispose() {
      disposed = true
      window.removeEventListener('popstate', pop)
      if (dialog.open) dialog.close()
      liveList.replaceChildren()
      lostList.replaceChildren()
      lostList.hidden = true
      unavailableList.replaceChildren()
      unavailable.hidden = true
      unavailable.open = false
      workspaceLink.hidden = true
      manage.hidden = true
      recover.hidden = true
      why('terminal-reason')
      lock(false)
      shell.set({ startWork: null, terminalLost: false, busy: false })
      shell.handle({ startWork: () => {} })
    },
  }
}
