// An agent's conversation in the browser (ov-776): the rows the runner's
// projector makes, held and followed the way the Mac holds them, the words
// they are drawn with, and the two things the page sends. Every runner call is
// played by the test, and time is a promise the test settles.

import { describe, expect, it } from 'vitest'

import { agentType, askTitle, groupPurpose, groupTitle, harnessTitle, outcomeLine, queuedLabel, sendFailure, shortDuration, taskProgress } from '../src/client/chat-copy'
import {
  type Delta,
  FOLLOW_WAIT_MS,
  type Phase,
  type Row,
  type RowChanges,
  RowFollower,
  RowLedger,
  type RowPage,
  type RowSource,
  itemsOf,
  nodesOf,
} from '../src/client/chat-rows'
import { Outbox, type Speaker } from '../src/client/chat-send'
import { sourceOf } from '../src/client/chat'
import { blocksOf } from '../src/client/chat-text'

const row = (id: string, ord: number, rev: number, kind: Row['kind'], turn: string | null = null): Row => ({ id, ord, rev, turn, provisional: false, kind })
const turn = (id: string, ord: number, rev: number, prompt: string, extra: object = {}) =>
  row(id, ord, rev, { Turn: { prompt, origin: 'Typed', outcome: null, activity: 'Busy', ...extra } })
const prose = (id: string, ord: number, rev: number, text: string) => row(id, ord, rev, { Prose: { text, conclusion: false } }, 'turn:a')
const tool = (id: string, ord: number, rev: number, name: string, status = 'Done', summary = '') => row(id, ord, rev, { Tool: { name, status, summary, diff: [] } }, 'turn:a')
const thinking = (id: string, ord: number, rev: number) => row(id, ord, rev, { Thinking: { started_ms: 1, ended_ms: 2 } }, 'turn:a')
const page = (rows: Row[], rev: number, epoch = 7, moreBefore = false): RowPage => ({ epoch, rev, moreBefore, rows })
const changes = (list: RowChanges['changes'], rev: number, epoch = 7, reset = false): RowChanges => ({ epoch, rev, reset, changes: list })
const upsert = (r: Row, kind: 'insert' | 'update' = 'update') => ({ kind, id: r.id, rev: r.rev, row: r })

describe('the rows held', () => {
  it('a page is the transcript in order, and a follow applies by id', () => {
    const ledger = new RowLedger()
    ledger.page(page([turn('turn:a', 0, 1, 'Fix the build'), prose('p1', 1, 2, 'Looking')], 2))
    expect(ledger.order).toEqual(['turn:a', 'p1'])
    expect(ledger.cursor).toEqual({ epoch: 7, rev: 2 })

    const delta = ledger.follow(changes([upsert(prose('p1', 1, 3, 'Looking at the log')), upsert(prose('p2', 2, 4, 'Found it'), 'insert')], 4)) as Delta
    expect(delta.changed).toEqual(['p1', 'p2'])
    expect(ledger.order).toEqual(['turn:a', 'p1', 'p2'])
    expect(ledger.rows.get('p1')?.kind.Prose.text).toBe('Looking at the log')
    expect(ledger.cursor?.rev).toBe(4)
  })

  it('a row taken back leaves the list, and a repeat of what is held changes nothing', () => {
    const ledger = new RowLedger()
    ledger.page(page([turn('turn:a', 0, 1, 'Hi'), prose('p1', 1, 2, 'a')], 2))
    const same = ledger.follow(changes([upsert(prose('p1', 1, 2, 'a'))], 2)) as Delta
    expect(same.changed).toEqual([])
    const gone = ledger.follow(changes([{ kind: 'remove', id: 'p1', rev: 3 }], 3)) as Delta
    expect(gone.removed).toEqual(['p1'])
    expect(ledger.order).toEqual(['turn:a'])
  })

  it('a follow from another projection, or one the runner reset, says to page again', () => {
    const ledger = new RowLedger()
    ledger.page(page([turn('turn:a', 0, 1, 'Hi')], 1))
    expect(ledger.follow(changes([], 2, 8))).toBeNull()
    expect(ledger.follow(changes([], 2, 7, true))).toBeNull()
  })

  it('an older page goes above what is held, and only once', () => {
    const ledger = new RowLedger()
    ledger.page(page([prose('p3', 3, 5, 'c'), prose('p4', 4, 6, 'd')], 6, 7, true))
    expect(ledger.moreBefore).toBe(true)
    expect(ledger.oldestOrd).toBe(3)
    const delta = ledger.older(page([prose('p1', 1, 1, 'a'), prose('p2', 2, 2, 'b'), prose('p3', 3, 5, 'c')], 6, 7, false))
    expect(delta.changed).toEqual(['p1', 'p2'])
    expect(ledger.order).toEqual(['p1', 'p2', 'p3', 'p4'])
    expect(ledger.moreBefore).toBe(false)
    expect(ledger.older(page([prose('p1', 1, 1, 'a')], 6)).changed).toEqual([])
  })

  it('a new page of the same projection keeps the rows below it', () => {
    const ledger = new RowLedger()
    ledger.page(page([prose('p1', 1, 1, 'a'), prose('p2', 2, 2, 'b')], 2, 7, true))
    ledger.page(page([prose('p2', 2, 3, 'b!')], 3, 7, true))
    expect(ledger.order).toEqual(['p1', 'p2'])
    ledger.page(page([prose('q1', 0, 1, 'new')], 1, 9))
    expect(ledger.order).toEqual(['q1'])
  })

  it('the hint is the composer’s words and the newest turn says whether the agent is working', () => {
    const ledger = new RowLedger()
    ledger.page(page([turn('turn:a', 0, 1, 'one', { outcome: 'Finished' }), turn('turn:b', 1, 2, 'two'), row('hint:composer', 2, 3, { Hint: { text: 'Try "fix the tests"' } })], 3))
    expect(ledger.hint).toBe('Try "fix the tests"')
    expect(ledger.newestTurn?.id).toBe('turn:b')
    expect(itemsOf(ledger.order, ledger.rows).map(i => i.id)).toEqual(['turn:a', 'turn:b'])
  })
})

describe('what the transcript draws', () => {
  it('folds two or more tool calls into one line, with the thinking between them', () => {
    const rows = [turn('turn:a', 0, 1, 'go'), tool('t1', 1, 1, 'Read'), thinking('k1', 2, 1), tool('t2', 3, 1, 'Read'), prose('p', 4, 1, 'done'), tool('t3', 5, 1, 'Bash'), thinking('k2', 6, 1)]
    const map = new Map(rows.map(r => [r.id, r]))
    expect(itemsOf(rows.map(r => r.id), map)).toEqual([
      { kind: 'row', id: 'turn:a' },
      { kind: 'tools', id: 'tools:t1', rows: ['t1', 'k1', 't2'] },
      { kind: 'row', id: 'p' },
      { kind: 'row', id: 't3' },
      { kind: 'row', id: 'k2' },
    ])
  })

  it('closes a turn with its ending line after its last row, not before the next turn', () => {
    const rows = [turn('turn:a', 0, 1, 'one', { outcome: 'Finished' }), prose('p1', 1, 1, 'x'), turn('turn:b', 2, 1, 'two')]
    const map = new Map(rows.map(r => [r.id, r]))
    const nodes = nodesOf(itemsOf(rows.map(r => r.id), map), map, t => t.id === 'turn:a')
    expect(nodes.map(n => n.id)).toEqual(['turn:a', 'p1', 'end:turn:a', 'turn:b'])
  })
})

describe('the words', () => {
  const t = (name: string, status = 'Done', summary = '') => ({ name, status, summary })
  it('names a run of calls by what they did, in the present tense while one runs', () => {
    expect(groupTitle([t('Bash'), t('Bash'), t('Bash')])).toBe('Ran 3 commands')
    expect(groupTitle([t('Read'), t('Read', 'Running')])).toBe('Reading 2 files')
    expect(groupTitle([t('Edit'), t('Write')])).toBe('Edited 2 files')
    expect(groupTitle([t('Grep'), t('Glob')])).toBe('Ran 2 searches')
    expect(groupTitle([t('Read'), t('Bash'), t('WebFetch')])).toBe('Used 3 tools')
  })

  it('says what a run was for once each, files by name and sentences lowered', () => {
    expect(groupPurpose([t('Read', 'Done', '/a/b/main.rs'), t('Bash', 'Done', 'Map the code'), t('Bash', 'Done', 'Map the code'), t('Bash', 'Done', 'README check')])).toBe('main.rs, map the code, README check')
  })

  it('says how a turn ended', () => {
    expect(outcomeLine({ outcome: 'Finished', duration_ms: 65_000 })).toBe('Took 1:05')
    expect(outcomeLine({ outcome: 'Finished' })).toBe('Done')
    expect(outcomeLine({ outcome: 'Interrupted' })).toBe('Interrupted')
    expect(outcomeLine({ outcome: { Failed: { detail: 'rate limit' } } })).toBe('Failed: rate limit')
    expect(outcomeLine({ outcome: null })).toBeNull()
    expect(shortDuration(3_723_000)).toBe('1:02:03')
  })

  it('titles asks, queued messages, subagents, reports and task lists as the Mac does', () => {
    expect(askTitle({ kind: 'Permission' })).toBe('Claude is asking for permission')
    expect(askTitle({ kind: 'Question', answered: true, answered_by: 'iPhone' })).toBe('Answered on iPhone')
    expect(queuedLabel('InBox')).toBe('Waiting to send')
    expect(agentType('general-purpose')).toBe('General purpose')
    expect(harnessTitle({ kind: 'Report', source: 'Map the code' })).toBe('Report from Map the code')
    expect(taskProgress([{ status: 'Completed' }, { status: 'Pending' }])).toBe('1 of 2 done')
  })

  it('says why a message did not go without a Rust string, and keeps it in the box', () => {
    expect(sendFailure('closed', 'studio', 'send')).toContain('still in the box')
    expect(sendFailure('runner_too_old', 'studio', 'stop')).toContain('newer Far Cooler')
    expect(sendFailure('refused', 'studio', 'send')).not.toContain('refused')
  })

  it('reads the small Markdown an agent writes and nothing that could be markup', () => {
    expect(blocksOf('Done.\n\n- one\n- **two**\n\n```\nx < y\n```\nUse `cargo test`')).toEqual([
      { kind: 'paragraph', spans: [{ text: 'Done.' }] },
      { kind: 'list', ordered: false, items: [[{ text: 'one' }], [{ text: 'two', bold: true }]] },
      { kind: 'code', text: 'x < y' },
      { kind: 'paragraph', spans: [{ text: 'Use ' }, { text: 'cargo test', code: true }] },
    ])
    expect(blocksOf('<img src=x onerror=1>')).toEqual([{ kind: 'paragraph', spans: [{ text: '<img src=x onerror=1>' }] }])
  })
})

/// A runner whose answers the test settles one at a time.
function rig() {
  type Call = { kind: 'page' | 'follow'; args: unknown[]; settle: (v: any) => void; fail: (e: Error) => void }
  const calls: Call[] = []
  const drawn: Delta[] = []
  const phases: Phase[] = []
  const sleeps: number[] = []
  const slept: (() => void)[] = []
  const ask = (kind: Call['kind'], args: unknown[]) => new Promise<any>((settle, fail) => calls.push({ kind, args, settle, fail }))
  const source: RowSource = { page: (b, l) => ask('page', [b, l]), follow: (e, r, w) => ask('follow', [e, r, w]) }
  const ledger = new RowLedger()
  const follower = new RowFollower(
    ledger,
    source,
    { changed: d => drawn.push(d), phase: p => phases.push(p) },
    { retryMs: 500, maxRetryMs: 2000, sleep: ms => new Promise<void>(done => (sleeps.push(ms), slept.push(done))) },
  )
  const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve()
  }
  return { calls, drawn, phases, sleeps, slept, ledger, follower, flush }
}

describe('following a conversation', () => {
  it('pages the newest, then follows, each follow held open by the runner', async () => {
    const t = rig()
    t.follower.start()
    expect(t.calls.map(c => [c.kind, ...c.args])).toEqual([['page', null, 100]])
    t.calls[0].settle(page([turn('turn:a', 0, 1, 'Hi')], 1))
    await t.flush()
    expect(t.drawn).toHaveLength(1)
    expect(t.calls[1].kind).toBe('follow')
    expect(t.calls[1].args).toEqual([7, 1, FOLLOW_WAIT_MS])
    t.follower.stop()
  })

  it('a conversation already held is caught up with a follow that does not wait, not paged again', async () => {
    const t = rig()
    t.ledger.page(page([turn('turn:a', 0, 1, 'Hi')], 1))
    t.follower.start()
    expect(t.calls.map(c => [c.kind, ...c.args])).toEqual([['follow', 7, 1, 0]])
    t.follower.stop()
  })

  it('draws a row the moment the runner says it changed, and asks again with no wait of its own', async () => {
    const t = rig()
    t.follower.start()
    t.calls[0].settle(page([turn('turn:a', 0, 1, 'Hi')], 1))
    await t.flush()
    const before = t.drawn.length
    // The agent speaks: the held follow answers, and the row is drawn in the
    // same turn of the event loop, the next follow already out, no timer
    // between (so the delay is the network's, well under a second).
    t.calls[1].settle(changes([upsert(prose('p1', 1, 2, 'Looking'), 'insert')], 2))
    await t.flush()
    expect(t.drawn.length).toBe(before + 1)
    expect(t.ledger.order).toEqual(['turn:a', 'p1'])
    expect(t.sleeps).toEqual([])
    expect(t.calls[2].args).toEqual([7, 2, FOLLOW_WAIT_MS])
    t.calls[2].settle(changes([upsert(prose('p1', 1, 3, 'Looking at the log'))], 3))
    await t.flush()
    expect(t.ledger.rows.get('p1')?.kind.Prose.text).toBe('Looking at the log')
    expect(t.sleeps).toEqual([])
    t.follower.stop()
  })

  it('pages again when the runner resets the follow', async () => {
    const t = rig()
    t.follower.start()
    t.calls[0].settle(page([turn('turn:a', 0, 1, 'Hi')], 1))
    await t.flush()
    t.calls[1].settle(changes([], 5, 8, true))
    await t.flush()
    expect(t.calls[2].kind).toBe('page')
    t.follower.stop()
  })

  it('backs off after a failure, doubling to a ceiling, and pages again', async () => {
    const t = rig()
    t.follower.start()
    t.calls[0].fail(new Error('closed'))
    await t.flush()
    expect(t.phases.at(-1)).toEqual({ kind: 'trouble', word: 'closed' })
    expect(t.sleeps).toEqual([500])
    t.slept[0]()
    await t.flush()
    t.calls[1].fail(new Error('closed'))
    await t.flush()
    expect(t.sleeps).toEqual([500, 1000])
    t.slept[1]()
    await t.flush()
    expect(t.calls[2].kind).toBe('page')
    t.calls[2].settle(page([], 0))
    await t.flush()
    expect(t.phases.at(-1)).toEqual({ kind: 'live' })
    t.follower.stop()
  })

  it('stops for a runner too old to serve rows, and drops an answer that comes after a stop', async () => {
    const old = rig()
    old.follower.start()
    old.calls[0].fail(new Error('runner_too_old'))
    await old.flush()
    expect(old.phases.at(-1)).toEqual({ kind: 'unavailable' })
    expect(old.calls).toHaveLength(1)

    const t = rig()
    t.follower.start()
    t.follower.stop()
    t.calls[0].settle(page([turn('turn:a', 0, 1, 'late')], 1))
    await t.flush()
    expect(t.drawn).toEqual([])
    expect(t.ledger.order).toEqual([])
  })

  it('a second start replaces the first, so a new connection never draws twice', async () => {
    const t = rig()
    t.follower.start()
    t.follower.start()
    t.calls[0].settle(page([turn('turn:a', 0, 1, 'old connection')], 1))
    await t.flush()
    expect(t.drawn).toEqual([])
    t.calls[1].settle(page([turn('turn:a', 0, 1, 'new connection')], 1))
    await t.flush()
    expect(t.drawn).toHaveLength(1)
    t.follower.stop()
  })

  it('loads the page above the oldest row only while there is one', async () => {
    const t = rig()
    t.follower.start()
    t.calls[0].settle(page([prose('p3', 3, 3, 'c')], 3, 7, true))
    await t.flush()
    const more = t.follower.older()
    expect(t.calls.at(-1)?.kind).toBe('page')
    expect(t.calls.at(-1)?.args).toEqual([3, 100])
    t.calls.at(-1)?.settle(page([prose('p2', 2, 2, 'b')], 3, 7, false))
    await more
    expect(t.ledger.order).toEqual(['p2', 'p3'])
    const n = t.calls.length
    await t.follower.older()
    expect(t.calls).toHaveLength(n)
    t.follower.stop()
  })
})

describe('the runner’s rows through the wasm module', () => {
  it('asks for the newest page with no cursor and reads the JSON the module gives', async () => {
    const asked: unknown[][] = []
    const runner = {
      agentRows: async (...args: unknown[]) => (asked.push(['rows', ...args]), JSON.stringify(page([], 4))),
      agentRowsFollow: async (...args: unknown[]) => (asked.push(['follow', ...args]), JSON.stringify(changes([], 4))),
    }
    const source = sourceOf(runner as any, 'term-1')
    expect((await source.page(null, 100)).rev).toBe(4)
    expect((await source.page(12, 50)).epoch).toBe(7)
    await source.follow(7, 4, 20000)
    expect(asked).toEqual([
      ['rows', 'term-1', undefined, 100],
      ['rows', 'term-1', 12, 50],
      ['follow', 'term-1', 7, 4, 20000],
    ])
  })
})

describe('sending and stopping', () => {
  function speaker() {
    const log: string[] = []
    let fail: Error | null = null
    let release: (() => void) | null = null
    const runner: Speaker = {
      compose: async (terminal, text) => {
        log.push(`compose ${terminal} ${JSON.stringify(text)}`)
        if (release === null && fail) throw fail
        if (release) await new Promise<void>(done => (release = done))
        return text.startsWith('queue')
      },
      interrupt: async terminal => {
        log.push(`interrupt ${terminal}`)
        if (fail) throw fail
      },
    }
    return {
      runner,
      log,
      failWith: (e: Error | null) => (fail = e),
      hold: () => (release = () => {}),
      go: () => release?.(),
    }
  }

  it('a message reaches the agent as one compose with the person’s words, trimmed', async () => {
    const s = speaker()
    const box = new Outbox(() => s.runner, 't-1', 'studio')
    expect(await box.send('  Run the tests too \n')).toEqual({ ok: true, queued: false })
    expect(s.log).toEqual(['compose t-1 "Run the tests too"'])
    expect(await box.send('queue this')).toEqual({ ok: true, queued: true })
  })

  it('nothing is sent for an empty box, a missing runner, or a second press while one is out', async () => {
    const s = speaker()
    const box = new Outbox(() => s.runner, 't-1', 'studio')
    expect((await box.send('   ')).ok).toBe(false)
    expect((await new Outbox(() => null, 't-1', 'studio').send('hi')).ok).toBe(false)
    s.hold()
    const first = box.send('one')
    expect(box.busy).toBe(true)
    expect(await box.send('two')).toMatchObject({ ok: false, ignored: true })
    s.go()
    await first
    expect(s.log).toEqual(['compose t-1 "one"'])
    expect(box.busy).toBe(false)
  })

  it('a message that did not go says why, in the page’s words', async () => {
    const s = speaker()
    s.failWith(new Error('closed'))
    const box = new Outbox(() => s.runner, 't-1', 'studio')
    const out = await box.send('hello')
    expect(out).toMatchObject({ ok: false })
    expect((out as { message: string }).message).toContain('studio')
    expect((out as { message: string }).message).not.toContain('closed')
    expect(box.busy).toBe(false)
  })

  it('Stop is one interrupt on this terminal and never a keystroke', async () => {
    const s = speaker()
    const box = new Outbox(() => s.runner, 't-9', 'studio')
    expect(await box.stop()).toEqual({ ok: true, queued: false })
    expect(s.log).toEqual(['interrupt t-9'])
    s.failWith(new Error('refused'))
    expect(await box.stop()).toMatchObject({ ok: false })
  })
})
