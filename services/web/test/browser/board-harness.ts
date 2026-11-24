import { mountBoard } from '../../src/client/board-screen'
import { workspaceUrl, WorkspaceHistory, type Route } from '../../src/client/workspace-history'

const get = (id: string) => document.getElementById(id)!
const call = async (args: unknown): Promise<string> => {
  const response = await fetch('/board-rpc', { method: 'POST', body: JSON.stringify(args) })
  if (!response.ok) throw new Error(await response.text())
  return await response.text()
}
async function main(): Promise<void> {
  const form = await call({ method: 'form' })
  const current = {
    board: (workspace: string, repository: string) => call({ method: 'board', workspace, repository }),
    board_ticket: (workspace: string, task: string) => call({ method: 'ticket', workspace, task }),
    board_intent: (workspace: string, intent: string) => call({ method: 'intent', workspace, ...JSON.parse(intent) }),
  }
  const runner = 'canary'
  const workspace = new URL(location.href).searchParams.get('workspace') ?? 'delivery'
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
  })
  get('runner-name').textContent = 'Far Cooler Canary'
  get('space-picker').replaceChildren(new Option('Web first · Far Cooler development', workspace))
  get('space-picker').hidden = false
  get('status').hidden = true
  get('work').hidden = false
  get('show-board').textContent = 'Board'
  get('show-board').removeAttribute('disabled')
  get('show-board').onclick = () => history.navigate(boardRoute())
  const render = (route: Route) => {
    const board = route.page === 'board'
    get('pane').hidden = board
    get('board').hidden = !board
    get('show-terminals').setAttribute('aria-pressed', String(!board))
    get('show-board').setAttribute('aria-pressed', String(board))
    screen.show(board, route.space, 'repo', route.boardTask)
  }
  const history = new WorkspaceHistory('board-check', runner, boardRoute(), route => route, render)
  render(history.route)
}
void main()
