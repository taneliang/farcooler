// Real cached Chromium over an isolated localhost fixture; no application,
// authentication, display, downloads, or owner data are involved.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import http from 'node:http'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
const out = process.argv[2]
const modules = Object.fromEntries(['workflow', 'workflow-view', 'workflow-session'].map(name => [name,
  fs.readFileSync(path.join(out, name + '.js'), 'utf8')]))
const ready = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'))
const literal = value => JSON.stringify(value).replaceAll('<', '\\u003c')
const html = `<!doctype html><meta charset="utf-8"><style>body{font:16px system-ui;max-width:1100px;margin:2rem auto}dialog{width:min(90vw,1000px)}button{font:inherit;padding:.6rem;margin:.3rem}article{border-top:1px solid #aaa;padding:1rem}</style>
<h1>Private FeatureA workspace</h1><button id="workspace">Workflow Runs</button><dialog id="dialog"></dialog><script>
const sources = ${literal(modules)}, cache = {};
function require(name) { name = name.replace('./', ''); if (!cache[name]) { const exports = {}; cache[name] = exports; new Function('exports', 'require', sources[name])(exports, require) }; return cache[name] }
const { openWorkflow, installWorkflowPort } = require('workflow-view'); const { WorkflowSessionPort } = require('workflow-session');
window.workspaceID = ${literal(ready.workspace)}; window.dropReply = !localStorage.getItem('discarded-once'); window.sent = JSON.parse(localStorage.getItem('sent') ?? '[]'); window.prepares = Number(localStorage.getItem('prepares') ?? '0');
window.current = { account: 'private-fixture-account', host: 'private-socket-host', session: { async workflow_call(method, raw) {
 if (method === 'workflow.prepare_selection') {prepares++;localStorage.setItem('prepares',String(prepares))};
 const response = await fetch('http://127.0.0.1:${ready.port}/call', { method:'POST', body: JSON.stringify({method,args:JSON.parse(raw)}) });
 const result = await response.text();
 if (method === 'workflow.submit_action') { sent.push(raw);localStorage.setItem('sent',JSON.stringify(sent)); if (dropReply) { dropReply=false;localStorage.setItem('discarded-once','yes'); throw Error('discarded reply after acceptance') } }
 return result;
}}};
window.port = new WorkflowSessionPort(() => current, localStorage);
installWorkflowPort(() => port);
workspace.onclick = () => { window.sheet = openWorkflow(dialog, 'private-socket-host', workspaceID); window.started = true };
window.focusText = text => [...dialog.querySelectorAll('button')].find(b => b.textContent.includes(text)).focus();
window.key = () => document.activeElement?.getAttribute('data-workflow-control');
</script>`
const server = http.createServer((req, res) => { res.setHeader('Content-Type', 'text/html'); res.end(html) })
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'ov930-session-'))
let browser; let socket
try {
  server.listen(0, '127.0.0.1'); await once(server, 'listening')
  const executable = '/Users/e-liang/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell'
  browser = spawn(executable, ['--headless', '--disable-gpu', '--no-first-run', '--disable-background-networking', '--disable-component-update', '--no-proxy-server', '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1', '--remote-debugging-port=0', '--user-data-dir=' + profile, 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] })
  let stderr = ''
  const endpoint = await new Promise((resolve, reject) => {
    browser.on('error', reject); browser.on('exit', code => reject(Error('browser exit ' + code + stderr)))
    browser.stderr.on('data', chunk => { stderr += chunk; const match = stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/); if (match) resolve(match[1]) })
  })
  socket = new WebSocket(endpoint); await once(socket, 'open')
  let serial = 0; const pending = new Map()
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data); if (!message.id) return
    const pair = pending.get(message.id); pending.delete(message.id)
    if (message.error) pair.reject(Error(JSON.stringify(message.error))); else pair.resolve(message.result)
  })
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++serial; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
  })
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' })
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true })
  const call = (method, params) => send(method, params, sessionId)
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails))
    return result.result.value
  }
  await call('Page.enable')
  await call('Emulation.setDeviceMetricsOverride', {width:1100,height:1600,deviceScaleFactor:1,mobile:false})
  await call('Page.navigate', { url: `http://127.0.0.1:${server.address().port}/` })
  for (let i = 0; i < 100; ++i) {
    if (await evaluate('Boolean(window.current)')) break
    await new Promise(r => setTimeout(r, 10))
  }
  const enter = async () => {
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' })
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 })
  }
  const waitFor = async condition => {
    for (let i=0;i<300;i++) { if(await evaluate(condition)) return; await new Promise(r=>setTimeout(r,20)) }
    throw Error('timeout '+condition+' body='+await evaluate('document.body.innerText'))
  }
  await evaluate('workspace.focus()'); await enter();
  await waitFor(`dialog.innerText.includes('FeatureA')`)
  await evaluate(`focusText('FeatureA')`); await enter();
  await waitFor(`[...dialog.querySelectorAll('button')].some(b => b.getAttribute('data-workflow-control')?.includes('Approve'))`)
  const approval = await evaluate(`(() => { const buttons=[...dialog.querySelectorAll('button')]; const b=buttons.find(b=>b.getAttribute('data-workflow-control')?.includes('Approve')); b.focus(); return key() })()`)
  assert.ok(approval)
  await enter()
  await waitFor(`dialog.innerText.includes('Retry Retained Action') && sent.length === 1`)
  assert.equal(await evaluate('prepares'),1)
  // Close/reopen reconstructs renderer; only the journal restores the intent.
  await evaluate('sheet.close(); port = new (require("workflow-session").WorkflowSessionPort)(() => current, localStorage); workspace.focus()')
  await enter(); await waitFor(`dialog.innerText.includes('Retry Retained Action')`)
  assert.equal(await evaluate('sent.length'),1)
  await call('Page.reload')
  await waitFor('Boolean(window.current)')
  await evaluate('workspace.focus()'); await enter()
  await waitFor(`dialog.innerText.includes('Retry Retained Action')`)
  assert.equal(await evaluate('sent.length'),1, 'page bootstrap restoration cannot submit')
  assert.equal(await evaluate('prepares'),1, 'page bootstrap restoration cannot prepare')
  await evaluate(`focusText('Retry Retained Action')`); await enter()
  await waitFor('sent.length === 2')
  await waitFor(`!dialog.innerText.includes('Retry Retained Action')`)
  assert.equal(await evaluate('prepares'),1)
  assert.equal(await evaluate('sent[0] === sent[1]'),true)
  // Explicit refresh reads until automatic fixture stages finish, no replay.
  for(let i=0;i<100;i++) {
    await evaluate(`focusText('Refresh')`); await enter()
    if(await evaluate(`dialog.innerText.includes('Completed')`)) break
    await new Promise(r=>setTimeout(r,30))
  }
  await waitFor(`dialog.innerText.includes('Completed')`)
  console.log('PASS real workspace Enter -> FeatureA -> opaque Approve -> discarded reply -> close/reopen + page reload journal restoration -> explicit retained retry identical bytes -> Completed; prepare once, submit twice')
  const capture = await call('Page.captureScreenshot', {format:'png'})
  fs.writeFileSync('reports/ov-930/browser-feature-a.png', Buffer.from(capture.data,'base64'))
  // Exercise max-u64 in this actual browser's decoder and serialization route.
  const lossless = await evaluate(`(async () => {
    const {WorkflowSessionPort}=require('workflow-session');
    let args;
    const snapshot=await current.session.workflow_call('workflow.snapshot',JSON.stringify({workspace:workspaceID}));
    const maximum=snapshot.replace(/"revision":\\d+/g,'"revision":18446744073709551614').replace(/"epoch":\\d+/g,'"epoch":18446744073709551615');
    const fake={workflow_call:async(method,raw)=> {if(method==='workflow.snapshot')return maximum;args=JSON.parse(raw);return await current.session.workflow_call(method,raw)}};
    const storage=new Map();const p=new WorkflowSessionPort(()=>({account:'lossless',host:'fixture',session:fake}),{getItem:k=>storage.get(k)??null,setItem:(k,v)=>storage.set(k,v)});
    const view=await p.read(workspaceID);try {await p.dispatch(workspaceID,view.runs[0].item,view.runs[0].actions[0]);throw Error('expected stale refusal')}catch(e){if(e.message==='expected stale refusal')throw e};
    return {revision:args.revision,epoch:args.epoch};
  })()`)
  assert.deepEqual(lossless,{revision:'18446744073709551614',epoch:'18446744073709551615'})
  console.log('PASS actual browser max-u64 snapshot decoder -> decimal prepare identity, no Number rounding')
  console.log(await evaluate('navigator.userAgent'))
} finally {
  socket?.close()
  if (browser && browser.exitCode === null) { browser.kill('SIGTERM'); await once(browser, 'exit') }
  await new Promise(resolve => server.close(resolve))
  fs.rmSync(profile, { recursive: true, force: true })
}
