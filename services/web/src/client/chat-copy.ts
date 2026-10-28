/// The words a conversation view says (ov-776): ported from AgentKit's
/// `AgentConversation` and `ConversationItems`, so the browser reads a tool
/// run, a turn's ending or an ask the way the Mac does. Pure strings, no DOM.

import { bodyOf, type Row } from './chat-rows'

/// Minutes and seconds, "1:05", or with hours, "1:02:03" (`AgentConversation.short`).
export function shortDuration(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000))
  const [h, m, s] = [Math.floor(seconds / 3600), Math.floor((seconds % 3600) / 60), seconds % 60]
  const two = (n: number) => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`
}

/// How a finished turn ended, or null while it is open (`outcome`).
export function outcomeLine(turn: any): string | null {
  const outcome = turn.outcome
  if (outcome === null || outcome === undefined) return null
  if (outcome === 'Finished') return typeof turn.duration_ms === 'number' ? `Took ${shortDuration(turn.duration_ms)}` : 'Done'
  if (outcome === 'Interrupted') return 'Interrupted'
  if (outcome === 'Unrecorded') return 'Not recorded'
  if (typeof outcome === 'object' && 'Failed' in outcome) {
    const detail = String(outcome.Failed?.detail ?? '')
    return detail ? `Failed: ${detail}` : 'Failed'
  }
  return 'Ended'
}

/// Whether the agent is working on this turn: open, and its process busy
/// (`AgentConversation.isWorking`).
export function isWorking(turn: any): boolean {
  return (turn.outcome === null || turn.outcome === undefined) && turn.activity === 'Busy'
}

/// What a turn's footer says about where its prompt came from, if anything.
export function originNote(turn: any): string | null {
  return turn.origin === 'Queued' ? 'Queued during the last turn' : null
}

/// A turn nobody typed: a background task finishing, or claude waking itself.
export function isNotice(turn: any): boolean {
  return turn.origin === 'Notification' || turn.origin === 'System'
}

export const SCHEDULED_TASK = 'Scheduled task'

/// A queued message's label (`queuedLabel`).
export function queuedLabel(state: string): string {
  switch (state) {
    case 'Withdrawn':
      return 'Withdrawn'
    case 'InBox':
      return 'Waiting to send'
    case 'Sent':
      return 'Sent mid-turn'
    default:
      return 'Queued'
  }
}

/// An ask's heading (`askTitle`).
export function askTitle(ask: any, agent = 'Claude'): string {
  if ((ask.answered || !ask.held) && ask.answered_by) return `Answered on ${ask.answered_by}`
  if (ask.answered) return 'Answered'
  switch (ask.kind) {
    case 'Permission':
      return `${agent} is asking for permission`
    case 'PlanExit':
      return `${agent} has a plan for you to review`
    default:
      return `${agent} is asking a question`
  }
}

/// A subagent's type as words: `general-purpose` reads "General purpose".
export function agentType(raw: string): string {
  const words = raw.replace(/[-_]/g, ' ')
  return words ? words[0].toUpperCase() + words.slice(1) : 'Agent'
}

/// "1 tool", "14 tools", or nothing while the count is unknown.
export function toolsLabel(count: number): string | null {
  return count <= 0 ? null : count === 1 ? '1 tool' : `${count} tools`
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
  return `${items.filter(i => i.status === 'Completed').length} of ${items.length} done`
}

// MARK: Tool runs

/// A run's line: what its calls did, and how many. "Ran 3 commands" when every
/// call is a shell command, "Read 2 files", "Edited 4 files", "Ran 2
/// searches", else "Used 5 tools"; in the present tense while any is running
/// (`groupTitle`).
export function groupTitle(tools: any[]): string {
  const n = tools.length
  const running = tools.some(t => t.status === 'Running')
  const names = new Set<string>(tools.map(t => t.name))
  const all = (these: string[]) => [...names].every(name => these.includes(name))
  if (all(['Bash', 'bash'])) return running ? `Running ${n} commands` : `Ran ${n} commands`
  if (all(['Read'])) return running ? `Reading ${n} files` : `Read ${n} files`
  if (all(['Edit', 'MultiEdit', 'Write', 'NotebookEdit'])) return running ? `Editing ${n} files` : `Edited ${n} files`
  if (all(['Grep', 'Glob'])) return running ? `Running ${n} searches` : `Ran ${n} searches`
  return running ? `Using ${n} tools` : `Used ${n} tools`
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
  switch (harness.kind) {
    case 'Report':
      return `Report from ${harnessAgentName(harness)}`
    case 'Message':
      return `Message from ${harnessAgentName(harness)}`
    case 'Status':
      return String(harness.title ?? '')
    default:
      return 'System note'
  }
}

function harnessAgentName(harness: any): string {
  if (harness.source) return String(harness.source)
  return harness.agent_id ? `agent ${String(harness.agent_id).slice(0, 6)}` : 'a subagent'
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
  return `${agent} is showing something only the terminal can.`
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
export function sendFailure(word: string, runner: string, what: 'send' | 'stop'): string {
  if (word === 'runner_too_old')
    return `${runner} needs a newer Far Cooler to ${what === 'send' ? 'take messages' : 'stop a turn'} from here. Update it, then try again.`
  if (word === 'refused')
    return what === 'send'
      ? 'The agent didn’t take that message. It may be showing a dialog only the terminal can answer.'
      : 'The agent didn’t stop. It may be showing a dialog only the terminal can answer.'
  return what === 'send'
    ? `Your message didn’t reach ${runner}. It’s still in the box. Try sending it again.`
    : `Stop didn’t reach ${runner}. Try again.`
}
