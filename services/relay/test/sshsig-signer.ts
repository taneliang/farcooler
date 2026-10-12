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
