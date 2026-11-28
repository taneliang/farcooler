// ov-1534 end to end: the relay's WebRTC signaling between the ov-1502 spike's
// Rust runner and its browser page.
//
// - The relay is the real worker, bundled by `wrangler deploy --dry-run`, run
//   in Miniflare (workerd) with a local D1 migrated by the real migrations and
//   the real `Signal` Durable Object.
// - Miniflare's outbound fetch stands in for WorkOS (a JWKS for a key made
//   here) and for Cloudflare's TURN credential endpoint (minting credentials
//   for the spike's local TURN server), so the run takes the minting path.
// - The runner is `dcspike relay`, on the relay's WebSocket with a daemon token.
// - The browser is the spike page in Chromium, on `?relay=`: open, send, poll.
//
// usage (from the repository root):
//   node reports/ov-1534/e2e.mjs <bundle-dir> <dcspike> <playwright-core> [policy] [ssh-target]
// with the bundle from `npx wrangler deploy --dry-run --outdir <bundle-dir>`
// in services/relay, and the local TURN server (reports/ov-1502/spike/turnsrv)
// running when `policy` is `relay`.

import { spawn } from 'node:child_process'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const { Miniflare } = require(path.join(root, 'services/relay/node_modules/miniflare'))

const [bundle, dcspike, playwrightCore, policy = 'relay', sshTarget = '127.0.0.1:22'] = process.argv.slice(2)
const pw = require(playwrightCore)

const CLIENT_ID = 'client_e2e'
const ISSUER = `https://api.workos.com/user_management/${CLIENT_ID}`
const TURN_KEY_ID = 'e2e-key'
const TURN_TOKEN = 'e2e-account-token-not-a-real-one'

// WorkOS, faked: a key made here, published as the JWKS.
const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 })
const jwk = { ...publicKey.export({ format: 'jwk' }), kid: 'e2e', alg: 'RS256', use: 'sig' }
const b64u = b => Buffer.from(b).toString('base64url')
function jwt(sub) {
  const now = Math.floor(Date.now() / 1000)
  const head = b64u(JSON.stringify({ alg: 'RS256', kid: 'e2e' }))
  const body = b64u(JSON.stringify({ sub, iss: ISSUER, client_id: CLIENT_ID, sid: `sid-${sub}`, iat: now, exp: now + 3600 }))
  return `${head}.${body}.${b64u(sign('sha256', Buffer.from(`${head}.${body}`), privateKey))}`
}

// The bundle's entry re-exported with only its handler and its Durable Object:
// standalone workerd refuses an entry module that also exports constants
// (index.ts exports several for its tests), main's bundle included.
const entry = path.join(bundle, 'e2e-entry.mjs')
writeFileSync(entry, "import worker from './index.js'\nexport { Signal } from './index.js'\nexport default worker\n")

const minted = []
const mf = new Miniflare({
  modulesRoot: bundle,
  modules: [
    { type: 'ESModule', path: entry },
    { type: 'ESModule', path: path.join(bundle, 'index.js') },
  ],
  compatibilityDate: '2025-09-06',
  host: '127.0.0.1',
  port: 0,
  d1Databases: ['DB'],
  durableObjects: { SIGNAL: { className: 'Signal', useSQLite: true } },
  analyticsEngineDatasets: { METRICS: { dataset: 'e2e' } },
  bindings: {
    WORKOS_CLIENT_ID: CLIENT_ID,
    WORKOS_ISSUER: ISSUER,
    WORKOS_API_KEY: 'unused',
    APNS_KEY_P8: '', APNS_KEY_ID: '', APNS_TEAM_ID: '', APNS_TOPIC: '', FCM_SERVICE_ACCOUNT: '{}',
    CHANNEL: 'local',
    ...(policy === 'relay' ? { TURN_KEY_ID, TURN_KEY_API_TOKEN: TURN_TOKEN } : {}),
  },
  outboundService: async request => {
    const url = request.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [jwk] }))
    if (url.startsWith('https://rtc.live.cloudflare.com/')) {
      minted.push({ url, authorized: request.headers.get('authorization') === `Bearer ${TURN_TOKEN}`, body: await request.json() })
      // The spike's TURN server, as Cloudflare would answer for its own.
      return new Response(JSON.stringify({ iceServers: { urls: ['turn:127.0.0.1:3478?transport=udp', 'turn:127.0.0.1:3478?transport=tcp'], username: 'spike', credential: 'spike' } }))
    }
    return new Response('not in this test', { status: 502 })
  },
})
const base = await mf.ready
const db = await mf.getD1Database('DB')

// The schema, from the real migrations, in order.
const migrations = path.join(root, 'services/relay/migrations')
for (const file of readdirSync(migrations).filter(f => f.endsWith('.sql')).sort()) {
  const sql = readFileSync(path.join(migrations, file), 'utf8').replace(/^\s*--.*$/gm, '')
  for (const statement of sql.split(/;\s*$/m).map(s => s.trim()).filter(Boolean)) await db.prepare(statement).run()
}

// The directory: alice, her team, her runner with its token, her browser's
// device, and that device's key on the runner's reported authorized_keys.
const sha = t => createHash('sha256').update(t).digest('hex')
const now = Date.now()
const token = 'runner-token-e2e'
const fingerprint = `SHA256:${createHash('sha256').update('alice-browser').digest('base64').replace(/=+$/, '')}`
const seed = [
  [`INSERT INTO accounts (id, created_at) VALUES ('alice', ?)`, now],
  [`INSERT INTO teams (id, name, personal_of, created_by, created_at) VALUES ('team-a', 'Personal', 'alice', 'alice', ?)`, now],
  [`INSERT INTO memberships (team_id, account_id, role, joined_at) VALUES ('team-a', 'alice', 'admin', ?)`, now],
  [`INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES ('daemon-a', 'alice', ?, 'Studio', ?)`, sha(token), now],
  [`INSERT INTO runners (id, team_id, runner_key, runner_key_fingerprint, install_key, name, daemon_id, created_at)
    VALUES ('runner-a', 'team-a', 'ssh-ed25519 AAAA', 'SHA256:runner', 'install', 'Studio', 'daemon-a', ?)`, now],
  [`INSERT INTO runner_roles (runner_id, account_id, role, granted_at) VALUES ('runner-a', 'alice', 'owner', ?)`, now],
  [`INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, state, created_at) VALUES ('device-a', 'alice', 'Chromium', 'ecdsa', ?, 'trusted', ?)`, fingerprint, now],
  [`INSERT INTO device_fingerprints (fingerprint, device_id) VALUES (?, 'device-a')`, fingerprint],
  [`INSERT INTO runner_inventory (runner_id, hash, total, entries, baseline_at, reported_at) VALUES ('runner-a', ?, 1, ?, ?, ?)`,
    'f'.repeat(64), JSON.stringify([{ fingerprint, comment: '', inside: true, scope: 'shell', file: 'authorized_keys', options: '', firstSeenAt: now }]), now, now],
]
for (const [sql, ...binds] of seed) await db.prepare(sql).bind(...binds).run()

// The page, and the relay's routes beside it on one origin.
const page = readFileSync(path.join(root, 'reports/ov-1502/spike/src/page.html'), 'utf8')
const front = http.createServer(async (req, res) => {
  if (!req.url.startsWith('/v1/')) {
    res.writeHead(200, { 'content-type': 'text/html' })
    return res.end(page)
  }
  const chunks = []
  for await (const c of req) chunks.push(c)
  try {
    const reply = await mf.dispatchFetch(new URL(req.url, base), { method: req.method, headers: req.headers, body: chunks.length ? Buffer.concat(chunks) : undefined })
    res.writeHead(reply.status, { 'content-type': 'application/json' })
    res.end(Buffer.from(await reply.arrayBuffer()))
  } catch {
    // The page's last long-poll, cut off when the relay stops at the end.
    res.destroy()
  }
})
await new Promise(r => front.listen(0, '127.0.0.1', r))
const frontUrl = `http://127.0.0.1:${front.address().port}`

// The runner, on the relay's WebSocket.
const wsUrl = `${base.href.replace(/^http/, 'ws')}v1/rtc/runner`
const runner = spawn(dcspike, ['relay', wsUrl, token, sshTarget], { env: { ...process.env, POLICY: policy === 'relay' ? 'relay' : '' }, stdio: ['ignore', 'inherit', 'pipe'] })
const runnerLog = []
runner.stderr.on('data', d => { runnerLog.push(String(d)); process.stderr.write(`  runner | ${d}`) })
await new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error('runner never connected')), 15000)
  runner.stderr.on('data', d => String(d).includes('connected') && (clearTimeout(timer), resolve()))
})

// A stranger is refused before the runner hears anything.
const stranger = await (await fetch(`${frontUrl}/v1/rtc/open`, { method: 'POST', headers: { authorization: `Bearer ${jwt('mallory')}` }, body: JSON.stringify({ runner: 'runner-a', device: 'device-a' }) })).json()
console.log('stranger open:', JSON.stringify(stranger))

const query = new URLSearchParams({ relay: '', token: jwt('alice'), runner: 'runner-a', device: 'device-a', n: '100', ssh: '1', ...(policy === 'relay' ? { policy: 'relay' } : {}) })
const browser = await pw.chromium.launch({ args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] })
let result
try {
  const tab = await browser.newPage()
  tab.on('console', m => m.type() === 'error' && console.error('  page |', m.text()))
  await tab.goto(`${frontUrl}/?${query}`)
  await tab.waitForFunction(() => window.result, null, { timeout: 60000 })
  result = await tab.evaluate(() => window.result)
} finally {
  await browser.close()
  runner.kill()
  front.close()
  await mf.dispose()
}
console.log('result:', JSON.stringify(result, null, 2))
console.log('TURN mints:', JSON.stringify(minted.map(m => ({ authorized: m.authorized, ttl: m.body.ttl }))))
const heard = runnerLog.join('')
console.log('runner heard ice_servers:', /ice_servers/.test(heard), '| offer with device:', /offer from device .*device-a/.test(heard), '| remote candidates:', (heard.match(/remote candidate/g) || []).length, '| trickled:', (heard.match(/trickle /g) || []).length)
console.log('account token in runner output:', heard.includes(TURN_TOKEN))
process.exit(result && !result.error ? 0 : 1)
