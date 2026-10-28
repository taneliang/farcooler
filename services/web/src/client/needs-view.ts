/// Needs You on the page (ov-773): the board's view drawn into a list.
///
/// Every value is the runner's or a repository's, so each goes in through
/// `textContent` and never markup (the CSP's Trusted Types refuse an HTML
/// string sink). The one link is checked to be github.com's (`githubLink`).

import { age, type BoardView, buttonsFor, githubLink, groups, type Item, kindLabel, type Press } from './needs'

export interface Handlers {
  /// Whether a terminal is one the page lists, so an Open can go to it.
  canOpen(terminal: string): boolean
  press(item: Item, press: Press): void
}

const make = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const el = document.createElement(tag)
  if (className) el.className = className
  if (text) el.textContent = text
  return el
}

/// Where the work is: `Billing · bil-7 Invoice PDF export`.
function context(item: Item): string {
  const parts: string[] = []
  if (item.workspace) parts.push(item.workspace)
  if (item.task) parts.push([item.task.key, item.task.title].filter(Boolean).join(' '))
  return parts.join(' · ')
}

function card(item: Item, view: BoardView, now: number, on: Handlers): HTMLElement {
  const li = make('li', `need need-${item.kind}`)
  const head = make('div', 'need-head')
  head.append(make('span', 'need-kind', kindLabel(item.kind)))
  const when = age(item.since, now)
  if (when) head.append(make('span', 'need-age', when))
  li.append(head)
  li.append(make('p', 'need-question', item.question))
  if (item.detail && !item.question.toLowerCase().includes(item.detail.toLowerCase())) li.append(make('pre', 'need-detail', item.detail))

  const where = context(item)
  if (where) li.append(make('p', 'need-where', where))
  if (item.worktree?.branch) {
    const lane = make('p', 'need-where')
    lane.append(make('span', 'need-branch', item.worktree.branch))
    if (item.worktree.insertions || item.worktree.deletions) {
      lane.append(make('span', 'need-plus', `+${item.worktree.insertions}`), make('span', 'need-minus', `−${item.worktree.deletions}`))
    }
    li.append(lane)
  }
  if (item.concern && !item.concern.yours) li.append(make('p', 'need-where', item.concern.name ? `${item.concern.name}’s` : 'The owner’s'))

  const busy = view.busy.has(item.id)
  const row = make('div', 'need-actions')
  for (const { action, press } of buttonsFor(item, on.canOpen)) {
    const button = make('button', action.destructive ? 'quiet danger' : action.primary ? '' : 'quiet', action.title)
    button.type = 'button'
    button.disabled = busy
    button.onclick = () => on.press(item, press)
    row.append(button)
  }
  const link = githubLink(item.link)
  if (link) {
    const a = make('a', 'need-link', 'View on GitHub')
    a.href = link
    a.target = '_blank'
    a.rel = 'noopener noreferrer'
    row.append(a)
  }
  if (row.childElementCount) li.append(row)
  const notice = view.notices.get(item.id)
  if (notice) {
    const p = make('p', 'need-notice', notice)
    p.setAttribute('role', 'alert')
    li.append(p)
  }
  if (busy) li.setAttribute('aria-busy', 'true')
  return li
}

function section(title: string, items: Item[], view: BoardView, now: number, on: Handlers): HTMLElement {
  const wrap = make('section', 'need-group')
  if (title) wrap.append(make('h2', 'need-group-title', title))
  const list = make('ul', 'need-list')
  for (const item of items) list.append(card(item, view, now, on))
  wrap.append(list)
  return wrap
}

/// Draw `view` into `root`, or leave it be when it would look the same (a
/// poll that found nothing new must not take a button out from under a click).
/// Returns what was drawn, to hand back as `last`.
export function renderNeeds(root: HTMLElement, view: BoardView, now: number, on: Handlers, last = ''): string {
  const signature = JSON.stringify([view.state, view.banner, view.items.map(i => [i, age(i.since, now)]), [...view.busy], [...view.notices]])
  if (signature === last) return last

  const children: HTMLElement[] = []
  if (view.banner) {
    const banner = make('p', 'note need-banner', view.banner)
    banner.setAttribute('role', 'status')
    children.push(banner)
  }
  if (view.state === 'loading') {
    children.push(make('p', 'need-empty', 'Checking what needs you…'))
  } else if (view.state === 'ready' && view.items.length === 0) {
    children.push(make('h2', 'need-clear', 'Nothing needs you'))
  } else if (view.state === 'ready') {
    const g = groups(view.items)
    if (g.split) {
      if (g.yours.length) children.push(section('Yours', g.yours, view, now, on))
      children.push(section('Everyone Else', g.others, view, now, on))
    } else {
      children.push(section('', view.items, view, now, on))
    }
  }
  root.replaceChildren(...children)
  return signature
}
