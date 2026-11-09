/// What the page adds to the plan column (ov-875, ov-1016): Needs You's rows,
/// which are the runner's own list that the plan doesn't carry, and the ticket
/// lists a footer count opens. The column itself (the facts, the initiatives,
/// the agent pipeline, every word and count) is crates/client's `view.column`,
/// drawn by plan-view.ts; nothing here sorts or words it.
///
/// Nothing here touches the page, so a test plays a plan by hand.

import type { Plan } from './plan'
import type { Count, PlanViewModel } from './plan-present'

/// A board status from the plan's card word: "In Progress" is `in_progress`,
/// "Cancelled" and "Canceled" are `cancelled`.
export function statusOf(word: string): string {
  const w = word.trim().toLowerCase().replace(/\s+/g, '_')
  if (w === 'to_do') return 'todo'
  if (w === 'canceled') return 'cancelled'
  return w
}

/// The statuses the plan carries, by task.
export function statusesOf(plan: Plan): Map<string, string> {
  const out = new Map<string, string>()
  for (const c of plan.cards) if (c.status) out.set(c.task, statusOf(c.status))
  return out
}

export type CountKind = Count['kind']

/// The tickets a footer count opens. Waiting ones are the view's, whole; the
/// rest are the cards the plan carries in that state, so the list can be
/// shorter than the count (the board has more cards than a plan names).
export function listOf(plan: Plan, view: PlanViewModel, kind: CountKind): { key: string; task: string; title: string; theme: string | null }[] {
  const themeOf = (task: string): string | null => plan.themes.find(t => t.tasks.includes(task))?.name ?? null
  const waiting = view.column.waiting
  if (kind === 'waiting') return waiting.map(t => ({ key: t.key, task: t.task, title: t.title, theme: themeOf(t.task) }))
  const statuses = statusesOf(plan)
  const inWaiting = new Set(waiting.map(t => t.task))
  const want = kind === 'done' ? ['done'] : ['backlog', 'todo']
  return plan.cards
    .filter(c => want.includes(statuses.get(c.task) ?? '') && !(kind === 'not_started' && inWaiting.has(c.task)))
    .map(c => ({ key: c.key, task: c.task, title: c.title || c.key, theme: themeOf(c.task) }))
}

// MARK: Needs You

/// One Needs You line: its title, what it asks, how long it has waited, and
/// the words a click leaves in the orchestrator's input.
export interface NeedsRow {
  id: string
  title: string
  detail: string
  since: number | null
  quote: string
}

/// `raw` as one line that does nothing when it reaches a composer: escape
/// sequences, control characters, line breaks and invisible format characters
/// removed, runs of space made one (the Mac's `AskAboutTask.oneLine`).
export function oneLine(raw: string): string {
  let out = ''
  const chars = [...raw]
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i].codePointAt(0) as number
    if (c === 0x1b) {
      const kind = chars[++i]
      if (kind === '[') {
        // CSI: parameters, then one final byte, 0x40 to 0x7E.
        while (++i < chars.length) {
          const n = chars[i].codePointAt(0) as number
          if (n >= 0x40 && n <= 0x7e) break
        }
      } else if (kind === ']') {
        // OSC: up to BEL, or ESC and a backslash.
        while (++i < chars.length) {
          const n = chars[i].codePointAt(0) as number
          if (n === 0x07) break
          if (n === 0x1b) {
            i++
            break
          }
        }
      }
    } else if ([0x09, 0x0a, 0x0d, 0x0b, 0x0c, 0x85, 0x2028, 0x2029].includes(c)) out += ' '
    else if (
      c <= 0x1f ||
      (c >= 0x7f && c <= 0x9f) ||
      c === 0xad ||
      (c >= 0x200b && c <= 0x200f) ||
      (c >= 0x202a && c <= 0x202e) ||
      (c >= 0x2060 && c <= 0x2064) ||
      (c >= 0x2066 && c <= 0x2069) ||
      c === 0xfeff
    ) {
      // dropped
    } else out += chars[i]
  }
  return out.split(' ').filter(Boolean).join(' ')
}

/// What a Needs You row leaves in the orchestrator's input: a reference for
/// the person to finish, never sent. "About ov-1 (“Site copy”): ".
export function quote(key: string | null, title: string): string {
  if (key) return `About ${oneLine(key)} (“${oneLine(title)}”): `
  return `About “${oneLine(title)}”: `
}

/// The shape of a Needs You item this needs.
export interface NeedsItem {
  id: string
  since: number | null
  question: string
  task: { key: string; title: string } | null
  terminal: { label: string } | null
}

/// The rows: each initiative asking the owner first (the view says which do;
/// the ask's own words are the plan's), then the runner's list.
export function needsRows(view: PlanViewModel | null, plan: Plan | null, items: NeedsItem[]): NeedsRow[] {
  const rows: NeedsRow[] = []
  for (const i of view ? view.column.initiatives.filter(i => i.asksOwner) : []) {
    const ask = plan?.themes.find(t => t.id === i.id)?.ask ?? ''
    if (ask) rows.push({ id: `theme:${i.id}`, title: i.name, detail: ask, since: null, quote: quote(null, `${i.name}: ${ask}`) })
  }
  for (const item of items) {
    const title = item.task?.title || item.question
    const detail = item.task ? item.question : item.terminal?.label ?? ''
    rows.push({ id: item.id, title, detail, since: item.since, quote: quote(item.task?.key || null, title) })
  }
  return rows
}
