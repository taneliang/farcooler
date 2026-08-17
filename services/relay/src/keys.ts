/// Public keys, and what a signature over one proves.
///
/// The relay never holds a private key and never generates one. What it does
/// here is read an SSH public key, name it the way `ssh-keygen -lf` names it,
/// and ask `crypto.subtle` whether a signature was made by it — no arithmetic of
/// its own, because Far Cooler implements no cryptography itself, here or
/// anywhere.
///
/// Ed25519 only. Key A is an ed25519 key by construction, and a route that
/// accepted an RSA key would have to verify one too — a second signature scheme
/// on the one path whose whole job is refusing a signature that does not check
/// out.

/// An ed25519 public key as it arrived: the SSH wire encoding, and the 32 bytes
/// inside it.
///
/// Both, because they answer different questions. The fingerprint is over the
/// whole blob — that is what `ssh-keygen` prints and what a person reads off a
/// screen at the confirmation — while WebCrypto imports only the raw key.
export interface PublicKey {
  blob: Uint8Array
  raw: Uint8Array
}

const ALGORITHM = 'ssh-ed25519'

/// Read `ssh-ed25519 AAAA… comment`, or the bare base64 of the same blob.
///
/// Structural, not a prefix check: the length-prefixed algorithm name INSIDE the
/// blob is what a verifier goes by, and the text before it is a label anyone can
/// write. A blob claiming to be ed25519 in its text and carrying something else
/// in its bytes is the exact shape of that confusion, so this reads the bytes
/// and ignores the text entirely — except to find the base64.
export function parseEd25519(text: string): PublicKey | null {
  const fields = text.trim().split(/\s+/)
  if (fields.length === 0) return null
  // `ssh-ed25519 AAAA…` or `AAAA…`. Anything naming another algorithm in its
  // text is refused here rather than after decoding, so the error a caller sees
  // is about the key they sent and not about base64.
  const encoded = fields.length > 1 ? (fields[0] === ALGORITHM ? fields[1] : null) : fields[0]
  if (!encoded) return null

  let blob: Uint8Array
  try {
    const binary = atob(encoded)
    blob = Uint8Array.from(binary, c => c.charCodeAt(0))
  } catch {
    return null
  }

  const name = readString(blob, 0)
  if (!name || new TextDecoder().decode(name.bytes) !== ALGORITHM) return null
  const key = readString(blob, name.next)
  // Exactly 32 bytes and nothing after them. Trailing bytes would mean two
  // readers could disagree about what this key is, and they would disagree
  // about its fingerprint too.
  if (!key || key.bytes.length !== 32 || key.next !== blob.length) return null

  return { blob, raw: key.bytes }
}

/// The string `ssh-keygen -lf` prints: SHA-256 of the blob, base64, unpadded.
///
/// The same string end to end — shown on the new device's screen, compared by
/// eye at the confirmation, stored here, and computed by the Rust client from
/// `ssh-key`. A fingerprint this relay invented for itself would agree with
/// nothing outside this file, and the one place it is used is a comparison.
export async function fingerprintOf(key: PublicKey): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', key.blob)
  const base64 = btoa(String.fromCharCode(...new Uint8Array(digest)))
  return `SHA256:${base64.replaceAll('=', '')}`
}

/// Whether this key really signed this message.
///
/// A raw 64-byte ed25519 signature in base64, not an `sshsig` envelope: the
/// message is a device id chosen by the same device, so there is no
/// cross-protocol confusion to defend against, and an envelope would be a
/// second format for both sides to agree about.
///
/// False rather than throwing, for every reason it can fail. A malformed
/// signature, a key WebCrypto rejects as non-canonical and a signature that
/// simply does not verify are all the same answer to the only question being
/// asked, and the caller has one response for all of them.
export async function verifyEd25519(
  key: PublicKey,
  signature: string,
  message: string,
): Promise<boolean> {
  try {
    const bytes = Uint8Array.from(atob(signature), c => c.charCodeAt(0))
    if (bytes.length !== 64) return false
    const material = await crypto.subtle.importKey('raw', key.raw, 'Ed25519', false, ['verify'])
    return await crypto.subtle.verify(
      'Ed25519',
      material,
      bytes,
      new TextEncoder().encode(message),
    )
  } catch {
    return false
  }
}

/// One length-prefixed field of an SSH blob, and where the next one starts.
function readString(blob: Uint8Array, at: number): { bytes: Uint8Array; next: number } | null {
  if (at + 4 > blob.length) return null
  const view = new DataView(blob.buffer, blob.byteOffset, blob.byteLength)
  const length = view.getUint32(at)
  const start = at + 4
  if (start + length > blob.length) return null
  return { bytes: blob.subarray(start, start + length), next: start + length }
}
