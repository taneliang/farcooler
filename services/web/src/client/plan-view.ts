/// The plan on the page (ov-775): the board's view drawn as the Mac's sections,
/// Now (Landing, In Review, Building, Next Up), Themes, and Decided For You.
///
/// Every value is the runner's or the orchestrator's, so each goes in through
/// `textContent` and never markup (the CSP's Trusted Types refuse an HTML
/// string sink).

import {
  cardsLine,
  laneStateTitle,
  liveThemes,
  needsAttention,
  openRulings,
  pastRulings,
  phaseTitle,
  type Lane,
  type Plan,
  type PlanBoardView,
  rulingStateTitle,
  type Ruling,
  sectionsOf,
  since,
  type Theme,
  type Train,
  WORDS,
} from './plan'

export interface PlanHandlers {
  choose(workspace: string): void
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

function heading(title: string, count?: number): HTMLElement {
  const h = make('h2', 'plan-title', title)
  if (count !== undefined && count > 0) h.append(make('span', 'plan-count', String(count)))
  return h
}

function trainCard(train: Train, lanes: Lane[], now: number): HTMLElement {
  const li = make('li', needsAttention(train.phase) ? 'plan-card plan-attention' : 'plan-card')
  const head = make('div', 'plan-head')
  head.append(make('span', 'plan-name', train.heading), make('span', needsAttention(train.phase) ? 'plan-phase plan-phase-amber' : 'plan-phase', phaseTitle(train.phase)))
  li.append(head)
  if (train.summary) li.append(make('p', 'plan-line', train.summary))
  const rows = (lanes.length ? lanes.map(l => l.heading) : train.lanes.map(l => l.name)).filter(r => r !== train.summary)
  if (rows.length) li.append(make('p', 'plan-sub', rows.join(' · ')))
  const when = since(train.phaseSince, now)
  if (when) li.append(make('p', 'plan-sub', `${when} in this step`))
  return li
}

function laneRow(lane: Lane, now: number): HTMLElement {
  const li = make('li', 'plan-card')
  const head = make('div', 'plan-head')
  head.append(make('span', 'plan-name', lane.heading), make('span', 'plan-phase', laneStateTitle(lane.state)))
  li.append(head)
  if (lane.reason) li.append(make('p', 'plan-line', lane.reason))
  const meta = [lane.cards.join(', '), lane.agent ?? '', since(lane.stateSince, now)].filter(Boolean)
  if (meta.length) li.append(make('p', 'plan-sub', meta.join(' · ')))
  return li
}

function section(title: string, count: number, body: HTMLElement[], empty?: string): HTMLElement {
  const wrap = make('section', 'plan-section')
  wrap.append(heading(title, count))
  if (body.length) {
    const ul = make('ul', 'plan-list')
    ul.append(...body)
    wrap.append(ul)
  } else if (empty) {
    wrap.append(make('p', 'plan-empty', empty))
  }
  return wrap
}

function notice(text: string | undefined): HTMLElement | null {
  if (!text) return null
  const p = make('p', 'plan-notice', text)
  p.setAttribute('role', 'alert')
  return p
}

function themeCard(theme: Theme, view: PlanBoardView, on: PlanHandlers): HTMLElement {
  const li = make('li', 'plan-card')
  const busy = view.busy.has(theme.id)
  if (busy) li.setAttribute('aria-busy', 'true')
  const head = make('div', 'plan-head')
  head.append(make('span', 'plan-name', theme.name), make('span', 'plan-phase', theme.state === 'active' ? cardsLine(theme) : `${theme.state[0].toUpperCase()}${theme.state.slice(1)} · ${cardsLine(theme)}`))
  li.append(head)
  if (theme.outcome) li.append(make('p', 'plan-line', theme.outcome))
  if (theme.story) li.append(make('p', 'plan-sub', theme.story))
  if (theme.ask) {
    const ask = make('p', 'plan-ask')
    ask.append(make('span', 'plan-label', 'Asking You'), document.createTextNode(` ${theme.ask}`))
    li.append(ask)
  }

  const editing = view.editing?.theme === theme.id ? view.editing : null
  if (editing) {
    const form = make('form', 'plan-next-edit')
    const label = make('label', 'plan-label', 'Next')
    const input = make('input', 'plan-next-input')
    input.type = 'text'
    input.value = editing.draft
    input.maxLength = 500
    input.placeholder = 'What happens next'
    input.disabled = busy
    input.id = `next-${theme.id}`
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
    const row = make('div', 'plan-next')
    const text = make('p', 'plan-line')
    text.append(make('span', 'plan-label', 'Next'), document.createTextNode(theme.next ? ` ${theme.next}` : ' Nothing yet'))
    if (!theme.next) text.classList.add('plan-dim')
    row.append(text, button(theme.next ? 'Edit' : 'Set Next', 'quiet', () => on.edit(theme.id), busy))
    li.append(row)
  }
  const n = notice(view.notices.get(theme.id))
  if (n) li.append(n)
  return li
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

function draw(plan: Plan, view: PlanBoardView, now: number, on: PlanHandlers): HTMLElement[] {
  const s = sectionsOf(plan)
  const out: HTMLElement[] = []
  const empty = plan.lanes.length === 0 && plan.trains.length === 0 && plan.themes.length === 0 && plan.rulings.length === 0
  if (empty) {
    const wrap = make('div', 'plan-none')
    wrap.append(make('h2', 'plan-clear', WORDS.nothingPlanned), make('p', 'plan-empty', WORDS.nothingPlannedDetail))
    return [wrap]
  }

  const nowBlock = make('div', 'plan-now')
  nowBlock.append(
    section(WORDS.landing, s.landing.length, s.landing.map(l => trainCard(l.train, l.lanes, now)), WORDS.nothingLanding),
  )
  if (s.inReview.length) nowBlock.append(section(WORDS.inReview, s.inReview.length, s.inReview.map(l => laneRow(l, now))))
  nowBlock.append(section(WORDS.building, s.building.length, s.building.map(l => laneRow(l, now)), WORDS.nothingBuilding))
  nowBlock.append(section(WORDS.nextUp, s.nextUp.length, s.nextUp.map(l => laneRow(l, now)), s.notRanked.length ? undefined : WORDS.nothingQueued))
  if (s.notRanked.length) nowBlock.append(section(WORDS.notRanked, s.notRanked.length, s.notRanked.map(l => laneRow(l, now))))
  out.push(nowBlock)

  const themes = liveThemes(plan)
  if (themes.length) out.push(section(WORDS.themes, themes.length, themes.map(t => themeCard(t, view, on))))

  if (plan.hasRulings) {
    const open = openRulings(plan)
    const past = pastRulings(plan)
    if (open.length || past.length) {
      const wrap = make('section', 'plan-section')
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
export function renderPlan(root: HTMLElement, view: PlanBoardView, now: number, on: PlanHandlers, last = ''): string {
  const signature = JSON.stringify([view.state, view.banner, view.plan, view.workspace, view.workspaces, view.editing?.theme ?? null, [...view.busy], [...view.notices], Math.floor(now / 60_000)])
  if (signature === last) return last

  // An open fold stays open across a redraw.
  const open = root.querySelector('details.plan-fold')?.hasAttribute('open') ?? false
  const typing = view.editing !== null
  const children: HTMLElement[] = []
  if (view.workspaces.length > 1) {
    const select = make('select', 'plan-board')
    select.setAttribute('aria-label', 'Board')
    for (const w of view.workspaces) {
      const o = make('option', '', w.name)
      o.value = w.id
      o.selected = w.id === view.workspace
      select.append(o)
    }
    select.onchange = () => on.choose(select.value)
    children.push(select)
  }
  if (view.banner) {
    const banner = make('p', 'note plan-banner', view.banner)
    banner.setAttribute('role', 'status')
    children.push(banner)
  }
  if (view.state === 'loading') children.push(make('p', 'plan-empty', 'Reading the plan…'))
  else if (view.state === 'ready' && view.plan) children.push(...draw(view.plan, view, now, on))
  else if (view.state === 'ready') children.push(make('h2', 'plan-clear', WORDS.nothingPlanned))
  root.replaceChildren(...children)
  if (open) root.querySelector('details.plan-fold')?.setAttribute('open', '')
  if (typing) root.querySelector<HTMLInputElement>('.plan-next-input')?.focus()
  return signature
}
