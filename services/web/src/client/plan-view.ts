/// The plan on the page (ov-775, ov-875): the plan column as the Mac draws it
/// (plan-column.ts): the facts line, Needs You, Initiatives, the pipeline of
/// agents with their tickets, and the footer counts; then Decided For You.
///
/// Every value is the runner's or the orchestrator's, so each goes in through
/// `textContent` and never markup (the CSP's Trusted Types refuse an HTML
/// string sink).

import { age } from './needs'
import {
  columnOf,
  statusesOf,
  type Agent,
  type Column,
  type Count,
  type CountKind,
  fraction,
  type Initiative,
  type NeedsRow,
  progressWords,
  type Role,
  STAGE_TITLE,
  span,
  type Ticket,
} from './plan-column'
import { openRulings, pastRulings, type CardRef, type Plan, type PlanBoardView, rulingStateTitle, type Ruling, WORDS } from './plan'

export interface PlanHandlers {
  /// A card's detail (a task, by its id on this runner).
  detail(card: CardRef): void
  /// A Needs You row: leave `text` in the orchestrator's input, unsent. False
  /// when there is no orchestrator to put it in.
  quote(text: string): boolean
  /// An agent's chat or terminal. False when it has none to open.
  openAgent(agent: Agent): boolean
  /// A footer count's list.
  openList(kind: CountKind): void
  edit(theme: string): void
  type(draft: string): void
  save(): void
  cancel(): void
  keep(ruling: string): void
  keepAll(): void
}

const make = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const el = document.createElement(tag)
  if (className) el.className = className
  if (text) el.textContent = text
  return el
}

const button = (label: string, className: string, onclick: () => void, disabled = false): HTMLButtonElement => {
  const b = make('button', className, label)
  b.type = 'button'
  b.disabled = disabled
  b.onclick = onclick
  return b
}

function heading(title: string, count?: number | string, prominent = false, tag: 'h2' | 'h3' = 'h2'): HTMLElement {
  const h = make(tag, prominent ? 'plan-title plan-title-ink' : 'plan-title', title)
  if (count !== undefined && count !== 0) h.append(make('span', 'plan-count', String(count)))
  return h
}

function notice(text: string | undefined): HTMLElement | null {
  if (!text) return null
  const p = make('p', 'plan-notice', text)
  p.setAttribute('role', 'alert')
  return p
}

const SVG = 'http://www.w3.org/2000/svg'

/// The glyphs of the roles, as the Mac's SF Symbols: a hammer, an eye, an
/// arrow onto a line, and a sparkle.
const GLYPHS: Record<Role, string[]> = {
  builder: ['M2.5 13.5 8 8', 'M7 4.5 9.5 2l4.5 4.5-2.5 2.5z'],
  reviewer: ['M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z', 'M8 6.2a1.8 1.8 0 1 0 0 3.6 1.8 1.8 0 0 0 0-3.6z'],
  lander: ['M8 2v8', 'M4.5 6.5 8 10l3.5-3.5', 'M3 13.5h10'],
  agent: ['M8 2l1.5 4.5L14 8l-4.5 1.5L8 14l-1.5-4.5L2 8l4.5-1.5z'],
}

function glyph(agent: Agent): SVGElement {
  const svg = document.createElementNS(SVG, 'svg')
  svg.setAttribute('viewBox', '0 0 16 16')
  svg.setAttribute('class', 'plan-glyph')
  svg.setAttribute('aria-hidden', 'true')
  // Activity is the agent's: a quiet pulse while it works, amber and still
  // when stuck, still when idle.
  if (agent.doingIsTrouble) svg.setAttribute('data-trouble', 'true')
  else if (agent.working) svg.setAttribute('data-working', 'true')
  for (const d of GLYPHS[agent.role]) {
    const path = document.createElementNS(SVG, 'path')
    path.setAttribute('d', d)
    svg.append(path)
  }
  return svg
}

/// The facts line: each fact whole; the line breaks between them, never
/// inside one.
function factsLine(column: Column): HTMLElement | null {
  if (column.facts.length === 0) return null
  const p = make('p', 'plan-facts')
  for (const fact of column.facts) {
    const span = make('span', fact.attention ? 'plan-fact plan-fact-amber' : 'plan-fact')
    for (const run of fact.runs) span.append(run.strong ? make('strong', '', run.text) : document.createTextNode(run.text))
    p.append(span)
  }
  return p
}

function needsSection(rows: NeedsRow[], now: number, on: PlanHandlers): HTMLElement | null {
  if (rows.length === 0) return null
  const wrap = make('section', 'plan-section plan-needs')
  wrap.append(heading('Needs You', rows.length, true))
  const list = make('div', 'plan-needs-list')
  for (const row of rows) {
    const b = make('button', 'plan-need')
    b.type = 'button'
    b.title = 'Quote this in the orchestrator’s input'
    b.append(make('span', 'plan-dot'))
    const words = make('span', 'plan-need-words')
    words.append(make('strong', '', row.title))
    if (row.detail) words.append(document.createTextNode(' '), make('span', 'plan-dim', row.detail))
    b.append(words)
    const when = age(row.since, now)
    if (when) b.append(make('span', 'plan-time', when))
    b.onclick = () => {
      list.querySelector('.plan-notice')?.remove()
      if (on.quote(row.quote)) return
      const n = notice('There’s no orchestrator running here to quote this in. Start one, then try again.')
      if (n) b.after(n)
    }
    list.append(b)
  }
  wrap.append(list)
  return wrap
}

function initiativeCard(i: Initiative, view: PlanBoardView, on: PlanHandlers): HTMLElement {
  const li = make('li', 'plan-soft plan-initiative')
  const busy = view.busy.has(i.id)
  if (busy) li.setAttribute('aria-busy', 'true')
  const editing = view.editing?.theme === i.id ? view.editing : null
  if (editing) {
    const form = make('form', 'plan-next-edit')
    form.append(make('strong', 'plan-init-name', i.name))
    const label = make('label', 'plan-label', 'Next')
    const input = make('input', 'plan-next-input')
    input.type = 'text'
    input.value = editing.draft
    input.maxLength = 500
    input.placeholder = 'What happens next'
    input.disabled = busy
    input.id = `next-${i.id}`
    label.htmlFor = input.id
    input.oninput = () => on.type(input.value)
    input.onkeydown = e => {
      if (e.key === 'Escape') on.cancel()
    }
    const actions = make('div', 'plan-actions')
    const save = make('button', '', 'Save')
    save.type = 'submit'
    save.disabled = busy
    actions.append(save, button('Cancel', 'quiet', on.cancel, busy))
    form.onsubmit = e => {
      e.preventDefault()
      on.save()
    }
    form.append(label, input, actions)
    li.append(form)
  } else {
    // Name and progress on one line, then what's next: the line that matters
    // most. The goal is in the title. A click writes its Next line.
    const b = make('button', 'plan-init')
    b.type = 'button'
    if (i.goal) b.title = i.goal
    b.setAttribute('aria-label', `${i.name}, ${progressWords(i)} done. ${i.next ?? 'No next step.'} Edit its Next line.`)
    b.disabled = busy
    b.onclick = () => on.edit(i.id)
    const top = make('span', 'plan-init-top')
    const meter = make('span', 'plan-meter')
    meter.setAttribute('aria-hidden', 'true')
    meter.append(make('span'))
    ;(meter.firstElementChild as HTMLElement).style.width = `${Math.round(fraction(i) * 100)}%`
    top.append(make('span', 'plan-init-name', i.name), meter, make('span', 'plan-figures', progressWords(i)))
    b.append(top)
    if (i.next) b.append(make('span', i.asksOwner ? 'plan-init-next plan-amber' : 'plan-init-next', i.next))
    li.append(b)
  }
  const n = notice(view.notices.get(i.id))
  if (n) li.append(n)
  return li
}

function initiativesSection(column: Column, view: PlanBoardView, on: PlanHandlers): HTMLElement | null {
  if (column.initiativeCount === 0) return null
  const wrap = make('section', 'plan-section plan-initiatives')
  wrap.append(heading('Initiatives', column.initiativeCount, true))
  const ul = make('ul', 'plan-list')
  ul.append(...column.initiatives.map(i => initiativeCard(i, view, on)))
  wrap.append(ul)
  if (column.quietLine) wrap.append(make('p', 'plan-quiet', column.quietLine))
  return wrap
}

function ticketRow(t: Ticket, on: PlanHandlers): HTMLElement {
  const b = make('button', 'plan-ticket')
  b.type = 'button'
  b.dataset.task = t.task
  b.setAttribute('aria-label', `${t.key}, ${t.title}${t.initiative ? `, for ${t.initiative}` : ''}, ${STAGE_TITLE[t.stage]}`)
  // Its key, in secondary ink at one width so every title starts on one edge;
  // the ticket never reads as progress.
  const key = make('span', 'plan-key', t.key)
  key.setAttribute('aria-hidden', 'true')
  b.append(key, make('span', 'plan-ticket-title', t.title))
  if (t.initiative) b.append(make('span', 'plan-ticket-for', t.initiative))
  b.onclick = () => on.detail({ task: t.task, key: t.key })
  return b
}

function agentCard(agent: Agent, more: string | null, on: PlanHandlers): HTMLElement {
  const li = make('li', 'plan-soft plan-agent')
  li.dataset.agent = agent.id
  const head = make('button', 'plan-agent-head')
  head.type = 'button'
  head.title = agent.terminal ? `Open ${agent.name}’s chat` : `Open ${agent.name}`
  head.setAttribute('aria-label', `${agent.name}, ${agent.doing}${agent.elapsed !== null ? `, ${span(agent.elapsed)}` : ''}`)
  const words = make('span', 'plan-agent-words')
  words.append(make('strong', '', agent.name), document.createTextNode(' '), make('span', agent.doingIsTrouble ? 'plan-amber' : '', agent.doing))
  head.append(glyph(agent), words)
  if (agent.elapsed !== null) head.append(make('span', 'plan-time', span(agent.elapsed)))
  head.onclick = () => {
    if (on.openAgent(agent)) return
    // No chat to open: its first ticket is the nearest page.
    const first = agent.tickets[0]
    if (first) on.detail({ task: first.task, key: first.key })
  }
  li.append(head)
  for (const t of agent.tickets) li.append(ticketRow(t, on))
  if (more) li.append(make('p', 'plan-more', more))
  return li
}

function pipelineSection(column: Column, on: PlanHandlers): HTMLElement | null {
  if (column.pipeline.length === 0) return null
  const wrap = make('section', 'plan-section plan-pipeline')
  wrap.append(heading('Pipeline', column.agentWords, true))
  for (const group of column.pipeline) {
    const stage = make('div', 'plan-stage')
    stage.dataset.stage = group.stage
    stage.append(heading(STAGE_TITLE[group.stage], group.count, false, 'h3'))
    const ul = make('ul', 'plan-list')
    for (const agent of group.agents) ul.append(agentCard(agent, agent.more > 0 ? group.unattendedWords : null, on))
    if (group.agents.length === 0 && group.unattendedWords) {
      const li = make('li', 'plan-soft plan-agent')
      li.append(make('p', 'plan-more', group.unattendedWords))
      for (const t of group.unattended) li.append(ticketRow(t, on))
      ul.append(li)
    } else if (group.agents.length > 0 && !group.agents.some(a => a.more > 0) && group.unattendedWords) {
      // No card has tickets to hang it on: its own line.
      ul.append(make('li', 'plan-quiet', group.unattendedWords))
    }
    stage.append(ul)
    wrap.append(stage)
  }
  return wrap
}

function footerLine(counts: Count[], on: PlanHandlers): HTMLElement | null {
  if (counts.length === 0) return null
  const p = make('p', 'plan-footer')
  counts.forEach((c, index) => {
    const b = make('button', 'plan-count-link', c.words)
    b.type = 'button'
    b.title = 'Open the list'
    b.dataset.kind = c.kind
    b.onclick = () => on.openList(c.kind)
    p.append(b)
    if (index < counts.length - 1) p.append(document.createTextNode(', '))
  })
  return p
}

function rulingCard(r: Ruling, view: PlanBoardView, on: PlanHandlers): HTMLElement {
  const li = make('li', 'plan-card')
  const busy = view.busy.has(r.id)
  if (busy) li.setAttribute('aria-busy', 'true')
  const head = make('div', 'plan-head')
  head.append(make('span', 'plan-name', r.decision), make('span', 'plan-phase', `${r.short} · ${rulingStateTitle(r)}`))
  li.append(head)
  if (r.why) li.append(make('p', 'plan-line', r.why))
  if (r.reversal) {
    const p = make('p', 'plan-sub')
    p.append(make('span', 'plan-label', 'Reversing'), document.createTextNode(` ${r.reversal}`))
    li.append(p)
  }
  if (r.theme) li.append(make('p', 'plan-sub', r.theme))
  if (r.note && r.state !== 'standing') li.append(make('p', 'plan-sub', r.note))
  if (r.state === 'standing') {
    const actions = make('div', 'plan-actions')
    actions.append(button('Keep', 'quiet', () => on.keep(r.id), busy))
    li.append(actions)
  }
  const n = notice(view.notices.get(r.id))
  if (n) li.append(n)
  return li
}

function draw(plan: Plan, column: Column, needs: NeedsRow[], view: PlanBoardView, now: number, on: PlanHandlers): HTMLElement[] {
  const out: HTMLElement[] = []
  const empty = plan.lines.length === 0 && plan.landings.length === 0 && plan.themes.length === 0 && plan.rulings.length === 0 && plan.agents.length === 0 && needs.length === 0
  if (empty) {
    const wrap = make('div', 'plan-none')
    wrap.append(make('h2', 'plan-clear', WORDS.nothingPlanned), make('p', 'plan-empty', WORDS.nothingPlannedDetail))
    return [wrap]
  }
  for (const part of [factsLine(column), needsSection(needs, now, on), initiativesSection(column, view, on), pipelineSection(column, on), footerLine(column.footer, on)]) {
    if (part) out.push(part)
  }

  if (plan.hasRulings) {
    const open = openRulings(plan)
    const past = pastRulings(plan)
    if (open.length || past.length) {
      const wrap = make('section', 'plan-section plan-rulings')
      wrap.append(heading(WORDS.decidedForYou, open.length))
      if (open.length > 1) {
        const all = button('Keep All', 'quiet plan-keep-all', on.keepAll, view.busy.has('all'))
        wrap.append(all)
        const n = notice(view.notices.get('all'))
        if (n) wrap.append(n)
      }
      if (open.length) {
        const ul = make('ul', 'plan-list')
        ul.append(...open.map(r => rulingCard(r, view, on)))
        wrap.append(ul)
      }
      if (past.length) {
        const fold = make('details', 'plan-fold')
        fold.append(make('summary', '', WORDS.pastDecisions))
        const ul = make('ul', 'plan-list')
        ul.append(...past.map(r => rulingCard(r, view, on)))
        fold.append(ul)
        wrap.append(fold)
      }
      out.push(wrap)
    }
  }
  return out
}

/// Draw `view` into `root`, or leave it be when it would look the same (a poll
/// that found nothing new must not take a button out from under a click, or an
/// open fold closed). Returns what was drawn, to hand back as `last`.
export function renderPlan(root: HTMLElement, view: PlanBoardView, now: number, on: PlanHandlers, last = '', needs: NeedsRow[] = []): string {
  const signature = JSON.stringify([needs, view.state, view.banner, view.plan, view.workspace, view.workspaces, view.editing?.theme ?? null, [...view.busy], [...view.notices], Math.floor(now / 60_000)])
  if (signature === last) return last

  // An open fold stays open across a redraw.
  const open = root.querySelector('details.plan-fold')?.hasAttribute('open') ?? false
  const typing = view.editing !== null
  const children: HTMLElement[] = []
  if (view.banner) {
    const banner = make('p', 'note plan-banner', view.banner)
    banner.setAttribute('role', 'status')
    children.push(banner)
  }
  if (view.state === 'loading') children.push(make('p', 'plan-empty', 'Reading the plan…'))
  else if (view.state === 'ready' && view.plan) children.push(...draw(view.plan, columnOf(view.plan, needs.length, statusesOf(view.plan)), needs, view, now, on))
  else if (view.state === 'ready') children.push(make('h2', 'plan-clear', WORDS.nothingPlanned))
  root.replaceChildren(...children)
  if (open) root.querySelector('details.plan-fold')?.setAttribute('open', '')
  if (typing) root.querySelector<HTMLInputElement>('.plan-next-input')?.focus()
  return signature
}
