import { WorkflowSessionPort } from '../../src/client/workflow-session'
import { mountBoard } from '../../src/client/board-screen'
import { DetailBoard } from '../../src/client/detail'
import { DetailSheet } from '../../src/client/detail-view'
import { workspaceUrl, WorkspaceHistory, type Route } from '../../src/client/workspace-history'
import { WorkspaceLayout } from '../../src/client/workspace-layout'
import type { WorkView } from '../../src/client/work-screen'
import { harnessShell, runnerNamed } from './harness-shell'
import { shell } from '../../src/client/shell/store'

const get = (id: string) => document.getElementById(id)!
const runnerName = 'Far Cooler Canary'
const call = async (args: unknown): Promise<string> => {
  const response = await fetch('/board-rpc', { method: 'POST', body: JSON.stringify(args) })
  if (!response.ok) {
    const raw = await response.text()
    if (response.headers.get('content-type') === 'application/json') {
      const error = JSON.parse(raw)
      throw Object.assign(new Error(error.error), error)
    }
    throw new Error(raw)
  }
  return await response.text()
}

// Like WASM, the native layout projector answers synchronously. The browser
// fixture routes this request straight to the host build of the projector.
function projectLayout(request: string): string {
  const rpc = new XMLHttpRequest()
  rpc.open('POST', '/board-rpc', false)
  rpc.send(JSON.stringify({ method: 'layout', request: JSON.parse(request) }))
  if (rpc.status !== 200) throw new Error(rpc.responseText)
  return rpc.responseText
}

function mountLayout(work: WorkView): WorkspaceLayout {
  const layout = new WorkspaceLayout('board-check', runnerName, projectLayout, view => {
    const label = (id: string) => view.actions.find(item => item.id === id)!.label
    shell.command('nav', 'board', { label: label('board'), disabled: false })
    for (const tool of ['orchestrator', 'workflow', 'commands'] as const) shell.command('tools', tool, { label: label(tool) })
    shell.set({ startWork: !work.create && !work.start ? null : label('start_work') })
    for (const [id, command] of [
      ['close-split', 'close_split'], ['side-conversation', 'conversation'], ['side-terminal', 'raw_terminal'],
    ] as const) {
      const action = view.actions.find(item => item.id === command)!
      get(id).textContent = action.label
    }
    const side = get('workspace-side')
    side.hidden = !view.side
    if (view.side) {
      side.setAttribute('aria-label', view.side.title)
      get('side-title').textContent = view.side.title
      get('side-switch').hidden = !view.side.attach
      get('side-conversation').setAttribute('aria-pressed', String(view.side.mode === 'conversation'))
    }
  }, effect => {
    if (effect.kind === 'focus') get(`workspace-${effect.slot}`).focus({ preventScroll: true })
  })
  shell.handle({ tool: tool => layout.run(tool) })
  get('close-split').onclick = () => layout.run('close_split')
  return layout
}

async function main(): Promise<void> {
  harnessShell()
  const current = {
    web_review: (workspace: string, task: string, request: string) =>
      call({ method: 'review', workspace, task, request: JSON.parse(request) }),
    workflow_call: (method: string, args: string) => call({ method, args: JSON.parse(args) }),
    board: (workspace: string, repository: string) => call({ method: 'board', workspace, repository }),
    board_ticket: (workspace: string, task: string) => call({ method: 'ticket', workspace, task }),
    board_intent: async (workspace: string, intent: string) => {
      await call({ method: 'intent', workspace, ...JSON.parse(intent) })
    },
  }
  const runner = 'canary'
  const params = new URL(location.href).searchParams
  const workspace = params.get('workspace') ?? 'delivery'
  const repo = params.get('repo') ?? sessionStorage.getItem('fixture-repo') ?? 'repo'
  sessionStorage.setItem('fixture-repo', repo)
  const work: WorkView | null = params.has('layout') ? JSON.parse(await call({ method: 'work', workspace, repo })) : null
  const layout = work ? mountLayout(work) : null
  const boardRoute = (id?: string): Route => ({ space: workspace, page: 'board', tab: 'board', boardTask: id })
  const terminalRoute = (id: string, kind: 'terminal' | 'worktree'): Route =>
    ({ space: workspace, page: 'terminals', tab: 'terminals', [kind]: id })
  const sheet = new DetailSheet(get('detail') as HTMLDialogElement, get, () => 'Waiting on You', () => sheet.close())
  const screen = mountBoard({
    root: get('board'),
    current: () => current,
    failure: error => error.message,
    href: (id, kind) => workspaceUrl(runner, kind && id ? terminalRoute(id, kind) : boardRoute(id)),
    openTicket: id => history.navigate(boardRoute(id)),
    openAgent: id => history.navigate(terminalRoute(id, 'terminal')),
    openWorktree: id => history.navigate(terminalRoute(id, 'worktree')),
    openActivity: card => {
      const subject = { runner, record: true, ...card }
      sheet.open(new DetailBoard({
        runner: runnerName,
        task: task => call({ method: 'record', task }),
        addNote: async () => { throw new Error('a card from the board adds no note') },
        show: view => sheet.draw(view),
      }, subject), () => [], '', () => {})
    },
    review: {
      current: () => current,
      navigate: (ticket, review) => history.navigate({ ...boardRoute(ticket), review }),
      resolve: (ticket, review) => history.rememberReview(ticket, review),
      back: ticket => history.navigate(boardRoute(ticket)),
      workflow: new WorkflowSessionPort(() => ({ account: 'fixture', host: runner, session: current }), localStorage),
      openWorkflow: () => {},
      approvalResult: raw => JSON.parse(raw).receipt.result.Ok !== undefined ? 'Approval accepted' : 'Approval refused',
      failure: error => error.what === 'review_stale' ? 'The candidate or diff changed. Refresh before continuing.' : error.message,
      storageKey: 'fixture-review:',
    },
  })
  shell.set({
    runner: runnerNamed(runnerName, runner),
    spaces: [{ id: workspace, label: 'Web first · Far Cooler development' }],
    space: workspace,
  })
  shell.command('nav', 'board', { label: 'Board', disabled: false })
  if (layout) shell.command('nav', 'needs', { label: 'Needs You (123)' })
  shell.handle({ showTab: tab => { if (tab === 'board') history.navigate(boardRoute()) } })
  get('status').hidden = true
  get('work').hidden = false
  get('terminal-list').hidden = true
  const render = (route: Route) => {
    const board = route.page === 'board'
    get('pane').hidden = board
    get('board').hidden = !board
    shell.set({ page: board ? 'board' : 'terminals' })
    if (work) layout?.contextFor(route.space, route.page, work, 'seat')
    screen.show(board, route.space, repo, route.boardTask, route.review)
  }
  const history = new WorkspaceHistory('board-check', runner, boardRoute(), route => route, render)
  render(history.route)
}
void main()
