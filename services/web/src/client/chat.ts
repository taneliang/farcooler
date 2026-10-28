/// One agent's conversation: its rows streaming in, a box to send a message,
/// and Stop (ov-776). The same stream the Mac and the phones follow: the
/// runner's projector rows (`agent.rows`, `agent.rows_follow`), a message sent
/// through `terminal.compose` so every device shows it as the person's, and
/// Stop through `terminal.interrupt`.
///
/// A `ChatView` is a `Pane` so the page's set keeps it with the terminals, but
/// it holds no SSH channel of its own (everything rides the control session),
/// so it reports itself inactive and never takes one of the connection's few
/// channels from a terminal.

import type { WebRunner } from 'fc-web'

import { chatTrouble, isWorking } from './chat-copy'
import { Outbox } from './chat-send'
import { closesWithLine, drawEnd, drawRow, type Memory, toolsGroup } from './chat-draw'
import { bodyOf, type Delta, itemsOf, nodesOf, type Phase, RowFollower, RowLedger, type RowSource, type ViewNode } from './chat-rows'
import type { Pane } from './panes'

/// A runner's rows, through the wasm module's JSON.
export function sourceOf(runner: WebRunner, terminal: string): RowSource {
  return {
    page: async (before, limit) => JSON.parse(await runner.agentRows(terminal, before ?? undefined, limit)),
    follow: async (epoch, rev, waitMs) => JSON.parse(await runner.agentRowsFollow(terminal, epoch, rev, waitMs)),
  }
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text) node.textContent = text
  return node
}

/// How near the bottom still counts as "at the bottom", in pixels.
const STICK = 64

export class ChatView implements Pane<WebRunner> {
  private readonly ledger = new RowLedger()
  private follower: RowFollower | null = null
  private runner: WebRunner | null = null
  private terminal = ''
  private shownBefore = false
  private outbox: Outbox | null = null
  private stick = true
  private phase: Phase = { kind: 'loading' }
  private readonly memory: Memory = { open: new Set() }
  /// What is drawn, by node id: the element and what it was drawn from.
  private readonly drawn = new Map<string, { el: HTMLElement | null; sig: string }>()

  private readonly scroller = el('div', 'chat-scroll')
  private readonly older = el('button', 'quiet chat-older', 'Load Earlier Messages')
  private readonly list = el('div', 'chat-list')
  private readonly empty = el('p', 'chat-empty')
  private readonly latest = el('button', 'chat-latest', 'Latest')
  private readonly note = el('p', 'chat-note')
  private readonly working = el('div', 'chat-working')
  private readonly stop = el('button', 'quiet chat-stop', 'Stop')
  private readonly box = el('textarea', 'chat-input')
  private readonly send = el('button', 'chat-send')
  private readonly composer = el('form', 'chat-composer')
  private readonly problem = el('p', 'chat-problem')

  constructor(
    private readonly host: HTMLElement,
    private readonly runnerLabel: string,
  ) {
    host.classList.add('chat-host')
    this.scroller.setAttribute('role', 'log')
    this.scroller.setAttribute('aria-label', 'Conversation')
    this.scroller.tabIndex = 0
    this.older.type = 'button'
    this.older.hidden = true
    this.latest.type = 'button'
    this.latest.hidden = true
    this.stop.type = 'button'
    this.working.setAttribute('role', 'status')
    this.working.hidden = true
    this.working.append(el('span', 'working-dots'), el('span', 'working-text', 'Working'), this.stop)
    this.note.setAttribute('role', 'status')
    this.note.hidden = true
    this.problem.setAttribute('role', 'alert')
    this.problem.hidden = true
    this.box.rows = 1
    this.box.placeholder = 'Message'
    this.box.setAttribute('aria-label', 'Message')
    this.box.enterKeyHint = 'send'
    this.send.type = 'submit'
    this.send.setAttribute('aria-label', 'Send')
    this.send.disabled = true
    const arrow = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
    arrow.setAttribute('viewBox', '0 0 16 16')
    arrow.setAttribute('aria-hidden', 'true')
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
    path.setAttribute('d', 'M8 13V3M3.5 7.5 8 3l4.5 4.5')
    arrow.append(path)
    this.send.append(arrow)
    const field = el('div', 'chat-field')
    field.append(this.box, this.send)
    this.composer.append(field)
    this.scroller.append(this.older, this.list, this.empty)
    const foot = el('div', 'chat-foot')
    foot.append(this.working, this.problem, this.composer)
    host.append(this.note, this.scroller, this.latest, foot)

    this.scroller.addEventListener('scroll', () => {
      this.stick = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight <= STICK
      this.latest.hidden = this.stick
    })
    this.latest.onclick = () => {
      this.stick = true
      this.toBottom()
    }
    this.older.onclick = () => void this.loadOlder()
    this.stop.onclick = () => void this.interrupt()
    this.composer.addEventListener('submit', event => {
      event.preventDefault()
      void this.submit()
    })
    this.box.addEventListener('input', () => this.sizeBox())
    this.box.addEventListener('keydown', event => {
      // Return sends where there is a keyboard; on a touch screen it is a new
      // line and the arrow sends. Never while an input method is composing.
      if (event.key !== 'Enter' || event.isComposing || event.shiftKey) return
      if (matchMedia('(pointer: coarse)').matches && !(event.metaKey || event.ctrlKey)) return
      event.preventDefault()
      void this.submit()
    })
    this.paint()
  }

  // MARK: Pane

  /// A conversation holds no channel of its own.
  get active(): boolean {
    return false
  }

  get used(): boolean {
    return this.shownBefore
  }

  show(on: boolean): void {
    this.host.hidden = !on
    if (!on) return
    if (this.stick) this.toBottom()
    // Not on a touch screen: focusing the box would raise the keyboard over
    // the conversation before the person asked to write.
    if (!matchMedia('(pointer: coarse)').matches) this.box.focus()
  }

  async attach(runner: WebRunner, terminal: string, _ended: () => void, _resuming = false): Promise<void> {
    // Opened again on the same connection: already following.
    if (this.runner === runner && this.follower?.following) return
    this.follower?.stop()
    this.runner = runner
    this.terminal = terminal
    this.outbox ??= new Outbox(() => this.runner, terminal, this.runnerLabel)
    this.shownBefore = true
    this.host.dataset.attached = 'true'
    this.follower = new RowFollower(this.ledger, sourceOf(runner, terminal), {
      changed: delta => this.draw(delta),
      phase: phase => {
        this.phase = phase
        this.paint()
      },
    })
    this.follower.start()
  }

  detach(): void {
    this.follower?.stop()
    this.follower = null
    delete this.host.dataset.attached
  }

  dispose(): void {
    this.detach()
    this.host.remove()
  }

  // MARK: Drawing

  /// Bring the page up to the ledger: only what changed is built again, and
  /// the list is walked once to put every node in its place.
  private draw(_delta: Delta): void {
    const rows = this.ledger.rows
    const items = itemsOf(this.ledger.order, rows)
    const nodes = nodesOf(items, rows, turn => closesWithLine(turn))
    const wanted = new Set(nodes.map(n => n.id))
    for (const [id, was] of this.drawn) {
      if (wanted.has(id)) continue
      was.el?.remove()
      this.drawn.delete(id)
    }
    const height = this.scroller.scrollHeight
    let after: Element | null = null
    for (const node of nodes) {
      const sig = this.signature(node)
      let held = this.drawn.get(node.id)
      if (!held || held.sig !== sig) {
        const made = this.build(node)
        if (held?.el && made) made.classList.toggle('settled', true)
        if (held?.el) held.el.replaceWith(...(made ? [made] : []))
        held = { el: made, sig }
        this.drawn.set(node.id, held)
      }
      if (!held.el) continue
      const expected: Element | null = after ? after.nextElementSibling : this.list.firstElementChild
      if (held.el !== expected) this.list.insertBefore(held.el, expected)
      after = held.el
    }
    // Rows put above what the person is reading must not move it.
    if (!this.stick && this.scroller.scrollHeight !== height && this.prepended) this.scroller.scrollTop += this.scroller.scrollHeight - height
    this.prepended = false
    this.paint()
    if (this.stick) this.toBottom()
  }
  private prepended = false

  private signature(node: ViewNode): string {
    const rows = this.ledger.rows
    if (node.kind === 'end') return `${rows.get(node.turn)?.rev}`
    const ids = node.item.kind === 'row' ? [node.item.id] : node.item.rows
    return ids.map(id => `${id}@${rows.get(id)?.rev}`).join(',')
  }

  private build(node: ViewNode): HTMLElement | null {
    const rows = this.ledger.rows
    if (node.kind === 'end') {
      const turn = rows.get(node.turn)
      return turn ? drawEnd(turn) : null
    }
    if (node.item.kind === 'tools') {
      return toolsGroup(
        node.item.id,
        node.item.rows.flatMap(id => rows.get(id) ?? []),
        this.memory,
      )
    }
    const row = rows.get(node.item.id)
    return row ? drawRow(row, this.memory) : null
  }

  /// Everything but the list: what the foot says, whether the box can send,
  /// the placeholder, and the empty state.
  private paint(): void {
    const newest = this.ledger.newestTurn
    const turn = newest ? bodyOf(newest) : null
    // Stop only where one Esc stops the turn: while a dialog is up it would
    // answer the dialog No (`isWorking`).
    const working = !!turn && isWorking(turn)
    this.working.hidden = !working
    this.host.dataset.working = working ? 'true' : 'false'
    this.older.hidden = !this.ledger.moreBefore
    this.empty.hidden = this.ledger.order.some(id => id !== 'hint:composer')
    this.empty.textContent = this.phase.kind === 'loading' ? 'Loading the conversation…' : 'No messages yet. Send one below.'
    this.box.placeholder = this.ledger.hint || 'Message'
    const trouble = this.phase.kind === 'trouble' ? chatTrouble(this.phase.word, this.runnerLabel) : this.phase.kind === 'unavailable' ? chatTrouble('runner_too_old', this.runnerLabel) : ''
    this.note.textContent = trouble
    this.note.hidden = trouble === ''
    this.host.dataset.stale = trouble ? 'true' : 'false'
    this.send.disabled = !!this.outbox?.busy || this.box.value.trim() === ''
  }

  private toBottom(): void {
    this.scroller.scrollTop = this.scroller.scrollHeight
    this.latest.hidden = true
  }

  private sizeBox(): void {
    this.box.style.height = 'auto'
    this.box.style.height = `${Math.min(this.box.scrollHeight, 200)}px`
    this.send.disabled = !!this.outbox?.busy || this.box.value.trim() === ''
  }

  private say(text: string): void {
    this.problem.textContent = text
    this.problem.hidden = text === ''
  }

  // MARK: Doing

  private async loadOlder(): Promise<void> {
    this.older.disabled = true
    this.prepended = true
    try {
      await this.follower?.older()
    } catch {
      this.prepended = false
    } finally {
      this.older.disabled = false
    }
  }

  /// Send the box's words. They leave the box only once the runner has taken
  /// them, so a message that didn't go is never lost.
  private async submit(): Promise<void> {
    const text = this.box.value
    if (!this.outbox) return
    this.say('')
    const sending = this.outbox.send(text)
    this.paint()
    const done = await sending
    if (done.ok) {
      // The person's own words stay if they went on typing meanwhile.
      if (this.box.value === text) {
        this.box.value = ''
        this.sizeBox()
      }
      this.stick = true
    } else if (!done.ignored) this.say(done.message)
    this.paint()
    this.box.focus({ preventScroll: true })
  }

  private async interrupt(): Promise<void> {
    if (!this.outbox) return
    this.stop.disabled = true
    this.say('')
    const done = await this.outbox.stop()
    if (!done.ok && !done.ignored) this.say(done.message)
    this.stop.disabled = false
  }
}
