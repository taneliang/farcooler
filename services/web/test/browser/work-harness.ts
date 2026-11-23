import type { WebRunner } from 'fc-web'
import { PaneSet } from '../../src/client/panes'
import { PaneView } from '../../src/client/terminal'
import { mountWork } from '../../src/client/work-screen'

const runner = 'paired-runner'
const repository = 'platform'
const workspace = 'delivery'
const shellTree = 'release'
const lost = 'lost-agent'
const title = 'Bring back the observability and deployment compatibility agent'
const branch = 'ov-1444-web-start-recover-work'
const roots = [
  { id: repository, label: 'Platform Infrastructure and Deployment Automation' },
  { id: 'client', label: 'Cross-platform Far Cooler Client Applications' },
]
const workspaces = [
  { id: workspace, name: 'Deployment Safety and Release Engineering', repository, isMain: true },
  { id: 'next', name: 'Cross-platform application delivery and release coordination', repository: 'client', isMain: true },
]
type Term = { id: string; title: string; worktree: string; workspace: string; agent: boolean; state: string; columns: number; rows: number }
type Tree = { id: string; label: string; repository: string; workspace: string }
const tree: Tree = { id: shellTree, label: 'Web: start and recover work on a paired runner · ' + branch, repository, workspace }
const terminal: Term = { id: lost, title, worktree: shellTree, workspace, agent: true, state: 'lost', columns: 80, rows: 24 }
const seed = { worktrees: [tree], terminals: [
  { ...terminal, id: 'shell', title: 'Check the release logs and reconcile the deployment manifests', agent: false, state: 'running' },
  terminal,
] }
let data: typeof seed = JSON.parse(localStorage.getItem('test-work') ?? JSON.stringify(seed))
const params = new URLSearchParams(location.search)
const scope = params.has('read') ? 'read' : 'control'
const calls: unknown[] = []
Object.assign(window, { calls })

let dropStream: (() => void) | undefined
const fake = {
  terminals: async () => JSON.stringify(data.terminals),
  attach: async (terminal: string, output: (bytes: Uint8Array) => void, ended: () => void) => {
    calls.push({ action: 'attach', terminal })
    dropStream = ended
    output(new TextEncoder().encode('$ git status --short\r\n$ git branch --show-current\r\n' + branch + '\r\n$ '))
    return { write: async () => {}, resize: async () => {}, close() {}, free() {} }
  },
  work_inventory: async () => JSON.stringify({ ...data, scope, workspaces, repositories: roots }),
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
    }
    localStorage.setItem('test-work', JSON.stringify(data))
    return result
  },
}

const stage = document.getElementById('terminal')!
const panes = new PaneSet<WebRunner>(() => {
  const host = document.createElement('div')
  host.className = 'terminal-host'
  stage.append(host)
  return new PaneView(host, word => { throw new Error(word) })
})
document.getElementById('status')!.hidden = true
document.getElementById('work')!.hidden = false
document.getElementById('runner-name')!.textContent = 'Production macOS runner for Continuous Delivery and release verification'
const screen = mountWork({
  runner,
  current: () => fake,
  project: async (inventory: string, selection: string) => {
    const reply = await fetch('/projection', { method: 'POST', body: JSON.stringify([JSON.parse(inventory), JSON.parse(selection)]) })
    return await reply.text()
  },
  select: (_view, terminal, open, replace) => {
    if (replace) panes.forget(replace)
    if (!terminal?.attach) panes.clear()
    else if (open) void panes.open(terminal.id, fake as unknown as WebRunner, () => screen.streamEnded(terminal.id))
  },
  started: () => {},
  note: (words: string) => {
    const note = document.getElementById('pane-note')!
    note.hidden = !words
    note.textContent = words
  },
  prefer: () => null,
  failure: (error: Error & { code?: string }) => fakeFailure(error),
})
Object.assign(window, { dropStream: () => dropStream?.() })
void screen.refresh(true, 'replaceState')
async function fakeFailure(error: Error & { code?: string }): Promise<string> {
  const reply = await fetch('/failure', { method: 'POST', body: JSON.stringify([error.message, error.code ?? '', '']) })
  return await reply.text()
}
