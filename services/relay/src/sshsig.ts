/// SSH public keys and SSH signatures (`ssh-keygen -Y sign`), Ed25519 only.
///
/// The directory holds public keys and checks signatures made with the private
/// halves, which never leave a device. Two things here need that: a device
/// proving it holds the key it registers, and a person's existing device
/// certifying their new one (ov-480 §3). The runner checks grants with the
/// same format in Rust (`ssh-key`'s `SshSig`, phase 4), so this follows
/// OpenSSH's PROTOCOL.sshsig exactly rather than inventing a wire format, and
/// the tests check it against a signature `ssh-keygen` made.
///
/// Ed25519 only, because the fence admits nothing else
/// (`crates/fence/src/lib.rs`, `Algorithm::Ed25519`): a key of any other type
/// could never be enrolled anywhere, so there is nothing to register.

export interface PublicKey {
  /// `ssh-ed25519 AAAA…`, with no comment. A comment is free text from the
  /// device and the directory has no use for it.
  text: string
  /// `SHA256:…`, unpadded, as `ssh-keygen -lf` prints it.
  fingerprint: string
  raw: Uint8Array
}

const ALGORITHM = 'ssh-ed25519'
const MAGIC = 'SSHSIG'

/// Parse `ssh-ed25519 <base64> [comment]`, or null for anything else: another
/// algorithm, options in front, a blob that doesn't decode to exactly one
/// 32-byte Ed25519 key.
export async function parsePublicKey(line: unknown): Promise<PublicKey | null> {
  if (typeof line !== 'string' || line.length > 1024) return null
  const parts = line.trim().split(/\s+/)
  if (parts.length < 2 || parts[0] !== ALGORITHM) return null
  const blob = fromBase64(parts[1])
  if (!blob) return null
  return await keyFromBlob(blob)
}

/// Verify an armored SSH signature over `message` in `namespace`, and return
/// the public key that made it, or null.
///
/// A null is any failure: bad armor, a version other than 1, the wrong
/// namespace, a key or signature that isn't Ed25519, trailing bytes, or a
/// signature that doesn't verify. The caller then decides whether the key it
/// got back is one it trusts; a valid signature by a stranger is still valid.
export async function verifySignature(
  armored: unknown,
  namespace: string,
  message: string,
): Promise<PublicKey | null> {
  if (typeof armored !== 'string' || armored.length > 4096) return null
  const body = armored.trim().match(/^-----BEGIN SSH SIGNATURE-----\s+([A-Za-z0-9+/=\s]+?)\s*-----END SSH SIGNATURE-----$/)
  if (!body) return null
  const bytes = fromBase64(body[1].replace(/\s+/g, ''))
  if (!bytes) return null

  const reader = new Reader(bytes)
  if (reader.raw(6) !== MAGIC) return null
  if (reader.u32() !== 1) return null
  const keyBlob = reader.bytes()
  const signedNamespace = reader.bytes()
  const reserved = reader.bytes()
  const hashBlob = reader.bytes()
  const signatureBlob = reader.bytes()
  if (!reader.done() || !keyBlob || !signedNamespace || !reserved || !hashBlob || !signatureBlob) return null
  if (decode(signedNamespace) !== namespace) return null

  const hash = decode(hashBlob)
  if (hash !== 'sha512' && hash !== 'sha256') return null

  const key = await keyFromBlob(keyBlob)
  if (!key) return null

  const signature = new Reader(signatureBlob)
  if (decode(signature.bytes()) !== ALGORITHM) return null
  const raw = signature.bytes()
  if (!signature.done() || !raw || raw.length !== 64) return null

  const digest = new Uint8Array(
    await crypto.subtle.digest(hash === 'sha512' ? 'SHA-512' : 'SHA-256', new TextEncoder().encode(message)),
  )
  const signed = concat(
    new TextEncoder().encode(MAGIC),
    sshString(signedNamespace),
    sshString(reserved),
    sshString(hashBlob),
    sshString(digest),
  )
  const material = await crypto.subtle.importKey('raw', key.raw, { name: 'Ed25519' }, false, ['verify'])
  if (!(await crypto.subtle.verify({ name: 'Ed25519' }, material, raw, signed))) return null
  return key
}

async function keyFromBlob(blob: Uint8Array): Promise<PublicKey | null> {
  const reader = new Reader(blob)
  if (decode(reader.bytes()) !== ALGORITHM) return null
  const raw = reader.bytes()
  if (!reader.done() || !raw || raw.length !== 32) return null
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', blob))
  return {
    text: `${ALGORITHM} ${toBase64(blob)}`,
    fingerprint: `SHA256:${toBase64(digest).replace(/=+$/, '')}`,
    raw,
  }
}

/// SSH's wire encoding: a big-endian u32 length, then that many bytes. Every
/// read returns null rather than throwing once the input runs out.
class Reader {
  private at = 0
  constructor(private readonly buf: Uint8Array) {}

  raw(n: number): string | null {
    if (this.at + n > this.buf.length) return null
    const out = decode(this.buf.subarray(this.at, this.at + n))
    this.at += n
    return out
  }

  u32(): number | null {
    if (this.at + 4 > this.buf.length) return null
    const b = this.buf
    const n = ((b[this.at] << 24) | (b[this.at + 1] << 16) | (b[this.at + 2] << 8) | b[this.at + 3]) >>> 0
    this.at += 4
    return n
  }

  bytes(): Uint8Array | null {
    const n = this.u32()
    if (n === null || this.at + n > this.buf.length) return null
    const out = this.buf.subarray(this.at, this.at + n)
    this.at += n
    return out
  }

  done(): boolean {
    return this.at === this.buf.length
  }
}

/// Encode `bytes` as an SSH string. Exported for the tests, which build
/// signatures the way `ssh-keygen` does.
export function sshString(bytes: Uint8Array): Uint8Array {
  const out = new Uint8Array(4 + bytes.length)
  new DataView(out.buffer).setUint32(0, bytes.length)
  out.set(bytes, 4)
  return out
}

export function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

function decode(bytes: Uint8Array | null): string | null {
  return bytes ? new TextDecoder().decode(bytes) : null
}

function fromBase64(text: string): Uint8Array | null {
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(text)) return null
  try {
    return Uint8Array.from(atob(text), c => c.charCodeAt(0))
  } catch {
    return null
  }
}

export function toBase64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
}
