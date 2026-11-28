import type { ReviewRoute } from './review-screen'
import { destination, linkedRunner } from './work-screen'
import type { Page, Slot, Tab } from './workspace-layout'

export interface Route {
  space: string | null
  page: Page
  tab: Tab
  terminal?: string
  worktree?: string
  boardTask?: string
  review?: ReviewRoute
  slot?: Slot
  ticket?: { runner: string; id: string; key: string; title: string; record?: boolean }
}

export function workspaceUrl(runner: string, route: Route): URL {
  const { space, page, terminal, worktree, boardTask, ticket } = route
  const url = destination(runner, space, terminal, worktree)
  if (page === 'board') {
    url.searchParams.set('view', 'board')
    if (boardTask) url.searchParams.set('ticket', boardTask)
    if (route.review) url.searchParams.set('review', route.review.view === 'changes' ? 'changes' : 'files')
  } else {
    const hash = new URLSearchParams({ page })
    if (ticket) hash.set('ticket', ticket.id)
    url.hash = hash.toString()
  }
  return url
}

export class WorkspaceHistory {
  private readonly listeners = new AbortController()
  private parent = false
  private leaving = false
  route: Route

  constructor(
    private readonly scope: string,
    private readonly runner: string,
    private readonly fallback: Route,
    private readonly resolve: (route: Route) => Route,
    private readonly show: (route: Route) => void,
  ) {
    const restored = this.read()
    this.parent = !!restored?.parent
    this.route = resolve(restored?.route ?? this.linked())
    this.write(false)
    window.addEventListener('popstate', () => {
      if (linkedRunner() && linkedRunner() !== this.runner) return location.reload()
      this.leaving = false
      const restored = this.read()
      this.parent = !!restored?.parent
      this.route = resolve(restored?.route ?? this.linked())
      this.show(this.route)
    }, { signal: this.listeners.signal })
  }

  navigate(route: Route, parent = false, replace = false): void {
    route = this.resolve(route)
    if (JSON.stringify(this.route) === JSON.stringify(route)) return
    this.route = route
    this.leaving = false
    this.parent = parent
    this.write(!replace)
    this.show(route)
  }

  rememberReview(ticket: string, review: ReviewRoute): void {
    if (this.route.page !== 'board' || this.route.boardTask !== ticket || !this.route.review) return
    this.route = { ...this.route, review }
    this.write(false)
  }

  leave(): void {
    if (this.leaving) return
    // Closing follows the browser's actual history when the entry has a parent.
    if (this.parent) {
      this.leaving = true
      history.back()
    } else this.navigate({ space: this.route.space, page: this.route.tab, tab: this.route.tab })
  }

  stop(): void { this.listeners.abort() }

  private read(): { route: Route; parent: boolean } | null {
    const value = history.state
    return value?.workspaceScope === this.scope ? value : null
  }

  private linked(): Route {
    const params = new URLSearchParams(location.search)
    const space = params.get('workspace') ?? this.fallback.space
    if (params.get('view') === 'needs') return { space, page: 'needs', tab: 'needs' }
    if (params.get('view') === 'board') {
      const boardTask = params.get('ticket') ?? undefined
      const view = params.get('review')
      const review: ReviewRoute | undefined = view === 'changes' ? { view } :
        view === 'files' ? { view: 'directory', path: '' } : undefined
      return { space, page: 'board', tab: 'board', boardTask, ...(boardTask && review ? { review } : {}) }
    }
    return { ...this.fallback, space, terminal: params.get('terminal') ?? undefined, worktree: params.get('worktree') ?? undefined }
  }

  private write(push: boolean): void {
    const url = workspaceUrl(this.runner, this.route)
    const state = { workspaceScope: this.scope, route: this.route, parent: this.parent }
    if (push) history.pushState(state, '', url)
    else history.replaceState(state, '', url)
  }
}
