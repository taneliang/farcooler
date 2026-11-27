/// shell-check.mjs's page: the React shell (ov-1510) over index.html's parked
/// views, with long real names, a terminal list and the Start Work and
/// Commands dialogs wired the way app.ts and work-screen.ts wire them. What
/// each control asked for is in `window.asked`.

import { harnessShell, runnerNamed } from './harness-shell'
import { shell, type ShellTab } from '../../src/client/shell/store'

const asked: unknown[] = []
const get = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T
const params = new URLSearchParams(location.search)
const page = (params.get('page') ?? 'board') as ShellTab

harnessShell({
  runner: { ...runnerNamed('Production macOS runner for Continuous Delivery and release verification', 'studio'), health: 'Connected', runtime: 'Runner health unavailable' },
  runners: [
    { id: 'studio', label: 'Production macOS runner for Continuous Delivery and release verification' },
    { id: 'build-box', label: 'Linux build box in the basement' },
    { id: 'gone', label: 'Retired laptop', disabled: true },
  ],
  spaces: [
    { id: 'delivery', label: 'Deployment Safety and Release Engineering · Platform Infrastructure and Deployment Automation (2)' },
    { id: 'client', label: 'Cross-platform application delivery and release coordination · Far Cooler Client Applications' },
    { id: 'sandbox', label: 'Sandbox · Studio' },
  ],
  space: 'delivery',
  repository: 'Platform Infrastructure and Deployment Automation',
  page,
  startWork: 'Start Work',
  hrefs: { terminals: '?view=terminals', needs: '?view=needs', board: '?view=board', plan: '?view=plan' },
})
shell.command('nav', 'board', { label: 'Board', disabled: false, keys: 'Alt+Shift+B' })
shell.command('nav', 'needs', { label: 'Needs You (12)' })

shell.handle({
  showTab: tab => {
    asked.push(['tab', tab])
    shell.set({ page: tab })
    show(tab)
  },
  pickSpace: id => {
    asked.push(['space', id])
    shell.set({ space: id })
  },
  pickRunner: id => asked.push(['runner', id]),
  addRunner: () => asked.push(['add runner']),
  tool: tool => {
    asked.push(['tool', tool])
    if (tool === 'commands') commands()
  },
  startWork: () => {
    asked.push(['start work'])
    const dialog = get<HTMLDialogElement>('start-work')
    if (!dialog.open) dialog.showModal()
  },
})

// The terminal list, as work-screen.ts draws it.
const live = get('live-terminals')
for (const [id, title] of [['seat', 'orchestrator'], ['shell', 'Check the release logs and reconcile the deployment manifests'], ['agent', 'Bring back the observability and deployment compatibility agent']]) {
  const link = document.createElement('a')
  link.className = 'terminal-choice'
  link.href = `?terminal=${id}`
  link.dataset.terminal = id
  const name = document.createElement('span')
  name.textContent = title
  link.append(name)
  link.title = title
  if (id === 'seat') link.setAttribute('aria-current', 'true')
  link.onclick = event => { event.preventDefault(); asked.push(['terminal', id]) }
  live.append(link)
}
const workspaceLink = get<HTMLAnchorElement>('workspace-link')
workspaceLink.hidden = false
workspaceLink.textContent = 'Workspace Link'
workspaceLink.href = '?workspace=delivery'
get('who').textContent = 'owner.with.a.long.address@example.com'
get('bar-count').hidden = false
get('bar-count').textContent = '12 items need your attention'

// The Start Work dialog's fields, as work-screen.ts labels them.
get('work-title').textContent = 'Start Work'
get('work-repository-label').textContent = 'Repository'
get('work-worktree-label').textContent = 'Worktree'
get('work-repository').replaceChildren(new Option('Platform Infrastructure and Deployment Automation', 'platform'))
get('work-worktree').replaceChildren(new Option('main', 'main'))
get('work-done').textContent = 'Done'
get('work-done').onclick = () => get<HTMLDialogElement>('start-work').close()
get('work-refresh').textContent = 'Refresh'

// The Commands palette, as workspace-layout.ts builds it.
function commands(): void {
  const dialog = get<HTMLDialogElement>('workspace-commands')
  const title = document.createElement('h2')
  title.textContent = 'Commands'
  const close = document.createElement('button')
  close.type = 'button'
  close.className = 'quiet'
  close.textContent = 'Close'
  close.onclick = () => dialog.close()
  const split = document.createElement('button')
  split.type = 'button'
  split.textContent = 'Split Pane'
  dialog.replaceChildren(title, split, close)
  dialog.setAttribute('aria-label', 'Commands')
  dialog.showModal()
}

function show(tab: ShellTab): void {
  for (const id of ['needs', 'plan', 'board'] as const) get(id).hidden = id !== tab
  get('pane').hidden = tab !== 'terminals'
}

// The board, with enough of its own markup to read as one.
const heading = document.createElement('h2')
heading.textContent = 'Ready'
const card = document.createElement('button')
card.type = 'button'
card.className = 'quiet'
card.textContent = 'ov-1510 Web: React shell on the new design system'
get('board').replaceChildren(heading, card)
get('status').hidden = true
get('work').hidden = false
show(page)
Object.assign(window, { asked })
