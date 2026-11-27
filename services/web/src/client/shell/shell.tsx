/// The signed-in shell in React (ov-1510, from ov-1483's report): the sidebar
/// with the runner, the workspace picker and text navigation; the page header
/// with the view's title and at most one filled action; the page; and the
/// orchestrator pane beside it.
///
/// It renders `ShellStore`'s view model and calls its handlers; what a
/// control does is decided elsewhere (app.ts, work-screen.ts) and in the Rust
/// core. Views not rewritten yet (status and enrollment screens, terminals,
/// Needs You, board, plan, the orchestrator pane's content, dialogs) keep
/// their DOM and mount into the shell through `LegacyView`: each is a set of
/// parked nodes, moved into place on mount and back on dispose, so code that
/// finds them by id finds the same nodes wherever the shell has put them.
///
/// Base UI provides the pickers. Its styles are ours (shell.css), never
/// injected: `CSPProvider disableStyleElements` keeps the strict `style-src
/// 'self'` intact. React Router supplies the navigation context; the routes
/// themselves stay with WorkspaceHistory, which owns the URL.

import { useLayoutEffect, useRef, useState, useSyncExternalStore, type MouseEvent, type ReactNode } from 'react'
import { flushSync } from 'react-dom'
import { createRoot } from 'react-dom/client'
import { Link, Route, Router, Routes, type Navigator } from 'react-router'
import { CSPProvider } from '@base-ui/react/csp-provider'
import { Select } from '@base-ui/react/select'

import { shell as defaultStore, type Choice, type ShellPage, type ShellState, type ShellStore, type ShellTab, type ShellTool } from './store'

declare const __ICONS__: { mark: string } | undefined
/// The app icon's 64 px rendering, hashed by the build; none in a test page.
const MARK = typeof __ICONS__ === 'undefined' ? '' : __ICONS__.mark

/// A view the shell does not render itself. `mount` puts it in `host`,
/// `update` hears each new shell state while it is mounted, and `dispose`
/// takes it back out, leaving it as it was found.
export interface LegacyView {
  mount(host: HTMLElement): void
  update(state: ShellState): void
  dispose(): void
}

/// The nodes with these ids, parked in `#legacy` until the shell mounts them.
export function parked(...ids: string[]): LegacyView {
  let nodes: HTMLElement[] = []
  return {
    mount(host) {
      nodes = ids.map(id => document.getElementById(id)).filter((n): n is HTMLElement => !!n)
      host.append(...nodes)
    },
    update() {},
    dispose() {
      const park = document.getElementById('legacy')
      if (park) park.append(...nodes)
      nodes = []
    },
  }
}

const VIEWS = {
  banners: parked('update', 'copy-note'),
  screens: parked('status', 'enroll', 'compare', 'join', 'work'),
  terminals: parked('workspace-link', 'terminal-list'),
  account: parked('bar-count', 'who', 'web-push', 'web-push-note', 'show-saved-copies', 'sign-out-form'),
  side: parked('workspace-side'),
}

function Legacy({ view, state, className }: { view: LegacyView; state: ShellState; className?: string }) {
  const host = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    view.mount(host.current!)
    return () => view.dispose()
  }, [view])
  useLayoutEffect(() => view.update(state), [view, state])
  return <div className={className} ref={host} />
}

const TABS: ShellTab[] = ['terminals', 'needs', 'board', 'plan']
const TOOLS: ShellTool[] = ['orchestrator', 'workflow', 'commands']

/// A page's heading. Needs You's nav label carries its count; the heading
/// doesn't.
function titleOf(page: ShellPage, state: ShellState): string {
  if (page === 'needs') return 'Needs You'
  if (page === 'workflow') return state.tools.workflow.label
  if (page === 'ticket') return titleOf(state.nav.board.disabled ? 'needs' : 'board', state)
  return state.nav[page].label
}

function Picker({ id, label, items, value, disabled, change }: {
  id: string; label: string; items: Choice[]; value: string | null; disabled?: boolean; change: (id: string) => void
}) {
  const chosen = items.find(item => item.id === value)
  return <Select.Root
    items={items.map(item => ({ value: item.id, label: item.label }))}
    value={value ?? null}
    disabled={disabled}
    onValueChange={next => { if (typeof next === 'string' && next !== value) change(next) }}>
    <Select.Trigger id={id} className="control picker" aria-label={label} data-value={value ?? ''} title={chosen?.label}>
      <Select.Value className="picker-value" />
      <Select.Icon className="picker-icon" aria-hidden>⌄</Select.Icon>
    </Select.Trigger>
    <Select.Portal>
      <Select.Positioner className="select-position" align="start" sideOffset={6} alignItemWithTrigger={false}>
        <Select.Popup className="select-popup">
          <Select.List>
            {items.map(item => <Select.Item key={item.id} value={item.id} disabled={item.disabled} className="select-item">
              <Select.ItemText>{item.label}</Select.ItemText>
              <Select.ItemIndicator className="select-check" aria-hidden>✓</Select.ItemIndicator>
            </Select.Item>)}
          </Select.List>
        </Select.Popup>
      </Select.Positioner>
    </Select.Portal>
  </Select.Root>
}

function Sidebar({ state, store }: { state: ShellState; store: ShellStore }) {
  const work = state.screen === 'work'
  /// At phone width (shell.css) the runner and the tools fold behind one
  /// button, so the page starts near the top; wider, both always show.
  const [more, setMore] = useState(false)
  const go = (tab: ShellTab) => (event: MouseEvent) => {
    if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
    event.preventDefault()
    store.handlers.showTab(tab)
  }
  return <aside className="sidebar" aria-label="Far Cooler">
    <div className="brand">
      {MARK ? <img className="brand-mark" src={MARK} alt="" width={28} height={28} /> : <span className="brand-mark" aria-hidden />}
      <span>Far Cooler</span>
      {work && <button id="rail-more" className="control rail-more" type="button" aria-expanded={more}
        aria-controls="rail-runner rail-tools" aria-label={`${state.runner.label}, Runner and Tools`}
        onClick={() => setMore(open => !open)}>
        <span className="picker-value">{state.runner.label}</span>
        <span className="picker-icon" aria-hidden>⌄</span>
      </button>}
    </div>
    {work && <div className="rail-body" data-more={more || undefined}>
      <section className="rail-group" id="rail-runner">
        <h2 className="meta">Runner</h2>
        {state.runners.length > 1
          ? <Picker id="runner-picker" label="Switch Runner" items={state.runners} value={state.runner.id} change={id => store.handlers.pickRunner(id)} />
          : <p className="runner-name" id="runner-name">{state.runner.label}</p>}
        <p className="runner-health">
          <span id="runner-health" role="status">{state.runner.health}</span>
          {state.runner.runtime && <span id="runtime-health">{state.runner.runtime}</span>}
        </p>
        <button className="quiet link" id="add-runner" type="button" onClick={() => store.handlers.addRunner()}>Add Runner</button>
      </section>
      {state.spaces.length > 0 && <section className="rail-group">
        <h2 className="meta">Workspace</h2>
        <Picker id="space-picker" label="Workspace" items={state.spaces} value={state.space} disabled={state.busy} change={id => store.handlers.pickSpace(id)} />
      </section>}
      <nav className="views" aria-label="Views">
        {TABS.map(tab => {
          const item = state.nav[tab]
          const current = state.page === tab || (state.page === 'ticket' && tab === 'board' && !item.disabled)
          if (item.disabled) return <span key={tab} id={`show-${tab}`} className="view-link" aria-disabled="true">{item.label}</span>
          return <Link key={tab} id={`show-${tab}`} className="view-link" to={state.hrefs[tab] ?? '#'}
            aria-current={current ? 'page' : undefined} aria-keyshortcuts={item.keys} onClick={go(tab)}>{item.label}</Link>
        })}
      </nav>
      <div className="tools" id="rail-tools" role="group" aria-label="Workspace tools">
        {TOOLS.map(tool => {
          const item = state.tools[tool]
          return <button key={tool} id={`show-${tool}`} className="quiet tool" type="button" disabled={item.disabled}
            aria-keyshortcuts={item.keys} aria-haspopup={tool === 'commands' ? 'dialog' : undefined}
            onClick={() => store.handlers.tool(tool)}>{item.label}</button>
        })}
      </div>
      <Legacy view={VIEWS.terminals} state={state} className="rail-terminals" />
    </div>}
  </aside>
}

function PageHead({ state, store }: { state: ShellState; store: ShellStore }) {
  const offered = state.startWork && (state.page === 'terminals' || state.page === 'board')
  const recovering = state.linkLost || (state.page === 'terminals' && state.terminalLost)
  const action = offered
    ? <button id="show-start-work" className={recovering ? 'control' : 'control primary'} type="button"
      aria-haspopup="dialog" onClick={() => store.handlers.startWork()}>{state.startWork}</button>
    : null
  const head = (page: ShellPage): ReactNode => <header className="page-head" data-page={page}>
    <p className="crumb">{[state.runner.label, state.repository].filter(Boolean).join(' · ')}</p>
    <div className="title-row">
      <h1 id="page-title">{titleOf(page, state)}</h1>
      {action}
    </div>
  </header>
  return <Routes location={{ pathname: `/${state.page}` }}>
    {(['terminals', 'needs', 'board', 'plan', 'workflow', 'ticket'] as const).map(page =>
      <Route key={page} path={`/${page}`} element={head(page)} />)}
  </Routes>
}

function App({ store }: { store: ShellStore }) {
  const state = useSyncExternalStore(store.subscribe, store.read)
  const work = state.screen === 'work'
  return <div className="app" data-screen={state.screen} data-page={work ? state.page : undefined}>
    <Sidebar state={state} store={store} />
    <div className="page">
      <Legacy view={VIEWS.banners} state={state} className="banners" />
      {work && <PageHead state={state} store={store} />}
      <Legacy view={VIEWS.screens} state={state} className="page-body" />
    </div>
    {work && <Legacy view={VIEWS.side} state={state} className="side-pane" />}
    {/* After the work in the document, so focus reaches it last at every width. */}
    <footer className="rail-footer" aria-label="Account"><Legacy view={VIEWS.account} state={state} className="account" /></footer>
  </div>
}

/// The links in the shell are real hrefs (open in a new tab works); a plain
/// click is the handlers', so the router's navigator only ever reloads.
function navigatorFor(): Navigator {
  const href = (to: Parameters<Navigator['createHref']>[0]) =>
    typeof to === 'string' ? to : `${to.pathname ?? ''}${to.search ?? ''}${to.hash ?? ''}`
  return {
    createHref: href,
    go: delta => history.go(delta),
    push: to => location.assign(href(to)),
    replace: to => location.replace(href(to)),
  }
}

/// Render the shell into `root`, synchronously, so every parked view is in
/// place when this returns. Returns what takes it down again.
export function mountShell(root: HTMLElement, store: ShellStore = defaultStore): () => void {
  const react = createRoot(root)
  const navigator = navigatorFor()
  flushSync(() => react.render(
    <CSPProvider disableStyleElements>
      <Router location={{ pathname: '/' }} navigator={navigator}>
        <App store={store} />
      </Router>
    </CSPProvider>,
  ))
  return () => react.unmount()
}
