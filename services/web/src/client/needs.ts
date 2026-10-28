/// Needs You in the browser (ov-773, plan-763 slice 10): everything a person
/// has to act on for the runner this tab is connected to, and what it takes to
/// answer one.
///
/// The list is the runner's own, in the shape every device reads
/// (`needs_you_json`, pinned by test/fixtures/needs-you.json): the daemon
/// decides what an item is and how it ranks, and this keeps its order. What is
/// here is only what a page does with it: read it, count it, offer the buttons
/// the runner offered, send the one chosen, and say in words why a refusal
/// was one.
///
/// Nothing here touches the page or wasm, so a test runs it with a runner it
/// plays by hand. The DOM is needs-view.ts.

import { answerRefusal, needsYouFailure } from './copy'

/// What an item is about, in the daemon's words. A kind this build doesn't
/// know is `unknown`: it is listed and counted, never dropped.
export type Kind = 'ask' | 'blocked' | 'decision' | 'review' | 'pr_review' | 'pr_thread' | 'claude_login' | 'fyi' | 'unknown'

const KINDS = new Set<string>(['ask', 'blocked', 'decision', 'review', 'pr_review', 'pr_thread', 'claude_login', 'fyi'])

export interface Action {
  id: string
  title: string
  destructive: boolean
  primary: boolean
}

export interface Item {
  id: string
  kind: Kind
  /// Unix milliseconds by the runner's clock, or null: shows no age.
  since: number | null
  workspace: string
  task: { id: string; key: string; title: string } | null
  terminal: { id: string; label: string } | null
  worktree: { branch: string; insertions: number; deletions: number } | null
  question: string
  detail: string | null
  askId: string | null
  link: string | null
  actions: Action[]
  concern: { name: string; yours: boolean } | null
}

const text = (v: unknown): string => (typeof v === 'string' ? v : '')
const orNull = (v: unknown): string | null => (typeof v === 'string' ? v : null)

/// The items in the runner's `{"items": [...]}`, in its order. An item that
/// isn't an object with an id and a question is left out rather than failing
/// the list, and a missing optional key reads as none.
export function parseList(json: string): Item[] {
  let raw: any
  try {
    raw = JSON.parse(json)
  } catch {
    return []
  }
  if (!Array.isArray(raw?.items)) return []
  const items: Item[] = []
  for (const r of raw.items) {
    if (!r || typeof r.id !== 'string' || typeof r.question !== 'string') continue
    items.push({
      id: r.id,
      kind: KINDS.has(r.kind) ? r.kind : 'unknown',
      since: typeof r.since === 'number' ? r.since : null,
      workspace: text(r.workspace_name),
      task: r.task && typeof r.task.id === 'string' ? { id: r.task.id, key: text(r.task.key), title: text(r.task.title) } : null,
      terminal: r.terminal && typeof r.terminal.id === 'string' ? { id: r.terminal.id, label: text(r.terminal.label) } : null,
      worktree: r.worktree
        ? { branch: text(r.worktree.branch), insertions: Number(r.worktree.insertions) || 0, deletions: Number(r.worktree.deletions) || 0 }
        : null,
      question: r.question,
      detail: orNull(r.detail),
      askId: orNull(r.ask_id),
      link: orNull(r.link),
      actions: Array.isArray(r.actions)
        ? r.actions
            .filter((a: any) => a && typeof a.id === 'string' && typeof a.title === 'string')
            .map((a: any) => ({ id: a.id, title: a.title, destructive: a.destructive === true, primary: a.primary === true }))
        : [],
      concern: r.concern && typeof r.concern === 'object' ? { name: text(r.concern.name), yours: r.concern.yours === true } : null,
    })
  }
  return items
}

/// Whether an item adds to the count. A quiet notice (a member kept a ruling)
/// is read and never acted on, so it is a row and nothing more.
export const isCounted = (item: Item): boolean => item.kind !== 'fyi'

export const countOf = (items: Item[]): number => items.filter(isCounted).length

/// The tab's title: the count in front when something needs the person.
export function pageTitle(count: number): string {
  return count > 0 ? `(${count}) Far Cooler` : 'Far Cooler'
}

/// What the Needs You button and the bar's count say.
export function countLabel(count: number): string {
  return count > 0 ? `Needs You (${count})` : 'Needs You'
}

/// What an item is, in a word or three.
export function kindLabel(kind: Kind): string {
  switch (kind) {
    case 'ask':
      return 'Asks to Run'
    case 'blocked':
      return 'Blocked'
    case 'decision':
      return 'Decision'
    case 'review':
      return 'Ready for Review'
    case 'pr_review':
      return 'Pull Request to Review'
    case 'pr_thread':
      return 'Review Thread'
    case 'claude_login':
      return 'Claude Sign-In'
    case 'fyi':
      return 'Note'
    case 'unknown':
      return 'Needs You'
  }
}

/// How long ago, from the runner's `since`: nothing when it didn't say.
export function age(since: number | null, now: number): string {
  if (since === null) return ''
  const seconds = Math.max(0, Math.floor((now - since) / 1000))
  if (seconds < 60) return 'Just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} min`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} hr`
  return `${Math.floor(hours / 24)} d`
}

/// The reader's own items and nobody's, then everyone else's. A list with
/// nothing of anyone else's isn't split, as on a phone.
export function groups(items: Item[]): { yours: Item[]; others: Item[]; split: boolean } {
  const others = items.filter(i => i.concern !== null && !i.concern.yours)
  return { yours: items.filter(i => !(i.concern !== null && !i.concern.yours)), others, split: others.length > 0 }
}

/// A pull request's or review thread's link, if it is github.com's: https, a
/// bare host and that host, whole. It is text from a repository, so it is
/// checked here too (AgentKit's GitHubLink, the same rule).
export function githubLink(link: string | null): string | null {
  if (!link || /[\s\\]/.test(link)) return null
  let url: URL
  try {
    url = new URL(link)
  } catch {
    return null
  }
  const ok = url.protocol === 'https:' && url.username === '' && url.password === '' && url.port === '' && url.hostname === 'github.com'
  return ok && /^https:\/\/github\.com(\/|$)/i.test(link) ? url.href : null
}

/// What pressing a button does. `open` goes to the item's terminal and sends
/// nothing.
export type Press =
  | { kind: 'ask'; terminal: string; ask: string; option: string }
  | { kind: 'decision'; task: string; body: string }
  | { kind: 'open'; terminal: string }

/// The button's effect, or null when this browser can't do it: a login's
/// sign-in, or an answer below Control, where the runner leaves out the ask's
/// id and the buttons with it.
export function pressFor(item: Item, action: Action): Press | null {
  if (action.id === 'open') return item.terminal ? { kind: 'open', terminal: item.terminal.id } : null
  if (item.kind === 'ask') return item.askId && item.terminal ? { kind: 'ask', terminal: item.terminal.id, ask: item.askId, option: action.id } : null
  // A decision's buttons are its options as written; the option's text is the
  // answer (`task.note`). A review thread's (`Retry`) is answered the same way.
  if (item.kind === 'decision' || item.kind === 'pr_thread') return item.task ? { kind: 'decision', task: item.task.id, body: action.id } : null
  return null
}

/// The buttons to draw for an item: the runner's, minus any this browser can't
/// press.
///
/// An Open is drawn only where `canOpen` says the terminal is one this page
/// lists, so no button leads nowhere.
export function buttonsFor(item: Item, canOpen: (terminal: string) => boolean = () => true): { action: Action; press: Press }[] {
  const buttons: { action: Action; press: Press }[] = []
  for (const action of item.actions) {
    const press = pressFor(item, action)
    if (press && (press.kind !== 'open' || canOpen(press.terminal))) buttons.push({ action, press })
  }
  return buttons
}

// MARK: The board

/// What the screen is showing.
export interface BoardView {
  /// `loading` until the first read lands.
  state: 'loading' | 'ready' | 'old' | 'unreadable'
  items: Item[]
  /// Items being answered right now.
  busy: ReadonlySet<string>
  /// Why an answer didn't land, by item.
  notices: ReadonlyMap<string, string>
  /// Said above the list: a read that failed after a good one, or a runner
  /// that can't show this.
  banner: string
}

/// How a rejection reaches here from wasm: the word as the message, and for a
/// refusal the runner's code and `what`.
export type Failure = Error & { code?: string; what?: string }

export interface BoardDeps {
  runner: string
  /// `needs_you.list`, as JSON.
  list(): Promise<string>
  ask(terminal: string, ask: string, option: string): Promise<void>
  decide(task: string, body: string): Promise<void>
  show(view: BoardView): void
  /// Go to a terminal (an Open button).
  open(terminal: string): void
}

export class Board {
  private items: Item[] = []
  private state: BoardView['state'] = 'loading'
  private banner = ''
  private readonly busy = new Set<string>()
  private readonly notices = new Map<string, string>()
  /// A read in flight, so a poll that lands inside one waits for it.
  private reading: Promise<void> | null = null
  private dead = false

  constructor(private readonly deps: BoardDeps) {}

  get count(): number {
    return countOf(this.items)
  }

  /// Read the list now. The runner's answer replaces what is shown, in the
  /// runner's order.
  refresh(): Promise<void> {
    if (this.dead) return Promise.resolve()
    this.reading ??= this.read().finally(() => {
      this.reading = null
    })
    return this.reading
  }

  private async read(): Promise<void> {
    try {
      this.items = parseList(await this.deps.list())
      this.state = 'ready'
      this.banner = ''
      // A notice for an item the runner no longer lists was about an answer
      // that has since landed, here or on another device.
      for (const id of [...this.notices.keys()]) if (!this.items.some(i => i.id === id)) this.notices.delete(id)
    } catch (e) {
      if (this.dead) return
      const word = (e as Error).message
      if (word === 'runner_too_old') {
        this.items = []
        this.state = 'old'
        this.banner = needsYouFailure(word, this.deps.runner)
      } else if (this.state === 'ready') {
        // The list on screen was good; say it may be behind.
        this.banner = needsYouFailure(word, this.deps.runner)
      } else {
        this.state = 'unreadable'
        this.banner = needsYouFailure(word, this.deps.runner)
      }
    }
    this.publish()
  }

  /// Press one of an item's buttons. An answer is sent at once; the list is
  /// read again either way, so a refusal that means "someone got there first"
  /// leaves the screen as the runner has it.
  async press(item: Item, press: Press): Promise<void> {
    if (this.dead) return
    if (press.kind === 'open') {
      this.deps.open(press.terminal)
      return
    }
    if (this.busy.has(item.id)) return
    this.busy.add(item.id)
    this.notices.delete(item.id)
    this.publish()
    try {
      if (press.kind === 'ask') await this.deps.ask(press.terminal, press.ask, press.option)
      else await this.deps.decide(press.task, press.body)
    } catch (e) {
      const failure = e as Failure
      this.notices.set(item.id, answerRefusal({ word: failure.message, code: failure.code, what: failure.what }, this.deps.runner, item.terminal?.label || 'the agent'))
    }
    this.busy.delete(item.id)
    await this.refresh()
    this.publish()
  }

  stop(): void {
    this.dead = true
  }

  private publish(): void {
    if (this.dead) return
    this.deps.show({ state: this.state, items: this.items, busy: new Set(this.busy), notices: new Map(this.notices), banner: this.banner })
  }
}
