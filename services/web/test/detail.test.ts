// A card's detail (src/client/detail.ts, ov-871): the task read from the
// runner in `task show --json`'s shape, a note added as the person's, and the
// words when either fails. The runner is played by the test.

import { describe, expect, it } from 'vitest'

import { DetailBoard, type DetailView, detailFailure, noteKindTitle, noteRefusal, parseDetail, statusTitle } from '../src/client/detail'

const TASK = '01990000-0000-7000-8000-000000000d01'
const wire = {
  task: {
    id: TASK,
    key: 'bil-7',
    title: 'Invoice export',
    status: 'in_progress',
    intent: 'Customers download an invoice as a file.',
    acceptance: [
      { id: 'a', text: 'A PDF opens in Preview', met: true },
      { id: 'b', text: 'A CSV opens in Numbers', met: false },
    ],
    constraints: ['No new dependencies'],
    labels: ['billing'],
  },
  notes: [
    { id: 'n1', kind: 'question', actor: 'orchestrator', at: 1_800_000_000_000, body: 'PDF or CSV first?' },
    { id: 'n2', kind: 'status_change', actor: 'system', at: 1_800_000_100_000, body: 'todo to in_progress' },
  ],
  blocks: [{ short: 'ab12cd34', reason: '' }],
}

describe('reading a task', () => {
  it('reads its status, why, what makes it done, and its notes in order', () => {
    const d = parseDetail(JSON.stringify(wire))!
    expect([d.id, d.key, d.title, d.status]).toEqual([TASK, 'bil-7', 'Invoice export', 'in_progress'])
    expect(d.intent).toBe('Customers download an invoice as a file.')
    expect(d.acceptance.map(a => [a.text, a.met])).toEqual([
      ['A PDF opens in Preview', true],
      ['A CSV opens in Numbers', false],
    ])
    expect(d.constraints).toEqual(['No new dependencies'])
    expect(d.notes.map(n => [n.kind, n.actor, n.body])).toEqual([
      ['question', 'orchestrator', 'PDF or CSV first?'],
      ['status_change', 'system', 'todo to in_progress'],
    ])
    expect(d.blockedBy).toEqual(['ab12cd34'])
  })

  it('reads nothing from what is not a task, and leaves out a note with no body', () => {
    expect(parseDetail('nonsense')).toBeNull()
    expect(parseDetail('{"task":{}}')).toBeNull()
    const d = parseDetail(JSON.stringify({ ...wire, notes: [{ id: 'x' }, wire.notes[0]] }))!
    expect(d.notes).toHaveLength(1)
  })

  it('says a status and a kind of note in the Mac’s words', () => {
    expect(statusTitle('needs_decision')).toBe('Needs Decision')
    expect(statusTitle('in_review')).toBe('In Review')
    expect(statusTitle('mystery')).toBe('')
    expect(noteKindTitle('comment')).toBe('Note')
    expect(noteKindTitle('decision')).toBe('Decision')
  })
})

function board(overrides: { task?: (id: string) => Promise<string>; addNote?: (id: string, body: string) => Promise<void> } = {}) {
  const views: DetailView[] = []
  const asked: string[] = []
  const notes: [string, string][] = []
  let json = JSON.stringify(wire)
  const b = new DetailBoard(
    {
      runner: 'studio',
      task: overrides.task ?? (async id => (asked.push(id), json)),
      addNote:
        overrides.addNote ??
        (async (id, body) => {
          notes.push([id, body])
          json = JSON.stringify({ ...wire, notes: [...wire.notes, { id: 'n3', kind: 'comment', actor: 'user', at: 1, body }] })
        }),
      show: v => void views.push(v),
    },
    { runner: 'r1', id: TASK, key: 'bil-7', title: 'Invoice export' },
  )
  return { b, views, asked, notes, last: () => views[views.length - 1] }
}

describe('the card on the page', () => {
  it('reads the task it was opened on', async () => {
    const t = board()
    await t.b.refresh()
    expect(t.asked).toEqual([TASK])
    expect(t.last().state).toBe('ready')
    expect(t.last().detail?.key).toBe('bil-7')
  })

  it('adds a note, trimmed, and reads the card again to show it', async () => {
    const t = board()
    await t.b.refresh()
    expect(await t.b.note('  Looks right  ')).toBe(true)
    expect(t.notes).toEqual([[TASK, 'Looks right']])
    expect(t.last().detail?.notes.at(-1)?.body).toBe('Looks right')
    expect(t.last().busy).toBe(false)
  })

  it('sends nothing for an empty note, or a second while one is out', async () => {
    let release = () => {}
    const sent: string[] = []
    const t = board({ addNote: async (_id, body) => void (sent.push(body), await new Promise<void>(done => (release = done))) })
    await t.b.refresh()
    expect(await t.b.note('   ')).toBe(false)
    const first = t.b.note('one')
    expect(await t.b.note('two')).toBe(false)
    expect(t.last().busy).toBe(true)
    release()
    await first
    expect(sent).toEqual(['one'])
  })

  it('keeps a note’s words and says why when the runner refuses it', async () => {
    const t = board({ addNote: async () => Promise.reject(Object.assign(new Error('refused'), { code: 'scope-denied' })) })
    await t.b.refresh()
    expect(await t.b.note('hello')).toBe(false)
    expect(t.last().notice).toContain('can only look at studio')
    expect(t.last().notice).not.toContain('scope-denied')
  })

  it('shows a card already read under the line that says the runner went quiet', async () => {
    let fail = false
    const t = board({ task: async () => (fail ? Promise.reject(new Error('closed')) : JSON.stringify(wire)) })
    await t.b.refresh()
    fail = true
    await t.b.refresh()
    expect(t.last().state).toBe('ready')
    expect(t.last().detail?.key).toBe('bil-7')
    expect(t.last().banner).toBe('studio didn’t answer. Far Cooler is reconnecting.')
  })

  it('says a card could not be read when it never was', async () => {
    const t = board({ task: async () => Promise.reject(new Error('runner_too_old')) })
    await t.b.refresh()
    expect(t.last().state).toBe('failed')
    expect(t.last().banner).toBe('Update Far Cooler on studio to see this card here.')
  })

  it('stops drawing once it is stopped', async () => {
    const t = board()
    t.b.stop()
    await t.b.refresh()
    expect(t.views).toEqual([])
  })
})

describe('words', () => {
  it('words a refusal without the runner’s code', () => {
    expect(noteRefusal({ word: 'refused', code: 'not-found' }, 'studio')).toBe('This card isn’t on the runner anymore.')
    expect(noteRefusal({ word: 'closed' }, 'studio')).toContain('studio didn’t answer')
    expect(detailFailure('refused', 'studio')).toBe('This card isn’t on the runner anymore.')
  })
})
