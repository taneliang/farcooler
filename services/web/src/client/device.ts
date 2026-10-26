/// What this browser keeps about itself, in IndexedDB.
///
/// - **The SSH key** is a WebCrypto Ed25519 key made non-extractable: the page
///   can ask it to sign and can read its public half, and nothing, this page
///   included, can read the private half out. IndexedDB holds the CryptoKey
///   object itself, which is how a non-extractable key survives a reload.
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
  ssh: CryptoKeyPair
  nodePriv: string
  nodePub: string
  /// The offer on screen, as its JSON, until a reply is taken.
  offer?: string
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

/// This browser's device, made the first time it is needed.
///
/// Throws `no_ed25519` when WebCrypto here has no Ed25519, which the fence
/// requires (the only key type it writes).
export async function loadDevice(mintNodeKey: () => { priv: string; pub: string }): Promise<Device> {
  const kept = await readDevice()
  if (kept) return kept
  let ssh: CryptoKeyPair
  try {
    ssh = (await crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify'])) as CryptoKeyPair
  } catch {
    throw new Error('no_ed25519')
  }
  const node = mintNodeKey()
  const device: Device = { ssh, nodePriv: node.priv, nodePub: node.pub }
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
