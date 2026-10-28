/// What this browser keeps about itself, in IndexedDB, for ONE account.
///
/// - **The SSH key is a passkey** (ov-767, owner R-88): an ES256 credential the
///   authenticator holds (Touch ID, Windows Hello, a phone or a security key),
///   made for this build's app host as its RP ID. Its private half never
///   reaches this page or this profile's files. What is kept here is only how
///   to name it to the authenticator (its credential id) and its public key.
///   Every sign-in asks for it, at one touch.
/// - **The account** that enrolled it. A different account signed in on this
///   browser never sees or uses the passkey or the runners: they are deleted,
///   and Sign Out deletes them too (ov-764 review). The passkey itself stays
///   in the authenticator, where only its owner can delete it.
/// - **The node key** is 32 raw bytes, because WireGuard runs in Go and needs
///   them (plan-763 §0 point 8). It grants a route to a runner, never a login.
/// - **The open offer** survives a reload, so a phone that already scanned the
///   code can still be answered.
/// - **The granted runners**, each with the host key every connection pins.

export interface Granted {
  id: string
  label: string
  user: string
  host_key: string
  token: string
  /// Empty when this browser can dial it; `pending` or `address_only` if not.
  why: string
  /// The phone's reason for `pending`, or empty (crates/web `enroll::CAUSES`).
  /// Absent from runners stored before ov-828.
  cause?: string
}

export interface Passkey {
  /// The credential id, to name it in `allowCredentials`.
  id: Uint8Array
  /// Its public key as `getPublicKey()` gave it: a P-256 SPKI.
  spki: Uint8Array
}

export interface Device {
  /// The relay's `user.id` this device belongs to.
  account: string
  /// Absent until the person makes one, which needs a click (Safari).
  passkey?: Passkey
  nodePriv: string
  nodePub: string
  /// The offer on screen, as its JSON, until a reply is taken or it expires.
  offer?: string
  /// When that offer went up, on this browser's clock (`Date.now()`).
  offerShownAt?: number
  runners?: Granted[]
}

const DB = 'farcooler'
const STORE = 'device'
const KEY = 'this'

function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1)
    open.onupgradeneeded = () => open.result.createObjectStore(STORE)
    open.onsuccess = () => resolve(open.result)
    open.onerror = () => reject(open.error)
  })
}

async function transact<T>(mode: IDBTransactionMode, work: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await database()
  try {
    return await new Promise<T>((resolve, reject) => {
      const request = work(db.transaction(STORE, mode).objectStore(STORE))
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
  } finally {
    db.close()
  }
}

export const readDevice = () => transact<Device | undefined>('readonly', store => store.get(KEY))
export const saveDevice = (device: Device) => transact('readwrite', store => store.put(device, KEY))
export const forgetDevice = () => transact('readwrite', store => store.delete(KEY))

/// This browser's device for `account`, made the first time it is needed.
///
/// A device kept for any other account, or for none, is deleted first: its key
/// is enrolled on someone else's runners. So is one from before passkeys
/// (slice 1's WebCrypto Ed25519 key): `migrated` says so, and the page asks
/// for a passkey and a new enrollment. Its old line on a runner admits nobody
/// once the key is deleted, and can be removed from Far Cooler's device list.
///
/// A record that can't be read back at all (a slice-1 CryptoKey a newer
/// browser won't restore, say) is deleted too, rather than leaving the page
/// stuck on it: what it held can't be used anyway.
export async function loadDevice(account: string, mintNodeKey: () => { priv: string; pub: string }): Promise<{ device: Device; migrated: boolean }> {
  let kept: (Device & { ssh?: unknown }) | undefined
  try {
    kept = (await readDevice()) as (Device & { ssh?: unknown }) | undefined
  } catch {
    await forgetDevice()
    kept = undefined
  }
  if (kept && kept.account === account && !kept.ssh) return { device: kept, migrated: false }
  if (kept) await forgetDevice()
  const node = mintNodeKey()
  const device: Device = { account, nodePriv: node.priv, nodePub: node.pub }
  await saveDevice(device)
  return { device, migrated: kept?.account === account && !!kept.ssh }
}

/// Whether this browser can make and use a passkey at all.
export function canUsePasskeys(): boolean {
  return typeof PublicKeyCredential === 'function' && typeof navigator.credentials?.create === 'function'
}

/// WebAuthn's "the person said no, or the browser wouldn't ask": a dismissed
/// prompt, a timeout, or Safari without a click just before the call.
function declined(e: unknown): boolean {
  return e instanceof DOMException && (e.name === 'NotAllowedError' || e.name === 'AbortError')
}

const bytes = (n: number) => crypto.getRandomValues(new Uint8Array(n))

/// The passkey's WebAuthn user handle for `account`: the same bytes every
/// time, so an authenticator replaces this account's earlier Far Cooler
/// passkey on this host rather than keeping one per enrollment. A hash, so the
/// relay's account id isn't handed to the authenticator as it is.
export async function userHandle(account: string): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`farcooler-web passkey ${account}`))
  return new Uint8Array(digest).slice(0, 32)
}

/// Make this browser's passkey, for `rpId` (crates/web's `rpId()`, this
/// build's app host, never `location`), under `user` (`userHandle`, worked
/// out beforehand so nothing is awaited between the click and the prompt). Call it from a click: Safari shows no
/// prompt otherwise. Throws `declined`, or `no_passkey` when the browser can't
/// make an ES256 passkey or won't say its public key.
///
/// - `residentKey: 'required'`, so a later slice can find it without the
///   stored id (plan-763 slice 16).
/// - `userVerification: 'discouraged'`: one touch, no PIN or face (R-89). sshd
///   checks presence, which every assertion carries, not verification.
/// - ES256 only: the one algorithm OpenSSH's sk keys have.
export async function makePasskey(rpId: string, user: Uint8Array, email: string): Promise<Passkey> {
  let credential: PublicKeyCredential
  try {
    credential = (await navigator.credentials.create({
      publicKey: {
        rp: { id: rpId, name: 'Far Cooler' },
        user: { id: user as Uint8Array<ArrayBuffer>, name: email || 'Far Cooler', displayName: email || 'Far Cooler' },
        challenge: bytes(32),
        pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
        authenticatorSelection: { residentKey: 'required', requireResidentKey: true, userVerification: 'discouraged' },
        attestation: 'none',
        timeout: 120_000,
      },
    })) as PublicKeyCredential
  } catch (e) {
    throw new Error(declined(e) ? 'declined' : 'no_passkey')
  }
  const response = credential?.response as AuthenticatorAttestationResponse | undefined
  const spki = response?.getPublicKey?.()
  if (!spki || response?.getPublicKeyAlgorithm?.() !== -7) throw new Error('no_passkey')
  return { id: new Uint8Array(credential.rawId), spki: new Uint8Array(spki) }
}

export interface Signed {
  authenticatorData: ArrayBuffer
  clientDataJSON: ArrayBuffer
  signature: ArrayBuffer
}

/// Ask the passkey to sign `challenge` (the SSH login's data; crates/web
/// encodes the answer as `webauthn-sk-ecdsa-sha2-nistp256@openssh.com`). One
/// touch. Throws `declined` when the person or the browser said no.
export async function assertWith(passkey: Passkey, rpId: string, challenge: Uint8Array): Promise<Signed> {
  try {
    const credential = (await navigator.credentials.get({
      publicKey: {
        challenge: challenge as Uint8Array<ArrayBuffer>,
        rpId,
        allowCredentials: [{ type: 'public-key', id: passkey.id as Uint8Array<ArrayBuffer> }],
        userVerification: 'discouraged',
        timeout: 120_000,
      },
    })) as PublicKeyCredential
    const response = credential.response as AuthenticatorAssertionResponse
    return { authenticatorData: response.authenticatorData, clientDataJSON: response.clientDataJSON, signature: response.signature }
  } catch (e) {
    throw new Error(declined(e) ? 'declined' : 'failed')
  }
}
