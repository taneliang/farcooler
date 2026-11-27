import init, { workFailure, workView, type WebRunner } from 'fc-web'
import { PaneSet } from '../../src/client/panes'
import { PaneView } from '../../src/client/terminal'
import { TerminalWriter } from '../../src/client/terminal-writer'
import { mountWork } from '../../src/client/work-screen'
import { harnessShell, runnerNamed } from './harness-shell'

const runner = 'paired-runner'
const repository = 'platform'
const workspace = 'delivery'
const shellTree = 'release'
const lost = 'lost-agent'
const title = 'Bring back the observability and deployment compatibility agent'
const branch = 'ov-1444-web-start-recover-work'
const params = new URLSearchParams(location.search)
const sidebar = params.has('sidebar')
const roots = sidebar ? [
  { id: repository, label: 'overnight' },
  { id: 'client', label: 'Verdela' },
] : [
  { id: repository, label: 'Platform Infrastructure and Deployment Automation' },
  { id: 'client', label: 'Cross-platform Far Cooler Client Applications' },
]
const workspaces = sidebar ? [
  { id: workspace, name: 'Main', repository, isMain: true, orchestrator: 'seat' },
  { id: 'next', name: 'Main', repository: 'client', isMain: true, orchestrator: 'other-seat' },
] : [
  { id: workspace, name: 'Deployment Safety and Release Engineering', repository, isMain: true },
  { id: 'next', name: 'Cross-platform application delivery and release coordination', repository: 'client', isMain: true },
]
type Term = { id: string; title: string; worktree: string; workspace: string; agent: boolean; state: string; columns: number; rows: number; orchestrator?: boolean }
type Tree = { id: string; label: string; repository: string; workspace: string }
const tree: Tree = { id: shellTree, label: 'Web: start and recover work on a paired runner · ' + branch, repository, workspace }
const terminal: Term = { id: lost, title, worktree: shellTree, workspace, agent: true, state: 'lost', columns: 80, rows: 24 }
const shell: Term = { ...terminal, id: 'shell', title: 'Check the release logs and reconcile the deployment manifests', agent: false, state: 'running' }
const orchestrator: Term = { ...terminal, title: 'orchestrator', orchestrator: true }
const seed = sidebar ? { worktrees: [tree], terminals: [
  { ...shell, title: 'Web: permanent orchestrator and workstation navigation' },
  { ...orchestrator, id: 'lost-seat' },
  { ...orchestrator, id: 'exited-seat', state: 'exited' },
  { ...shell, id: 'ended', title: 'Release log tail', state: 'exited' },
  { ...orchestrator, id: 'seat', state: 'running' },
  { ...orchestrator, id: 'other-seat', workspace: 'next', state: 'running' },
]} : { worktrees: [tree], terminals: [
  shell,
  terminal,
] }
let data: typeof seed = JSON.parse(localStorage.getItem('test-work') ?? JSON.stringify(seed))
const scope = params.has('read') ? 'read' : 'control'
const calls: unknown[] = []
const client = crypto.randomUUID()
Object.assign(window, { calls })

let dropStream: (() => void) | undefined
const fake = {
  terminal_writer_supported: () => scope === 'control',
  terminal_writer: async (terminal: string, action: string) => {
    calls.push({ action, terminal })
    const response = await fetch('/writer', { method: 'POST', body: JSON.stringify({ client, terminal, action }) })
    return response.text()
  },
  terminals: async () => JSON.stringify(data.terminals),
  attach: async (terminal: string, output: (bytes: Uint8Array) => void, ended: () => void) => {
    calls.push({ action: 'attach', terminal })
    dropStream = ended
    output(new TextEncoder().encode('$ git status --short\r\n$ git branch --show-current\r\n' + branch + '\r\n$ '))
    return { write: async (bytes: Uint8Array) => {
      const sent = await fetch('/writer', {
        method: 'POST', body: JSON.stringify({ client, terminal, action: 'write' }),
      })
      if (!sent.ok) throw new Error('viewer input was refused')
      calls.push({ action: 'write', terminal, text: new TextDecoder().decode(bytes) })
    }, resize: async () => {}, close() {}, free() {} }
  },
  work_inventory: async () => JSON.stringify({
    ...data, scope, workspaces, repositories: roots,
    ...(params.has('agents') ? { agents: ['claude', 'codex'] } : {}),
    // A member who doesn't lead, as the runner marks the seat for them.
    ...(params.has('member') ? { terminals: data.terminals.map(t => t.id === 'seat' ? { ...t, restartRefusal: 'not_lead' } : t) } : {}),
  }),
  work_intent: async (json: string): Promise<string | undefined> => {
    const arg = JSON.parse(json)
    calls.push(arg)
    if (params.has('refuse')) throw Object.assign(new Error('refused'), { code: 'scope-denied' })
    let result: string | undefined
    switch (arg.action) {
      case 'create':
        result = 'created'
        data.worktrees.push({ id: result, label: arg.name, repository: arg.repository, workspace: arg.workspace })
        break
      case 'start':
        result = arg.preset
        data.terminals.unshift({ ...terminal, id: result!, title: arg.preset, state: 'running', worktree: arg.worktree, workspace: data.worktrees.find(w => w.id === arg.worktree)!.workspace, agent: arg.preset !== 'shell' })
        break
      case 'rename':
        data.terminals.find(t => t.id === arg.terminal)!.title = arg.name
        break
      case 'close':
        data.terminals = data.terminals.filter(t => t.id !== arg.terminal)
        break
      case 'restart':
        result = 'recovered'
        Object.assign(data.terminals.find(t => t.id === arg.terminal)!, { id: result, state: 'running' })
        break
      case 'restart_orchestrator':
        result = arg.terminal
        break
    }
    localStorage.setItem('test-work', JSON.stringify(data))
    return result
  },
}

await init({ module_or_path: '/farcooler_web.wasm' })
const stage = document.getElementById('terminal')!
function writerConnection() {
  let freed = false
  return {
    terminal_writer_supported() {
      if (freed) throw new Error('null pointer passed to rust')
      return fake.terminal_writer_supported()
    },
    terminal_writer(terminal: string, action: string) {
      if (freed) throw new Error('null pointer passed to rust')
      return fake.terminal_writer(terminal, action)
    },
    free() { freed = true },
  }
}
let writerRunner = writerConnection()
const writers = new Map<string, { root: HTMLDivElement; view: TerminalWriter }>()
const panes = new PaneSet<WebRunner>(id => {
  const root = document.createElement('div')
  root.className = 'workspace-screen'
  const content = document.createElement('div')
  content.className = 'workspace-screen-content'
  const host = document.createElement('div')
  host.className = 'terminal-host'
  content.append(host)
  root.append(content)
  stage.append(root)
  const pane = new PaneView(host, word => {
    const note = document.getElementById('pane-note')!
    note.textContent = workFailure(word, '', '')
    note.hidden = false
  })
  writers.set(id, {
    root,
    view: new TerminalWriter(root, id, input => pane.setInput(input), explain,
      connection => connection === writerRunner, reason => queueMicrotask(() => screen.viewing(id, reason))),
  })
  return pane
})
harnessShell({ runner: runnerNamed('Production macOS runner for Continuous Delivery and release verification', runner) })
document.getElementById('status')!.hidden = true
document.getElementById('work')!.hidden = false
const screen = mountWork({
  runner,
  current: () => fake,
  project: workView,
  select: (_view, terminal, open, replace) => {
    if (replace) {
      panes.forget(replace)
      writers.get(replace)?.view.show(null)
      writers.get(replace)?.root.remove()
      writers.delete(replace)
    }
    if (!terminal?.attach) panes.clear()
    else if (open) void panes.open(terminal.id, fake as unknown as WebRunner, () => screen.streamEnded(terminal.id))
    for (const [id, { root, view }] of writers) {
      root.hidden = id !== terminal?.id || !terminal.attach
      view.show(root.hidden ? null : writerRunner)
    }
  },
  started: () => {},
  note: (words: string) => {
    const note = document.getElementById('pane-note')!
    note.hidden = !words
    note.textContent = words
  },
  prefer: () => null,
  failure: explain,
})
window.addEventListener('pagehide', () => {
  for (const [id] of writers) {
    // The test endpoint lacks an SSH connection to close for a navigated tab.
    navigator.sendBeacon('/writer', JSON.stringify({ client, terminal: id, action: 'release' }))
  }
})
function reconnectWriter(): void {
  writerRunner.free()
  writerRunner = writerConnection()
  for (const { root, view } of writers.values()) {
    if (!root.hidden) view.show(writerRunner)
  }
}
Object.assign(window, { dropStream: () => dropStream?.(), chooseTerminal: screen.chooseTerminal, reconnectWriter,
  writerChanged: (terminal: string) => writers.get(terminal)?.view.refresh(),
})
void screen.refresh(true, 'replaceState')
function explain(error: Error & { code?: string }): string {
  return workFailure(error.message, error.code ?? '', '')
}
