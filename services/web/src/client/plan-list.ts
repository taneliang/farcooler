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

const TITLES: Record<CountKind, string> = { waiting: 'Waiting for a Builder', notStarted: 'Not Started', done: 'Done' }

export class PlanList {
  constructor(
    private readonly dialog: HTMLDialogElement,
    private readonly $: (id: string) => HTMLElement,
    private readonly openCard: (card: CardRef) => void,
  ) {
    $('plan-list-close').addEventListener('click', () => dialog.close())
    dialog.addEventListener('click', event => {
      if (event.target === dialog) dialog.close()
    })
  }

  /// Show `kind`'s tickets from `plan`; `count` is what the footer said.
  open(plan: Plan, kind: CountKind, count: number): void {
    const rows = listOf(plan, kind)
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
        const b = document.createElement('button')
        b.type = 'button'
        b.className = 'plan-ticket'
        const key = document.createElement('span')
        key.className = 'plan-key'
        key.textContent = row.key
        const title = document.createElement('span')
        title.className = 'plan-ticket-title'
        title.textContent = row.title
        b.append(key, title)
        if (row.initiative) {
          const initiative = document.createElement('span')
          initiative.className = 'plan-ticket-for'
          initiative.textContent = row.initiative
          b.append(initiative)
        }
        b.onclick = () => {
          this.dialog.close()
          this.openCard({ task: row.task, key: row.key })
        }
        return b
      }),
    )
    if (!this.dialog.open) this.dialog.showModal()
  }

  close(): void {
    if (this.dialog.open) this.dialog.close()
  }
}
