// What the page adds to the plan column (src/client/plan-column.ts, ov-1016):
// Needs You's rows and the ticket lists a footer count opens. The column
// itself is Rust's (`view.column`), pinned by the golden every platform
// decodes (test/fixtures/plan-view-apple.json, rewritten by
// `UPDATE_PLAN_VIEW_GOLDEN=1 cargo test -p farcooler-client --lib plan_view`);
// its facts, counts and words are tested there.

import { describe, expect, it } from 'vitest'

import golden from '../../../test/fixtures/plan-view-apple.json'
import { parsePlan } from '../src/client/plan'
import { listOf, needsRows, oneLine, quote, statusesOf } from '../src/client/plan-column'
import type { PlanViewModel } from '../src/client/plan-present'

const view = golden.answer.view as unknown as PlanViewModel
const plan = () => parsePlan(JSON.stringify(golden.request.plan))!

describe('the footer’s lists', () => {
  it('opens what waits for a builder as the view lists it, with each ticket’s theme', () => {
    const rows = listOf(plan(), view, 'waiting')
    expect(rows.map(r => r.key)).toEqual(view.column.waiting.map(t => t.key))
    expect(rows.find(r => r.key === 'ov-302')?.theme).toBe('Marketing')
  })

  it('opens the cards the plan names as done, and as not started less the ones already waiting', () => {
    expect(listOf(plan(), view, 'done').map(r => r.key)).toEqual(['ov-486', 'ov-480', 'ov-460', 'ov-461'])
    expect(listOf(plan(), view, 'not_started').map(r => r.key)).toEqual(['ov-485', 'ov-484', 'ov-301'].filter(k => !view.column.waiting.some(w => w.key === k)))
  })

  it('reads a status in the board’s word or the store’s, and Canceled as Cancelled', () => {
    const p = parsePlan(JSON.stringify({ lines: [], themes: [], cards: [{ task: 'a', key: 'ov-1', title: 'A', status: 'Canceled' }, { task: 'b', key: 'ov-2', title: 'B', status: 'To Do' }] }))!
    expect([...statusesOf(p)]).toEqual([['a', 'cancelled'], ['b', 'todo']])
  })
})

describe('Needs You', () => {
  it('quotes a task by key and title, else the words, on one line and with nothing that acts in a terminal', () => {
    expect(quote('ov-1', 'Site copy')).toBe('About ov-1 (“Site copy”): ')
    expect(quote(null, 'Allow\nBash?')).toBe('About “Allow Bash?”: ')
    expect(oneLine('a\u001b[201~b\u0003 c​d\u001b]0;x\u0007e')).toBe('ab cde')
  })

  it('lists each initiative the view says asks the owner first, then the runner’s items, each with what a click quotes', () => {
    const rows = needsRows(view, plan(), [
      { id: 'a', since: 5, question: 'Allow Bash?', task: null, terminal: { label: 'Builder 1' } },
      { id: 'b', since: null, question: 'Pick one', task: { key: 'ov-9', title: 'Site copy' }, terminal: null },
    ])
    expect(rows.map(r => [r.title, r.detail])).toEqual([['Marketing', 'How do we ship the site?'], ['Allow Bash?', 'Builder 1'], ['Site copy', 'Pick one']])
    expect(rows[0].quote).toBe('About “Marketing: How do we ship the site?”: ')
    expect(rows[2].quote).toBe('About ov-9 (“Site copy”): ')
    expect(needsRows(null, null, [])).toEqual([])
  })
})
