import { destination, linkedRunner } from './work-screen'
import type { Page, Slot, Tab } from './workspace-layout'

export interface Route {
  space: string | null
  page: Page
  tab: Tab
  terminal?: string
  slot?: Slot
  ticket?: { runner: string; id: string; key: string; title: string }
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
    this.route = resolve(restored?.route ?? fallback)
    this.write(false)
    window.addEventListener('popstate', () => {
      if (linkedRunner() && linkedRunner() !== this.runner) return location.reload()
      this.leaving = false
      const restored = this.read()
      this.parent = !!restored?.parent
      this.route = resolve(restored?.route ?? fallback)
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

  leave(): void {
    if (this.leaving) return
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

  private write(push: boolean): void {
    const { space, page, terminal, ticket } = this.route
    const url = destination(this.runner, space, terminal)
    const hash = new URLSearchParams({ page })
    if (ticket) hash.set('ticket', ticket.id)
    url.hash = hash.toString()
    const state = { workspaceScope: this.scope, route: this.route, parent: this.parent }
    if (push) history.pushState(state, '', url)
    else history.replaceState(state, '', url)
  }
}
