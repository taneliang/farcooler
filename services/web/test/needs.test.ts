// Needs You in the browser (src/client/needs.ts, ov-773): the runner's list as
// every device reads it, the buttons it offers, and what a refused answer says.
// The runner is played by the test; the list is the fixture the Mac and both
// phones decode (test/fixtures/needs-you.json at the repo root).

import { describe, expect, it } from 'vitest'

import fixture from '../../../test/fixtures/needs-you.json'
import { answerRefusal, needsYouFailure } from '../src/client/copy'
import { age, Board, type BoardView, buttonsFor, countLabel, countOf, githubLink, groups, type Item, pageTitle, parseList, pressFor } from '../src/client/needs'

const runner = (name: string) => (fixture.runners as any[]).find(r => r.runner === name).needs_you as { items: any[] }
const studio = () => parseList(JSON.stringify(runner('studio')))
const buildBox = () => parseList(JSON.stringify(runner('build-box')))
const byId = (items: Item[], id: string) => items.find(i => i.id === id)!
const ASK = 'ask:hook-ask-01000000-0000-7000-8000-000000000029'

describe('the list', () => {
  // ov-773 acceptance 1: the same list the Mac decodes, for the same runner.
  it('keeps the runner’s items, order, questions and buttons, as the fixture the Mac decodes holds them', () => {
    for (const name of ['studio', 'build-box']) {
      const wire = runner(name).items
      const shown = parseList(JSON.stringify(runner(name)))
      expect(shown.map(i => i.id)).toEqual(wire.map(i => i.id))
      expect(shown.map(i => i.kind)).toEqual(wire.map(i => i.kind))
      expect(shown.map(i => i.question)).toEqual(wire.map(i => i.question))
      expect(shown.map(i => i.actions.map(a => a.id))).toEqual(wire.map(i => i.actions.map((a: any) => a.id)))
      expect(shown.map(i => i.since)).toEqual(wire.map(i => i.since))
    }
  })

  it('reads an ask’s terminal and ask id, and a decision’s task', () => {
    const ask = byId(studio(), ASK)
    expect(ask.askId).toBe('hook-ask-01000000-0000-7000-8000-000000000029')
    expect(ask.terminal?.id).toBe('01000000-0000-7000-8000-000000000004')
    expect(ask.worktree).toEqual({ branch: 'bil/webhooks', insertions: 18, deletions: 40 })
    expect(studio().find(i => i.kind === 'decision')?.task?.key).toBe('bil-9')
  })

  it('leaves out an item with no id or question, and reads a kind it doesn’t know as one that still counts', () => {
    const items = parseList(JSON.stringify({ items: [{ id: 'x' }, { id: 'a', kind: 'from_the_future', question: 'Hm?' }, null, 'no'] }))
    expect(items.map(i => [i.id, i.kind])).toEqual([['a', 'unknown']])
    expect(countOf(items)).toBe(1)
    expect(parseList('not json')).toEqual([])
    expect(parseList('{}')).toEqual([])
  })

  it('counts every item but a quiet note', () => {
    const items = parseList(JSON.stringify({ items: [{ id: 'a', kind: 'ask', question: 'q' }, { id: 'b', kind: 'fyi', question: 'Bob kept a ruling' }] }))
    expect(items).toHaveLength(2)
    expect(countOf(items)).toBe(1)
    expect(countOf(studio())).toBe(6)
  })

  it('puts the count in front of the tab’s title, and nothing when nothing needs you', () => {
    expect(pageTitle(3)).toBe('(3) Far Cooler')
    expect(pageTitle(0)).toBe('Far Cooler')
    expect(countLabel(2)).toBe('Needs You (2)')
    expect(countLabel(0)).toBe('Needs You')
  })

  it('says how long, and nothing when the runner didn’t say', () => {
    const now = 1_000_000_000
    expect(age(null, now)).toBe('')
    expect(age(now - 5_000, now)).toBe('Just now')
    expect(age(now - 4 * 60_000, now)).toBe('4 min')
    expect(age(now - 3 * 3_600_000, now)).toBe('3 hr')
    expect(age(now - 50 * 3_600_000, now)).toBe('2 d')
    expect(age(now + 5_000, now)).toBe('Just now')
  })

  it('splits only when something is someone else’s', () => {
    const g = groups(studio())
    expect(g.split).toBe(true)
    expect(g.others.map(i => i.concern?.name)).toContain('Bob Okafor')
    expect(g.yours.length + g.others.length).toBe(6)
    expect(groups(buildBox()).split).toBe(false)
  })

  it('links only to github.com, whole', () => {
    expect(githubLink('https://github.com/acme/app/pull/4')).toBe('https://github.com/acme/app/pull/4')
    for (const bad of ['https://github.com.evil.example/x', 'https://github.com@evil.example/', 'http://github.com/x', 'https://github.com:8443/x', 'javascript:alert(1)', 'https://github.com/a b', null, '']) {
      expect(githubLink(bad)).toBeNull()
    }
  })
})

describe('the buttons', () => {
  it('sends an ask’s answer on its terminal, by its ask id and the button’s id', () => {
    const ask = byId(studio(), ASK)
    const [allow, deny] = buttonsFor(ask)
    expect(allow.press).toEqual({ kind: 'ask', terminal: '01000000-0000-7000-8000-000000000004', ask: 'hook-ask-01000000-0000-7000-8000-000000000029', option: 'allow' })
    expect(deny.press).toMatchObject({ kind: 'ask', option: 'deny' })
    expect(deny.action.destructive).toBe(true)
  })

  it('answers a decision with the option’s text on its task', () => {
    const decision = studio().find(i => i.kind === 'decision')!
    expect(buttonsFor(decision).map(b => b.press)).toEqual([
      { kind: 'decision', task: '01000000-0000-7000-8000-00000000000c', body: 'Postgres' },
      { kind: 'decision', task: '01000000-0000-7000-8000-00000000000c', body: 'SQLite' },
    ])
  })

  // ov-773 acceptance 3: below Control the runner sends no ask id and no
  // buttons, and the page draws none rather than a button that can't work.
  it('draws no button for a Read-scoped item, though it lists it', () => {
    const items = buildBox()
    expect(items).toHaveLength(2)
    expect(items.every(i => buttonsFor(i).length === 0)).toBe(true)
    expect(items[0].askId).toBeNull()
  })

  it('refuses to send an ask with no id, even if it was handed a button', () => {
    const ask = { ...byId(studio(), ASK), askId: null }
    expect(pressFor(ask, { id: 'allow', title: 'Allow', destructive: false, primary: true })).toBeNull()
  })

  it('draws Open only where the page has that terminal', () => {
    const blocked = studio().find(i => i.kind === 'blocked')!
    expect(buttonsFor(blocked, () => true).map(b => b.press.kind)).toEqual(['open'])
    expect(buttonsFor(blocked, () => false)).toEqual([])
  })

  it('has no button for a Claude sign-in, which a browser can’t run', () => {
    const login = parseList(JSON.stringify({ items: [{ id: 'l', kind: 'claude_login', question: 'Sign in', terminal: null, actions: [{ id: 'sign_in', title: 'Sign In', destructive: false, primary: true }] }] }))[0]
    expect(buttonsFor(login)).toEqual([])
  })
})

/// A runner played by hand: a list it holds, answers it takes or refuses.
function rig(initial: any, opts: { refuse?: { word?: string; code?: string; what?: string }; failList?: string[] } = {}) {
  const held = { items: [...initial.items] }
  const calls: string[] = []
  const views: BoardView[] = []
  const opened: string[] = []
  const fail = (word: string, extra: object = {}) => Object.assign(new Error(word), extra)
  const board = new Board({
    runner: 'studio',
    list: async () => {
      calls.push('list')
      const next = opts.failList?.shift()
      if (next) throw fail(next)
      return JSON.stringify(held)
    },
    ask: async (terminal, ask, option) => {
      calls.push(`ask ${terminal} ${ask} ${option}`)
      if (opts.refuse) throw fail(opts.refuse.word ?? 'refused', { code: opts.refuse.code, what: opts.refuse.what })
      held.items = held.items.filter(i => i.ask_id !== ask)
    },
    decide: async (task, body) => {
      calls.push(`decide ${task} ${body}`)
      if (opts.refuse) throw fail(opts.refuse.word ?? 'refused', { code: opts.refuse.code, what: opts.refuse.what })
      held.items = held.items.filter(i => i.task?.id !== task)
    },
    show: v => views.push(v),
    open: t => opened.push(t),
  })
  return { board, calls, views, opened, held, last: () => views[views.length - 1], failNext: (word: string) => (opts.failList ??= []).push(word) }
}

describe('the board', () => {
  // ov-773 acceptance 2: an answer is sent the moment it is pressed, and the
  // list is read straight after, so the item is gone from this screen as the
  // runner drops it everywhere else (the Mac reads the same runner's list).
  it('sends an answer at once, then reads the list again, and the item is gone', async () => {
    const t = rig(runner('studio'))
    await t.board.refresh()
    const ask = byId(t.last().items, ASK)
    const [allow] = buttonsFor(ask)

    const pending = t.board.press(ask, allow.press)
    // Before anything is awaited: no debounce, no timer.
    expect(t.calls.at(-1)).toBe('ask 01000000-0000-7000-8000-000000000004 hook-ask-01000000-0000-7000-8000-000000000029 allow')
    expect(t.last().busy.has(ASK)).toBe(true)
    await pending

    expect(t.calls).toEqual(['list', 'ask 01000000-0000-7000-8000-000000000004 hook-ask-01000000-0000-7000-8000-000000000029 allow', 'list'])
    expect(t.last().items.some(i => i.id === ASK)).toBe(false)
    expect(t.last().items).toHaveLength(5)
    expect(t.last().busy.size).toBe(0)
    expect(t.last().notices.size).toBe(0)
  })

  it('answers a decision with its option', async () => {
    const t = rig(runner('studio'))
    await t.board.refresh()
    const decision = t.last().items.find(i => i.kind === 'decision')!
    await t.board.press(decision, buttonsFor(decision)[1].press)
    expect(t.calls).toContain('decide 01000000-0000-7000-8000-00000000000c SQLite')
    expect(t.last().items.some(i => i.id === decision.id)).toBe(false)
  })

  it('presses once: a second press while one is out sends nothing', async () => {
    const t = rig(runner('studio'))
    await t.board.refresh()
    const ask = byId(t.last().items, ASK)
    const press = buttonsFor(ask)[0].press
    const first = t.board.press(ask, press)
    const second = t.board.press(ask, press)
    await Promise.all([first, second])
    expect(t.calls.filter(c => c.startsWith('ask'))).toHaveLength(1)
  })

  // ov-773 acceptance 3: the runner says no to a device that only looks, and
  // the page keeps its list and says so in words.
  it('shows a refused answer as the runner’s refusal in words, keeps the list, and does not throw', async () => {
    const t = rig(runner('studio'), { refuse: { code: 'scope-denied', what: '' } })
    await t.board.refresh()
    const ask = byId(t.last().items, ASK)
    await t.board.press(ask, buttonsFor(ask)[0].press)

    const notice = t.last().notices.get(ASK)!
    expect(notice).toBe('This browser can only look at studio. Answering needs control, which is granted from a device that already has it.')
    expect(notice).not.toMatch(/scope|denied|refused/)
    expect(t.last().items).toHaveLength(6)
    expect(t.last().state).toBe('ready')
    expect(t.last().busy.size).toBe(0)
  })

  it('says someone already answered, and the next read drops the item and its notice', async () => {
    const t = rig(runner('studio'), { refuse: { code: 'resource-conflict', what: 'not_held' } })
    await t.board.refresh()
    const ask = byId(t.last().items, ASK)
    await t.board.press(ask, buttonsFor(ask)[0].press)
    expect(t.last().notices.get(ASK)).toBe('Someone already answered this.')
    t.held.items = t.held.items.filter(i => i.id !== ASK)
    await t.board.refresh()
    expect(t.last().notices.size).toBe(0)
  })

  it('goes to a terminal for Open, and sends nothing', async () => {
    const t = rig(runner('studio'))
    await t.board.refresh()
    const blocked = t.last().items.find(i => i.kind === 'blocked')!
    await t.board.press(blocked, buttonsFor(blocked)[0].press)
    expect(t.opened).toEqual([blocked.terminal!.id])
    expect(t.calls).toEqual(['list'])
  })

  it('reads the list once for polls that land together', async () => {
    const t = rig(runner('studio'))
    await Promise.all([t.board.refresh(), t.board.refresh(), t.board.refresh()])
    expect(t.calls).toEqual(['list'])
    await t.board.refresh()
    expect(t.calls).toEqual(['list', 'list'])
  })

  it('says a runner too old to send the list needs updating, with nothing listed', async () => {
    const t = rig({ items: [] }, { failList: ['runner_too_old'] })
    await t.board.refresh()
    expect(t.last().state).toBe('old')
    expect(t.last().banner).toBe('Update Far Cooler on studio to see decisions and asks here.')
    expect(t.last().items).toEqual([])
  })

  it('says a first read that fails could not read the list', async () => {
    const t = rig(runner('studio'), { failList: ['closed'] })
    await t.board.refresh()
    expect(t.last().state).toBe('unreadable')
    expect(t.last().banner).toContain('didn’t answer')
  })

  it('keeps a good list when a later read fails, and says it may be behind', async () => {
    const t = rig(runner('studio'))
    await t.board.refresh()
    t.failNext('closed')
    await t.board.refresh()
    expect(t.last().state).toBe('ready')
    expect(t.last().items).toHaveLength(6)
    expect(t.last().banner).toContain('may be behind')
  })

  it('draws nothing once stopped', async () => {
    const t = rig(runner('studio'))
    t.board.stop()
    await t.board.refresh()
    expect(t.views).toEqual([])
  })
})

describe('the words', () => {
  it('words each refusal as the phones do, and never with the runner’s own words', () => {
    const say = (f: Parameters<typeof answerRefusal>[0]) => answerRefusal(f, 'studio', 'claude')
    expect(say({ word: 'refused', code: 'resource-conflict', what: 'not_held' })).toBe('Someone already answered this.')
    expect(say({ word: 'refused', code: 'resource-conflict', what: 'not_delivered' })).toBe('Couldn’t reach claude. Try again.')
    expect(say({ word: 'refused', code: 'resource-conflict', what: 'unconfirmed' })).toBe('Sent, but claude hasn’t moved on yet. Check the terminal before answering again.')
    expect(say({ word: 'refused', code: 'resource-conflict', what: 'too_soon' })).toBe('An answer was just sent. Check the terminal before answering again.')
    expect(say({ word: 'refused', code: 'resource-conflict', what: 'not_on_yes' })).toBe('The prompt’s cursor isn’t on Yes. Answer it in the terminal.')
    expect(say({ word: 'refused', code: 'capability-unsupported' })).toBe('Far Cooler on studio is too old for this. Update it there, then try again.')
    expect(say({ word: 'refused', code: 'something-new' })).toBe('studio didn’t take your answer. Try again.')
    expect(say({ word: 'protocol' })).toBe('Your answer may not have reached the runner. Try again.')
    expect(say({ word: 'closed' })).toBe('studio didn’t answer. Your answer may not have arrived. Check the terminal, then try again.')
  })

  it('words a failed read by whether the runner answered', () => {
    expect(needsYouFailure('closed', 'studio')).toContain('reconnecting')
    expect(needsYouFailure('protocol', 'studio')).toBe('Far Cooler couldn’t read what needs you on studio. It tries again in a few seconds.')
  })
})
