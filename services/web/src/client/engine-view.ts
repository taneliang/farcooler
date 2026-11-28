// The Engine view and a person's actions on it (ov-1486). The page it draws, the
// actions on each row and every sentence about what came of a press are the Rust
// core's (`engine_page`); this draws them and sends back what was pressed. The
// runner decides who may act.

/** The core's `engine_page::Page` and its parts, as `engine.read` sends them. */
export interface EngineOffer {
  kind: string; label: string; confirm: string; destructive: boolean; reasonLabel: string | null; guard: string
}
export interface EngineDraft { id: string; question: string; note: string; problem: string | null }
export interface EngineTicket {
  id: string; title: string; step: string; status: string; tone: 'normal' | 'waiting' | 'failed'
  summary: string | null; reason: string | null; failure: string | null; agent: string | null; review: string
}
export interface EngineLanding {
  id: string; label: string; status: string; tone: 'normal' | 'waiting' | 'failed'; historical: boolean
  members: { id: string; title: string }[]; tickets: EngineTicket[]; failure: string | null
  ci: { status: string; sha: string } | null
}
export interface EnginePage {
  view: {
    workspace: string
    capacity: { id: string; label: string; used: number | null; limit: number | null; queued: number | null }[]
    stations: { id: string; label: string; tickets: EngineTicket[] }[]
    landings: EngineLanding[]
  }
  actions: Record<string, EngineOffer[]>
  drafts: EngineDraft[]
  draftsProblem: string | null
  notice: string | null
  words: Record<string, string>
}
export interface EngineOutcome { ok: boolean; message: string; detail: string | null }
export interface EngineSession { workflow_call(method: string, args: string): Promise<string> }

/** How often an open, visible Engine view reads again. */
const REFRESH_MS = 30_000

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = '') => {
  const node = document.createElement(tag)
  node.textContent = text
  if (className) node.className = className
  return node
}
const button = (title: string, key: string, act: () => void, disabled = false, className = '') => {
  const node = el('button', title, className)
  node.type = 'button'
  node.disabled = disabled
  node.onclick = act
  node.dataset.engineControl = key
  return node
}
/** A status line in its tone's color. */
const toned = (text: string, tone: string) => {
  const node = el('p', text, 'engine-status')
  node.classList.add(`engine-${tone}`)
  return node
}
const fill = (line: string, values: Record<string, string>) =>
  Object.entries(values).reduce((out, [name, value]) => out.replace(`{${name}}`, value), line)

/** What a person has pressed and not yet confirmed. */
type Pending =
  | { kind: 'action'; key: string; item: string; offer: EngineOffer }
  | { kind: 'draft'; key: string; draft: EngineDraft; approve: boolean }

/** `words` is the core's table (`engine_page::words`), which a read sends again with its page. */
export function openEngine(
  dialog: HTMLDialogElement, session: () => EngineSession | null, workspace: string,
  words: Record<string, string>, closePage: () => void, openRuns: () => void,
): { close(): void; connection(up: boolean): void } {
  const root = el('div', '', 'plan-sheet-body')
  root.classList.add('engine')
  const bar = el('div', '', 'plan-sheet-bar')
  bar.append(el('h2', words.engine), button(words.close, 'close', closePage))
  dialog.replaceChildren(bar, root)

  let page: EnginePage | null = null
  let observed = ''
  // Why what is shown can't be acted on: the link is down, or the last read failed.
  let problem: '' | 'offline' | 'unread' = ''
  let outcome: EngineOutcome | null = null
  let pending: Pending | null = null
  let reason = ''
  let busy = false
  let generation = 0
  let closed = false
  let linkUp = true
  // The read in flight, so that a refresh asked for meanwhile isn't a second one.
  let reading: Promise<void> | null = null

  const call = async (method: string, args: object): Promise<any> => {
    const current = session()
    if (!current) throw new Error('offline')
    return JSON.parse(await current.workflow_call(method, JSON.stringify(args)))
  }
  const read = (): Promise<void> => {
    if (reading) return reading
    const mine = ++generation
    const done = (async () => {
      try {
        const next = await call('engine.read', { workspace }) as EnginePage
        if (closed || mine !== generation) return
        if (next.view.workspace !== workspace) throw new Error('workspace')
        page = next
        problem = ''
        observed = new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
        // A question that is no longer asked, or a row that moved, takes its confirmation with it.
        if (pending && !stillThere(next, pending)) pending = null
      } catch {
        if (closed || mine !== generation) return
        problem = 'unread'
      }
      draw()
    })().finally(() => { if (reading === done) reading = null })
    reading = done
    draw()
    return done
  }
  const stillThere = (next: EnginePage, was: Pending) => was.kind === 'action'
    ? !!next.actions[was.item]?.some(o => o.kind === was.offer.kind && o.guard === was.offer.guard)
    : next.drafts.some(d => d.id === was.draft.id && d.note === was.draft.note)

  const send = async (was: Pending) => {
    if (busy) return
    busy = true
    outcome = null
    // What was read before this press says nothing about what follows it.
    ++generation
    reading = null
    draw()
    let result: EngineOutcome
    try {
      result = was.kind === 'action'
        ? await call('engine.act', { workspace, selection: {
            item: was.item, kind: was.offer.kind, guard: was.offer.guard,
            ...(was.offer.reasonLabel ? { reason } : {}),
          } })
        : await call('engine.decide', { workspace, draft: was.draft.id, note: was.draft.note, approve: was.approve })
    } catch {
      result = { ok: false, message: words.unconfirmed, detail: null }
    }
    if (closed) return
    busy = false
    outcome = result
    if (result.ok) { pending = null; reason = '' }
    // Read again unless the link is down now; if it is, the read waits for it.
    if (linkUp) await read()
    else draw()
    focus('outcome')
  }

  const choose = (next: Pending | null) => {
    const back = pending?.key
    pending = next
    reason = ''
    outcome = null
    draw()
    focus(next ? `confirm:${next.key}` : back ?? 'refresh')
  }
  const focus = (key: string) => {
    const target = root.querySelector<HTMLElement>(`[data-engine-control="${CSS.escape(key)}"]`)
      ?? root.querySelector<HTMLElement>('[data-engine-control="refresh"]')
    target?.focus({ preventScroll: key !== 'outcome' })
  }

  function confirmation(was: Pending): HTMLElement {
    const box = el('div', '', 'engine-confirm')
    const sentence = was.kind === 'action' ? was.offer.confirm : was.approve ? words.confirmApprove : words.confirmDecline
    box.append(el('p', sentence))
    if (was.kind === 'action' && was.offer.reasonLabel) {
      const label = el('label', was.offer.reasonLabel)
      const input = el('textarea')
      input.value = reason
      input.disabled = busy
      input.dataset.engineControl = `reason:${was.key}`
      input.oninput = () => { reason = input.value }
      label.append(input)
      box.append(label)
    }
    const row = el('div', '', 'engine-buttons')
    const title = was.kind === 'action' ? was.offer.label : was.approve ? words.approve : words.decline
    const destructive = was.kind === 'action' ? was.offer.destructive : false
    row.append(
      button(busy ? words.working : title, `confirm:${was.key}`, () => void send(was), busy || !!problem,
        destructive ? 'engine-danger' : 'primary'),
      button(words.keep, `keep:${was.key}`, () => choose(null), busy),
    )
    box.append(row)
    return box
  }

  function offers(item: string): HTMLElement[] {
    const list = page!.actions[item]
    if (!list?.length) return []
    const row = el('div', '', 'engine-buttons')
    for (const offer of list) {
      const key = `act:${item}:${offer.kind}`
      const node = button(offer.label, key, () => choose({ kind: 'action', key, item, offer }), busy || !!problem,
        offer.destructive ? 'engine-danger' : '')
      node.setAttribute('aria-expanded', String(pending?.key === key))
      row.append(node)
    }
    const out: HTMLElement[] = [row]
    if (pending?.kind === 'action' && pending.item === item) out.push(confirmation(pending))
    return out
  }

  function ticket(row: EngineTicket): HTMLElement {
    const node = el('li', '', 'engine-ticket')
    node.dataset.item = row.id
    const heading = el('h4')
    heading.append(el('span', row.id, 'engine-key'), ' ', row.title === row.id ? '' : row.title)
    node.append(heading)
    node.append(toned(`${row.step} · ${row.status}`, row.tone))
    if (row.summary) node.append(el('p', row.summary, 'engine-summary'))
    const more = [
      [words.currentReason, row.reason !== row.summary ? row.reason : null],
      [words.failureDetails, row.failure !== row.summary ? row.failure : null],
    ].filter((line): line is [string, string] => !!line[1])
    if (more.length) {
      const details = el('details')
      details.append(el('summary', words.runDetails))
      for (const [title, text] of more) details.append(el('h5', title), el('p', text, 'engine-detail'))
      node.append(details)
    }
    node.append(...offers(row.id))
    return node
  }

  function landing(group: EngineLanding): HTMLElement {
    const node = el('li', '', 'engine-landing')
    node.dataset.item = group.id
    node.append(el('h4', group.label), toned(group.status, group.tone))
    if (group.ci) node.append(el('p', `${words.githubCI}: ${group.ci.status}`))
    if (group.failure) node.append(el('p', group.failure, 'engine-detail'))
    const shown = new Set(group.tickets.map(t => t.id))
    if (group.tickets.length) {
      const list = el('ul', '', 'engine-list')
      list.append(...group.tickets.map(t => ticket(t)))
      node.append(list)
    }
    const rest = group.members.filter(m => !shown.has(m.id))
    if (rest.length) {
      const list = el('ul', '', 'engine-members')
      list.append(...rest.map(m => el('li', m.title === m.id ? m.id : `${m.id} ${m.title}`)))
      node.append(list)
    }
    node.append(...offers(group.id))
    return node
  }

  function drafts(): HTMLElement {
    const section = el('section', '', 'engine-drafts')
    section.append(el('h3', words.drafts))
    if (page!.draftsProblem) section.append(el('p', page!.draftsProblem, 'engine-detail'))
    else if (!page!.drafts.length) section.append(el('p', words.noDrafts, 'engine-empty'))
    for (const draft of page!.drafts) {
      const card = el('article', '', 'engine-draft')
      if (draft.problem) card.append(el('p', draft.problem, 'engine-detail'))
      else {
        // The whole question, never cut: the approval's code is only in it.
        card.append(el('div', draft.question, 'engine-question'))
        const row = el('div', '', 'engine-buttons')
        for (const approve of [true, false]) {
          const key = `draft:${draft.id}:${approve}`
          const node = button(approve ? words.approve : words.decline, key,
            () => choose({ kind: 'draft', key, draft, approve }), busy || !!problem)
          node.setAttribute('aria-expanded', String(pending?.key === key))
          row.append(node)
        }
        card.append(row)
        if (pending?.kind === 'draft' && pending.draft.id === draft.id) card.append(confirmation(pending))
      }
      section.append(card)
    }
    return section
  }

  function body(): HTMLElement[] {
    const { view, notice } = page!
    const out: HTMLElement[] = []
    if (notice) out.push(el('p', notice, 'engine-notice'))
    const capacity = el('ul', '', 'engine-capacity')
    for (const pool of view.capacity) {
      if (pool.used === null || pool.limit === null) continue
      const used = fill(words.used, { used: String(pool.used), limit: String(pool.limit) })
      const queued = pool.queued ? ` · ${fill(words.queued, { count: String(pool.queued) })}` : ''
      capacity.append(el('li', `${pool.label}: ${used}${queued}`))
    }
    if (capacity.childElementCount) out.push(capacity)
    out.push(drafts())

    const running = view.landings.filter(g => !g.historical)
    const landings = el('section')
    landings.append(el('h3', words.landingGroups))
    if (!running.length) landings.append(el('p', words.noLandings, 'engine-empty'))
    else {
      const list = el('ul', '', 'engine-list')
      list.append(...running.map(landing))
      landings.append(list)
    }
    const past = view.landings.filter(g => g.historical)
    if (past.length) {
      const details = el('details', '', 'engine-past')
      details.append(el('summary', `${words.earlierLandings} · ${past.length}`))
      const list = el('ul', '', 'engine-list')
      list.append(...past.map(landing))
      details.append(list)
      landings.append(details)
    }
    out.push(landings)

    const stations = el('section')
    stations.append(el('h3', words.stations))
    const used = view.stations.filter(s => s.tickets.length)
    if (!used.length) stations.append(el('p', words.noTickets, 'engine-empty'))
    for (const station of used) {
      const group = el('section', '', 'engine-station')
      group.dataset.station = station.id
      group.append(el('h3', `${station.label} · ${station.tickets.length}`))
      const list = el('ul', '', 'engine-list')
      list.append(...station.tickets.map(t => ticket(t)))
      group.append(list)
      stations.append(group)
    }
    out.push(stations)
    return out
  }

  function draw(): void {
    if (closed) return
    const active = root.contains(document.activeElement) ? document.activeElement as HTMLElement : null
    const key = active?.dataset.engineControl
    const caret = active instanceof HTMLTextAreaElement ? [active.selectionStart, active.selectionEnd] : null
    const open = new Set([...root.querySelectorAll('details[open]')].map(d => d.closest('[data-item]')?.getAttribute('data-item') ?? 'past'))
    const scroll = root.scrollTop

    const head = el('div', '', 'engine-head')
    head.append(button(problem ? words.tryAgain : words.refresh, 'refresh', () => void read(), busy))
    head.append(button(words.runs, 'runs', openRuns))
    if (page && !problem && observed) head.append(el('span', fill(words.updated, { time: observed }), 'engine-observed'))
    const out: HTMLElement[] = [head]
    if (problem) {
      const why = problem === 'offline' ? words.offline : words.invalidResponse
      const note = el('p', page ? fill(words.stale, { time: observed }) : why, 'engine-notice')
      note.setAttribute('role', 'status')
      out.push(note)
    }
    if (outcome) {
      const said = el('div', '', 'engine-outcome')
      said.classList.toggle('engine-failed', !outcome.ok)
      said.setAttribute('role', 'status')
      said.tabIndex = -1
      said.dataset.engineControl = 'outcome'
      said.append(el('p', outcome.message))
      if (outcome.detail) said.append(el('pre', outcome.detail))
      out.push(said)
    }
    if (page) out.push(...body())
    else if (!problem) out.push(el('p', words.reading, 'engine-empty'))
    root.replaceChildren(...out)
    for (const details of root.querySelectorAll('details')) {
      if (open.has(details.closest('[data-item]')?.getAttribute('data-item') ?? 'past')) details.open = true
    }
    root.scrollTop = scroll
    if (key) {
      const target = root.querySelector<HTMLElement>(`[data-engine-control="${CSS.escape(key)}"]`)
      target?.focus({ preventScroll: true })
      if (caret && target instanceof HTMLTextAreaElement) target.setSelectionRange(caret[0], caret[1])
    }
  }

  const refresh = () => {
    // Not under a person who is deciding: a redraw would move what they are reading.
    if (!closed && dialog.open && !busy && !pending && !document.hidden) void read()
  }
  const timer = setInterval(refresh, REFRESH_MS)
  const offline = () => {
    linkUp = false
    ++generation
    reading = null
    problem = 'offline'
    draw()
  }
  const online = () => {
    linkUp = true
    if (!closed && dialog.open && !busy) void read()
  }
  window.addEventListener('online', online)
  window.addEventListener('offline', offline)
  dialog.setAttribute('open', '')
  void read()
  return {
    close() {
      closed = true
      clearInterval(timer)
      window.removeEventListener('online', online)
      window.removeEventListener('offline', offline)
      dialog.removeAttribute('open')
    },
    connection: up => { if (up) online(); else offline() },
  }
}
