import { createRequire } from 'node:module'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { webcrypto } from 'node:crypto'
if (!globalThis.crypto) Object.defineProperty(globalThis, 'crypto', { value: webcrypto })
const require = createRequire(import.meta.url)
const { WorkflowSessionPort, workflowJSON } = require(path.resolve(process.argv[2], 'workflow-session.js'))
const fixture = JSON.parse(fs.readFileSync('crates/client/src/testdata/workflow-presentation.json', 'utf8'))[0].response.card
const maximum = '18446744073709551615'
const snapshot = JSON.stringify({ host: 'actual-host', presentation: fixture }).replace(/"revision":\d+/, '"revision":18446744073709551614').replace(/"epoch":\d+/, '"epoch":'+maximum)
assert.equal(workflowJSON(snapshot).presentation.epoch, maximum)
assert.equal(workflowJSON('{"label":"max 18446744073709551615 \\\"123\\\"","negative":-18446744073709551615}').negative, '-'+maximum)
const data = new Map()
const saved = () => [...data.values()].flatMap(raw => JSON.parse(JSON.parse(raw).payload).entries)
const storage = { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value) }
let actualHost = 'actual-host'
let account = 'account', available = true, sessionID = 1, preparations = 0, submits = [], fail = true
const prepared = { schema: 1, workspace: fixture.workspace, json: '{"mutation":{"epoch":'+maximum+'},"id":"opaque"}' }
const session = { generation: 1, async workflow_call(method, raw) {
  const args = JSON.parse(raw)
  if (method === 'workflow.snapshot') return snapshot.replace('actual-host', actualHost)
  if (method === 'workflow.prepare_selection') {
    preparations++
    assert.equal(args.revision, '18446744073709551614'); assert.equal(args.epoch, maximum)
    assert.deepEqual(args.action, fixture.runs[0].actions[0])
    assert.equal(saved().some(e => e.attempts === 0), true)
    return JSON.stringify({ kind: 'Mutation', prepared })
  }
  assert.equal(method, 'workflow.submit_action')
  assert.deepEqual(args.prepared, prepared, 'opaque Prepared must cross submit unchanged')
  assert.equal(saved().some(e => e.prepared?.json.includes(maximum)), true, 'prepared persisted before submit')
  submits.push({ sessionID: this.generation, raw })
  if (fail) throw new Error('discarded answer')
  return '{"receipt":{"revision":'+maximum+'}}'
} }
let activeSession = session
const current = () => available ? {account,host:'configured-alias',session:activeSession} : null
const exclusive = async (_key, body) => body()
let port = new WorkflowSessionPort(current, storage, exclusive)
const view = await port.read(fixture.workspace)
assert.equal(view.revision, '18446744073709551614')
await assert.rejects(port.dispatch(fixture.workspace, fixture.runs[0].item, fixture.runs[0].actions[0]))
assert.equal(preparations, 1)
let pending = await port.pending(fixture.workspace); assert.equal(pending.length, 1)
// Reconnect the same adapter, then restore a new process object. Each
// transport records its own identity, so a stale captured Session is detectable.
sessionID = 2; activeSession = {generation:2,workflow_call:session.workflow_call}
await port.read(fixture.workspace); assert.equal(submits.length, 1, 'reconnect/read cannot replay')
await assert.rejects(port.dispatch(fixture.workspace, fixture.runs[0].item, fixture.runs[0].actions[0])); assert.equal(preparations, 1)
await assert.rejects(port.retry(fixture.workspace, pending[0].id))
port = new WorkflowSessionPort(current, storage, exclusive); sessionID = 3; activeSession = {generation:3,workflow_call:session.workflow_call}
await port.read(fixture.workspace); assert.equal(submits.length, 2, 'process restoration cannot replay')
await assert.rejects(port.retry(fixture.workspace, pending[0].id))
assert.equal((await port.pending(fixture.workspace)).length, 1, 'three failed invocations cannot permanently ban identical retained retry')
fail = false
assert.equal(await port.retry(fixture.workspace, pending[0].id), '{"receipt":{"revision":'+maximum+'}}')
assert.equal(submits.length, 4); assert.equal(new Set(submits.map(s=>s.raw)).size, 1)
assert.deepEqual(submits.map(s=>s.sessionID), [1,2,3,3]); assert.equal((await port.pending(fixture.workspace)).length, 0)
await assert.rejects(port.retry(fixture.workspace, pending[0].id))
account = 'other-account'; await port.read(fixture.workspace); assert.equal((await port.pending(fixture.workspace)).length, 0)
account = 'account'; actualHost = 'different-actual-server'; await port.read(fixture.workspace)
assert.equal((await port.pending(fixture.workspace)).length, 0)
fail = true; await assert.rejects(port.dispatch(fixture.workspace, fixture.runs[0].item, fixture.runs[0].actions[0])); assert.equal(preparations, 2, 'same alias on a different actual server has its own journal')
available = false; await assert.rejects(port.read(fixture.workspace))
// A storage refusal prevents even prepare (and therefore every submit).
let calls = 0
const brokenSession = { async workflow_call() { calls++; return snapshot } }
const broken = new WorkflowSessionPort(() => ({ account: 'a', host: 'h', session: brokenSession }),
  { getItem: () => null, setItem: () => { throw new Error('disk full') } }, exclusive)
await broken.read(fixture.workspace)
await assert.rejects(broken.dispatch(fixture.workspace, fixture.runs[0].item, fixture.runs[0].actions[0])); assert.equal(calls, 1)
console.log('PASS lossless max-u64 snapshot/identity, whole Action, persisted-before-submit, discarded reply, process restoration, current-session retry, one-submit invocation bound and later identical retry, account isolation, storage refusal, no lifecycle replay')

let lostPrepares = 0
const lostData = new Map()
const lostSession = {async workflow_call(method) {if(method === 'workflow.snapshot')return snapshot;lostPrepares++;throw Error('prepare reply lost')}}
const lostCurrent = () => ({account:'prepare-loss',host:'alias',session:lostSession})
const lostStorage = {getItem:k=>lostData.get(k)??null,setItem:(k,v)=>lostData.set(k,v)}
let lostPort = new WorkflowSessionPort(lostCurrent,lostStorage,exclusive)
await lostPort.read(fixture.workspace);await assert.rejects(lostPort.dispatch(fixture.workspace,fixture.runs[0].item,fixture.runs[0].actions[0]))
lostPort = new WorkflowSessionPort(lostCurrent,lostStorage,exclusive);await lostPort.read(fixture.workspace)
await assert.rejects(lostPort.dispatch(fixture.workspace,fixture.runs[0].item,fixture.runs[0].actions[0]));assert.equal(lostPrepares,1)
console.log('PASS actual-host alias reassignment and prepare-answer loss do not reuse or reprepare an old intent')

const recovery = (await lostPort.pending(fixture.workspace)).find(e=>e.reinspect)
assert.ok(recovery)
assert.match(await lostPort.retry(fixture.workspace,recovery.id),/No action was submitted/)
assert.equal(lostPrepares,1,'reinspection reset cannot prepare or submit')
assert.equal((await lostPort.pending(fixture.workspace)).length,0)
await assert.rejects(lostPort.dispatch(fixture.workspace,fixture.runs[0].item,fixture.runs[0].actions[0]));assert.equal(lostPrepares,2,'explicit new selection after fresh inspection is recoverable')
const [corruptKey,corruptValue]=[...lostData.entries()][0]
lostData.set(corruptKey,corruptValue.replace('preparing','abandoned'))
await assert.rejects(lostPort.pending(fixture.workspace),/integrity/)
await assert.rejects(lostPort.retry(fixture.workspace,recovery.id),/integrity/)
console.log('PASS explicit preparing-only reinspection, no implicit prepare/submit, corrupt journal refuses reset')

// Feature B shares Rust's finite form/text fixture; semantic validation remains Rust-owned.
const rework = JSON.parse(fs.readFileSync('crates/client/src/testdata/workflow-rework.json','utf8'))
const b = structuredClone(fixture), action = {kind:'RequestRework',label:rework.form.submit_label,guard:'reasoned-guard'}
b.runs[0].actions = [action]; b.runs[0].forms = [rework.form]
const bdata = new Map(), bstorage = {getItem:k=>bdata.get(k)??null,setItem:(k,v)=>bdata.set(k,v)}
let bp = 0, bs = 0, validation = true, lost = true
const exact = {schema:1,workspace:b.workspace,json:JSON.stringify({action:'Review',reason:rework.reason,mutation:{id:'immutable'}})}
const sent = []
const bsession = {async workflow_call(method,raw) {
  const args = JSON.parse(raw)
  if (method === 'workflow.snapshot') return JSON.stringify({host:'b-host',presentation:b})
  if (method === 'workflow.prepare_selection') {
    bp++;assert.deepEqual(args.action,action)
    if (validation) {assert.equal(args.reason,'');return JSON.stringify({kind:'Validation',error:{code:'workflow_reason',text:'shared Rust error'}})}
    assert.equal(args.reason,rework.reason); return JSON.stringify({kind:'Mutation',prepared:exact})
  }
  assert.equal(method,'workflow.submit_action');bs++;sent.push(raw);assert.deepEqual(args.prepared,exact)
  if (lost) throw Error('accepted answer lost')
  return '{"receipt":{"result":{"Ok":9}}}'
}}
const bcurrent = () => ({account:'b-account',host:'b-alias',session:bsession})
let bport = new WorkflowSessionPort(bcurrent,bstorage,exclusive)
await bport.read(b.workspace)
assert.equal(await bport.dispatch(b.workspace,b.runs[0].item,action,''),'shared Rust error')
assert.equal(bs,0);assert.equal((await bport.pending(b.workspace)).length,0)
validation=false
await assert.rejects(bport.dispatch(b.workspace,b.runs[0].item,action,rework.reason))
assert.equal(bp,2);assert.equal(bs,1)
await assert.rejects(bport.dispatch(b.workspace,b.runs[0].item,action,'edited reason'))
assert.equal(bp,2,'editing a draft cannot replace retained prepared bytes')
bport=new WorkflowSessionPort(bcurrent,bstorage,exclusive);await bport.read(b.workspace)
lost=false;await bport.retry(b.workspace,(await bport.pending(b.workspace))[0].id)
assert.equal(bp,2);assert.equal(bs,2);assert.equal(sent[0],sent[1])
console.log('PASS B shared form, validation/no submit, exact borrowed text, edit refusal, restored immutable retry')
