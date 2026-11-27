/// Browser navigation and rendering for the shared Rust work view.
import { type Space } from './scope'
import { shell } from './shell/store'
import { optionLabel } from './scope'

interface Choice { id: string; label: string }
interface Prompt { label: string; prompt: string }

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
  recover: { action: 'restart' | 'reattach'; label: string; sidebarLabel?: string; sidebarName?: string } | null
}

export interface WorkView {
  workspaces: Space[]
  repositories: Choice[]
  worktrees: Choice[]
  workspace: string | null
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
  create: { open: string; submit: string; name: string; branch: string } | null
  start: { preset: string; submit: string; presets: Choice[] } | null
}

interface Selection {
  workspace?: string | null
  preferredWorkspace?: string | null
  repository?: string | null
  worktree?: string | null
  terminal?: string | null
  streamEnded?: boolean
}

type Intent =
  | { action: 'create'; repository: string | null; workspace: string | null; name: string; branch: string }
  | { action: 'start'; worktree: string | null; preset: string }
  | { action: 'rename'; terminal: string; name: string }
  | { action: 'close' | 'restart'; terminal: string }

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

function choices(select: HTMLSelectElement, items: Choice[], chosen?: string | null): void {
  const signature = JSON.stringify(items)
  if (select.dataset.choices !== signature) {
    select.replaceChildren(...items.map(({ id, label }) => new Option(label, id)))
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
  const mutationControls = [repo, tree, preset, create, start, newTree, refreshButton, rename, close, recover]
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

  const notify = (words: string) => {
    workNote.hidden = !words || !dialog.open
    workNote.textContent = words
    options.note(words)
  }

  function lock(on: boolean): void {
    busy = on
    for (const control of mutationControls) control.disabled = on
    for (const button of lostList.querySelectorAll('button')) button.disabled = on
    shell.set({ busy: on })
    dialog.setAttribute('aria-busy', String(on))
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
    const next = JSON.parse(await options.project(inventory, JSON.stringify(selection))) as WorkView
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
        workNote.hidden = !workNote.textContent
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
    if (!view.create) form.hidden = true
    if (view.create) {
      get('work-name-label').textContent = view.create.name
      get('work-branch-label').textContent = view.create.branch
      create.textContent = view.create.submit
    }
    get('work-start-fields').hidden = !view.start
    if (view.start) {
      get('work-preset-label').textContent = view.start.preset
      choices(preset, view.start.presets)
      action(start, view.start.submit, () => {
        void send({ action: 'start', worktree: view.worktree, preset: preset.value })
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
          button.disabled = busy
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
    shell.set({ terminalLost: !!terminal?.recover })
    action(recover, terminal?.recover?.label, () => {
      if (!terminal?.recover) return
      if (terminal.recover.action === 'restart') void send({ action: 'restart', terminal: terminal.id })
      else void refresh(true)
    })
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
    const pushHistory = intent.action === 'start' || (intent.action === 'restart' && intent.terminal !== view?.terminal)
    lock(true)
    notify('')
    try {
      const id = await runner.work_intent(JSON.stringify(intent))
      if (disposed) return
      if (intent.action === 'close' || intent.action === 'restart') replaced = intent.terminal
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
          if (intent.action === 'start') options.started()
          selection = { workspace: destination.workspace, terminal: id }
          if (dialog.open) dialog.close()
          break
        case 'close':
          selection = destination
          break
      }
      const opensTerminal = intent.action === 'start' || intent.action === 'restart'
      await refresh(opensTerminal, pushHistory ? 'pushState' : 'replaceState')
    } catch (e) {
      if (!disposed) notify(await options.failure(e as WorkError))
    } finally {
      if (!disposed) lock(false)
    }
  }

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
      lock(false)
      shell.set({ startWork: null, terminalLost: false, busy: false })
      shell.handle({ startWork: () => {} })
    },
  }
}
