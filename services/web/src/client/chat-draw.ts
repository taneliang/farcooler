/// A conversation's rows as elements (ov-776). Every string from the runner
/// goes in through `textContent`, never markup (the page's Trusted Types).
/// Which row looks like what follows the Mac: the person's message is a
/// bubble on the right; the agent's words run plain on the left; a tool run
/// is one line that opens; asks, subagents and task lists are cards.

import {
  agentType,
  askTitle,
  gapLine,
  groupPurpose,
  groupStatus,
  groupTitle,
  handoffLine,
  harnessTitle,
  isNotice,
  isWorking,
  noticeLine,
  noticeText,
  originNote,
  outcomeLine,
  queuedLabel,
  SCHEDULED_TASK,
  subagentState,
  taskProgress,
  toolsLabel,
} from './chat-copy'
import { bodyOf, kindOf, type Row } from './chat-rows'
import { blocksOf, type Span } from './chat-text'

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

function spans(into: HTMLElement, list: Span[]): void {
  for (const span of list) {
    if (span.code || span.bold) {
      const node = el(span.code ? 'code' : 'strong', '', span.text)
      into.append(node)
    } else into.append(document.createTextNode(span.text))
  }
}

/// Prose as paragraphs, lists and code.
export function prose(text: string): HTMLElement {
  const box = el('div', 'prose')
  for (const block of blocksOf(text)) {
    switch (block.kind) {
      case 'paragraph': {
        const p = el('p')
        spans(p, block.spans)
        box.append(p)
        break
      }
      case 'heading': {
        const h = el('p', 'prose-heading')
        spans(h, block.spans)
        box.append(h)
        break
      }
      case 'list': {
        const list = el(block.ordered ? 'ol' : 'ul')
        for (const item of block.items) {
          const li = el('li')
          spans(li, item)
          list.append(li)
        }
        box.append(list)
        break
      }
      case 'code':
        box.append(el('pre', 'code-block', block.text))
        break
    }
  }
  return box
}

/// A long message folded to a few lines with Show More, measured at the width
/// it is drawn at (a guess from its length would be wrong at some width). The
/// button appears once the element is on the page and is seen to overflow.
function folded(content: HTMLElement, expanded: boolean, remember: (open: boolean) => void): HTMLElement {
  const box = el('div', 'fold')
  content.classList.add('fold-body')
  box.classList.toggle('open', expanded)
  const button = el('button', 'quiet fold-toggle', expanded ? 'Show Less' : 'Show More')
  button.type = 'button'
  button.hidden = !expanded
  button.onclick = () => {
    const open = !box.classList.contains('open')
    box.classList.toggle('open', open)
    button.textContent = open ? 'Show Less' : 'Show More'
    remember(open)
  }
  box.append(content, button)
  // Measured on the next frame, when the element has a width.
  requestAnimationFrame(() => {
    if (!box.isConnected) return
    if (!box.classList.contains('open')) button.hidden = content.scrollHeight <= content.clientHeight + 1
  })
  return box
}

/// What a row remembers of how the person left it, across redraws.
export interface Memory {
  open: Set<string>
}

const dot = (status: string) => {
  const node = el('span', `status status-${status.toLowerCase()}`)
  node.setAttribute('role', 'img')
  node.setAttribute('aria-label', status === 'Running' ? 'Running' : status === 'Failed' ? 'Failed' : 'Done')
  return node
}

function details(id: string, memory: Memory, head: HTMLElement, body: () => HTMLElement | null): HTMLElement {
  const box = el('details', 'disclosure')
  const summary = el('summary')
  summary.append(head)
  box.append(summary)
  const fill = () => {
    if (box.querySelector('.disclosure-body')) return
    const inner = body()
    if (!inner) return
    inner.classList.add('disclosure-body')
    box.append(inner)
  }
  if (memory.open.has(id)) {
    box.open = true
    fill()
  }
  box.addEventListener('toggle', () => {
    if (box.open) {
      memory.open.add(id)
      fill()
    } else memory.open.delete(id)
  })
  return box
}

function toolHead(tool: any): HTMLElement {
  const head = el('span', 'tool-head')
  head.append(dot(tool.status), el('span', 'tool-name', tool.name))
  const summary = String(tool.summary ?? '')
  if (summary) head.append(el('span', 'tool-summary', summary))
  return head
}

function toolBody(tool: any): HTMLElement | null {
  const diff: any[] = tool.diff ?? []
  if (!tool.input && !tool.result && diff.length === 0) return null
  const body = el('div')
  if (tool.input) body.append(el('pre', 'code-block', tool.input))
  for (const hunk of diff) {
    const pre = el('pre', 'code-block diff')
    for (const line of hunk.lines as string[]) {
      const kind = line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : 'ctx'
      pre.append(el('span', `diff-${kind}`, `${line}\n`))
    }
    body.append(pre)
  }
  if (tool.result) body.append(el('pre', 'code-block', tool.result))
  return body
}

/// A lone tool call: one line, opening to what it was called with, and what
/// it answered, where there is something to show.
function toolRow(row: Row, memory: Memory): HTMLElement {
  const tool = bodyOf(row)
  if (!tool.input && !tool.result && !(tool.diff ?? []).length) {
    const line = el('div', 'tool-line')
    line.append(toolHead(tool))
    return line
  }
  return details(row.id, memory, toolHead(tool), () => toolBody(tool))
}

/// Two or more calls in a row, as one line: "Ran 3 commands", and what for.
export function toolsGroup(id: string, rows: Row[], memory: Memory): HTMLElement {
  const tools = rows.filter(r => kindOf(r) === 'Tool').map(bodyOf)
  const head = el('span', 'tool-head')
  head.append(dot(groupStatus(tools)), el('span', 'tool-name', groupTitle(tools)))
  const purpose = groupPurpose(tools)
  if (purpose) head.append(el('span', 'tool-summary', purpose))
  return details(id, memory, head, () => {
    const list = el('div', 'tool-list')
    for (const row of rows) {
      if (kindOf(row) !== 'Tool') continue
      const tool = bodyOf(row)
      const inner = toolBody(tool)
      if (inner) list.append(details(row.id, memory, toolHead(tool), () => toolBody(tool)))
      else {
        const line = el('div', 'tool-line')
        line.append(toolHead(tool))
        list.append(line)
      }
    }
    return list
  })
}

function turnRow(row: Row, memory: Memory): HTMLElement | null {
  const turn = bodyOf(row)
  const harness = turn.harness
  if (harness) {
    const card = el('div', 'card harness')
    card.append(el('p', 'card-title', harnessTitle(harness)))
    if (harness.body) card.append(folded(prose(String(harness.body)), memory.open.has(`fold:${row.id}`), o => toggle(memory, `fold:${row.id}`, o)))
    return card
  }
  if (turn.origin === 'Scheduled') {
    const card = el('div', 'card scheduled')
    card.append(el('p', 'card-title', SCHEDULED_TASK))
    card.append(folded(prose(String(turn.prompt ?? '')), memory.open.has(`fold:${row.id}`), o => toggle(memory, `fold:${row.id}`, o)))
    return card
  }
  if (isNotice(turn)) return el('p', 'notice', noticeText(turn))
  const wrap = el('div', 'bubble-row')
  const bubble = el('div', `bubble${row.provisional ? ' provisional' : ''}`)
  bubble.append(folded(el('p', 'bubble-text', String(turn.prompt ?? '')), memory.open.has(`fold:${row.id}`), o => toggle(memory, `fold:${row.id}`, o)))
  wrap.append(bubble)
  const note = originNote(turn)
  if (note) wrap.append(el('p', 'bubble-note', note))
  if ((turn.images ?? []).length) wrap.append(el('p', 'bubble-note', turn.images.length === 1 ? '1 image attached' : `${turn.images.length} images attached`))
  return wrap
}

function toggle(memory: Memory, key: string, on: boolean): void {
  if (on) memory.open.add(key)
  else memory.open.delete(key)
}

function subagentRow(row: Row): HTMLElement {
  const agent = bodyOf(row)
  const card = el('div', 'card subagent')
  const head = el('p', 'card-title')
  head.append(dot(agent.status === 'Running' ? 'Running' : agent.status === 'Completed' ? 'Done' : 'Failed'), document.createTextNode(` ${agentType(agent.agent_type ?? '')}`))
  card.append(head)
  if (agent.description) card.append(el('p', 'card-body', agent.description))
  const meta = [subagentState(agent.status), toolsLabel(agent.tool_count ?? 0), agent.status === 'Running' ? agent.current_action : ''].filter(Boolean)
  card.append(el('p', 'card-meta', meta.join(' · ')))
  return card
}

function askRow(row: Row): HTMLElement {
  const ask = bodyOf(row)
  const card = el('div', `card ask${ask.answered ? ' answered' : ''}`)
  card.append(el('p', 'card-title', askTitle(ask)))
  if (ask.text) card.append(el('p', 'card-body', ask.text))
  for (const q of ask.questions ?? []) {
    const block = el('div', 'ask-question')
    block.append(el('p', 'card-body', q.question))
    const options = el('ul', 'ask-options')
    for (const o of q.options ?? []) options.append(el('li', '', o.description ? `${o.label} — ${o.description}` : o.label))
    block.append(options)
    card.append(block)
  }
  if (ask.plan) card.append(folded(prose(ask.plan), false, () => {}))
  if (!ask.answered) card.append(el('p', 'card-meta', 'Waiting for an answer. Answer it from Needs You, or in the terminal.'))
  return card
}

function queuedRow(row: Row): HTMLElement {
  const queued = bodyOf(row)
  const wrap = el('div', 'bubble-row')
  wrap.append(el('div', `bubble queued state-${String(queued.state).toLowerCase()}`, String(queued.text ?? '')))
  wrap.append(el('p', 'bubble-note', queuedLabel(String(queued.state))))
  return wrap
}

function tasksRow(row: Row): HTMLElement {
  const tasks = bodyOf(row)
  const items: { subject: string; status: string }[] = tasks.items ?? []
  const card = el('div', 'card tasks')
  card.append(el('p', 'card-title', taskProgress(items)))
  const list = el('ul', 'task-list')
  for (const item of items) {
    const li = el('li', `task task-${item.status.toLowerCase()}`)
    li.append(el('span', 'task-box'), document.createTextNode(item.subject))
    list.append(li)
  }
  card.append(list)
  return card
}

/// One row as an element, or null for a kind that draws nothing here (the
/// hint is the composer's; a kind this build doesn't know is skipped).
export function drawRow(row: Row, memory: Memory): HTMLElement | null {
  switch (kindOf(row)) {
    case 'Turn':
      return turnRow(row, memory)
    case 'Prose':
      return prose(String(bodyOf(row).text ?? ''))
    case 'Thinking': {
      const t = bodyOf(row)
      const done = t.ended_ms !== null && t.ended_ms !== undefined
      const seconds = done && typeof t.started_ms === 'number' ? Math.max(1, Math.round((t.ended_ms - t.started_ms) / 1000)) : 0
      return el('p', `thinking${done ? '' : ' live'}`, done ? (seconds ? `Thought for ${seconds}s` : 'Thought') : 'Thinking…')
    }
    case 'Tool':
      return toolRow(row, memory)
    case 'Subagent':
      return subagentRow(row)
    case 'Ask':
      return askRow(row)
    case 'Queued':
      return queuedRow(row)
    case 'Notice':
      return el('p', 'notice', noticeLine(row))
    case 'Handoff':
      return el('p', 'notice', handoffLine())
    case 'Gap':
      return el('p', 'notice', gapLine(bodyOf(row)))
    case 'Tasks':
      return tasksRow(row)
    default:
      return null
  }
}

/// The line under a finished turn's rows.
export function drawEnd(turn: Row): HTMLElement | null {
  const line = outcomeLine(bodyOf(turn))
  return line ? el('p', 'turn-end', line) : null
}

/// Whether a turn closes with a line of its own: not while it is working, not
/// a turn claude wrote for itself, not a notice.
export function closesWithLine(turn: Row): boolean {
  const body = bodyOf(turn)
  return !body.harness && !isNotice(body) && !isWorking(body) && outcomeLine(body) !== null
}
