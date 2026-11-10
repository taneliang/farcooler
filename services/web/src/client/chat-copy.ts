/// The words a conversation view says (ov-776): ported from AgentKit's
/// `AgentConversation` and `ConversationItems`, so the browser reads a tool
/// run, a turn's ending or an ask the way the Mac does. Pure strings, no DOM.

import { bodyOf, type Row } from './chat-rows'
import type { PlanPresenter } from './plan-present'

// The words the apps share come from crates/client's `conversation_words`
// through wasm's `planView` (ov-1049, ruling R-112): one table per agent, asked
// once. app.ts hands the asker in; a test hands in the golden
// (test/fixtures/conversation-words.json). Blank while nothing can answer.
let asker: PlanPresenter | null = null
const tables = new Map<string, any>()

export function useConversationWords(ask: PlanPresenter): void {
  asker = ask
  tables.clear()
}

function table(agent = 'Claude'): any {
  if (tables.has(agent)) return tables.get(agent)
  try {
    const found = JSON.parse(asker?.(JSON.stringify({ schema: 1, conversation: { style: 'apple', agent } })) ?? 'null')?.conversation
    if (found) tables.set(agent, found)
    return found ?? null
  } catch {
    return null
  }
}

/// The sentence for a send the runner refused with `what`, as the apps say it.
export const refused = (what: string): string => String(table()?.send?.[what]?.text ?? '')

/// A phrase with its `{placeholders}` filled: `name_<by>` where the table has one, else `name`.
function phrase(name: string, fill: Record<string, string | number> = {}, by?: string, agent = 'Claude'): string {
  const phrases = table(agent)?.phrases ?? {}
  const words = String((by !== undefined && phrases[`${name}_${by}`]) || phrases[name] || '')
  return Object.entries(fill).reduce((line, [key, value]) => line.replaceAll(`{${key}}`, String(value)), words)
}

/// One item's logic, asked of the core (`conversation_words::item`); blank while nothing can answer.
function item(asked: Record<string, string | number>): any {
  try {
    return JSON.parse(asker?.(JSON.stringify({ schema: 1, item: asked })) ?? 'null')?.item ?? {}
  } catch {
    return {}
  }
}

/// Minutes and seconds, "1:05", or with hours, "1:02:03".
export const shortDuration = (ms: number): string => item({ ms: Math.round(ms) }).ms ?? '0:00'

/// How a finished turn ended, or null while it is open (`outcome`).
export function outcomeLine(turn: any): string | null {
  const outcome = turn.outcome
  if (outcome === null || outcome === undefined) return null
  if (outcome === 'Finished') return typeof turn.duration_ms === 'number' ? phrase('outcome_took', { time: shortDuration(turn.duration_ms) }) : phrase('outcome_done')
  if (outcome === 'Interrupted') return phrase('outcome_interrupted')
  if (outcome === 'Unrecorded') return phrase('outcome_unrecorded')
  if (typeof outcome === 'object' && 'Failed' in outcome) {
    const detail = String(outcome.Failed?.detail ?? '')
    return detail ? phrase('outcome_failed_detail', { detail }) : phrase('outcome_failed')
  }
  return phrase('outcome_ended')
}

/// Whether the agent is working on this turn: open, and its process busy
/// (`AgentConversation.isWorking`).
export function isWorking(turn: any): boolean {
  return (turn.outcome === null || turn.outcome === undefined) && turn.activity === 'Busy'
}

/// What a turn's footer says about where its prompt came from, if anything.
export function originNote(turn: any): string | null {
  return turn.origin === 'Queued' ? phrase('origin_queued') : null
}

/// A turn nobody typed: a background task finishing, or claude waking itself.
export const isNotice = (turn: any): boolean => item({ origin: String(turn.origin) }).origin === true

export const scheduledTask = (): string => phrase('scheduled_task')

/// A queued message's label (`queuedLabel`).
export function queuedLabel(state: string): string {
  return phrase('queued', {}, state)
}

/// An ask's heading (`askTitle`).
export function askTitle(ask: any, agent = 'Claude'): string {
  if ((ask.answered || !ask.held) && ask.answered_by) return phrase('ask_answered_on', { by: ask.answered_by })
  if (ask.answered) return phrase('ask_answered')
  return phrase('ask', {}, ask.kind, agent)
}

/// A subagent's type as words: `general-purpose` reads "General purpose".
export const agentType = (raw: string): string => item({ agent_type: raw }).agent_type ?? 'Agent'

/// "1 tool", "14 tools", or nothing while the count is unknown.
export function toolsLabel(count: number): string | null {
  return count <= 0 ? null : phrase(count === 1 ? 'tools_one' : 'tools', { n: count })
}

/// A subagent's state as one word.
export function subagentState(status: unknown): string {
  if (status === 'Running') return 'Running'
  if (status === 'Completed') return 'Done'
  if (status === 'Failed') return 'Failed'
  if (status === 'Killed') return 'Stopped'
  return typeof status === 'string' ? status : 'Ended'
}

/// "2 of 5 done".
export function taskProgress(items: { status: string }[]): string {
  return phrase('tasks_progress', { done: items.filter(i => i.status === 'Completed').length, n: items.length })
}

// MARK: Tool runs

/// A run's line: what its calls did, and how many. "Ran 3 commands" when every
/// call is a shell command, "Read 2 files", "Edited 4 files", "Ran 2
/// searches", else "Used 5 tools"; in the present tense while any is running
/// (`groupTitle`).
export function groupTitle(tools: any[]): string {
  const names = tools.map(t => String(t.name))
  const group = (table()?.groups ?? []).find((g: any) => !g.names.length || names.every(name => g.names.includes(name)))
  const line = tools.some(t => t.status === 'Running') ? group?.running : group?.done
  return String(line ?? '').replaceAll('{n}', String(tools.length))
}

/// A run's state: running while any call is, failed if any failed, else done.
export function groupStatus(tools: any[]): 'Running' | 'Failed' | 'Done' {
  if (tools.some(t => t.status === 'Running')) return 'Running'
  if (tools.some(t => t.status === 'Failed')) return 'Failed'
  return 'Done'
}

/// One summary as a phrase inside a sentence: a path as its file's name, and a
/// sentence's capital lowered, unless the word is all capitals or a name like
/// "README" (`purpose(of:)`).
function purpose(summary: string): string {
  if ((summary.startsWith('/') || summary.startsWith('~/')) && !summary.includes(' ')) return summary.slice(summary.lastIndexOf('/') + 1)
  const [a, b] = [...summary]
  if (!a || !b || a !== a.toUpperCase() || a === a.toLowerCase() || b !== b.toLowerCase() || b === b.toUpperCase()) return summary
  return a.toLowerCase() + summary.slice(a.length)
}

/// What a run's calls were for, from their own summaries: each said once, in
/// order, joined by commas; a view cuts it to fit (`groupPurpose`).
export function groupPurpose(tools: any[]): string {
  const seen = new Set<string>()
  const phrases: string[] = []
  for (const tool of tools) {
    const summary = String(tool.summary ?? '').trim()
    const phrase = summary ? purpose(summary) : tool.name
    if (!seen.has(phrase)) {
      seen.add(phrase)
      phrases.push(phrase)
    }
  }
  return phrases.join(', ')
}

// MARK: Turns claude wrote for itself

/// The row's heading: "Report from <the agent's description>", else from a
/// short id (`harnessTitle`).
export function harnessTitle(harness: any): string {
  if (harness.kind === 'Status') return String(harness.title ?? '')
  return phrase('harness', { name: harnessAgentName(harness) }, harness.kind)
}

function harnessAgentName(harness: any): string {
  if (harness.source) return String(harness.source)
  return harness.agent_id ? phrase('harness_agent', { id: String(harness.agent_id).slice(0, 6) }) : phrase('harness_a_subagent')
}

/// A notice turn's words, without claude's straight quotes.
export function noticeText(turn: any): string {
  return String(turn.prompt ?? '').replace(/"/g, '')
}

/// A notice row's words: its own text when the runner gave some, else the
/// kind's.
export function noticeLine(row: Row): string {
  const notice = bodyOf(row)
  const said = String(notice.text ?? '').trim()
  if (said) return said
  switch (notice.kind) {
    case 'Compacted':
      return 'Earlier messages were summarized to make room'
    case 'Cleared':
      return 'Started a new conversation'
    case 'Resumed':
      return 'Picked the conversation back up'
    default:
      return ''
  }
}

/// The handoff row: something only the terminal can show.
export function handoffLine(agent = 'Claude'): string {
  return phrase('handoff', {}, undefined, agent)
}

/// What a gap in the transcript says.
export function gapLine(gap: any): string {
  return gap.count > 1 ? `${gap.count} parts of the conversation couldn’t be shown` : 'Part of the conversation couldn’t be shown'
}

// MARK: Failures

/// Why the conversation can't be shown or reached, by the stable word the wasm
/// module gives. No Rust string reaches the screen.
export function chatTrouble(word: string, runner: string): string {
  switch (word) {
    case 'runner_too_old':
      return `${runner} needs a newer Far Cooler to show conversations here. Update it, then reload this page.`
    case 'closed':
    case 'no_answer':
      return `Lost touch with ${runner}. Reconnecting…`
    default:
      return `Couldn’t update this conversation. Trying again…`
  }
}

/// Why a message or Stop didn't go through. A message that failed stays in the
/// box, so it is never lost.
export function sendFailure(word: string, runner: string, what: 'send' | 'stop', refusal = ''): string {
  if (word === 'runner_too_old')
    return `${runner} needs a newer Far Cooler to ${what === 'send' ? 'take messages' : 'stop a turn'} from here. Update it, then try again.`
  if (word === 'refused') {
    // The runner's own word for what it refused, in the apps' sentences
    // (AgentConversation.swift).
    if (what === 'send') {
      if (['images_too_large', 'image_too_large', 'images', 'image'].includes(refusal)) return refused(refusal)
    }
    return what === 'send'
      ? 'The agent didn’t take that message. It may be showing a dialog only the terminal can answer.'
      : 'The agent didn’t stop. It may be showing a dialog only the terminal can answer.'
  }
  return what === 'send'
    ? `Your message didn’t reach ${runner}. It’s still in the box. Try sending it again.`
    : `Stop didn’t reach ${runner}. Try again.`
}
