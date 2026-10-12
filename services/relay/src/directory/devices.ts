/// A person's devices and their public keys (ov-480 §3).
///
/// A device registers its Key A, and on a Mac its Key B, each with a signature
/// made by that key, so nobody can claim a key they don't hold. The node key is
/// a Curve25519 key that can't sign, so it is held to being unique and nothing
/// more.
///
/// The first device a person ever registers is trusted on first use. Every
/// later one waits until one of their trusted or certified devices signs a
/// certificate for it. The directory can't sign one itself, because it never
/// holds a private key, so a compromised directory can't add "Bob's new laptop"
/// and inherit Bob's access. A person who has lost every device stays pending
/// until a team admin re-certifies them, which comes later.

import { json, type Env } from '../index'
import { parsePublicKey, verifySignature, type PublicKey } from '../sshsig'
import { audit, bodyOf, isConstraint, nameOf, requireUser } from './common'

/// SSH signature namespaces, one per statement, so a signature made for one
/// purpose can never be replayed as another. Grants (phase 4) use
/// `farcooler-grant-v1`.
export const REGISTER_NAMESPACE = 'farcooler-device-v1'
export const CERTIFY_NAMESPACE = 'farcooler-certify-v1'

const NODE_KEY = /^nodekey:[0-9a-f]{64}$/

/// What a device's keys sign to prove it holds them, and what an existing
/// device signs to certify it. Both name the account, so a signature can't be
/// lifted onto someone else's profile, and every key the device claims.
export function registerStatement(account: string, keyA: string, keyB: string | null, node: string | null): string {
  return `farcooler register-device\naccount ${account}\nkey-a ${keyA}\nkey-b ${keyB ?? 'none'}\nnode ${node ?? 'none'}\n`
}

export function certifyStatement(account: string, keyA: string, keyB: string | null, node: string | null): string {
  return `farcooler certify-device\naccount ${account}\nkey-a ${keyA}\nkey-b ${keyB ?? 'none'}\nnode ${node ?? 'none'}\n`
}

/// Register this device's public keys under the signed-in person.
///
/// Body: `label`, `keyA`, `proofA` (an SSH signature by Key A over
/// `registerStatement`), and optionally `keyB` with `proofB`, and `nodeKey`.
/// Registering the same keys again answers with the same device; a key another
/// profile holds, or one that was revoked, is refused.
export async function registerDeviceKeys(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (!body) return json({ error: 'body' }, 400)
  const label = nameOf(body.label)
  if (!label) return json({ error: 'label' }, 400)

  const keyA = await parsePublicKey(body.keyA)
  if (!keyA) return json({ error: 'keyA' }, 400)
  let keyB: PublicKey | null = null
  if (body.keyB !== undefined && body.keyB !== null) {
    keyB = await parsePublicKey(body.keyB)
    if (!keyB) return json({ error: 'keyB' }, 400)
    if (keyB.fingerprint === keyA.fingerprint) return json({ error: 'keyB' }, 400)
  }
  let node: string | null = null
  if (body.nodeKey !== undefined && body.nodeKey !== null) {
    if (typeof body.nodeKey !== 'string' || !NODE_KEY.test(body.nodeKey)) return json({ error: 'nodeKey' }, 400)
    node = body.nodeKey
  }

  const statement = registerStatement(account, keyA.fingerprint, keyB?.fingerprint ?? null, node)
  const signerA = await verifySignature(body.proofA, REGISTER_NAMESPACE, statement)
  if (signerA?.fingerprint !== keyA.fingerprint) return json({ error: 'proofA' }, 403)
  if (keyB) {
    const signerB = await verifySignature(body.proofB, REGISTER_NAMESPACE, statement)
    if (signerB?.fingerprint !== keyB.fingerprint) return json({ error: 'proofB' }, 403)
  }

  const existing = await env.DB.prepare(
    `SELECT id, account_id, state, key_b_fingerprint, node_key FROM device_keys WHERE key_a_fingerprint = ?`,
  )
    .bind(keyA.fingerprint)
    .first<{ id: string; account_id: string; state: string; key_b_fingerprint: string | null; node_key: string | null }>()
  if (existing) {
    if (existing.account_id !== account) return json({ error: 'claimed' }, 409)
    if (existing.state === 'revoked') return json({ error: 'revoked' }, 409)
    if (existing.key_b_fingerprint !== (keyB?.fingerprint ?? null) || existing.node_key !== node) {
      // A device's keys are what it was certified with; new keys are a new device.
      return json({ error: 'keys changed' }, 409)
    }
    await env.DB.prepare(`UPDATE device_keys SET label = ? WHERE id = ?`).bind(label, existing.id).run()
    return json({ device: existing.id, state: existing.state })
  }

  // Trusted on first use only if this person has never registered a device:
  // one who revoked or lost them all doesn't get to start over unvouched.
  const id = crypto.randomUUID()
  const now = Date.now()
  try {
    await env.DB.batch([
      env.DB.prepare(
        `INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, key_b, key_b_fingerprint, node_key,
                                  state, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                CASE WHEN EXISTS (SELECT 1 FROM device_keys WHERE account_id = ?2) THEN 'pending' ELSE 'trusted' END, ?9`,
      ).bind(id, account, label, keyA.text, keyA.fingerprint, keyB?.text ?? null, keyB?.fingerprint ?? null, node, now),
      audit(env, {
        team: null,
        actor: account,
        action: 'device.registered',
        subject: id,
        detail: { label, keyA: keyA.fingerprint, keyB: keyB?.fingerprint ?? null },
      }),
    ])
  } catch (error) {
    // Key B or the node key is already someone's, or Key A raced in.
    if (isConstraint(error)) return json({ error: 'claimed' }, 409)
    throw error
  }
  const row = await env.DB.prepare(`SELECT state FROM device_keys WHERE id = ?`).bind(id).first<{ state: string }>()
  return json({ device: id, state: row?.state })
}

/// Certify a pending device with a signature from one of the same person's
/// trusted or certified devices.
///
/// Body: `device` (the pending one) and `certificate`, an SSH signature over
/// `certifyStatement` for that device's keys. The signer is whatever key made
/// the signature, looked up by fingerprint; the caller's session only decides
/// whose devices are in play.
export async function certifyDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.device !== 'string' || !body.device) return json({ error: 'device' }, 400)

  const device = await env.DB.prepare(
    `SELECT id, state, key_a_fingerprint, key_b_fingerprint, node_key FROM device_keys WHERE id = ? AND account_id = ?`,
  )
    .bind(body.device, account)
    .first<{ id: string; state: string; key_a_fingerprint: string; key_b_fingerprint: string | null; node_key: string | null }>()
  if (!device) return json({ error: 'not found' }, 404)
  if (device.state !== 'pending') return json({ error: 'state' }, 409)

  const statement = certifyStatement(account, device.key_a_fingerprint, device.key_b_fingerprint, device.node_key)
  const signer = await verifySignature(body.certificate, CERTIFY_NAMESPACE, statement)
  if (!signer) return json({ error: 'certificate' }, 403)
  const voucher = await env.DB.prepare(
    `SELECT id FROM device_keys WHERE key_a_fingerprint = ? AND account_id = ? AND state IN ('trusted', 'certified')`,
  )
    .bind(signer.fingerprint, account)
    .first<{ id: string }>()
  // The device itself can't be its own voucher: it is pending, and a pending
  // device vouches for nothing.
  if (!voucher) return json({ error: 'certificate' }, 403)

  const [changed] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE device_keys SET state = 'certified', certified_by = ?, certificate = ?, certified_at = ?
       WHERE id = ? AND state = 'pending'`,
    ).bind(voucher.id, body.certificate, Date.now(), device.id),
    audit(
      env,
      { team: null, actor: account, action: 'device.certified', subject: device.id, detail: { by: voucher.id } },
      { sql: `EXISTS (SELECT 1 FROM device_keys WHERE id = ? AND certified_by = ?)`, binds: [device.id, voucher.id] },
    ),
  ])
  if ((changed.meta?.changes ?? 0) === 0) return json({ error: 'state' }, 409)
  return json({ device: device.id, state: 'certified' })
}

/// Revoke one of the caller's devices. Its keys stay on record as revoked, so
/// they can't be registered again; taking them off runners is phase 6.
export async function revokeDeviceKeys(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.device !== 'string' || !body.device) return json({ error: 'device' }, 400)

  const device = await env.DB.prepare(`SELECT state FROM device_keys WHERE id = ? AND account_id = ?`)
    .bind(body.device, account)
    .first<{ state: string }>()
  if (!device) return json({ error: 'not found' }, 404)
  if (device.state === 'revoked') return json({ ok: false })

  await env.DB.batch([
    env.DB.prepare(`UPDATE device_keys SET state = 'revoked', revoked_at = ? WHERE id = ? AND account_id = ?`).bind(
      Date.now(),
      body.device,
      account,
    ),
    audit(env, { team: null, actor: account, action: 'device.revoked', subject: body.device }),
  ])
  return json({ ok: true })
}
