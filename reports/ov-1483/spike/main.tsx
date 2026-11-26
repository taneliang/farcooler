import { createContext, useContext, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { createRoot } from 'react-dom/client'
import { HashRouter, Link, NavLink, Route, Routes, useNavigate, useParams } from 'react-router'
import { Button } from '@base-ui/react/button'
import { CSPProvider } from '@base-ui/react/csp-provider'
import { Dialog } from '@base-ui/react/dialog'
import { Select } from '@base-ui/react/select'
import { CanvasAddon } from '@xterm/addon-canvas'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import './tokens.css'
import './shell.css'
import { RunnerSession, spaces, stages, type Card, type Snapshot } from './runner'

const runner = new RunnerSession()
const RunnerContext = createContext(runner)

function useRunner() {
  const session = useContext(RunnerContext)
  return [session, useSyncExternalStore(session.subscribe, session.read)] as const
}

function WorkspacePicker({ value, change }: { value: string; change: (id: string) => void }) {
  return <Select.Root items={spaces.map(({ id, label }) => ({ value: id, label }))}
    value={value} onValueChange={id => { if (id) change(id) }}>
    <Select.Trigger className="control picker" aria-label="Workspace">
      <Select.Value /><Select.Icon aria-hidden>⌄</Select.Icon>
    </Select.Trigger>
    <Select.Portal>
      <Select.Positioner className="select-position" align="start" sideOffset={6} alignItemWithTrigger={false}>
        <Select.Popup className="select-popup">
          <Select.List>
            {spaces.map(space => <Select.Item key={space.id} value={space.id} className="select-item">
              <Select.ItemText>{space.label}</Select.ItemText><Select.ItemIndicator>✓</Select.ItemIndicator>
            </Select.Item>)}
          </Select.List>
        </Select.Popup>
      </Select.Positioner>
    </Select.Portal>
  </Select.Root>
}

function StartDialog({ cards, start }: { cards: Card[]; start: (key: string) => void }) {
  const ready = cards.filter(card => card.stage === 'Ready')
  const [key, setKey] = useState('')
  const selected = ready.find(card => card.key === key) ?? ready[0]
  return <Dialog.Root>
    <Dialog.Trigger className="control primary" disabled={!selected}><span aria-hidden>+</span> Start work</Dialog.Trigger>
    <Dialog.Portal>
      <Dialog.Backdrop className="backdrop" />
      <Dialog.Popup className="dialog">
        <Dialog.Title>Start work</Dialog.Title>
        <Dialog.Description>Choose a ready ticket to send to the orchestrator.</Dialog.Description>
        <label className="field">Ticket
          <select value={selected?.key ?? ''} onChange={event => setKey(event.target.value)}>
            {ready.map(card => <option key={card.key} value={card.key}>{card.title}</option>)}
          </select>
        </label>
        <div className="dialog-actions">
          <Dialog.Close className="control">Cancel</Dialog.Close>
          <Dialog.Close className="control primary" onClick={() => { if (selected) start(selected.key) }}>Start ticket</Dialog.Close>
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>
}

function TerminalView({ status }: { status: Snapshot['status'] }) {
  const session = useContext(RunnerContext)
  const host = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal>(null)

  useEffect(() => {
    const element = host.current!
    const colors = getComputedStyle(element)
    const fit = new FitAddon()
    const terminal = new Terminal({
      fontFamily: colors.getPropertyValue('--mono').trim(),
      fontSize: 13, lineHeight: 1.35, scrollback: 5000, screenReaderMode: true,
      theme: {
        background: colors.getPropertyValue('--terminal').trim(),
        foreground: colors.getPropertyValue('--terminal-text').trim(),
        cursor: colors.getPropertyValue('--terminal-positive').trim(),
      },
    })
    terminal.loadAddon(new CanvasAddon())
    terminal.loadAddon(fit)
    terminal.open(element)
    termRef.current = terminal
    const detach = session.attach(bytes => terminal.write(bytes))
    const input = terminal.onData(session.send.bind(session))
    const observer = new ResizeObserver(() => { if (element.offsetWidth) fit.fit() })
    observer.observe(element)
    return () => {
      observer.disconnect()
      detach()
      input.dispose()
      terminal.dispose()
      termRef.current = null
    }
  }, [session])

  useEffect(() => {
    if (termRef.current) termRef.current.options.disableStdin = status !== 'Connected'
  }, [status])
  return <div className="terminal-box"><div className="terminal-host" ref={host} /></div>
}

function Workspace() {
  const [session, snapshot] = useRunner()
  const { space: id, view } = useParams()
  const navigate = useNavigate()
  const space = spaces.find(space => space.id === id) ?? spaces[0]
  const boardPath = '/workspace/' + space.id
  const terminalView = view === 'orchestrator'
  const cards = snapshot.boards[space.id]
  const [ticket, setTicket] = useState<Card>()
  const backToBoard = () => navigate(boardPath)
  return <div className="workspace" data-view={terminalView ? 'orchestrator' : 'board'}>
    <aside className="sidebar" aria-label="Workspace navigation">
      <div className="brand"><span className="brand-mark" aria-hidden>❋</span> Far Cooler</div>
      <div className="rail-body">
        <div>
          <div className="meta">Workspace</div>
          <WorkspacePicker value={space.id} change={id => navigate('/workspace/' + id)} />
        </div>
        <nav className="workspace-nav" aria-label="Views">
          <NavLink to={boardPath} end><span aria-hidden>▤</span> Board</NavLink>
          <NavLink to={boardPath + '/orchestrator'}><span aria-hidden>›_</span> Orchestrator</NavLink>
        </nav>
      </div>
      <footer className="rail-footer"><Link to="/design">Design system</Link></footer>
    </aside>
    <main className="board">
      <header className="board-head">
        <div className="breadcrumb">{space.repository}</div>
        <div className="title-row">
          <h1>Board</h1>
          <StartDialog cards={cards} start={key => session.start(space.id, key)} />
        </div>
        <p className="summary" aria-live="polite">{cards.length} tickets across delivery stages</p>
      </header>
      <div className="lanes">
        {stages.map((stage, index) => {
          const entries = cards.filter(card => card.stage === stage)
          return <section className="lane" key={stage} aria-label={stage}>
            <h2><span className={'stage-mark stage-' + index} aria-hidden />{stage}<span className="count">{entries.length}</span></h2>
            {entries.map(card => <Button className="card" key={card.key} onClick={() => setTicket(card)}>
              <span className="key">{card.key}</span>
              <span className="card-title">{card.title}</span>
              <span className="card-detail">{card.detail}</span>
            </Button>)}
            {entries.length === 0 && <p className="empty">No tickets</p>}
          </section>
        })}
      </div>
      <Dialog.Root open={!!ticket} onOpenChange={open => { if (!open) setTicket(undefined) }}>
        <Dialog.Portal>
          <Dialog.Backdrop className="backdrop" />
          <Dialog.Popup className="dialog">
            <div className="key">{ticket?.key}</div>
            <Dialog.Title>{ticket?.title}</Dialog.Title>
            <Dialog.Description>{ticket?.detail}</Dialog.Description>
            <div className="dialog-actions"><Dialog.Close className="control primary">Done</Dialog.Close></div>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>
    </main>
    <aside className="orchestrator" aria-label="Orchestrator">
      <header className="pane-head">
        <h2>Orchestrator</h2>
        <div className="pane-meta"><span className="key">cosmo / main</span>
          <span className={snapshot.status === 'Connected' ? 'status good' : 'status attention'} role="status">{snapshot.status}</span>
        </div>
      </header>
      {snapshot.status === 'Session lost' && <div className="recovery">
        <p>The runner stopped this terminal session.</p>
        <Button className={terminalView ? "control primary" : "control"} onClick={() => session.recover()}>Recover terminal</Button>
      </div>}
      <TerminalView status={snapshot.status} />
      <footer className="pane-footer">
        {terminalView && snapshot.status === 'Connected' && <Button className="control primary" onClick={event => {
          event.currentTarget.closest('.orchestrator')?.querySelector<HTMLTextAreaElement>('.xterm-helper-textarea')?.focus()
        }}>Focus terminal</Button>}
        {terminalView
          ? <Button className="control" onClick={backToBoard}>Back to board</Button>
          : <Link className="control" to={boardPath + '/orchestrator'}>Open full pane <span aria-hidden>↗</span></Link>}
      </footer>
    </aside>
  </div>
}

function DesignSample() {
  const [space, setSpace] = useState(spaces[0].id)
  const [theme, setTheme] = useState(document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light')
  useEffect(() => { document.documentElement.dataset.theme = theme }, [theme])
  return <main className="sample">
    <header className="sample-header">
      <Link to="/workspace/overnight">← Back to workspace</Link>
      <Button className="control" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>{theme === 'dark' ? 'Light mode' : 'Dark mode'}</Button>
    </header>
    <h1>Workspace design system</h1>
    <p>Typography, surfaces and controls for the Far Cooler web app.</p>
    <div className="sample-grid">
      <section className="sample-panel">
        <h2>Type scale</h2>
        <div className="type-title">Board</div>
        <div className="type-section">Orchestrator</div>
        <div className="type-body">Watch delivery work across stages.</div>
        <div className="type-label">Workspace</div>
        <div className="type-small">Runner connected</div>
        <div className="key">ov-1483 · cosmo / main</div>
      </section>
      <section className="sample-panel">
        <h2>Colors</h2>
        <ul className="swatches">
          <li className="swatch canvas">Canvas</li><li className="swatch surface">Surface</li>
          <li className="swatch rail">Sidebar</li><li className="swatch accent">Action</li>
          <li className="swatch good">Connected</li><li className="swatch attention">Session lost</li>
        </ul>
      </section>
      <section className="sample-panel">
        <h2>Controls</h2>
        <WorkspacePicker value={space} change={setSpace} />
        <div className="sample-actions">
          <StartDialog cards={runner.read().boards[space]} start={() => {}} />
          <Button className="control" onClick={() => setSpace(spaces[0].id)}>Reset workspace</Button>
        </div>
      </section>
      <section className="sample-panel">
        <h2>Spacing and corners</h2>
        <div className="sample-steps">
          {[1, 2, 3, 4, 6, 8].map((step, index) => <div key={step}><span className={'measure measure-' + index} />{step * 4}</div>)}
        </div>
        <p className="type-small">5 px marks · 8 px controls · 12 px panels</p>
      </section>
    </div>
  </main>
}

if (new URLSearchParams(location.search).get('theme') === 'dark') document.documentElement.dataset.theme = 'dark'

createRoot(document.getElementById('root')!).render(
  <CSPProvider disableStyleElements>
    <RunnerContext.Provider value={runner}>
      <HashRouter>
        <Routes>
          <Route path="/design" element={<DesignSample />} />
          <Route path="/workspace/:space/:view?" element={<Workspace />} />
          <Route path="*" element={<Workspace />} />
        </Routes>
      </HashRouter>
    </RunnerContext.Provider>
  </CSPProvider>,
)
