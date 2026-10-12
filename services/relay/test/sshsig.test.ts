import { describe, expect, it } from 'vitest'

import { parsePublicKey, verifySignature } from '../src/sshsig'
import { makeKey } from './sshsig-signer'

// Made with OpenSSH, not with this code, so the verifier is held to the real
// format:
//
//   ssh-keygen -t ed25519 -N '' -C fixture -f fx
//   printf 'farcooler register-device\nhello\n' > msg
//   ssh-keygen -Y sign -f fx -n farcooler-device-v1 msg
//   ssh-keygen -lf fx.pub
//
// The private key was thrown away. Only its public half and one signature are
// here.
const FIXTURE_KEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIP3wb3VEKvnMYxRWb/bNalAYQFhjQs/nc6u0za8hwIQB fixture'
const FIXTURE_FINGERPRINT = 'SHA256:NkqYFDhLHLULKgZMdTbE3TqXuOlaGHDT2wAyjXtcFqs'
const FIXTURE_MESSAGE = 'farcooler register-device\nhello\n'
const FIXTURE_SIGNATURE = `-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg/fBvdUQq+cxjFFZv9s1qUBhAWG
NCz+dzq7TNryHAhAEAAAATZmFyY29vbGVyLWRldmljZS12MQAAAAAAAAAGc2hhNTEyAAAA
UwAAAAtzc2gtZWQyNTUxOQAAAEA8ZYD6hdt855pAwN+XqdDHJ1Cod2WgX5rrHdxuI2sfj9
14tB9XOLRVt6aUKaeqwwCrBBnZvc2rn4IRGBcSt4IH
-----END SSH SIGNATURE-----
`

/// The armored signature with one byte of its decoded body changed.
function tamper(armored: string, at: number): string {
  const body = armored.replace(/-----[A-Z ]+-----/g, '').replace(/\s+/g, '')
  const bytes = Uint8Array.from(atob(body), c => c.charCodeAt(0))
  bytes[at] ^= 0x01
  return `-----BEGIN SSH SIGNATURE-----\n${btoa(String.fromCharCode(...bytes))}\n-----END SSH SIGNATURE-----\n`
}

describe('public keys', () => {
  it('reads an Ed25519 line and fingerprints it the way ssh-keygen -lf does', async () => {
    const key = await parsePublicKey(FIXTURE_KEY)
    expect(key?.fingerprint).toBe(FIXTURE_FINGERPRINT)
    // The comment is dropped: it is the device's free text.
    expect(key?.text).toBe(FIXTURE_KEY.replace(' fixture', ''))
  })

  it.each([
    ['another algorithm', 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC7 x'],
    ['options in front', `restrict,command="x" ${FIXTURE_KEY}`],
    ['a blob that says ed25519 but is short', `ssh-ed25519 ${btoa('\0\0\0\x0bssh-ed25519\0\0\0\x10' + 'a'.repeat(16))}`],
    ['trailing bytes after the key', `ssh-ed25519 ${btoa('\0\0\0\x0bssh-ed25519\0\0\0\x20' + 'a'.repeat(33))}`],
    ['not base64', 'ssh-ed25519 !!!! x'],
    ['a private key', '-----BEGIN OPENSSH PRIVATE KEY-----'],
    ['nothing', ''],
    ['not a string', 42],
  ])('refuses %s', async (_, line) => {
    expect(await parsePublicKey(line)).toBeNull()
  })
})

describe('SSH signatures', () => {
  it('verifies a signature ssh-keygen made, and names its key', async () => {
    const key = await verifySignature(FIXTURE_SIGNATURE, 'farcooler-device-v1', FIXTURE_MESSAGE)
    expect(key?.fingerprint).toBe(FIXTURE_FINGERPRINT)
  })

  it('verifies what the test signer makes, with either hash', async () => {
    const key = await makeKey()
    const fingerprint = (await parsePublicKey(key.line))!.fingerprint
    for (const hash of ['sha512', 'sha256'] as const) {
      const signature = await key.sign('ns-v1', 'hello', hash)
      expect((await verifySignature(signature, 'ns-v1', 'hello'))?.fingerprint).toBe(fingerprint)
    }
  })

  it('refuses another namespace', async () => {
    expect(await verifySignature(FIXTURE_SIGNATURE, 'farcooler-grant-v1', FIXTURE_MESSAGE)).toBeNull()
  })

  it('refuses another message', async () => {
    expect(await verifySignature(FIXTURE_SIGNATURE, 'farcooler-device-v1', FIXTURE_MESSAGE + 'x')).toBeNull()
  })

  /// The last byte is inside the Ed25519 signature itself, so this reaches
  /// `crypto.subtle.verify` and is turned away there, not by the parser.
  it('refuses a signature with one bit flipped', async () => {
    const bytes = atob(FIXTURE_SIGNATURE.replace(/-----[A-Z ]+-----/g, '').replace(/\s+/g, '')).length
    expect(await verifySignature(tamper(FIXTURE_SIGNATURE, bytes - 1), 'farcooler-device-v1', FIXTURE_MESSAGE)).toBeNull()
  })

  /// Byte 60 is inside the embedded public key: the signature is then checked
  /// against a key that didn't make it.
  it('refuses a signature presented with another key', async () => {
    expect(await verifySignature(tamper(FIXTURE_SIGNATURE, 60), 'farcooler-device-v1', FIXTURE_MESSAGE)).toBeNull()
  })

  it.each([
    ['no armor', FIXTURE_SIGNATURE.replace('-----BEGIN SSH SIGNATURE-----', '')],
    ['the wrong magic', tamper(FIXTURE_SIGNATURE, 0)],
    ['version 0', tamper(FIXTURE_SIGNATURE, 9)],
    ['not a string', null],
  ])('refuses %s', async (_, armored) => {
    expect(await verifySignature(armored, 'farcooler-device-v1', FIXTURE_MESSAGE)).toBeNull()
  })

  /// TextDecoder strips a leading byte-order mark, so a name compared as text
  /// let one key carry a second fingerprint (review S1). Each of these is a
  /// real signature by a real key over exactly what it says.
  describe('compares wire names as bytes', () => {
    const BOM = new Uint8Array([0xef, 0xbb, 0xbf])

    it('refusing a key blob whose algorithm name has a byte-order mark', async () => {
      const key = await makeKey()
      const signature = await key.sign('ns-v1', 'hello', 'sha512', { namePrefix: BOM })
      expect(await verifySignature(signature, 'ns-v1', 'hello')).toBeNull()
    })

    it('refusing such a key as a public key line too', async () => {
      const blob = Uint8Array.from(atob(FIXTURE_KEY.split(' ')[1]), c => c.charCodeAt(0))
      const bom = new Uint8Array([0, 0, 0, 14, ...BOM, ...blob.subarray(4)])
      expect(await parsePublicKey(`ssh-ed25519 ${btoa(String.fromCharCode(...bom))}`)).toBeNull()
    })

    it('refusing a namespace with a byte-order mark', async () => {
      const key = await makeKey()
      const signature = await key.sign('ns-v1', 'hello', 'sha512', { namespacePrefix: BOM })
      expect(await verifySignature(signature, 'ns-v1', 'hello')).toBeNull()
    })

    it('refusing a non-empty reserved field, as ssh-key does', async () => {
      const key = await makeKey()
      const signature = await key.sign('ns-v1', 'hello', 'sha512', { reserved: new Uint8Array([1]) })
      expect(await verifySignature(signature, 'ns-v1', 'hello')).toBeNull()
    })
  })
})
