/// What this browser keeps about itself, in IndexedDB, for ONE account.
///
/// - **The SSH key** is a WebCrypto Ed25519 key made non-extractable: no script
///   can export the private half, this page included. That is all it means.
///   The key's bytes still sit in the browser profile's IndexedDB files on
///   disk, not in an OS keychain, and any script running on this origin can
///   ask it to sign. Think of it as an unencrypted `~/.ssh` key in the profile;
///   slice 4's passkey is what moves it into hardware.
/// - **The account** that enrolled it. A different account signed in on this
///   browser never sees or uses the key or the runners: they are deleted, and
///   Sign Out deletes them too (ov-764 review).
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
}

export interface Device {
  /// The relay's `user.id` this device belongs to.
  account: string
  ssh: CryptoKeyPair
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
/// is enrolled on someone else's runners.
///
/// Throws `no_ed25519` when WebCrypto here has no Ed25519, which the fence
/// requires (the only key type it writes).
export async function loadDevice(account: string, mintNodeKey: () => { priv: string; pub: string }): Promise<Device> {
  const kept = await readDevice()
  if (kept && kept.account === account) return kept
  if (kept) await forgetDevice()
  let ssh: CryptoKeyPair
  try {
    ssh = (await crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify'])) as CryptoKeyPair
  } catch {
    throw new Error('no_ed25519')
  }
  const node = mintNodeKey()
  const device: Device = { account, ssh, nodePriv: node.priv, nodePub: node.pub }
  await saveDevice(device)
  return device
}

/// The SSH public key's 32 raw bytes.
export async function publicKeyBytes(device: Device): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.exportKey('raw', device.ssh.publicKey))
}

/// Sign with the non-extractable key; crates/web frames the 64 bytes for SSH.
export function signer(device: Device): (data: Uint8Array) => Promise<ArrayBuffer> {
  return data => crypto.subtle.sign({ name: 'Ed25519' }, device.ssh.privateKey, data as Uint8Array<ArrayBuffer>)
}
