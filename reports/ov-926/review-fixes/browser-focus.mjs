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
const modules = Object.fromEntries(['workflow', 'workflow-view'].map(name => [name,
  fs.readFileSync(path.join(out, name + '.js'), 'utf8')]))
const fixture = JSON.parse(fs.readFileSync('crates/client/src/testdata/workflow-presentation.json'))[0].response.card
const literal = value => JSON.stringify(value).replaceAll('<', '\\u003c')
const html = `<!doctype html><button id="outside">Outside</button><dialog id="dialog"><div id="root"></div><button id="close">Close</button></dialog><script>
const sources = ${literal(modules)}, cache = {};
function require(name) { name = name.replace('./', ''); if (!cache[name]) { const exports = {}; cache[name] = exports; new Function('exports', 'require', sources[name])(exports, require) }; return cache[name] }
const { WorkflowBoard } = require('workflow'); const { renderWorkflow } = require('workflow-view');
window.p = ${literal(fixture)};
p.runs[0].actions[0].label = 'Refresh'; // Deliberate copy collision with toolbar.
window.failRead = false; window.resolveAction = null;
window.port = { read: async () => { if (failRead) throw Error('read failed'); return p }, dispatch: () => new Promise(r => { window.resolveAction = r }) };
window.board = new WorkflowBoard(p.workspace, () => port, v => { window.view = v; renderWorkflow(document.getElementById('root'), v, board) });
dialog.showModal(); window.started = board.refresh();
window.key = () => document.activeElement?.getAttribute('data-workflow-control');
window.focusControl = prefix => [...root.querySelectorAll('button')].find(b => b.getAttribute('data-workflow-control').startsWith(prefix)).focus();
</script>`
const server = http.createServer((req, res) => { res.setHeader('Content-Type', 'text/html'); res.end(html) })
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'ov926-focus-'))
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
  await call('Page.navigate', { url: `http://127.0.0.1:${server.address().port}/` })
  for (let i = 0; i < 100; ++i) {
    if (await evaluate('Boolean(window.started)')) break
    await new Promise(r => setTimeout(r, 10))
  }
  await evaluate('started')
  const enter = async () => {
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' })
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 })
  }
  await evaluate(`focusControl('["run"')`); await enter()
  assert.equal(await evaluate('key()'), 'all-runs'); console.log('PASS real Enter run entry -> All Runs')
  await evaluate(`focusControl('["action"')`); const actionKey = await evaluate('key()'); await enter()
  assert.equal(await evaluate('view.busy'), true); assert.equal(await evaluate('key()'), 'all-runs')
  await evaluate(`resolveAction('evidence'); Promise.resolve()`)
  assert.equal(await evaluate('key()'), actionKey); assert.equal(await evaluate('view.busy'), false)
  console.log('PASS real Enter action -> busy fallback -> action restored')
  await evaluate(`p = structuredClone(p); p.runs[0].actions[0].label = 'Changed copy'; board.refresh()`)
  assert.equal(await evaluate('key()'), actionKey); console.log('PASS stable focus through shared copy change and colliding label')
  await evaluate('failRead = true; board.refresh()'); assert.equal(await evaluate('key()'), 'all-runs')
  await evaluate('failRead = false; board.refresh()'); assert.equal(await evaluate('key()'), actionKey)
  console.log('PASS failed read fallback and recovery')
  await evaluate('p = structuredClone(p); p.runs[0].actions = []; board.refresh()')
  assert.equal(await evaluate('key()'), 'all-runs'); console.log('PASS removed action -> All Runs')
  await evaluate('document.getElementById("close").focus(); board.refresh()')
  assert.equal(await evaluate('document.activeElement.id'), 'close'); console.log('PASS redraw does not steal focus from dialog Close')
  console.log(await evaluate('navigator.userAgent'))
} finally {
  socket?.close()
  if (browser && browser.exitCode === null) { browser.kill('SIGTERM'); await once(browser, 'exit') }
  await new Promise(resolve => server.close(resolve))
  fs.rmSync(profile, { recursive: true, force: true })
}
