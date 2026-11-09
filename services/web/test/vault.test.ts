// The node key survives storage eviction (src/client/vault.ts, ov-779): what
// the passkey's PRF output derives, the sealed copy at the relay, and the
// restore after the site's storage was cleared. The passkey, the relay and the
// Tailcat module are played by the test.

import { describe, expect, it } from 'vitest'

import type { Device, Granted } from '../src/client/device'
import { b64url, type Copy, copyOf, deriveKeys, fromB64url, needsSaving, openCopy, type Prf, prfOf, PRF_SALT, prfUse, relayHoldsOurs, restore, type RestoreDeps, saveCopy, sealCopy, type Vault } from '../src/client/vault'

const ACCOUNT = 'user_OWNER'
const RUNNERS: Granted[] = [{ id: 'r1', label: 'studio', user: 'owner', host_key: 'ssh-ed25519 AAAAhost', token: 'tok', why: '' }]
const CRED = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])
const SPKI = new Uint8Array([48, 89, 48, 19, 6, 7, 42])
const HANDLE = new Uint8Array(32).fill(9)

const prfBytes = (fill: number) => new Uint8Array(32).fill(fill)

/// What `nodeKeyFromSeed` answers: a stand-in that is a function of the seed
/// alone, as the Go one is (crates/tailcat/go, TestASeedGivesTheSameUsableNodeKeyEveryTime).
const nodeKey = (seed: Uint8Array) => ({ priv: `priv-${b64url(seed)}`, pub: `pub-${b64url(seed)}` })

function enrolled(over: Partial<Device> = {}): Device {
  return { account: ACCOUNT, passkey: { id: CRED, spki: SPKI }, nodePriv: 'np', nodePub: 'nP', registered: true, runners: RUNNERS, nodeDerived: true, ...over }
}

describe('the keys a PRF output derives', () => {
  it('are the same from the same output, in any browser: the node key a second browser derives equals the enrolled one', async () => {
    const first = await deriveKeys(prfBytes(7))
    const second = await deriveKeys(prfBytes(7))
    expect(nodeKey(first.nodeSeed)).toEqual(nodeKey(second.nodeSeed))
    // And the second browser opens what the first sealed.
    const sealed = await sealCopy(first.vaultKey, ACCOUNT, CRED, copyOf(enrolled()))
    expect(await openCopy(second.vaultKey, ACCOUNT, CRED, sealed)).toEqual(copyOf(enrolled()))
  })

  it('differ for another output, and the node seed is not the copy’s key', async () => {
    const a = await deriveKeys(prfBytes(7))
    const b = await deriveKeys(prfBytes(8))
    expect(b64url(a.nodeSeed)).not.toBe(b64url(b.nodeSeed))
    expect(a.nodeSeed).toHaveLength(32)
    const sealed = await sealCopy(a.vaultKey, ACCOUNT, CRED, copyOf(enrolled()))
    expect(await openCopy(b.vaultKey, ACCOUNT, CRED, sealed)).toBeNull()
    // The copy's key can't be read out, so the seed isn't it in another form.
    await expect(crypto.subtle.exportKey('raw', a.vaultKey)).rejects.toThrow()
  })
})

describe('the sealed copy', () => {
  it('opens only for its own account and passkey, and not once a byte changed', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const sealed = await sealCopy(vaultKey, ACCOUNT, CRED, copyOf(enrolled()))
    expect(await openCopy(vaultKey, 'user_OTHER', CRED, sealed)).toBeNull()
    expect(await openCopy(vaultKey, ACCOUNT, new Uint8Array([9]), sealed)).toBeNull()
    const bytes = fromB64url(sealed)
    bytes[bytes.length - 1] ^= 1
    expect(await openCopy(vaultKey, ACCOUNT, CRED, b64url(bytes))).toBeNull()
    expect(await openCopy(vaultKey, ACCOUNT, CRED, 'not base64!')).toBeNull()
  })

  it('holds nothing the relay could read: no runner, host key or token in the clear', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const sealed = await sealCopy(vaultKey, ACCOUNT, CRED, copyOf(enrolled()))
    const clear = new TextDecoder('latin1').decode(fromB64url(sealed))
    for (const secret of ['studio', 'ssh-ed25519', 'tok', '"runners"']) expect(clear).not.toContain(secret)
    // Two seals of the same copy differ, so the relay can't tell a resave from a change.
    expect(await sealCopy(vaultKey, ACCOUNT, CRED, copyOf(enrolled()))).not.toBe(sealed)
  })

  it('carries the node key only when it isn’t derived from the passkey', () => {
    expect(copyOf(enrolled()).node).toBeUndefined()
    expect(copyOf(enrolled({ nodeDerived: false })).node).toEqual({ priv: 'np', pub: 'nP' })
  })

  it('is saved when it changed, and not again when it didn’t', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const device = enrolled()
    const posts: unknown[] = []
    const call = async (route: string, body: unknown) => {
      posts.push([route, body])
      return { status: 200, body: { ok: true } }
    }
    expect(await needsSaving(device)).toBe(true)
    expect(await saveCopy(call, vaultKey, device, true)).toBe('saved')
    expect(posts).toHaveLength(1)
    const [route, body] = posts[0] as [string, any]
    expect(route).toBe('web/vault/save')
    expect(body.credential).toBe(b64url(CRED))
    expect(body.uv).toBe(true)
    expect(await openCopy(vaultKey, ACCOUNT, CRED, body.sealed)).toEqual(copyOf(device))
    expect(await needsSaving(device)).toBe(false)
    device.runners = [...RUNNERS, { ...RUNNERS[0], id: 'r2', label: 'den' }]
    expect(await needsSaving(device)).toBe(true)
  })

  it('isn’t marked saved when the relay refused it, and says why', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const answers: [{ status: number; body: any } | Error, string][] = [
      [{ status: 409, body: { error: 'full' } }, 'full'],
      [{ status: 413, body: { error: 'too large' } }, 'too_large'],
      [{ status: 429, body: { error: 'limited' } }, 'unreachable'],
      [{ status: 502, body: { error: 'unreachable' } }, 'unreachable'],
      [new Error('offline'), 'unreachable'],
      [{ status: 400, body: { error: 'sealed' } }, 'refused'],
    ]
    for (const [answer, word] of answers) {
      const device = enrolled()
      const call = async () => {
        if (answer instanceof Error) throw answer
        return answer
      }
      expect(await saveCopy(call, vaultKey, device, false), word).toBe(word)
      expect(await needsSaving(device)).toBe(true)
      expect(device.vaultSealed).toBeUndefined()
    }
  })

  it('knows whether the relay still holds the copy this browser last sent', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const device = enrolled()
    let sent = ''
    await saveCopy(async (_route, body: any) => ((sent = body.sealed), { status: 200, body: { ok: true } }), vaultKey, device, false)
    const ours = { credential: b64url(CRED), uv: false, sealed: sent }
    expect(await relayHoldsOurs([ours], device)).toBe(true)
    // Overwritten by a session without the passkey, or gone: not ours.
    expect(await relayHoldsOurs([{ ...ours, sealed: sent.slice(0, -2) + 'AA' }], device)).toBe(false)
    expect(await relayHoldsOurs([], device)).toBe(false)
    expect(await relayHoldsOurs([{ ...ours, credential: 'b3RoZXI' }], device)).toBe(false)
    // A device that never sent one holds nothing there.
    expect(await relayHoldsOurs([ours], enrolled({ vaultSealed: undefined }))).toBe(false)
  })
})

describe('what one assertion’s PRF output may change', () => {
  const prf = (uv: boolean): Prf => ({ output: prfBytes(7), uv })
  const fresh = (over: Partial<Device> = {}): Device => ({ account: ACCOUNT, passkey: { id: CRED, spki: SPKI }, nodePriv: 'random', nodePub: 'Random', ...over })

  it('derives the node key only for a browser that hasn’t named one in an offer or been granted runners', () => {
    expect(prfUse(fresh(), prf(false))).toEqual({ use: true, deriveNode: true })
    // An offer on screen names the random key: a phone may already have it.
    expect(prfUse(fresh({ offer: '{}' }), prf(false))).toEqual({ use: true, deriveNode: false })
    // A browser enrolled before ov-779: its runners admit the random key.
    expect(prfUse(fresh({ runners: RUNNERS }), prf(false))).toEqual({ use: true, deriveNode: false })
    // Derived already: never again.
    expect(prfUse(fresh({ nodeDerived: true }), prf(false))).toEqual({ use: true, deriveNode: false })
  })

  it('is nothing at all without PRF, or with the other verification flag than this device’s', () => {
    expect(prfUse(fresh(), null)).toEqual({ use: false })
    expect(prfUse(fresh({ prfUv: true }), prf(false))).toEqual({ use: false })
    expect(prfUse(fresh({ prfUv: false }), prf(true))).toEqual({ use: false })
    expect(prfUse(fresh({ prfUv: true, runners: RUNNERS }), prf(true))).toEqual({ use: true, deriveNode: false })
  })
})

describe('the PRF result of an assertion', () => {
  const credential = (prf: unknown, flags: number) =>
    ({
      getClientExtensionResults: () => prf,
      response: { authenticatorData: new Uint8Array([...new Uint8Array(32), flags, 0, 0, 0, 0]).buffer },
    }) as unknown as PublicKeyCredential

  it('is the first output and whether the authenticator verified the person', () => {
    const out = prfOf(credential({ prf: { results: { first: prfBytes(3).buffer } } }, 0x05))
    expect(out).toEqual({ output: prfBytes(3), uv: true })
    expect(prfOf(credential({ prf: { results: { first: prfBytes(3).buffer } } }, 0x01))?.uv).toBe(false)
  })

  it('is nothing from a browser or authenticator without PRF', () => {
    expect(prfOf(credential({}, 0x01))).toBeNull()
    expect(prfOf(credential({ prf: { enabled: false } }, 0x01))).toBeNull()
    expect(prfOf(credential({ prf: { results: { first: new Uint8Array(16).buffer } } }, 0x01))).toBeNull()
  })

  it('asks with a fixed salt', () => {
    expect(new TextDecoder().decode(PRF_SALT)).toBe('farcooler-web prf v1')
  })
})

describe('restoring after the site’s storage was cleared', () => {
  async function stored(device: Device, prf = prfBytes(7), uv = false): Promise<Vault> {
    const { vaultKey } = await deriveKeys(prf)
    return { credential: b64url(CRED), uv, sealed: await sealCopy(vaultKey, ACCOUNT, CRED, copyOf(device)) }
  }

  function passkey(answer: { id?: Uint8Array; userHandle?: Uint8Array | null; prf?: Prf | null } | Error) {
    const asked: { ids: string[]; uv: boolean }[] = []
    const deps: RestoreDeps = {
      ask: async (ids, uv) => {
        asked.push({ ids: ids.map(b64url), uv })
        if (answer instanceof Error) throw answer
        return { id: answer.id ?? CRED, userHandle: answer.userHandle === undefined ? HANDLE : answer.userHandle, prf: answer.prf === undefined ? { output: prfBytes(7), uv: false } : answer.prf }
      },
      nodeKey,
    }
    return { deps, asked }
  }

  it('gets the runners and the enrolled node key back with one touch and no ceremony', async () => {
    const { nodeSeed } = await deriveKeys(prfBytes(7))
    const before = enrolled({ nodePriv: nodeKey(nodeSeed).priv, nodePub: nodeKey(nodeSeed).pub })
    const vaults = [await stored(before)]
    const { deps, asked } = passkey({})
    const done = await restore(deps, ACCOUNT, HANDLE, vaults)
    expect(asked).toEqual([{ ids: [b64url(CRED)], uv: false }])
    if (!done.ok) throw new Error(done.word)
    expect(done.device.runners).toEqual(RUNNERS)
    expect(done.device.passkey).toEqual({ id: CRED, spki: SPKI })
    expect(done.device.registered).toBe(true)
    expect([done.device.nodePriv, done.device.nodePub]).toEqual([before.nodePriv, before.nodePub])
    expect(done.device.nodeDerived).toBe(true)
    expect(done.device.prfUv).toBe(false)
    // No ceremony: nothing about an offer comes back.
    expect(done.device.offer).toBeUndefined()
    // The copy it came from is the one the relay holds: nothing to save again.
    expect(await needsSaving(done.device)).toBe(false)
    expect(await relayHoldsOurs(vaults, done.device)).toBe(true)
  })

  it('gets back a node key enrolled before ov-779 from the copy, since the passkey can’t derive it', async () => {
    const vaults = [await stored(enrolled({ nodeDerived: false, nodePriv: 'random-priv', nodePub: 'random-pub' }))]
    const done = await restore(passkey({}).deps, ACCOUNT, HANDLE, vaults)
    if (!done.ok) throw new Error(done.word)
    expect([done.device.nodePriv, done.device.nodePub, done.device.nodeDerived]).toEqual(['random-priv', 'random-pub', false])
  })

  it('asks for verification when the copy was sealed with it', async () => {
    const vaults = [await stored(enrolled(), prfBytes(7), true)]
    const { deps, asked } = passkey({ prf: { output: prfBytes(7), uv: true } })
    expect((await restore(deps, ACCOUNT, HANDLE, vaults)).ok).toBe(true)
    expect(asked[0].uv).toBe(true)
  })

  it('says why it couldn’t, and restores nothing', async () => {
    const vaults = [await stored(enrolled())]
    const cases: [Parameters<typeof passkey>[0], string][] = [
      [new Error('declined'), 'declined'],
      [new Error('failed'), 'failed'],
      [{ prf: null }, 'no_prf'],
      [{ userHandle: new Uint8Array(32).fill(1) }, 'other_account'],
      [{ id: new Uint8Array([42]) }, 'not_found'],
      [{ prf: { output: prfBytes(8), uv: false } }, 'unopened'],
    ]
    for (const [answer, word] of cases) {
      const done = await restore(passkey(answer).deps, ACCOUNT, HANDLE, vaults)
      expect(done, word).toEqual({ ok: false, word })
    }
  })

  it('takes a passkey that gives no user handle, if its copy opens', async () => {
    const done = await restore(passkey({ userHandle: null }).deps, ACCOUNT, HANDLE, [await stored(enrolled())])
    expect(done.ok).toBe(true)
  })

  it('refuses a copy whose contents aren’t a device, though it opened', async () => {
    const { vaultKey } = await deriveKeys(prfBytes(7))
    const bad = { ...copyOf(enrolled()), runners: 'nope' } as unknown as Copy
    const vaults = [{ credential: b64url(CRED), uv: false, sealed: await sealCopy(vaultKey, ACCOUNT, CRED, bad) }]
    expect(await restore(passkey({}).deps, ACCOUNT, HANDLE, vaults)).toEqual({ ok: false, word: 'unopened' })
  })
})
