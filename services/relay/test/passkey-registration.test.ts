import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { CERTIFY_NAMESPACE, certifyStatement, REGISTER_NAMESPACE, registerStatement } from '../src/directory/devices'
import { parseDeviceKey, parsePublicKey, verifyDeviceSignature, verifySignature } from '../src/sshsig'
import wranglerToml from '../wrangler.toml?raw'
import fixture from './fixtures/passkey-proof.json'
import { makeKey, makePasskey, type PasskeyTweak, type TestKey, type TestPasskey } from './sshsig-signer'

// A passkey as a device's Key A (ov-766, ov-752 slice 3): a browser's WebAuthn
// credential, presented as `sk-ecdsa-sha2-nistp256@openssh.com`, registering
// and being certified in the directory. Against the real migration chain in
// workerd's D1, through the routes.
//
// `fixtures/passkey-proof.json` was signed by Chromium's virtual
// authenticator (`scripts/passkey-fixture.mjs`), and OpenSSH 10.3's
// `ssh-keygen -Y verify` accepts both of its proofs, the parent-domain one
// included: only the RP ID rule refuses that one. The software passkey in
// `sshsig-signer.ts` signs what a browser never would, to show each check
// refusing it.

const RP_ID = 'app.canary.farcooler.com'

const signing = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const publicJwk = { ...(await crypto.subtle.exportKey('jwk', signing.publicKey)), kid: 'test-key' }

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

async function sessionFor(user: string): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: 's', iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

beforeEach(() => {
  vi.stubGlobal('fetch', async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    return new Response('{}')
  })
})

/// Canary's relay: the one channel with a web app so far.
const canary: any = { ...env, DEVICE_KEYS_ENFORCED: '1', PASSKEY_RP_ID: RP_ID }

async function as(user: string, route: string, body: unknown = {}, routeEnv: any = canary): Promise<{ status: number; body: any }> {
  const response = await worker.fetch(
    new Request(`https://relay.test/v1/directory/${route}`, {
      method: 'POST',
      headers: { authorization: `Bearer ${await sessionFor(user)}` },
      body: JSON.stringify(body),
    }),
    routeEnv as never,
    { waitUntil() {}, passThroughOnException() {} } as never,
  )
  return { status: response.status, body: await response.json() }
}

async function fingerprint(key: TestKey | TestPasskey): Promise<string> {
  return (await parseDeviceKey(key.line, RP_ID))!.fingerprint
}

/// The body a device sends to register `key` as `user`, with its proof made
/// with `tweak`.
async function proven(user: string, key: TestKey | TestPasskey, tweak?: PasskeyTweak) {
  const challenge = (await as(user, 'devices/challenge')).body.challenge
  const statement = registerStatement(user, challenge, await fingerprint(key), null, null)
  return { label: 'Browser', challenge, keyA: key.line, proofA: await key.sign(REGISTER_NAMESPACE, statement, 'sha512', tweak as never) }
}

/// A fixture proof's body, its challenge minted as the relay would have.
async function fixtureBody(proof: typeof fixture.exact) {
  const now = Date.now()
  await as(fixture.account, 'me')
  await env.DB.prepare(`INSERT INTO device_challenges (nonce, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)`)
    .bind(fixture.challenge, fixture.account, now, now + 60_000)
    .run()
  return { label: 'Browser', challenge: fixture.challenge, keyA: proof.publicKey, proofA: proof.proof }
}

/// `armored` with one bit of the ECDSA signature's `s` flipped.
function flipS(armored: string): string {
  const b64 = armored.replace(/-----[A-Z ]+-----/g, '').replace(/\s+/g, '')
  const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0))
  const view = new DataView(bytes.buffer)
  let at = 6 + 4
  for (let i = 0; i < 4; i++) at += 4 + view.getUint32(at) // key, namespace, reserved, hash
  at += 4 // the signature blob's length
  at += 4 + view.getUint32(at) // its format name
  at += 4 // the ECDSA signature's length
  at += 4 + view.getUint32(at) // r
  const sLength = view.getUint32(at)
  bytes[at + 4 + sLength - 1] ^= 1
  return `-----BEGIN SSH SIGNATURE-----\n${btoa(String.fromCharCode(...bytes))}\n-----END SSH SIGNATURE-----\n`
}

describe('a passkey registering as Key A (ov-766)', () => {
  it("registers a proof a browser's virtual authenticator made", async () => {
    const statement = registerStatement(fixture.account, fixture.challenge, fixture.exact.fingerprint, null, null)
    expect(statement).toBe(fixture.exact.statement)
    const answer = await as(fixture.account, 'devices/register', await fixtureBody(fixture.exact))
    expect(answer).toEqual({ status: 200, body: { device: expect.any(String), state: 'trusted' } })
    const row = await env.DB.prepare(`SELECT key_a, key_a_fingerprint FROM device_keys WHERE id = ?`)
      .bind(answer.body.device)
      .first<{ key_a: string; key_a_fingerprint: string }>()
    expect(row).toEqual({ key_a: fixture.exact.publicKey, key_a_fingerprint: fixture.exact.fingerprint })
  })

  it("refuses the same browser's proof from a passkey scoped to farcooler.com", async () => {
    // A good signature (`ssh-keygen -Y verify` accepts it), made on the
    // exact host's page: the passkey's parent-domain RP ID is all that's wrong.
    expect(JSON.parse(fixture.parent.clientDataJSON).origin).toBe(`https://${RP_ID}`)
    expect(await as(fixture.account, 'devices/register', await fixtureBody(fixture.parent))).toEqual({
      status: 400,
      body: { error: 'keyA' },
    })
    expect(await verifyDeviceSignature(fixture.parent.proof, REGISTER_NAMESPACE, fixture.parent.statement, RP_ID)).toBeNull()
  })

  it('refuses every passkey on a channel with no web app', async () => {
    const stable = { ...canary, PASSKEY_RP_ID: undefined }
    const body = await fixtureBody(fixture.exact)
    expect(await as(fixture.account, 'devices/register', body, stable)).toEqual({ status: 400, body: { error: 'keyA' } })
    // Nor does a passkey's signature verify where only Ed25519 is taken.
    expect(await verifySignature(fixture.exact.proof, REGISTER_NAMESPACE, fixture.exact.statement)).toBeNull()
    expect(await parsePublicKey(fixture.exact.publicKey)).toBeNull()
  })

  it('is on for canary alone', () => {
    const vars = [...wranglerToml.matchAll(/^PASSKEY_RP_ID = "(.*)"$/gm)].map(m => m[1])
    expect(vars).toEqual([RP_ID])
    expect(wranglerToml.split('[env.canary.vars]')[1].split(/^\[/m)[0]).toContain(`PASSKEY_RP_ID = "${RP_ID}"`)
  })

  it('refuses a flipped bit with 403, and leaves the challenge unspent', async () => {
    const body = await fixtureBody(fixture.exact)
    expect(await as(fixture.account, 'devices/register', { ...body, proofA: flipS(body.proofA) })).toEqual({
      status: 403,
      body: { error: 'proofA' },
    })
    expect((await as(fixture.account, 'devices/register', body)).status).toBe(200)
  })

  it('refuses a signature made with nobody present with 403', async () => {
    const passkey = await makePasskey(RP_ID)
    expect(await as('alice', 'devices/register', await proven('alice', passkey, { flags: 0x00 }))).toEqual({
      status: 403,
      body: { error: 'proofA' },
    })
    expect(await as('alice', 'devices/register', await proven('alice', passkey, { format: 'sk', flags: 0x04 }))).toEqual({
      status: 403,
      body: { error: 'proofA' },
    })
    expect((await as('alice', 'devices/register', await proven('alice', passkey))).status).toBe(200)
  })

  it('refuses an assertion over another challenge with 403', async () => {
    const passkey = await makePasskey(RP_ID)
    const other = new TextEncoder().encode('SSHSIG and anything else')
    expect(await as('alice', 'devices/register', await proven('alice', passkey, { challenge: other }))).toEqual({
      status: 403,
      body: { error: 'proofA' },
    })
  })

  it('refuses a WebAuthn record of anything but a get, on this exact origin, in its own window', async () => {
    const passkey = await makePasskey(RP_ID)
    for (const tweak of [
      { type: 'webauthn.create' },
      { origin: 'https://farcooler.com' },
      { origin: `http://${RP_ID}` },
      { crossOrigin: true },
      { signatureOrigin: 'https://farcooler.com' },
    ]) {
      expect(await as('alice', 'devices/register', await proven('alice', passkey, tweak))).toEqual({
        status: 403,
        body: { error: 'proofA' },
      })
    }
  })

  it('refuses an authenticator asked for another RP ID', async () => {
    const passkey = await makePasskey(RP_ID)
    const answer = await as('alice', 'devices/register', await proven('alice', passkey, { application: 'farcooler.com' }))
    expect(answer).toEqual({ status: 403, body: { error: 'proofA' } })
  })

  it('refuses flags no assertion carries, and an mpint with a spare zero', async () => {
    const passkey = await makePasskey(RP_ID)
    for (const tweak of [{ flags: 0x41 }, { flags: 0x81 }, { paddedR: true }]) {
      expect(await as('alice', 'devices/register', await proven('alice', passkey, tweak))).toEqual({
        status: 403,
        body: { error: 'proofA' },
      })
    }
  })

  it("takes a security key's own signature format", async () => {
    const passkey = await makePasskey(RP_ID)
    expect((await as('alice', 'devices/register', await proven('alice', passkey, { format: 'sk' }))).body.state).toBe('trusted')
  })

  it('keeps Key B to Ed25519', async () => {
    const [mac, passkey] = await Promise.all([makeKey(), makePasskey(RP_ID)])
    const challenge = (await as('alice', 'devices/challenge')).body.challenge
    const statement = registerStatement('alice', challenge, await fingerprint(mac), await fingerprint(passkey), null)
    const body = {
      label: 'Mac',
      challenge,
      keyA: mac.line,
      proofA: await mac.sign(REGISTER_NAMESPACE, statement),
      keyB: passkey.line,
      proofB: await passkey.sign(REGISTER_NAMESPACE, statement),
    }
    expect(await as('alice', 'devices/register', body)).toEqual({ status: 400, body: { error: 'keyB' } })
  })

  it('reads a passkey line only with its exact application, on the curve, with nothing after it', async () => {
    const passkey = await makePasskey(RP_ID)
    expect((await parseDeviceKey(fixture.exact.publicKey, RP_ID))?.fingerprint).toBe(fixture.exact.fingerprint)
    expect(await parseDeviceKey(passkey.line, 'App.canary.farcooler.com')).toBeNull()
    expect(await parseDeviceKey(passkey.line, 'canary.farcooler.com')).toBeNull()
    expect(await parseDeviceKey(passkey.line, undefined)).toBeNull()
    const blob = Uint8Array.from(atob(passkey.line.split(' ')[1]), c => c.charCodeAt(0))
    const offCurve = blob.slice()
    offCurve[4 + 34 + 4 + 8 + 4 + 64] ^= 1
    const trailing = new Uint8Array([...blob, 0])
    for (const bad of [offCurve, trailing]) {
      expect(await parseDeviceKey(`sk-ecdsa-sha2-nistp256@openssh.com ${btoa(String.fromCharCode(...bad))}`, RP_ID)).toBeNull()
    }
  })
})

describe('a passkey device and certificates (ov-766)', () => {
  it('is certified by its Key A, by an Ed25519 device', async () => {
    const [phone, passkey] = await Promise.all([makeKey(), makePasskey(RP_ID)])
    expect((await as('alice', 'devices/register', await proven('alice', phone))).body.state).toBe('trusted')
    expect((await as('alice', 'devices/register', await proven('alice', passkey))).body.state).toBe('pending')
    const certificate = await phone.sign(CERTIFY_NAMESPACE, certifyStatement('alice', await fingerprint(passkey), null, null))
    expect(await as('alice', 'devices/certify', { keyA: passkey.line, certificate })).toEqual({
      status: 200,
      body: { device: expect.any(String), state: 'certified' },
    })
  })

  it('never signs a certificate itself: a runner checks those offline, Ed25519 alone', async () => {
    const [passkey, phone] = await Promise.all([makePasskey(RP_ID), makeKey()])
    expect((await as('alice', 'devices/register', await proven('alice', passkey))).body.state).toBe('trusted')
    expect((await as('alice', 'devices/register', await proven('alice', phone))).body.state).toBe('pending')
    const certificate = await passkey.sign(CERTIFY_NAMESPACE, certifyStatement('alice', await fingerprint(phone), null, null))
    expect(await as('alice', 'devices/certify', { keyA: phone.line, certificate })).toEqual({
      status: 403,
      body: { error: 'certificate' },
    })
  })

  it("is confirmed by a team's admin by its Key A, when held", async () => {
    const team = (await as('alice', 'teams/create', { name: 'Acme' })).body.team.id
    const invite = await as('alice', 'teams/invites/create', { team, uses: 1, role: 'member' })
    expect((await as('bob', 'invites/accept', { code: invite.body.code })).status).toBe(200)
    // Bob's first device, removed, so his passkey is trusted by R-75's reset and held.
    const old = await makeKey()
    const oldId = (await as('bob', 'devices/register', await proven('bob', old))).body.device
    expect((await as('bob', 'devices/revoke', { device: oldId })).body.ok).toBe(true)
    const passkey = await makePasskey(RP_ID)
    expect((await as('bob', 'devices/register', await proven('bob', passkey))).body.state).toBe('trusted')
    expect(await as('alice', 'teams/devices/confirm', { team, user: 'bob', keyA: passkey.line })).toEqual({
      status: 200,
      body: { ok: true },
    })
  })
})
