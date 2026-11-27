// The join walk of csp-check.mjs (ov-1490): no native app anywhere.
//
// - a new browser's first screen is Add Runner; the one command it shows, run
//   as `farcooler join` in the real runner's terminal, adds the runner, which
//   appears without a reload with its workspaces;
// - a runner the account has but this browser can't open is listed, with the
//   step to set it up;
// - a second browser, on its own profile and passkey, is approved by the
//   first (its code pasted there), with both screens showing one code;
// - the first removes the second, which loses its connection at once and can't
//   sign in again; the runner's audit log holds each approval and removal;
// - with every enrolled browser gone, a third gets back in with the same
//   command, run at the runner's terminal.
//
// The relay is stubbed here, as far as the page and the CLI can tell; its own
// rules are services/relay/test/join.test.ts's. Everything else is real: the
// wasm, the CLI, the daemon, and an sshd running the forced command the fence
// writes.

import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer } from 'node:http'
import { randomBytes } from 'node:crypto'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const hex = () => randomBytes(32).toString('hex')
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } })
const DAY = 86_400_000

export async function startJoinStub() {
  const tickets = new Map()
  const claims = new Map()
  const replies = new Map()
  const audit = []
  // The account already has a runner this browser has never been added to.
  const runners = [{ id: 'dir-build-box', name: 'build-box', lastSeenAt: Date.now() - 3 * DAY, access: [{ user: 'acct-test' }] }]
  const put = (ceremony, sealed) => replies.set(ceremony, [{ sealed, posted_by: 'acct-test', posted_by_email: 'owner@example.com' }])

  /// What the CLI calls, over real HTTP on loopback.
  const server = createServer(async (request, response) => {
    let text = ''
    for await (const chunk of request) text += chunk
    const body = JSON.parse(text || '{}')
    const answer = (value, status = 200) => {
      response.writeHead(status, { 'content-type': 'application/json' })
      response.end(JSON.stringify(value))
    }
    const route = request.url.replace('/v1/join/', '')
    if (route === 'check') return tickets.get(body.ticket)?.claim === null ? answer({ ok: true }) : answer({ error: 'not found' }, 404)
    if (route === 'reply') {
      const ticket = tickets.get(body.ticket)
      if (!ticket || ticket.claim !== null) return answer({ error: 'not found' }, 404)
      ticket.claim = hex()
      claims.set(ticket.claim, ticket)
      put(ticket.ceremony, body.sealed)
      return answer({ claim: ticket.claim })
    }
    if (route === 'register') {
      const ticket = claims.get(body.claim)
      if (!ticket) return answer({ error: 'not found' }, 404)
      if (!ticket.approved) return answer({ error: 'waiting' }, 409)
      if (!String(body.proof).includes('BEGIN SSH SIGNATURE') || !String(body.runnerKey).startsWith('ssh-ed25519 ')) return answer({ error: 'proof' }, 403)
      claims.delete(body.claim)
      if (!runners.some(r => r.name === body.name)) runners.push({ id: `dir-${body.name}`, name: body.name, lastSeenAt: Date.now(), access: [{ user: 'acct-test' }] })
      audit.push('runner.registered')
      return answer({ runner: `dir-${body.name}`, team: 'team-personal', token: hex() })
    }
    answer({ error: 'not found' }, 404)
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')

  return {
    relay: `http://127.0.0.1:${server.address().port}`,
    audit,
    runners,
    close: () => server.close(),
    /// What the page calls through the Worker, or null for a route the shell
    /// server answers.
    async page(request) {
      const path = new URL(request.url).pathname
      if (!path.startsWith('/api/relay/')) return null
      const route = path.slice('/api/relay/'.length)
      const body = await request.json().catch(() => ({}))
      if (route === 'directory/me') return json({ user: { id: 'acct-test', email: 'owner@example.com' }, teams: [{ id: 'team-personal', personal: true, role: 'admin' }] })
      if (route === 'directory/teams/runners') return json({ runners })
      if (route === 'join/ticket') {
        const ticket = hex()
        tickets.set(ticket, { ceremony: body.ceremony, claim: null, approved: false })
        return json({ ticket, team: 'team-personal', expiresAt: Date.now() + 120_000 })
      }
      if (route === 'join/approve') {
        const ticket = [...claims.values()].find(t => t.ceremony === body.ceremony)
        if (!ticket) return json({ error: 'not found' }, 404)
        ticket.approved = true
        audit.push('runner.join_approved')
        return json({ ok: true })
      }
      if (route === 'ceremony/reply') {
        const waiting = replies.get(body.ceremony)
        replies.delete(body.ceremony)
        return waiting ? json({ replies: waiting }) : json({ error: 'not found' }, 404)
      }
      if (route === 'ceremony/reply/post') {
        put(body.ceremony, body.sealed)
        return json({ ok: true })
      }
      return null
    },
  }
}

const assert = (ok, message) => {
  if (!ok) throw new Error(message)
}
const wait = { timeout: 60_000 }
const code = async (page, id = '#compare-code') => (await page.textContent(id)).replace(/\s/g, '')

export async function joinWalk({ session, startRunner, stub, cli, me, shoot, origin, statusSays, makePasskey, fresh, device }) {
  const runner = startRunner()
  const env = { ...process.env, FARCOOLER_HOME: runner.runtime, USER: me }
  const keys = join(runner.dir, '.ssh', 'authorized_keys')
  const lines = () => (existsSync(keys) ? readFileSync(keys, 'utf8').split('\n').filter(l => l.includes('--client ')) : [])
  const shot = (page, name) => shoot(page, `join-${name}`, 320)
  /// The page's width never grows past the window's.
  const fits = async (page, name) => {
    for (const width of [320, 1280]) {
      await page.setViewportSize({ width, height: 844 })
      await page.waitForTimeout(150)
      const over = await page.evaluate(() => [document.documentElement.scrollWidth, ...[...document.querySelectorAll('dialog[open]')].map(d => d.scrollWidth - d.clientWidth)])
      assert(over[0] <= width && over.slice(1).every(n => n <= 0), `${name} is wider than ${width}px: ${over}`)
    }
    await page.setViewportSize({ width: 1280, height: 800 })
  }

  /// Add Runner on `page`: its command, run in the runner's terminal, then the
  /// code both show.
  async function addByCommand(page, label, capture) {
    await page.waitForSelector('#runner-add:not([hidden])', wait)
    await page.locator('#join-command').filter({ hasText: / join / }).waitFor(wait)
    const command = (await page.textContent('#join-command')).trim()
    const [name, verb, joinCode, ...rest] = command.split(' ')
    assert(/^farcooler(-canary)?$/.test(name) && verb === 'join' && joinCode && rest.length === 0, `Add Runner shows ${command.slice(0, 60)}`)
    if (capture) {
      await fits(page, 'Add Runner')
      await shot(page, capture)
    }
    const run = spawn(cli, [verb, joinCode, '--relay', stub.relay, '--host-key', join(runner.dir, 'host.pub'), '--label', label, '--user', me], { env, stdio: ['pipe', 'pipe', 'pipe'] })
    let said = ''
    run.stderr.on('data', d => (said += d))
    run.stdout.on('data', d => (said += d))
    const exited = once(run, 'exit')
    run.stdin.end('yes\n')
    try {
      await page.waitForSelector('#compare:not([hidden])', wait).catch(e => {
        throw new Error(`no code on screen; farcooler join ${run.exitCode === null ? 'is still running' : `ended ${run.exitCode}`}: ${said.replaceAll('\n', ' ').slice(-600)}; page note: ${e.message.split('\n')[0]}`)
      })
      const fingerprint = /The browser's passkey: (SHA256:\S+)/.exec(said)?.[1]
      assert(fingerprint && (await page.textContent('#join-fingerprint')).includes(fingerprint), `the terminal and the page show one passkey (${fingerprint})`)
      const shown = /Check that it's (\S+),/.exec(said)?.[1]
      assert(shown && (await code(page)) === shown, `the terminal says ${shown}, the page ${await code(page)}`)
      assert((await page.textContent('#compare-title')).includes('terminal'), 'the compare screen names the terminal')
      if (capture) await shot(page, `${capture}-compare`)
      // The relay has recorded nothing new, over more than one of the
      // command's polls: an earlier join's lines are already there.
      const before = stub.audit.length
      await new Promise(r => setTimeout(r, 4000))
      assert(run.exitCode === null && stub.audit.length === before, `the command waits for the approval: exit ${run.exitCode}, the relay saw ${stub.audit.slice(before)}`)
      await page.click('#compare-yes')
      const [status] = await exited
      assert(status === 0 && /Added\./.test(said), `farcooler join ended ${status}: ${said.replaceAll('\n', ' ').slice(-400)}`)
      assert(stub.audit.slice(before).join() === 'runner.join_approved,runner.registered', `the relay saw ${stub.audit.slice(before)}`)
    } finally {
      if (run.exitCode === null) run.kill('SIGKILL')
    }
  }

  async function open(page, label, capture) {
    await statusSays(page, `Connect to ${label}`)
    await page.click('#status-action')
    await page.waitForSelector('#work:not([hidden]) .terminal-choice', wait)
    await page.locator('#runner-health').filter({ hasText: /^Connected$/ }).waitFor(wait)
    await page.locator('#space-picker option').first().waitFor({ state: 'attached', ...wait })
    if (capture) await shot(page, capture)
  }

  // 1. A new account's first browser: Add Runner is the first screen.
  const a = session.page
  await a.goto(origin)
  await makePasskey(a, false)
  assert((await device(a)).runners === undefined, 'a new browser has no runners')
  await a.evaluate(() => (window.__sameDocument = true))
  await a.locator('#unenrolled:not([hidden])').filter({ hasText: 'build-box' }).waitFor(wait)
  await addByCommand(a, 'studio', 'first')
  assert(lines().length === 1, `the runner has ${lines().length} device lines`)
  await open(a, 'studio', 'runner-added')
  assert(await a.evaluate(() => window.__sameDocument === true), 'the runner appeared without a reload')
  assert((await a.locator('#space-picker option').allTextContents()).length > 0, 'its workspaces are listed')
  console.log('join: a new browser’s first screen was Add Runner, and farcooler join added the runner and its workspaces with no reload')

  // 2. The account's other runner is listed here, with the step to add it.
  await a.locator('#side-unenrolled:not([hidden])').filter({ hasText: 'build-box' }).waitFor(wait)
  assert(!(await a.textContent('#side-unenrolled')).includes('studio'), 'a runner this browser opens isn’t listed')
  await fits(a, 'the work screen')
  await shot(a, 'unenrolled-sidebar')
  await a.getByRole('button', { name: 'Set Up build-box' }).click()
  await a.locator('#unenrolled:not([hidden])').filter({ hasText: 'build-box' }).waitFor(wait)
  await a.locator('#join-command').filter({ hasText: / join / }).waitFor(wait)
  assert((await a.textContent('#unenrolled')).includes('run the command below'), 'the step to set it up is said')
  await fits(a, 'the not-set-up list')
  await shot(a, 'unenrolled-add-runner')
  await a.click('#join-back')
  await open(a, 'studio')
  console.log('join: a runner the account has and this browser can’t open was listed, with the step to set it up')

  // 3. A second browser, approved by the first with no app.
  const second = await fresh()
  const b = second.page
  await b.goto(origin)
  await makePasskey(b)
  const offer = await b.locator('#cli-offer').inputValue()
  await fits(b, 'Approve From Another Device')
  await shot(b, 'approve-new-device')
  const bPasskey = (await b.textContent('#offer-fingerprint')).split(': ')[1]

  await a.click('#show-devices')
  const sheet = a.locator('#devices')
  await sheet.locator('#devices-list li').first().waitFor(wait)
  await fits(a, 'Devices')
  await shot(a, 'devices')
  // This browser's own code is refused, before any passkey prompt.
  await sheet.locator('#device-code').fill('{"not":"a code"}')
  await sheet.locator('#device-check').click()
  await sheet.locator('#device-result').filter({ hasText: /couldn’t be read|newer Far Cooler/ }).waitFor(wait)
  await sheet.locator('#device-code').fill(offer)
  await sheet.locator('#device-check').click()
  await sheet.locator('#device-confirm:not([hidden])').waitFor(wait)
  assert((await sheet.locator('#device-fingerprint').textContent()) === bPasskey, 'the approver shows the new browser’s passkey')
  await fits(a, 'the approval')
  await shot(a, 'approve-confirm')
  assert(lines().length === 1, 'nothing was added before Approve')
  await sheet.locator('#device-approve').click()
  await sheet.locator('#device-compare:not([hidden])').waitFor(wait)
  await b.waitForSelector('#compare:not([hidden])', wait)
  const [onA, onB] = [await code(a, '#device-compare'), await code(b)]
  assert(onA === onB, `the two screens show ${onA} and ${onB}`)
  await fits(b, 'the new browser’s code')
  await shot(b, 'approve-new-device-compare')
  await shot(a, 'approve-code')
  assert(lines().length === 2, 'the runner holds the new browser’s line')
  await b.click('#compare-yes')
  await open(b, 'studio', 'approved-browser-open')
  console.log('join: a second browser was approved from the first, both showed one code, and it opened the runner')

  // 4. Removed, it loses access at once.
  await sheet.locator('#devices-close').click()
  await a.click('#show-devices')
  await sheet.locator('#devices-list li').nth(1).waitFor(wait)
  await sheet.getByRole('button', { name: /^Remove / }).click()
  await fits(a, 'Remove')
  await shot(a, 'remove-confirm')
  const asked = Date.now()
  await sheet.getByRole('button', { name: /^Confirm Remove / }).click()
  await sheet.locator('#device-result').filter({ hasText: 'can no longer open studio' }).waitFor(wait)
  await b.locator('#runner-health').filter({ hasNotText: /^Connected$/ }).waitFor({ timeout: 15_000 })
  console.log(`join: the removed browser lost its connection ${Date.now() - asked} ms after Confirm Remove`)
  await shot(a, 'removed')
  assert(lines().length === 1, 'the removed browser’s line is gone')
  const reconnect = b.locator('#link-action:not([hidden])')
  await reconnect.waitFor(wait)
  await reconnect.click()
  await b.locator('#link-detail, #status-detail').filter({ hasText: /rejected|didn’t accept|removed|passkey|key/i }).first().waitFor(wait)
  assert((await b.textContent('#runner-health')) !== 'Connected', 'the removed browser can’t sign in again')
  await shot(b, 'removed-browser')

  // The runner's audit log holds each approval and removal.
  const events = () => execFileSync('sqlite3', [join(runner.runtime, 'farcooler.db'), "SELECT kind || ' ' || detail FROM people_events WHERE kind LIKE 'device_%' ORDER BY id"], { encoding: 'utf8' }).trim().split('\n')
  const logged = events()
  assert(logged.map(l => l.split(' ')[0]).join() === 'device_added,device_added,device_removed', `the audit log holds ${logged}`)
  console.log('join: the runner’s audit log holds both approvals and the removal')

  // 5. No enrolled browser left: back in from the runner's own terminal.
  await session.context.close()
  await second.context.close()
  const left = lines().map(l => /--client (\S+)/.exec(l)[1])
  for (const id of left) execFileSync(cli, ['client', 'revoke', id], { env })
  assert(lines().length === 0, 'every enrolled browser is gone')
  const third = await fresh()
  const c = third.page
  await c.goto(origin)
  await makePasskey(c, false)
  await addByCommand(c, 'studio', 'recovery')
  await open(c, 'studio', 'recovered')
  assert(lines().length === 1, 'the recovering browser has the only line')
  console.log('join: with every enrolled browser gone, farcooler join at the runner’s terminal let a new one back in')
}
