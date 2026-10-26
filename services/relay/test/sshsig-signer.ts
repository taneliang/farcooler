// A device's half of SSH signatures, for tests: an Ed25519 key made in the
// test, its public line, and `ssh-keygen -Y sign` rebuilt byte for byte.
//
// The verifier is checked against a signature real `ssh-keygen` made (see
// sshsig.test.ts), so this signer agreeing with it is agreeing with OpenSSH,
// not only with itself.

import { concat, sshString, toBase64 } from '../src/sshsig'

/// Non-canonical encodings a hostile client could send, each still a valid
/// Ed25519 signature over what it says.
export interface Tweak {
  /// Bytes to put in front of the algorithm name in the key blob.
  namePrefix?: Uint8Array
  /// Bytes to put in front of the namespace.
  namespacePrefix?: Uint8Array
  reserved?: Uint8Array
}

export interface TestKey {
  /// `ssh-ed25519 AAAA… comment`, as a device would send it.
  line: string
  sign(namespace: string, message: string, hash?: 'sha512' | 'sha256', tweak?: Tweak): Promise<string>
}

const text = (s: string) => new TextEncoder().encode(s)

export async function makeKey(comment = 'test'): Promise<TestKey> {
  const pair = (await crypto.subtle.generateKey({ name: 'Ed25519' }, true, ['sign', 'verify'])) as CryptoKeyPair
  const raw = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer)
  const blob = concat(sshString(text('ssh-ed25519')), sshString(raw))
  return {
    line: `ssh-ed25519 ${toBase64(blob)} ${comment}`,
    async sign(namespace, message, hash = 'sha512', tweak = {}) {
      const keyBlob = concat(sshString(concat(tweak.namePrefix ?? new Uint8Array(), text('ssh-ed25519'))), sshString(raw))
      const ns = concat(tweak.namespacePrefix ?? new Uint8Array(), text(namespace))
      const reserved = tweak.reserved ?? new Uint8Array()
      const digest = new Uint8Array(await crypto.subtle.digest(hash === 'sha512' ? 'SHA-512' : 'SHA-256', text(message)))
      const signed = concat(text('SSHSIG'), sshString(ns), sshString(reserved), sshString(text(hash)), sshString(digest))
      const signature = new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, pair.privateKey, signed))
      const version = new Uint8Array([0, 0, 0, 1])
      const out = concat(
        text('SSHSIG'),
        version,
        sshString(keyBlob),
        sshString(ns),
        sshString(reserved),
        sshString(text(hash)),
        sshString(concat(sshString(text('ssh-ed25519')), sshString(signature))),
      )
      const b64 = toBase64(out).replace(/.{1,70}/g, '$&\n')
      return `-----BEGIN SSH SIGNATURE-----\n${b64}-----END SSH SIGNATURE-----\n`
    },
  }
}

/// What a passkey's signature can be made to say, each still a valid ECDSA
/// signature by the key over what it says (ov-766).
export interface PasskeyTweak {
  /// `sk` is a security key signing through OpenSSH; the default is a
  /// browser's `navigator.credentials.get`.
  format?: 'webauthn' | 'sk'
  /// authenticatorData's flags. The default, 0x01, is user present.
  flags?: number
  /// clientDataJSON's challenge, if not the SSHSIG blob being signed.
  challenge?: Uint8Array
  type?: string
  /// clientDataJSON's origin and the signature's own origin field.
  origin?: string
  crossOrigin?: boolean
  /// Another application to hash into authenticatorData, as an authenticator
  /// asked by a page on another RP ID would.
  application?: string
  /// mpint r with a superfluous leading zero byte.
  paddedR?: boolean
}

export interface TestPasskey {
  /// `sk-ecdsa-sha2-nistp256@openssh.com AAAA… comment`, as a browser
  /// presents its passkey (spike-761 §6).
  line: string
  sign(namespace: string, message: string, hash?: 'sha512' | 'sha256', tweak?: PasskeyTweak): Promise<string>
}

/// A software passkey with RP ID `rpId`: a P-256 key made in the test that
/// signs SSHSIG blobs as `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` the
/// way the web client encodes `navigator.credentials.get` (ov-752 slice 4).
/// The real browser's encoding is pinned by `fixtures/passkey-proof.json`;
/// this one exists to sign what a browser never would.
export async function makePasskey(rpId: string, comment = 'passkey'): Promise<TestPasskey> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair
  const point = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer)
  const name = 'sk-ecdsa-sha2-nistp256@openssh.com'
  const keyBlob = concat(sshString(text(name)), sshString(text('nistp256')), sshString(point), sshString(text(rpId)))
  const sha256 = async (bytes: Uint8Array) => new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
  const mpint = (v: Uint8Array, pad: boolean) => {
    let i = 0
    while (i < v.length - 1 && v[i] === 0) i++
    let out = v.subarray(i)
    if (out[0] & 0x80) out = concat(new Uint8Array([0]), out)
    if (pad) out = concat(new Uint8Array([0]), out)
    return sshString(out)
  }
  return {
    line: `${name} ${toBase64(keyBlob)} ${comment}`,
    async sign(namespace, message, hash = 'sha512', tweak = {}) {
      const ns = text(namespace)
      const digest = new Uint8Array(await crypto.subtle.digest(hash === 'sha512' ? 'SHA-512' : 'SHA-256', text(message)))
      const data = concat(text('SSHSIG'), sshString(ns), sshString(new Uint8Array()), sshString(text(hash)), sshString(digest))
      const flags = new Uint8Array([tweak.flags ?? 0x01])
      const counter = new Uint8Array([0, 0, 0, 7])
      const origin = tweak.origin ?? `https://${rpId}`
      const challenge = toBase64(tweak.challenge ?? data).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
      const clientData = text(
        `{"type":"${tweak.type ?? 'webauthn.get'}","challenge":"${challenge}","origin":"${origin}","crossOrigin":${tweak.crossOrigin ?? false}}`,
      )
      const webauthn = (tweak.format ?? 'webauthn') === 'webauthn'
      const authenticatorData = concat(await sha256(text(tweak.application ?? rpId)), flags, counter)
      const signed = concat(authenticatorData, await sha256(webauthn ? clientData : data))
      const raw = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, pair.privateKey, signed))
      const ecdsa = concat(mpint(raw.subarray(0, 32), tweak.paddedR ?? false), mpint(raw.subarray(32), false))
      const signature = webauthn
        ? concat(
            sshString(text('webauthn-sk-ecdsa-sha2-nistp256@openssh.com')),
            sshString(ecdsa),
            flags,
            counter,
            sshString(text(origin)),
            sshString(clientData),
            sshString(new Uint8Array()),
          )
        : concat(sshString(text(name)), sshString(ecdsa), flags, counter)
      const out = concat(
        text('SSHSIG'),
        new Uint8Array([0, 0, 0, 1]),
        sshString(keyBlob),
        sshString(ns),
        sshString(new Uint8Array()),
        sshString(text(hash)),
        sshString(signature),
      )
      const b64 = toBase64(out).replace(/.{1,70}/g, '$&\n')
      return `-----BEGIN SSH SIGNATURE-----\n${b64}-----END SSH SIGNATURE-----\n`
    },
  }
}
