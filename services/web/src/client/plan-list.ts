/// A footer count's list (ov-875): the tickets behind "10 tickets waiting for
/// a builder", "22 not started" or "796 done", in a native dialog beside the
/// plan. A ticket in it opens its card as one on the plan does.
///
/// The plan names the cards it knows. The waiting ones are all of them; the
/// board holds more cards than a plan carries, so the other two say how many
/// of the count they list. Every value is the runner's, set through
/// `textContent`.

import { type CardRef, type Plan } from './plan'
import { type CountKind, listOf } from './plan-column'
import type { PlanViewModel } from './plan-present'
import { planFailure } from './copy'

const TITLES: Record<CountKind, string> = { waiting: 'Waiting for a Builder', not_started: 'Not Started', done: 'Done' }

type OrderRow = CardRef & { title: string }
interface BoardOrder {
  sections: { title: string; rows: OrderRow[]; draggable: boolean }[]
  dragTitle: string
  failure: string
}

export class PlanList {
  private stopOrder?: () => void

  constructor(
    private readonly dialog: HTMLDialogElement,
    private readonly $: (id: string) => HTMLElement,
    private readonly openCard: (card: CardRef) => void,
  ) {
    $('plan-list-close').addEventListener('click', () => dialog.close())
    dialog.addEventListener('close', () => {
      if (!dialog.open) this.stopOrder?.()
    })
    dialog.addEventListener('click', event => {
      if (event.target === dialog) dialog.close()
    })
  }

  /// Show `kind`'s tickets from `plan`; `count` is what the footer said.
  open(plan: Plan, view: PlanViewModel, kind: CountKind, count: number): void {
    this.stopOrder?.()
    const rows = listOf(plan, view, kind)
    this.$('plan-list-title').textContent = TITLES[kind]
    const note = this.$('plan-list-note')
    note.textContent =
      rows.length === 0
        ? 'The plan doesn’t name any of these tickets.'
        : rows.length < count
          ? `The plan names ${rows.length} of the ${count}.`
          : ''
    note.hidden = note.textContent === ''
    const list = this.$('plan-list-body')
    list.replaceChildren(
      ...rows.map(row => {
        const b = this.ticket(row)
        if (row.theme) {
          const theme = document.createElement('span')
          theme.className = 'plan-ticket-for'
          theme.textContent = row.theme
          b.append(theme)
        }
        return b
      }),
    )
    if (!this.dialog.open) this.dialog.showModal()
  }

  openOrder(label: string, runner: string, read: () => Promise<string | undefined>, move: (task: string, anchor: string, after: boolean) => Promise<void>): void {
    this.stopOrder?.()
    this.$('plan-list-title').textContent = label
    this.$('plan-list-body').replaceChildren()
    if (!this.dialog.open) this.dialog.showModal()
    let active = true
    let inFlight = false
    let dragging: OrderRow | undefined
    let drawn: string | undefined
    let timer = 0
    this.stopOrder = () => {
      active = false
      clearInterval(timer)
    }
    const note = this.$('plan-list-note')
    note.hidden = true
    const refresh = async () => {
      if (!active || !this.dialog.open || dragging || inFlight) return
      inFlight = true
      try {
        const raw = await read()
        if (!active || !this.dialog.open || dragging) return
        if (raw === undefined) throw new Error('unsupported')
        if (raw === drawn) return
        const view: BoardOrder = JSON.parse(raw)
        drawn = raw
        note.hidden = true
        this.$('plan-list-body').replaceChildren(...view.sections.map(section => {
          const group = document.createElement('section')
          const title = document.createElement('h3')
          title.textContent = section.title
          group.append(title)
          for (const row of section.rows) {
            const button = this.ticket(row)
            button.draggable = section.draggable
            if (section.draggable) {
              button.title = view.dragTitle
              button.ondragstart = event => {
                if (!event.dataTransfer || inFlight) { event.preventDefault(); return }
                dragging = row
                event.dataTransfer.effectAllowed = 'move'
                event.dataTransfer.setData('text/plain', row.key)
              }
              button.ondragend = () => { dragging = undefined }
              const acceptsDrop = () => dragging !== undefined && dragging !== row && section.rows.includes(dragging)
              button.ondragover = event => {
                if (!acceptsDrop()) return
                event.preventDefault()
                if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
              }
              button.ondrop = async event => {
                if (!dragging || !acceptsDrop() || inFlight) return
                event.preventDefault()
                const task = dragging.task
                dragging = undefined
                const bounds = button.getBoundingClientRect()
                inFlight = true
                try {
                  await move(task, row.task, event.clientY > bounds.top + bounds.height / 2)
                  if (active) note.hidden = true
                } catch {
                  if (active) {
                    note.textContent = view.failure
                    note.hidden = false
                  }
                } finally {
                  inFlight = false
                }
                await refresh()
              }
            }
            group.append(button)
          }
          return group
        }))
      } catch {
        if (active && drawn === undefined) {
          note.textContent = planFailure('closed', runner)
          note.hidden = false
        }
      } finally {
        inFlight = false
      }
    }
    timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') void refresh()
    }, 15000)
    void refresh()
  }

  private ticket(row: OrderRow): HTMLButtonElement {
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'plan-ticket'
    const key = document.createElement('span')
    key.className = 'plan-key'
    key.textContent = row.key
    const title = document.createElement('span')
    title.className = 'plan-ticket-title'
    title.textContent = row.title
    button.append(key, title)
    button.onclick = () => {
      this.close()
      this.openCard({ task: row.task, key: row.key })
    }
    return button
  }

  close(): void {
    this.stopOrder?.()
    if (this.dialog.open) this.dialog.close()
  }
}
