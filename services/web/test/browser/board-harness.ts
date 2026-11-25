import { WorkflowSessionPort } from '../../src/client/workflow-session'
import { mountBoard } from '../../src/client/board-screen'
import { workspaceUrl, WorkspaceHistory, type Route } from '../../src/client/workspace-history'

const get = (id: string) => document.getElementById(id)!
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
async function main(): Promise<void> {
  const form = await call({ method: 'form' })
  const current = {
    web_review: (workspace: string, task: string, request: string) =>
      call({ method: 'review', workspace, task, request: JSON.parse(request) }),
    workflow_call: (method: string, args: string) => call({ method, args: JSON.parse(args) }),
    board: (workspace: string, repository: string) => call({ method: 'board', workspace, repository }),
    board_ticket: (workspace: string, task: string) => call({ method: 'ticket', workspace, task }),
    board_intent: (workspace: string, intent: string) => call({ method: 'intent', workspace, ...JSON.parse(intent) }),
  }
  const runner = 'canary'
  const params = new URL(location.href).searchParams
  const workspace = params.get('workspace') ?? 'delivery'
  const repo = params.get('repo') ?? sessionStorage.getItem('fixture-repo') ?? 'repo'
  sessionStorage.setItem('fixture-repo', repo)
  const boardRoute = (id?: string): Route => ({ space: workspace, page: 'board', tab: 'board', boardTask: id })
  const terminalRoute = (id: string, kind: 'terminal' | 'worktree'): Route =>
    ({ space: workspace, page: 'terminals', tab: 'terminals', [kind]: id })
  const screen = mountBoard({
    root: get('board'),
    dialog: get('board-form') as HTMLDialogElement,
    current: () => current,
    create: () => form,
    failure: error => error.message,
    href: (id, kind) => workspaceUrl(runner, kind && id ? terminalRoute(id, kind) : boardRoute(id)),
    openTicket: id => history.navigate(boardRoute(id)),
    openAgent: id => history.navigate(terminalRoute(id, 'terminal')),
    openWorktree: id => history.navigate(terminalRoute(id, 'worktree')),
    openActivity: () => {},
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
  get('runner-name').textContent = 'Far Cooler Canary'
  get('space-picker').replaceChildren(new Option('Web first · Far Cooler development', workspace))
  get('space-picker').hidden = false
  get('status').hidden = true
  get('work').hidden = false
  get('terminal-list').hidden = true
  get('show-orchestrator').textContent = 'Orchestrator'
  get('show-commands').textContent = 'Commands'
  get('show-board').textContent = 'Board'
  get('show-board').removeAttribute('disabled')
  get('show-board').onclick = () => history.navigate(boardRoute())
  const render = (route: Route) => {
    const board = route.page === 'board'
    get('pane').hidden = board
    get('board').hidden = !board
    get('show-terminals').setAttribute('aria-pressed', String(!board))
    get('show-board').setAttribute('aria-pressed', String(board))
    screen.show(board, route.space, repo, route.boardTask, route.review)
  }
  const history = new WorkspaceHistory('board-check', runner, boardRoute(), route => route, render)
  render(history.route)
}
void main()
