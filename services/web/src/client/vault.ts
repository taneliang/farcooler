/// The node key survives storage eviction (ov-779, plan-763 slice 16).
///
/// Everything this browser keeps about itself lives in its site storage
/// (device.ts), which Safari wipes after seven days without a visit and anyone
/// can clear. Without this module that wipe costs a new passkey and a new
/// enrollment from the phone.
///
/// With it, the passkey gives the keys back. Every assertion asks the passkey
/// for its WebAuthn PRF output (`PRF_SALT`), which is the same for the same
/// passkey in any browser it syncs to, and never leaves this page. HKDF turns
/// it into two keys, one per label:
///
/// - **`farcooler-node-v1`**: the seed of this browser's Tailcat node key
///   (crates/tailcat/go `nodeKeyFromSeed`). A browser that registers its
///   passkey after ov-779 derives its node key from it before its offer names
///   the key, so the node key runners admit is one the passkey can always make
///   again.
/// - **`farcooler-vault-v1`**: an AES-256-GCM key, non-extractable, for a
///   sealed copy of the device kept at the relay (services/relay
///   `src/web-vault.ts`): the passkey's public key, whether it is registered,
///   the runners with their host keys and tokens, and the node key itself only
///   when the passkey didn't derive it (a browser enrolled before ov-779).
///
/// After a wipe, `restore` asks the passkey once, opens the copy, and the
/// browser is what it was: no new passkey, no code to scan. The relay holds
/// only ciphertext bound to the account and the credential id, so it can't
/// read a runner or forge a host key; it could withhold a copy, which costs an
/// enrollment, or hand back an older one.
///
/// A browser or authenticator without PRF gets none of this and keeps working
/// as before, with its storage marked persistent (app.ts).

import type { Device, Granted } from './device'

/// What the passkey's PRF is asked to evaluate. The browser hashes it with its
/// own WebAuthn prefix before the authenticator sees it.
export const PRF_SALT = new TextEncoder().encode('farcooler-web prf v1')

/// The `extensions` member of every `navigator.credentials.get` here.
export function prfExtension(): AuthenticationExtensionsClientInputs {
  return { prf: { eval: { first: PRF_SALT } } } as AuthenticationExtensionsClientInputs
}

/// A passkey's PRF output, and whether the authenticator verified the person
/// for it. An authenticator's PRF can differ with and without verification
/// (CTAP2 hmac-secret keeps two secrets), so an output is only used alongside
/// the flag it came with.
export interface Prf {
  output: Uint8Array
  uv: boolean
}

/// The PRF result of an assertion, or null when the browser or authenticator
/// gave none.
export function prfOf(credential: PublicKeyCredential): Prf | null {
  const results = (credential.getClientExtensionResults?.() as { prf?: { results?: { first?: BufferSource } } } | undefined)?.prf?.results
  const first = results?.first
  if (!first) return null
  const output = new Uint8Array(ArrayBuffer.isView(first) ? first.buffer.slice(first.byteOffset, first.byteOffset + first.byteLength) : first)
  if (output.length !== 32) return null
  const data = new Uint8Array((credential.response as AuthenticatorAssertionResponse).authenticatorData)
  // authenticatorData: 32 bytes of RP ID hash, then the flags byte; bit 2 is UV.
  return { output, uv: (data[32] & 0x04) !== 0 }
}

export interface Keys {
  /// 32 bytes for `nodeKeyFromSeed`.
  nodeSeed: Uint8Array
  /// The copy's key. It can't be exported.
  vaultKey: CryptoKey
}

const LABEL = { node: 'farcooler-node-v1', vault: 'farcooler-vault-v1' }

export async function deriveKeys(prf: Uint8Array): Promise<Keys> {
  const base = await crypto.subtle.importKey('raw', prf as Uint8Array<ArrayBuffer>, 'HKDF', false, ['deriveBits', 'deriveKey'])
  const params = (info: string): HkdfParams => ({ name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(0), info: new TextEncoder().encode(info) })
  const nodeSeed = new Uint8Array(await crypto.subtle.deriveBits(params(LABEL.node), base, 256))
  const vaultKey = await crypto.subtle.deriveKey(params(LABEL.vault), base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])
  return { nodeSeed, vaultKey }
}

// MARK: The copy

/// What the sealed copy holds: enough to make the device again with the
/// passkey's help.
export interface Copy {
  v: 1
  /// The passkey's P-256 SPKI, which an assertion doesn't give.
  spki: string
  registered: boolean
  runners: Granted[]
  /// Only for a node key the passkey didn't derive.
  node?: { priv: string; pub: string }
}

export function copyOf(device: Device): Copy {
  const copy: Copy = { v: 1, spki: b64url(device.passkey?.spki ?? new Uint8Array()), registered: !!device.registered, runners: device.runners ?? [] }
  if (!device.nodeDerived) copy.node = { priv: device.nodePriv, pub: device.nodePub }
  return copy
}

/// Binds a copy to its account and passkey: the relay can't hand one account's
/// copy, or one passkey's, to another.
function aad(account: string, credential: Uint8Array): Uint8Array<ArrayBuffer> {
  return new TextEncoder().encode(`farcooler-vault-v1\n${account}\n${b64url(credential)}`)
}

/// A fresh 12-byte nonce, then the ciphertext, as base64url.
export async function sealCopy(key: CryptoKey, account: string, credential: Uint8Array, copy: Copy): Promise<string> {
  const iv = crypto.getRandomValues(new Uint8Array(12))
  const plain = new TextEncoder().encode(JSON.stringify(copy))
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: aad(account, credential) }, key, plain))
  const out = new Uint8Array(iv.length + sealed.length)
  out.set(iv)
  out.set(sealed, iv.length)
  return b64url(out)
}

/// The copy, or null for one that doesn't open under this key, account and
/// passkey, or isn't a copy.
export async function openCopy(key: CryptoKey, account: string, credential: Uint8Array, sealed: string): Promise<Copy | null> {
  try {
    const bytes = fromB64url(sealed)
    const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.slice(0, 12), additionalData: aad(account, credential) }, key, bytes.slice(12))
    const copy = JSON.parse(new TextDecoder().decode(plain)) as Copy
    return isCopy(copy) ? copy : null
  } catch {
    return null
  }
}

function isCopy(copy: Copy): boolean {
  const text = (v: unknown) => typeof v === 'string'
  return (
    copy?.v === 1 &&
    text(copy.spki) &&
    typeof copy.registered === 'boolean' &&
    Array.isArray(copy.runners) &&
    copy.runners.every(r => r && text(r.id) && text(r.label) && text(r.user) && text(r.host_key) && text(r.token) && text(r.why)) &&
    (copy.node === undefined || (text(copy.node.priv) && text(copy.node.pub)))
  )
}

/// A digest of what the copy would hold, so an unchanged device isn't sealed
/// and sent again after every connection.
async function digest(device: Device): Promise<string> {
  const bytes = new TextEncoder().encode(JSON.stringify(copyOf(device)))
  return b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)))
}

export async function needsSaving(device: Device): Promise<boolean> {
  return !!device.passkey && device.vaulted !== (await digest(device))
}

/// Seal `device` and keep it at the relay, through `call` (`/api/relay`).
/// True when the relay kept it; `device.vaulted` then says what was kept, and
/// the caller saves the device.
export async function saveCopy(call: (route: string, body: unknown) => Promise<{ status: number; body: any }>, key: CryptoKey, device: Device, uv: boolean): Promise<boolean> {
  const passkey = device.passkey
  if (!passkey) return false
  const kept = await digest(device)
  const sealed = await sealCopy(key, device.account, passkey.id, copyOf(device))
  const answer = await call('web/vault/save', { credential: b64url(passkey.id), uv, sealed }).catch(() => null)
  if (answer?.status !== 200) return false
  device.vaulted = kept
  return true
}

// MARK: Restoring

/// One copy as the relay lists it.
export interface Vault {
  credential: string
  uv: boolean
  sealed: string
}

export interface RestoreDeps {
  /// One passkey touch: `navigator.credentials.get` naming `ids`, asking for
  /// PRF, with verification required when `uv`. Throws `declined` or
  /// `failed`.
  ask: (ids: Uint8Array[], uv: boolean) => Promise<{ id: Uint8Array; userHandle: Uint8Array | null; prf: Prf | null }>
  /// crates/tailcat/go `nodeKeyFromSeed`.
  nodeKey: (seed: Uint8Array) => { priv: string; pub: string } | null
}

export type Restored = { ok: true; device: Device; vaultKey: CryptoKey } | { ok: false; word: 'declined' | 'failed' | 'no_prf' | 'other_account' | 'not_found' | 'unopened' }

/// Make this browser's device for `account` again from one of `vaults`, with
/// one touch of the passkey that sealed it. `handle` is the account's passkey
/// user handle (device.ts `userHandle`).
export async function restore(deps: RestoreDeps, account: string, handle: Uint8Array, vaults: Vault[]): Promise<Restored> {
  // Verification is asked for only when every copy was sealed with it: a copy
  // sealed without it needs the output the authenticator gives without it.
  const uv = vaults.length > 0 && vaults.every(v => v.uv)
  let answer: Awaited<ReturnType<RestoreDeps['ask']>>
  try {
    answer = await deps.ask(
      vaults.map(v => fromB64url(v.credential)),
      uv,
    )
  } catch (e) {
    return { ok: false, word: (e as Error).message === 'declined' ? 'declined' : 'failed' }
  }
  if (answer.userHandle && b64url(answer.userHandle) !== b64url(handle)) return { ok: false, word: 'other_account' }
  if (!answer.prf) return { ok: false, word: 'no_prf' }
  const vault = vaults.find(v => v.credential === b64url(answer.id))
  if (!vault) return { ok: false, word: 'not_found' }

  const { nodeSeed, vaultKey } = await deriveKeys(answer.prf.output)
  const copy = await openCopy(vaultKey, account, answer.id, vault.sealed)
  if (!copy) return { ok: false, word: 'unopened' }
  const node = copy.node ?? deps.nodeKey(nodeSeed)
  if (!node) return { ok: false, word: 'failed' }

  const device: Device = {
    account,
    passkey: { id: answer.id, spki: fromB64url(copy.spki) },
    nodePriv: node.priv,
    nodePub: node.pub,
    nodeDerived: !copy.node,
    prfUv: answer.prf.uv,
    registered: copy.registered,
    runners: copy.runners,
  }
  device.vaulted = await digest(device)
  return { ok: true, device, vaultKey }
}

// MARK: Bytes

export function b64url(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

/// Throws on anything but base64url.
export function fromB64url(text: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error('not base64url')
  const padded = text.replaceAll('-', '+').replaceAll('_', '/')
  const binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4))
  return Uint8Array.from(binary, c => c.charCodeAt(0))
}
