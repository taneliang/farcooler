/// SSH public keys and SSH signatures (`ssh-keygen -Y sign`): Ed25519, and
/// for a device's Key A, a passkey.
///
/// The directory holds public keys and checks signatures made with the private
/// halves, which never leave a device. Two things here need that: a device
/// proving it holds the key it registers, and a person's existing device
/// certifying their new one (ov-480 §3). The runner checks grants with the
/// same format in Rust (`ssh-key`'s `SshSig`, phase 4), so this follows
/// OpenSSH's PROTOCOL.sshsig exactly rather than inventing a wire format, and
/// the tests check it against a signature `ssh-keygen` made.
///
/// `parsePublicKey` and `verifySignature` are Ed25519 only. A runner's key, a
/// Key B and a certificate's signer are always Ed25519, and a runner checks a
/// certificate offline, in Rust.
///
/// A device's Key A may also be a passkey (ov-766): a WebAuthn ES256
/// credential presented as `sk-ecdsa-sha2-nistp256@openssh.com`, the way
/// OpenSSH 8.4 and later take one, with the passkey's RP ID as its
/// application. `parseDeviceKey` and `verifyDeviceSignature` take one only
/// when that RP ID is exactly this channel's app host (`PASSKEY_RP_ID`), never
/// a parent domain. A passkey scoped to `farcooler.com` could be asked to sign
/// by any page on any `*.farcooler.com` host, including www, which loads a
/// third-party beacon script.

export interface PublicKey {
  /// `ssh-ed25519 AAAA…` or `sk-ecdsa-sha2-nistp256@openssh.com AAAA…`, with
  /// no comment. A comment is free text from the device and the directory has
  /// no use for it.
  text: string
  /// `SHA256:…`, unpadded, as `ssh-keygen -lf` prints it.
  fingerprint: string
  /// The 32 Ed25519 key bytes, or a passkey's uncompressed P-256 point.
  raw: Uint8Array
  /// A passkey's application, which is its RP ID. Absent for Ed25519.
  application?: string
}

const ALGORITHM = 'ssh-ed25519'
const MAGIC = 'SSHSIG'
const SK_ALGORITHM = 'sk-ecdsa-sha2-nistp256@openssh.com'

// Every name on the wire is compared as BYTES, never through TextDecoder,
// which strips a leading byte-order mark by default: a key blob named
// "﻿ssh-ed25519" read as text passes for "ssh-ed25519", and its
// fingerprint, taken over the odd blob, gives one key a second identity.
const ALGORITHM_BYTES = new TextEncoder().encode(ALGORITHM)
const SK_ALGORITHM_BYTES = new TextEncoder().encode(SK_ALGORITHM)
const NISTP256 = new TextEncoder().encode('nistp256')

/// A passkey's two signature formats (PROTOCOL.u2f): what a security key
/// signs through OpenSSH, and what a browser's `navigator.credentials.get`
/// signs, which wraps the message in WebAuthn's clientDataJSON.
const SK_SIGNATURE = SK_ALGORITHM_BYTES
const WEBAUTHN_SIGNATURE = new TextEncoder().encode('webauthn-sk-ecdsa-sha2-nistp256@openssh.com')

/// authenticatorData's flags: user present, attested credential data (never
/// in an assertion), and extension data.
const FLAG_UP = 0x01
const FLAG_AT = 0x40
const FLAG_ED = 0x80

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

/// Parse a device's Key A: anything `parsePublicKey` takes, or a passkey line,
/// `sk-ecdsa-sha2-nistp256@openssh.com <base64> [comment]`, whose application
/// is exactly `rpId`. With no `rpId`, on a channel with no web app, every
/// passkey is null.
export async function parseDeviceKey(line: unknown, rpId: string | undefined): Promise<PublicKey | null> {
  if (typeof line !== 'string' || line.length > 1024) return null
  const parts = line.trim().split(/\s+/)
  if (parts.length < 2) return null
  if (parts[0] === ALGORITHM) return await parsePublicKey(line)
  if (parts[0] !== SK_ALGORITHM) return null
  const blob = fromBase64(parts[1])
  if (!blob) return null
  return await passkeyFromBlob(blob, rpId)
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
  return await verify(armored, namespace, message, null)
}

/// `verifySignature` for a device's Key A, whose signer may also be a passkey
/// with application exactly `rpId`, signing as a security key or through
/// WebAuthn. Either way the authenticator must say the user was present. A
/// WebAuthn signature must also be a `webauthn.get` over this very message,
/// made on `https://<rpId>`.
export async function verifyDeviceSignature(
  armored: unknown,
  namespace: string,
  message: string,
  rpId: string | undefined,
): Promise<PublicKey | null> {
  return await verify(armored, namespace, message, rpId || null)
}

/// Both of the above. A null `rpId` takes Ed25519 alone.
async function verify(armored: unknown, namespace: string, message: string, rpId: string | null): Promise<PublicKey | null> {
  if (typeof armored !== 'string' || armored.length > 4096) return null
  const body = armored.trim().match(/^-----BEGIN SSH SIGNATURE-----\s+([A-Za-z0-9+/=\s]+?)\s*-----END SSH SIGNATURE-----$/)
  if (!body) return null
  const bytes = fromBase64(body[1].replace(/\s+/g, ''))
  if (!bytes) return null

  const reader = new Reader(bytes)
  if (!same(reader.take(6), new TextEncoder().encode(MAGIC))) return null
  if (reader.u32() !== 1) return null
  const keyBlob = reader.bytes()
  const signedNamespace = reader.bytes()
  const reserved = reader.bytes()
  const hashBlob = reader.bytes()
  const signatureBlob = reader.bytes()
  if (!reader.done() || !keyBlob || !signedNamespace || !reserved || !hashBlob || !signatureBlob) return null
  if (!same(signedNamespace, new TextEncoder().encode(namespace))) return null
  // Empty, as PROTOCOL.sshsig says and as `ssh-key` insists: the runner's
  // verifier (phase 4) must not refuse what this one accepts.
  if (reserved.length !== 0) return null

  const hash = same(hashBlob, new TextEncoder().encode('sha512'))
    ? 'sha512'
    : same(hashBlob, new TextEncoder().encode('sha256'))
      ? 'sha256'
      : null
  if (!hash) return null

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

  if (rpId !== null && same(new Reader(keyBlob).bytes(), SK_ALGORITHM_BYTES)) {
    const key = await passkeyFromBlob(keyBlob, rpId)
    if (!key) return null
    return (await verifyPasskey(key, rpId, signatureBlob, signed)) ? key : null
  }

  const key = await keyFromBlob(keyBlob)
  if (!key) return null

  const signature = new Reader(signatureBlob)
  if (!same(signature.bytes(), ALGORITHM_BYTES)) return null
  const raw = signature.bytes()
  if (!signature.done() || !raw || raw.length !== 64) return null

  const material = await crypto.subtle.importKey('raw', key.raw, { name: 'Ed25519' }, false, ['verify'])
  if (!(await crypto.subtle.verify({ name: 'Ed25519' }, material, raw, signed))) return null
  return key
}

/// Whether `signatureBlob` is `key`'s signature over `data`, the SSHSIG
/// blob, in either passkey format. It checks what OpenSSH's
/// `ssh_ecdsa_sk_verify` checks, and is stricter where that costs nothing:
/// the user must have been present, mpints must be canonical, and a WebAuthn
/// signature must come from `https://<rpId>` and not from a frame on another
/// site.
///
/// What the authenticator signed is its authenticatorData followed by a
/// SHA-256: of `data` for a security key, or of clientDataJSON for WebAuthn,
/// whose challenge must be `data` itself. authenticatorData is rebuilt here
/// from the key's own application (so its RP ID hash can't name any other
/// site), the flags, the counter and any extensions.
async function verifyPasskey(key: PublicKey, rpId: string, signatureBlob: Uint8Array, data: Uint8Array): Promise<boolean> {
  const signature = new Reader(signatureBlob)
  const format = signature.bytes()
  const webauthn = same(format, WEBAUTHN_SIGNATURE)
  if (!webauthn && !same(format, SK_SIGNATURE)) return false
  const ecdsa = signature.bytes()
  const flags = signature.take(1)
  const counter = signature.take(4)
  if (!ecdsa || !flags || !counter) return false
  let extensions: Uint8Array = new Uint8Array()
  let clientData: Uint8Array | null = null
  if (webauthn) {
    const origin = signature.bytes()
    clientData = signature.bytes()
    const ext = signature.bytes()
    if (!origin || !clientData || !ext) return false
    if (!same(origin, new TextEncoder().encode(`https://${rpId}`))) return false
    if (!isAssertionOver(clientData, data, `https://${rpId}`)) return false
    extensions = ext
  }
  if (!signature.done()) return false

  // A touch every time: a passkey that signs with nobody there proves only
  // that something can reach the authenticator.
  if ((flags[0] & FLAG_UP) === 0) return false
  if ((flags[0] & FLAG_AT) !== 0) return false
  if (((flags[0] & FLAG_ED) !== 0) !== (extensions.length !== 0)) return false

  const rs = new Reader(ecdsa)
  const r = scalar(rs.bytes())
  const s = scalar(rs.bytes())
  if (!r || !s || !rs.done()) return false

  const application = await sha256(new TextEncoder().encode(key.application!))
  const signed = concat(application, flags, counter, extensions, await sha256(clientData ?? data))
  const material = await crypto.subtle.importKey('raw', key.raw, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify'])
  return await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, material, concat(r, s), signed)
}

/// Whether `clientData` is a browser's record of `navigator.credentials.get`
/// over `challenge` on `origin`. Its start is compared as bytes against the
/// one form browsers write, as OpenSSH compares it, so this verifier and the
/// runner's sshd agree. Then it is read as JSON, to refuse an assertion made
/// in a frame embedded on another site.
function isAssertionOver(clientData: Uint8Array, challenge: Uint8Array, origin: string): boolean {
  const expected = new TextEncoder().encode(`{"type":"webauthn.get","challenge":"${base64Url(challenge)}","origin":"${origin}"`)
  if (!same(clientData.subarray(0, expected.length), expected)) return false
  let fields: unknown
  try {
    fields = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(clientData))
  } catch {
    return false
  }
  if (typeof fields !== 'object' || fields === null) return false
  const record = fields as Record<string, unknown>
  return record.type === 'webauthn.get' && record.origin === origin && record.crossOrigin !== true && record.topOrigin === undefined
}

/// A canonical, positive, nonzero SSH mpint of at most 32 bytes, as the 32
/// big-endian bytes that WebCrypto's raw ECDSA signature wants.
function scalar(mpint: Uint8Array | null): Uint8Array | null {
  if (!mpint || mpint.length === 0 || mpint.length > 33) return null
  if ((mpint[0] & 0x80) !== 0) return null
  if (mpint[0] === 0 && (mpint.length === 1 || (mpint[1] & 0x80) === 0)) return null
  const value = mpint[0] === 0 ? mpint.subarray(1) : mpint
  if (value.length > 32) return null
  const out = new Uint8Array(32)
  out.set(value, 32 - value.length)
  return out
}

/// The key in `blob`, named and fingerprinted by its canonical encoding,
/// rebuilt from the 32 key bytes. The byte checks above it already refuse
/// every other encoding; the rebuild means the fingerprint never depends on
/// what the client sent, only on the key.
async function keyFromBlob(blob: Uint8Array): Promise<PublicKey | null> {
  const reader = new Reader(blob)
  if (!same(reader.bytes(), ALGORITHM_BYTES)) return null
  const raw = reader.bytes()
  if (!reader.done() || !raw || raw.length !== 32) return null
  const canonical = concat(sshString(ALGORITHM_BYTES), sshString(raw))
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', canonical))
  return {
    text: `${ALGORITHM} ${toBase64(canonical)}`,
    fingerprint: `SHA256:${toBase64(digest).replace(/=+$/, '')}`,
    raw,
  }
}

/// The passkey in `blob`: curve `nistp256`, an uncompressed point on that
/// curve, and an application that is exactly `rpId`. Named and fingerprinted
/// by its canonical encoding, rebuilt from those parts, as `keyFromBlob` does.
async function passkeyFromBlob(blob: Uint8Array, rpId: string | null | undefined): Promise<PublicKey | null> {
  if (!rpId) return null
  const reader = new Reader(blob)
  if (!same(reader.bytes(), SK_ALGORITHM_BYTES)) return null
  if (!same(reader.bytes(), NISTP256)) return null
  const point = reader.bytes()
  const application = reader.bytes()
  if (!reader.done() || !point || point.length !== 65 || point[0] !== 0x04) return null
  const rp = new TextEncoder().encode(rpId)
  if (!same(application, rp)) return null
  try {
    // Refuses a point that isn't on the curve.
    await crypto.subtle.importKey('raw', point, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify'])
  } catch {
    return null
  }
  const canonical = concat(sshString(SK_ALGORITHM_BYTES), sshString(NISTP256), sshString(point), sshString(rp))
  return {
    text: `${SK_ALGORITHM} ${toBase64(canonical)}`,
    fingerprint: `SHA256:${toBase64(await sha256(canonical)).replace(/=+$/, '')}`,
    raw: point,
    application: rpId,
  }
}

/// SSH's wire encoding: a big-endian u32 length, then that many bytes. Every
/// read returns null rather than throwing once the input runs out.
class Reader {
  private at = 0
  constructor(private readonly buf: Uint8Array) {}

  take(n: number): Uint8Array | null {
    if (this.at + n > this.buf.length) return null
    const out = this.buf.subarray(this.at, this.at + n)
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

function same(a: Uint8Array | null, b: Uint8Array): boolean {
  return a !== null && a.length === b.length && a.every((byte, i) => byte === b[i])
}

async function sha256(bytes: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
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

/// Unpadded base64url, as WebAuthn writes a challenge into clientDataJSON.
export function base64Url(bytes: Uint8Array): string {
  return toBase64(bytes).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
}
