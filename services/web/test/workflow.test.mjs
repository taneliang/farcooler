// Standalone Node test: compiled production modules + actual Rust golden bytes.
// No browser launch or device/UI parity claim.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const require = createRequire(import.meta.url)
const { WorkflowBoard } = require(path.resolve(process.argv[2], 'workflow.js'))
const { renderWorkflow } = require(path.resolve(process.argv[2], 'workflow-view.js'))
const fixtures = JSON.parse(fs.readFileSync(new URL('../../../crates/client/src/testdata/workflow-presentation.json', import.meta.url)))
const clone = value => structuredClone(value)
let count = 0
const test = async (name, fn) => { if (process.env.WORKFLOW_TEST_FILTER && !name.includes(process.env.WORKFLOW_TEST_FILTER)) return; await fn(); console.log(`ok ${++count} ${name}`) }
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
class Element {
  children = []; textContent = ''; disabled = false; attributes = {}; parent = null
  constructor(tag) { this.tagName = tag }
  append(...nodes) { for (const node of nodes) { node.parent = this; this.children.push(node) } }
  replaceChildren(...nodes) {
    if (this.contains(document.activeElement)) document.activeElement = null
    for (const child of this.children) child.parent = null
    this.children = []; this.append(...nodes)
  }
  setAttribute(key, value) { this.attributes[key] = value }
  getAttribute(key) { return this.attributes[key] ?? null }
  contains(node) { return node === this || this.children.some(c => c.contains(node)) }
  querySelectorAll(tag) { return this.children.flatMap(c => [...(c.tagName === tag ? [c] : []), ...c.querySelectorAll(tag)]) }
  focus() { document.activeElement = this }
}
globalThis.document = { createElement: tag => new Element(tag), activeElement: null }
const allText = node => [node.textContent, ...node.children.map(allText)].join('\n')
for (const fixture of fixtures) await test(`Rust ${fixture.name} renders shared slots and whole descriptor`, async () => {
  const presentation = fixture.response.card
  const sent = []; let view
  const port = { read: async () => presentation, dispatch: async (...args) => { sent.push(args); return 'retained inspection identity' } }
  const board = new WorkflowBoard(presentation.workspace, () => port, next => { view = next })
  await board.refresh(); board.select(presentation.runs[0].item)
  const root = new Element('div'); renderWorkflow(root, view, board)
  const run = presentation.runs[0]; const text = allText(root)
  for (const value of [run.label, run.phase_label, run.run, run.definition_pin, run.policy_pin, ...run.reasons.flatMap(r => [r.label, r.text])]) assert(text.includes(value))
  for (const value of Object.values(run.latest_evidence ?? {})) assert(text.includes(value))
  for (const action of run.actions) {
    await board.dispatch(run.item, action)
    assert.strictEqual(sent.at(-1)[2], action)
    assert.deepEqual(sent.at(-1), [presentation.workspace, run.item, action])
  }
  board.disconnect(); renderWorkflow(root, view, board)
  for (const button of root.querySelectorAll('button').filter(b => run.actions.some(a => a.label === b.textContent))) assert(button.disabled)
  const before = sent.length; if (run.actions[0]) await board.dispatch(run.item, run.actions[0]); assert.equal(sent.length, before)
})
await test('Feature A invalidated evidence stays visible; action has no shell mapping', async () => {
  const p = clone(fixtures.find(f => f.response.card.runs[0].reasons.some(r => r.kind === 'EvidenceInvalidated')).response.card)
  const action = p.runs[0].actions.find(a => a.kind === 'InspectLatestEvidence')
  action.label = '<img src=x onerror=fail()> ' + 'Long shared label '.repeat(100)
  action.extension = { opaque: 'keep me' }
  let view; let forwarded
  const board = new WorkflowBoard(p.workspace, () => ({ read: async () => p, dispatch: async (_w, _i, a) => { forwarded = a; return '' } }), v => { view = v })
  await board.refresh(); board.select(p.runs[0].item)
  const root = new Element('div'); renderWorkflow(root, view, board)
  assert(allText(root).includes(action.label)); assert.equal(root.querySelectorAll('img').length, 0)
  await board.dispatch(p.runs[0].item, action); assert.strictEqual(forwarded, action)
})
await test('disconnect fences delayed read; reconnect replaces actions; selection restores', async () => {
  const p = clone(fixtures[0].response.card); const read = deferred(); let view; let kept
  let port = { read: () => read.promise, dispatch: async () => '' }
  const board = new WorkflowBoard(p.workspace, () => port, v => { view = v }, item => { kept = item })
  board.select(p.runs[0].item); const loading = board.refresh(); board.disconnect(); read.resolve(p); await loading
  assert.equal(view.ready, false); assert.equal(view.presentation, null)
  port = { ...port, read: async () => p }; await board.refresh(); assert.equal(view.ready, true); assert.equal(view.selected, kept)
  const old = p.runs[0].actions[0]; const newer = clone(p); newer.runs[0].actions = []; port.read = async () => newer
  await board.refresh(); await board.dispatch(p.runs[0].item, old); assert.equal(view.presentation.runs[0].actions.length, 0)
  const reopened = new WorkflowBoard(p.workspace, () => port, v => { view = v }); reopened.select(kept); await reopened.refresh(); assert.equal(view.selected, kept)
})
await test('wrong-workspace/schema response stays disabled; pending seam is explicit', async () => {
  let view
  const board = new WorkflowBoard('other', () => ({ read: async () => fixtures[0].response.card }), v => { view = v })
  await board.refresh(); assert.equal(view.ready, false)
  const pending = new WorkflowBoard('other', () => null, v => { view = v }); await pending.refresh()
  assert(view.notice.includes('not connected')); assert.equal(view.ready, false)
})
await test('lost action reply does not auto-retry; overlapping refresh cannot unfreeze actions', async () => {
  const p = fixtures[0].response.card; let view; let calls = 0; const result = deferred()
  const port = { read: async () => p, dispatch: async () => { calls++; await result.promise; throw new Error('lost reply') } }
  const board = new WorkflowBoard(p.workspace, () => port, v => { view = v }); await board.refresh()
  const action = p.runs[0].actions[0]; const sending = board.dispatch(p.runs[0].item, action)
  await board.refresh(); assert.equal(view.busy, true); await board.dispatch(p.runs[0].item, action); assert.equal(calls, 1)
  result.resolve(); await sending; assert.equal(calls, 1); assert(view.notice.includes('wasn’t confirmed')); assert.equal(view.busy, false)
})
await test('lossless u64 identity and unknown shared copy use existing text slots', async () => {
  const p = clone(fixtures[0].response.card)
  p.runs[0].fence = 18446744073709551615n
  p.runs[0].reasons = [{ kind: 'FutureSharedReason', label: 'Shared label '.repeat(100), text: 'Shared text '.repeat(100) }]
  let view
  const board = new WorkflowBoard(p.workspace, () => ({ read: async () => p }), v => { view = v })
  await board.refresh(); board.select(p.runs[0].item)
  const root = new Element('div'); renderWorkflow(root, view, board)
  assert(allText(root).includes('18446744073709551615')); assert(allText(root).includes(p.runs[0].reasons[0].text))
})
await test('late inspection is fenced by selection while cleanup and refresh still finish', async () => {
  for (const target of [null, 'item-b', fixtures[0].response.card.runs[0].item]) {
    const p = clone(fixtures[0].response.card); const run = p.runs[0]
    p.runs.push({ ...clone(run), item: 'item-b', run: 'run-b' })
    const reply = deferred(); let view; let reads = 0
    const port = { read: async () => { reads++; return p }, dispatch: () => reply.promise }
    const board = new WorkflowBoard(p.workspace, () => port, v => { view = v })
    await board.refresh(); board.select(run.item)
    const sending = board.dispatch(run.item, run.actions[0])
    board.select(null); board.select(target)
    reply.resolve('A evidence'); await sending
    assert.equal(view.inspection, ''); assert.equal(view.busy, false); assert.equal(view.ready, true); assert.equal(reads, 2)
  }
})
await test('run replacement or removal clears retained inspection including post-action refresh', async () => {
  for (const remove of [false, true]) {
    let p = clone(fixtures[0].response.card); const run = p.runs[0]; let view
    const port = { read: async () => p, dispatch: async () => 'A evidence' }
    const board = new WorkflowBoard(p.workspace, () => port, v => { view = v })
    await board.refresh(); board.select(run.item); await board.dispatch(run.item, run.actions[0]); assert.equal(view.inspection, 'A evidence')
    p = clone(p); if (remove) p.runs = []; else p.runs[0].run = 'replacement'
    await board.refresh(); assert.equal(view.inspection, '')
  }
  const p = clone(fixtures[0].response.card); const run = p.runs[0]; let view; let reads = 0
  const board = new WorkflowBoard(p.workspace, () => ({
    read: async () => ++reads === 1 ? p : { ...p, runs: [{ ...run, run: 'replacement' }] }, dispatch: async () => 'old evidence'
  }), v => { view = v })
  await board.refresh(); board.select(run.item); await board.dispatch(run.item, run.actions[0]); assert.equal(view.inspection, '')
})
await test('old failed action cannot overwrite disconnect or newer session after its awaited read', async () => {
  for (const reconnect of [false, true]) {
    const p = clone(fixtures[0].response.card); const read = deferred(); const started = deferred(); let view; let reads = 0
    let port = { read: async () => { if (++reads === 1) return p; started.resolve(); return read.promise }, dispatch: async () => { throw Error('lost') } }
    const board = new WorkflowBoard(p.workspace, () => port, v => { view = v })
    await board.refresh(); const sending = board.dispatch(p.runs[0].item, p.runs[0].actions[0]); await started.promise
    board.disconnect()
    if (reconnect) { port = { read: async () => p }; await board.refresh() }
    const notice = view.notice; read.resolve(p); await sending
    assert.equal(view.notice, notice); assert.equal(view.ready, reconnect); assert.equal(view.inspection, '')
  }
})
await test('disconnect fences delayed action result through reconnect without replay', async () => {
  for (const reconnect of [false, true]) {
    const p = clone(fixtures[0].response.card); const reply = deferred(); let view; let calls = 0
    const port = { read: async () => p, dispatch: () => { calls++; return reply.promise } }
    const board = new WorkflowBoard(p.workspace, () => port, v => { view = v }); await board.refresh(); board.select(p.runs[0].item)
    const sending = board.dispatch(p.runs[0].item, p.runs[0].actions[0]); board.disconnect()
    if (reconnect) await board.refresh()
    reply.resolve('old evidence'); await sending
    assert.equal(view.inspection, ''); assert.equal(view.ready, reconnect); assert.equal(view.busy, false); assert.equal(calls, 1)
  }
})
await test('stable focus survives run entry, busy completion, copy changes, failed read and action removal', async () => {
  let p = clone(fixtures[0].response.card); p.runs[0].actions[0].label = 'Refresh'
  const root = new Element('div'); const reply = deferred(); let failRead = false
  const port = { read: async () => { if (failRead) throw Error('offline'); return p }, dispatch: () => reply.promise }
  const board = new WorkflowBoard(p.workspace, () => port, v => renderWorkflow(root, v, board))
  const controls = () => root.querySelectorAll('button')
  const activeKey = () => document.activeElement?.getAttribute('data-workflow-control')
  await board.refresh()
  const entry = controls()[1]
  entry.focus(); entry.onclick(); assert.equal(document.activeElement?.textContent, 'All Runs')
  let action = controls().find(b => b.getAttribute('data-workflow-control').startsWith('["action"'))
  const key = action.getAttribute('data-workflow-control'); action.focus()
  const sending = board.dispatch(p.runs[0].item, p.runs[0].actions[0]); assert.equal(activeKey(), 'all-runs')
  reply.resolve('evidence'); await sending; assert.equal(activeKey(), key)
  p = clone(p); p.runs[0].actions[0].label = 'New shared copy'; await board.refresh(); assert.equal(activeKey(), key)
  failRead = true; await board.refresh(); assert.equal(activeKey(), 'all-runs')
  failRead = false; await board.refresh(); assert.equal(activeKey(), key)
  p = clone(p); p.runs[0].actions = []; await board.refresh(); assert.equal(activeKey(), 'all-runs')
  assert(root.contains(document.activeElement)); assert(!document.activeElement.disabled)
})
console.log(`${count} workflow fixture/controller/DOM tests passed; simulated DOM only`)
