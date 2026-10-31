/// The task sheet on the page (ov-871): a native dialog, so focus is held
/// inside it, Esc closes it, and a screen reader hears it as a dialog. It
/// shows a card as the Mac does: its status, why it exists, what makes it
/// done, the notes on it, the buttons that answer it when it waits on the
/// person, and a box to add a note.
///
/// Every value is the runner's or an agent's, so each goes in through
/// `textContent` and never markup (the CSP's Trusted Types refuse an HTML
/// string sink).

import { type DetailBoard, type DetailView, isQuiet, noteKindTitle, statusTitle } from './detail'
import { age } from './needs'

/// A button that answers the card: a Needs You item's own.
export interface Answer {
  label: string
  primary: boolean
  destructive: boolean
  run(): void
}

const make = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const el = document.createElement(tag)
  if (className) el.className = className
  if (text) el.textContent = text
  return el
}

/// How often the card is read again while the sheet is open and in view.
export const DETAIL_POLL_MS = 5000

export class DetailSheet {
  private board: DetailBoard | null = null
  private answers: () => Answer[] = () => []
  private timer = 0
  private drawn = ''
  private opener: Element | null = null
  /// The task the opener's card was about, to find its button again if the list was drawn anew.
  private openerTask: string | null = null
  private last: DetailView | null = null

  constructor(
    private readonly dialog: HTMLDialogElement,
    private readonly $: (id: string) => HTMLElement,
  ) {
    $('detail-close').addEventListener('click', () => dialog.close())
    // A click on the backdrop is a click on the dialog itself.
    dialog.addEventListener('click', event => {
      if (event.target === dialog) dialog.close()
    })
    dialog.addEventListener('close', () => this.closed())
    $('detail-form').addEventListener('submit', event => {
      event.preventDefault()
      void this.add()
    })
  }

  get isOpen(): boolean {
    return this.dialog.open
  }

  /// Open the sheet on `board`. `answers` is asked every time it draws, so the
  /// buttons go when the card stops waiting on the person.
  open(board: DetailBoard, answers: () => Answer[]): void {
    this.board?.stop()
    clearInterval(this.timer)
    this.board = board
    this.answers = answers
    this.drawn = ''
    this.last = null
    const note = this.$('detail-note') as HTMLTextAreaElement
    note.value = ''
    this.$('detail-notice').hidden = true
    this.opener = document.activeElement
    this.openerTask = this.opener?.closest('[data-task]')?.getAttribute('data-task') ?? null
    if (!this.dialog.open) this.dialog.showModal()
    board.publish()
    void board.refresh()
    this.timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void board.refresh()
    }, DETAIL_POLL_MS)
  }

  close(): void {
    if (this.dialog.open) this.dialog.close()
  }

  /// The runner's list of what waits changed: the buttons may have.
  redraw(): void {
    this.drawn = ''
    if (this.last && this.dialog.open) this.draw(this.last)
  }

  private closed(): void {
    clearInterval(this.timer)
    this.board?.stop()
    this.board = null
    this.last = null
    // Back to where the person was: the button that opened it, or the same
    // card's button once the list was drawn anew, or the tab they are on.
    const again = this.openerTask ? document.querySelector<HTMLElement>(`[data-task="${CSS.escape(this.openerTask)}"] button`) : null
    const back = this.opener instanceof HTMLElement && this.opener.isConnected ? this.opener : again ?? document.querySelector<HTMLElement>('.seg [aria-pressed="true"]')
    back?.focus()
    this.opener = null
    this.openerTask = null
  }

  private async add(): Promise<void> {
    const note = this.$('detail-note') as HTMLTextAreaElement
    const board = this.board
    if (!board) return
    const body = note.value
    if (await board.note(body)) {
      if (note.value === body) note.value = ''
    }
    note.focus({ preventScroll: true })
  }

  /// Draw `view`, or leave the page be when it would look the same: a poll
  /// that found nothing new must not move what the person is reading.
  draw(view: DetailView): void {
    this.last = view
    const answers = this.answers()
    const note = this.$('detail-note') as HTMLTextAreaElement
    const add = this.$('detail-add') as HTMLButtonElement
    add.disabled = view.busy
    note.disabled = view.busy
    const notice = this.$('detail-notice')
    notice.textContent = view.notice
    notice.hidden = view.notice === ''

    const now = Date.now()
    const signature = JSON.stringify([view.state, view.banner, view.detail, view.subject, answers.map(a => a.label), view.detail?.notes.map(n => age(n.at, now))])
    if (signature === this.drawn) return
    this.drawn = signature

    const d = view.detail
    this.$('detail-key').textContent = d?.key || view.subject.key
    this.$('detail-title').textContent = d?.title || view.subject.title || 'Card'
    const status = this.$('detail-status')
    status.textContent = d ? statusTitle(d.status) : ''
    status.hidden = !status.textContent

    const body: HTMLElement[] = []
    if (view.banner) {
      const banner = make('p', 'note detail-banner', view.banner)
      banner.setAttribute('role', 'status')
      body.push(banner)
    }
    if (view.state === 'loading') body.push(make('p', 'detail-empty', 'Reading the card…'))
    if (d) {
      if (d.intent) body.push(make('p', 'detail-intent', d.intent))
      if (answers.length) {
        const row = make('div', 'detail-answers')
        row.append(make('span', 'plan-label', 'Waiting on You'))
        const buttons = make('div', 'need-actions')
        for (const answer of answers) {
          const button = make('button', answer.destructive ? 'quiet danger' : answer.primary ? '' : 'quiet', answer.label)
          button.type = 'button'
          button.onclick = () => answer.run()
          buttons.append(button)
        }
        row.append(buttons)
        body.push(row)
      }
      if (d.acceptance.length) {
        body.push(make('h3', 'detail-title', 'Done When'))
        const ul = make('ul', 'detail-criteria')
        for (const c of d.acceptance) {
          const li = make('li', c.met ? 'detail-criterion met' : 'detail-criterion')
          li.append(make('span', 'detail-box'), make('span', 'detail-text', c.text))
          li.setAttribute('aria-label', `${c.text}, ${c.met ? 'done' : 'not done yet'}`)
          ul.append(li)
        }
        body.push(ul)
      }
      if (d.constraints.length) {
        body.push(make('h3', 'detail-title', 'Constraints'))
        const ul = make('ul', 'detail-constraints')
        for (const c of d.constraints) ul.append(make('li', '', c))
        body.push(ul)
      }
      if (d.blockedBy.length) body.push(make('p', 'detail-meta', `Waiting on ${d.blockedBy.join(', ')}`))
      body.push(make('h3', 'detail-title', 'Notes'))
      if (d.notes.length === 0) body.push(make('p', 'detail-empty', 'No notes yet.'))
      const notes = make('ol', 'detail-notes')
      for (const n of d.notes) {
        const li = make('li', isQuiet(n) ? 'detail-note quiet' : 'detail-note')
        const head = make('div', 'detail-note-head')
        head.append(make('span', 'detail-note-kind', noteKindTitle(n.kind)))
        const byline = [n.actor, age(n.at, now)].filter(Boolean).join(' · ')
        if (byline) head.append(make('span', 'detail-note-by', byline))
        li.append(head, make('p', 'detail-note-body', n.body))
        notes.append(li)
      }
      body.push(notes)
    }
    const scroll = this.$('detail-body')
    const keep = scroll.scrollTop
    scroll.replaceChildren(...body)
    scroll.scrollTop = keep
  }
}
