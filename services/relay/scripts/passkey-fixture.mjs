// Makes test/fixtures/passkey-proof.json: device registration proofs signed by
// a real browser's WebAuthn stack (ov-766), not by the test's own encoder.
//
// Chromium, through Playwright, opens a page on https://app.canary.farcooler.com
// (served by a route handler, so nothing goes over the network) with the
// DevTools virtual authenticator attached. The page makes two passkeys: one
// whose RP ID is that exact host, and one scoped to the parent domain,
// farcooler.com, which a browser also allows from that page. Each signs its
// own registration statement as `webauthn-sk-ecdsa-sha2-nistp256@openssh.com`
// inside an SSHSIG, which is how the web client will sign it (ov-752 slice 4).
//
// Not run by CI, and the relay doesn't depend on Playwright. To regenerate:
//   npm i --prefix /tmp/pw playwright-core
//   NODE_PATH=/tmp/pw/node_modules node scripts/passkey-fixture.mjs <chromium binary>
// Every run makes new keys, so the fixture's bytes change each time.

import { writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { chromium } = require('playwright-core')

const HOST = 'app.canary.farcooler.com'
const ACCOUNT = 'user_01FIXTURE'
const CHALLENGE = 'fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210'

const browser = await chromium.launch({ executablePath: process.argv[2] })
const page = await browser.newPage()
await page.route(`https://${HOST}/**`, route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>fixture</title>' }))
await page.goto(`https://${HOST}/`)
const cdp = await page.context().newCDPSession(page)
await cdp.send('WebAuthn.enable')
await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: {
    protocol: 'ctap2',
    transport: 'internal',
    hasResidentKey: true,
    hasUserVerification: true,
    isUserVerified: true,
    automaticPresenceSimulation: true,
  },
})

const proofs = await page.evaluate(
  async ({ account, challenge }) => {
    const enc = new TextEncoder()
    const sshString = b => {
      const out = new Uint8Array(4 + b.length)
      new DataView(out.buffer).setUint32(0, b.length)
      out.set(b, 4)
      return out
    }
    const concat = (...parts) => {
      const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
      let at = 0
      for (const p of parts) {
        out.set(p, at)
        at += p.length
      }
      return out
    }
    const b64 = b => btoa(String.fromCharCode(...b))
    const sha = async (name, b) => new Uint8Array(await crypto.subtle.digest(name, b))
    const mpint = v => {
      let i = 0
      while (i < v.length - 1 && v[i] === 0) i++
      v = v.slice(i)
      if (v[0] & 0x80) v = concat(new Uint8Array([0]), v)
      return sshString(v)
    }
    // DER Ecdsa-Sig-Value to r and s.
    const rs = der => {
      let i = 2
      const r = der.slice(i + 2, i + 2 + der[i + 1])
      i = i + 2 + der[i + 1]
      const s = der.slice(i + 2, i + 2 + der[i + 1])
      return [r, s]
    }

    async function proofFor(rpId) {
      const credential = await navigator.credentials.create({
        publicKey: {
          rp: { id: rpId, name: 'Far Cooler' },
          user: { id: crypto.getRandomValues(new Uint8Array(16)), name: 'fixture', displayName: 'fixture' },
          challenge: crypto.getRandomValues(new Uint8Array(32)),
          pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
          authenticatorSelection: { residentKey: 'required', userVerification: 'discouraged' },
        },
      })
      const spki = new Uint8Array(credential.response.getPublicKey())
      const point = spki.slice(spki.length - 65)
      const keyBlob = concat(
        sshString(enc.encode('sk-ecdsa-sha2-nistp256@openssh.com')),
        sshString(enc.encode('nistp256')),
        sshString(point),
        sshString(enc.encode(rpId)),
      )
      const fingerprint = `SHA256:${b64(await sha('SHA-256', keyBlob)).replace(/=+$/, '')}`
      const statement = `farcooler register-device\naccount ${account}\nchallenge ${challenge}\nkey-a ${fingerprint}\nkey-b none\nnode none\n`
      const namespace = enc.encode('farcooler-device-v1')
      const hashName = enc.encode('sha512')
      const data = concat(
        enc.encode('SSHSIG'),
        sshString(namespace),
        sshString(new Uint8Array()),
        sshString(hashName),
        sshString(await sha('SHA-512', enc.encode(statement))),
      )
      const assertion = await navigator.credentials.get({
        publicKey: {
          challenge: data,
          rpId,
          userVerification: 'discouraged',
          allowCredentials: [{ type: 'public-key', id: credential.rawId }],
        },
      })
      const authData = new Uint8Array(assertion.response.authenticatorData)
      const clientData = new Uint8Array(assertion.response.clientDataJSON)
      const [r, s] = rs(new Uint8Array(assertion.response.signature))
      const origin = JSON.parse(new TextDecoder().decode(clientData)).origin
      const signature = concat(
        sshString(enc.encode('webauthn-sk-ecdsa-sha2-nistp256@openssh.com')),
        sshString(concat(mpint(r), mpint(s))),
        authData.slice(32, 33),
        authData.slice(33, 37),
        sshString(enc.encode(origin)),
        sshString(clientData),
        sshString(authData.slice(37)),
      )
      const blob = concat(
        enc.encode('SSHSIG'),
        new Uint8Array([0, 0, 0, 1]),
        sshString(keyBlob),
        sshString(namespace),
        sshString(new Uint8Array()),
        sshString(hashName),
        sshString(signature),
      )
      const armored = `-----BEGIN SSH SIGNATURE-----\n${b64(blob).replace(/.{1,70}/g, '$&\n')}-----END SSH SIGNATURE-----\n`
      return {
        rpId,
        publicKey: `sk-ecdsa-sha2-nistp256@openssh.com ${b64(keyBlob)}`,
        fingerprint,
        statement,
        proof: armored,
        clientDataJSON: new TextDecoder().decode(clientData),
      }
    }

    return { exact: await proofFor(location.hostname), parent: await proofFor('farcooler.com') }
  },
  { account: ACCOUNT, challenge: CHALLENGE },
)
await browser.close()

const fixture = {
  account: ACCOUNT,
  challenge: CHALLENGE,
  rpId: HOST,
  made: `Chromium ${browser.version()} virtual authenticator (ctap2, internal), scripts/passkey-fixture.mjs`,
  ...proofs,
}
const out = new URL('../test/fixtures/passkey-proof.json', import.meta.url)
writeFileSync(out, `${JSON.stringify(fixture, null, 2)}\n`)
console.log(`wrote ${out.pathname}`)
