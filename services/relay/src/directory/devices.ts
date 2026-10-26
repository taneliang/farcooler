/// A person's devices and their public keys (ov-480 §3).
///
/// A device registers its Key A, and on a Mac its Key B, each with a signature
/// made by that key, so nobody can claim a key they don't hold. A fingerprint
/// belongs to one device in either slot, for good: a revoked key can't come
/// back as anyone's Key A or Key B. The node key is a Curve25519 key that
/// can't sign, so it can't be proven: it is stored as the device's word, and
/// never unique, because a uniqueness rule on an unproven key would let anyone
/// who saw yours block your registration (review S5).
///
/// The first device a person ever registers is trusted on first use. Every
/// later one waits until one of their trusted or certified devices signs a
/// certificate for it. The directory can't sign one itself, because it never
/// holds a private key, so a compromised directory can't add "Bob's new laptop"
/// and inherit Bob's access. A person who has removed or lost every confirmed
/// device starts over: the next of their devices to register is trusted on
/// first use again, and every app on the account announces it (ruling R-75,
/// ov-561). There is no support desk or admin step to stand in for it.

import { json, type Env } from '../index'
import { parseDeviceKey, parsePublicKey, verifyDeviceSignature, verifySignature, type PublicKey } from '../sshsig'
import { audit, bodyOf, isConstraint, nameOf, requireUser } from './common'
import { deviceNotice, tellAccount } from './notices'

/// SSH signature namespaces, one per statement, so a signature made for one
/// purpose can never be replayed as another. Grants (phase 4) use
/// `farcooler-grant-v1`.
export const REGISTER_NAMESPACE = 'farcooler-device-v1'
export const CERTIFY_NAMESPACE = 'farcooler-certify-v1'

const NODE_KEY = /^nodekey:[0-9a-f]{64}$/

/// A challenge, as the relay mints it and as a device must sign it: 32 random
/// bytes in lowercase hex. The client refuses any other shape before signing
/// (`crates/client/src/device_keys.rs`), so a relay can't slip a line of its
/// own into the statement through it.
const CHALLENGE = /^[0-9a-f]{64}$/

/// How long a challenge may wait to be signed, and how many one account may
/// hold at once (two devices signing in together each need one).
export const CHALLENGE_TTL_MS = 5 * 60 * 1000
export const CHALLENGES_PER_ACCOUNT = 8

/// What a device's keys sign to prove it holds them, and what an existing
/// device signs to certify it. Both name the account, so a signature can't be
/// lifted onto someone else's profile, and every key the device claims.
///
/// Registration also names a challenge the relay chose and spends on use
/// (ov-549), so a proof registers once: a copy of it, from a log or a proxy,
/// is refused. A certificate names none, because a runner checks it later,
/// offline, against nothing the relay could hand it.
export function registerStatement(
  account: string,
  challenge: string,
  keyA: string,
  keyB: string | null,
  node: string | null,
): string {
  return `farcooler register-device\naccount ${account}\nchallenge ${challenge}\nkey-a ${keyA}\nkey-b ${keyB ?? 'none'}\nnode ${node ?? 'none'}\n`
}

export function certifyStatement(account: string, keyA: string, keyB: string | null, node: string | null): string {
  return `farcooler certify-device\naccount ${account}\nkey-a ${keyA}\nkey-b ${keyB ?? 'none'}\nnode ${node ?? 'none'}\n`
}

/// A fresh challenge for one of the signed-in person's devices to sign.
///
/// Answers `{challenge, expiresAt}`. Expired ones are dropped here, and an
/// account holding the most it may has its oldest dropped, so minting without
/// end costs the table nothing.
export async function deviceChallenge(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  // Per account, before any write. A device mints one per sign-in or launch,
  // so the limit (wrangler.toml) is far above that. Open with no binding, as
  // `AUTH_LIMIT` is: a local relay without one still registers devices.
  if (env.DEVICE_CHALLENGE_LIMIT && !(await env.DEVICE_CHALLENGE_LIMIT.limit({ key: account })).success) {
    return json({ error: 'slow down' }, 429)
  }
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  const challenge = [...bytes].map(b => b.toString(16).padStart(2, '0')).join('')
  const now = Date.now()
  const expiresAt = now + CHALLENGE_TTL_MS
  await env.DB.batch([
    env.DB.prepare(`DELETE FROM device_challenges WHERE expires_at <= ?`).bind(now),
    env.DB.prepare(
      `DELETE FROM device_challenges WHERE account_id = ?1 AND nonce IN (
         SELECT nonce FROM device_challenges WHERE account_id = ?1
         ORDER BY created_at DESC, nonce LIMIT -1 OFFSET ?2)`,
    ).bind(account, CHALLENGES_PER_ACCOUNT - 1),
    env.DB.prepare(`INSERT INTO device_challenges (nonce, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)`).bind(
      challenge,
      account,
      now,
      expiresAt,
    ),
  ])
  return json({ challenge, expiresAt })
}

/// Register this device's public keys under the signed-in person.
///
/// Body: `label`, `challenge` (from `deviceChallenge`, unspent and unexpired),
/// `keyA`, `proofA` (an SSH signature by Key A over `registerStatement`), and
/// optionally `keyB` with `proofB`, and `nodeKey`. Registering the same keys
/// again, with a new challenge, answers with the same device, under the label
/// it first registered with; a key another profile holds, or one that was
/// revoked, is refused.
export async function registerDeviceKeys(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (!body) return json({ error: 'body' }, 400)
  const label = nameOf(body.label)
  if (!label) return json({ error: 'label' }, 400)

  // Key A may be a passkey on this channel's app host (ov-766). Key B is a
  // Mac's second key, and stays Ed25519.
  const keyA = await parseDeviceKey(body.keyA, env.PASSKEY_RP_ID)
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

  if (typeof body.challenge !== 'string' || !CHALLENGE.test(body.challenge)) return json({ error: 'challenge' }, 400)
  const challenge = body.challenge
  const statement = registerStatement(account, challenge, keyA.fingerprint, keyB?.fingerprint ?? null, node)
  const signerA = await verifyDeviceSignature(body.proofA, REGISTER_NAMESPACE, statement, env.PASSKEY_RP_ID)
  if (signerA?.fingerprint !== keyA.fingerprint) return json({ error: 'proofA' }, 403)
  if (keyB) {
    const signerB = await verifySignature(body.proofB, REGISTER_NAMESPACE, statement)
    if (signerB?.fingerprint !== keyB.fingerprint) return json({ error: 'proofB' }, 403)
  }

  // Spent here, once the proof is good, by one DELETE whose change count
  // decides: of two requests carrying one challenge, exactly one gets past.
  // The account is in the WHERE, so a challenge minted for someone else is
  // nobody's here, and the statement named this account anyway.
  const spent = await env.DB.prepare(`DELETE FROM device_challenges WHERE nonce = ? AND account_id = ? AND expires_at > ?`)
    .bind(challenge, account, Date.now())
    .run()
  if ((spent.meta?.changes ?? 0) !== 1) return json({ error: 'challenge' }, 403)

  // Every device either key already belongs to, in either slot, revoked or not.
  const fingerprints = [keyA.fingerprint, ...(keyB ? [keyB.fingerprint] : [])]
  const holders = await env.DB.prepare(
    `SELECT DISTINCT d.id, d.account_id, d.state, d.label, d.key_a_fingerprint, d.key_b_fingerprint
     FROM device_fingerprints f JOIN device_keys d ON d.id = f.device_id
     WHERE f.fingerprint IN (?, ?)`,
  )
    .bind(keyA.fingerprint, keyB?.fingerprint ?? keyA.fingerprint)
    .all<{
      id: string
      account_id: string
      state: string
      label: string
      key_a_fingerprint: string
      key_b_fingerprint: string | null
    }>()
  if (holders.results.some(h => h.account_id !== account)) return json({ error: 'claimed' }, 409)
  if (holders.results.some(h => h.state === 'revoked')) return json({ error: 'revoked' }, 409)
  if (holders.results.length) {
    const same = holders.results[0]
    if (
      holders.results.length > 1 ||
      same.key_a_fingerprint !== keyA.fingerprint ||
      same.key_b_fingerprint !== (keyB?.fingerprint ?? null)
    ) {
      // A device's keys are what it was certified with; new keys are a new device.
      return json({ error: 'keys changed' }, 409)
    }
    const row = await env.DB.prepare(`SELECT node_key FROM device_keys WHERE id = ?`).bind(same.id).first<{ node_key: string | null }>()
    // The label stays the one the device first registered with (ov-560 R2):
    // a rename would let a device the person already dismissed take the name
    // of one of theirs, with nothing announced. A new name needs a new key,
    // which is a new device, and that is announced.
    const kept =
      row?.node_key === node || same.state !== 'certified'
        ? // A trusted device's only proof is its own signature, which this
          // request just made over the new node key too.
          [
            env.DB.prepare(`UPDATE device_keys SET node_key = ? WHERE id = ?`).bind(node, same.id),
            ...(row?.node_key === node
              ? []
              : [audit(env, { team: null, actor: account, action: 'device.node_changed', subject: same.id, detail: { node } })]),
          ]
        : // A certificate covers the node key (`certifyStatement`), so a
          // certified device with a new one goes back to pending until one of
          // the person's other devices certifies it again (review R3).
          // Guarded on the state read above, so a concurrent change can't be
          // overwritten. What it certified goes back to pending with it, as a
          // revoke's would (R-52): its certificates no longer vouch for
          // anything (fix round 1, H1).
          [
            ...uncertifyVouched(env, same.id, account, Date.now(), 'recertify'),
            env.DB.prepare(
              `UPDATE device_keys SET node_key = ?, state = 'pending', certified_by = NULL, certificate = NULL,
                                      certified_at = NULL
               WHERE id = ? AND state = 'certified'`,
            ).bind(node, same.id),
            audit(env, { team: null, actor: account, action: 'device.node_changed', subject: same.id, detail: { node, recertify: true } }),
          ]
    // Only a device that was pending when it was read starts over: a
    // certified one whose node key just changed is demoted by `kept`, and
    // mustn't be trusted again in the same breath (fix round 1, Low 1).
    const reset = same.state === 'pending' ? trustIfNoneConfirmed(env, account, same.id, Date.now()) : []
    const results = await env.DB.batch([...kept, ...reset])
    const trusted = reset.length > 0 && (results[kept.length].meta?.changes ?? 0) > 0
    if (trusted) await tellAccount(env, account, deviceNotice('trusted', same.label))
    const now = await env.DB.prepare(`SELECT state FROM device_keys WHERE id = ?`).bind(same.id).first<{ state: string }>()
    return json({ device: same.id, state: now?.state ?? same.state })
  }

  // Trusted on first use when none of this person's devices is trusted or
  // certified: their first device, and since R-75 (ov-561) the first to
  // register after they removed or lost every confirmed one. Decided inside
  // the INSERT, so two devices registering at once can't both see none.
  const id = crypto.randomUUID()
  const now = Date.now()
  try {
    await env.DB.batch([
      env.DB.prepare(
        `INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, key_b, key_b_fingerprint, node_key,
                                  state, created_at, trusted_at, trust_reset)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, c.state, ?9, CASE c.state WHEN 'trusted' THEN ?9 END,
                CASE WHEN c.state = 'trusted' AND c.had THEN 1 END
         FROM (SELECT CASE WHEN EXISTS (${CONFIRMED}) THEN 'pending' ELSE 'trusted' END AS state,
                      EXISTS (SELECT 1 FROM device_keys WHERE account_id = ?2) AS had) c`,
      ).bind(id, account, label, keyA.text, keyA.fingerprint, keyB?.text ?? null, keyB?.fingerprint ?? null, node, now),
      // One column for both slots: the schema itself refuses a key twice.
      ...fingerprints.map(fingerprint =>
        env.DB.prepare(`INSERT INTO device_fingerprints (fingerprint, device_id) VALUES (?, ?)`).bind(fingerprint, id),
      ),
      audit(env, {
        team: null,
        actor: account,
        action: 'device.registered',
        subject: id,
        detail: { label, keyA: keyA.fingerprint, keyB: keyB?.fingerprint ?? null },
      }),
      // Trusted though the account has had devices before: R-75's reset.
      audit(
        env,
        { team: null, actor: account, action: 'device.trusted', subject: id, detail: { reset: true } },
        { sql: `EXISTS (SELECT 1 FROM device_keys WHERE id = ? AND trust_reset = 1)`, binds: [id] },
      ),
    ])
  } catch (error) {
    // A key raced in under another device between the read and this write.
    if (isConstraint(error)) return json({ error: 'claimed' }, 409)
    throw error
  }
  const row = await env.DB.prepare(`SELECT state, trust_reset FROM device_keys WHERE id = ?`)
    .bind(id)
    .first<{ state: string; trust_reset: number | null }>()
  // Every new device, the first one most of all: first-use trust is decided
  // here, and only the person can tell theirs from a stolen session's (M1).
  // One trusted by R-75's reset says so: it's the account's confirmed device
  // now, though it had others (fix round 1, M1).
  await tellAccount(env, account, deviceNotice(row?.trust_reset === 1 ? 'trusted' : 'added', label))
  return json({ device: id, state: row?.state })
}

/// Whether any of `?2`'s devices is trusted or certified: SQL, so the check
/// runs inside the write it guards.
const CONFIRMED = `SELECT 1 FROM device_keys WHERE account_id = ?2 AND state IN ('trusted', 'certified')`

/// R-75 (ov-561) for a device already on the account: a pending one signing
/// in again while none of the account's devices is confirmed is trusted on
/// first use, as a new one would be. Otherwise "sign in on the device you
/// want to keep" could never work, since that device is registered already.
///
/// One guarded UPDATE, so of two pending devices signing in at once exactly
/// one is trusted, and the other waits to be certified from it. Its audit
/// line is written only if the UPDATE was. `trusted_at` is what each app's
/// banner announces: a stolen session that removes every confirmed device
/// and signs in again is seen doing it.
function trustIfNoneConfirmed(env: Env, account: string, id: string, now: number): D1PreparedStatement[] {
  return [
    env.DB.prepare(
      `UPDATE device_keys SET state = 'trusted', trusted_at = ?3, trust_reset = 1
       WHERE id = ?1 AND account_id = ?2 AND state = 'pending' AND NOT EXISTS (${CONFIRMED})`,
    ).bind(id, account, now),
    audit(
      env,
      { team: null, actor: account, action: 'device.trusted', subject: id, detail: { reset: true } },
      { sql: `EXISTS (SELECT 1 FROM device_keys WHERE id = ? AND trusted_at = ?)`, binds: [id, now] },
    ),
  ]
}

/// Certify a pending device with a signature from one of the same person's
/// trusted or certified devices.
///
/// Body: the pending device, named by `device` (its id) or by `keyA` (its
/// Key A, as the same-account ceremony scanned it, ov-550: the scanning device
/// knows the key from the code and has no other way to learn the id), and
/// `certificate`, an SSH signature over `certifyStatement` for that device's
/// keys. The signer is whatever key made the signature, looked up by
/// fingerprint; the caller's session only decides whose devices are in play.
///
/// A device that isn't pending answers 409 with its `state`, so a ceremony
/// that certifies a device already vouched for can say so rather than fail.
/// One the caller's account hasn't registered (yet: a new device registers
/// when it signs in) is a 404.
export async function certifyDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  let where: { column: 'id' | 'key_a_fingerprint'; value: string }
  if (typeof body?.device === 'string' && body.device) {
    where = { column: 'id', value: body.device }
  } else if (body?.keyA !== undefined) {
    const keyA = await parseDeviceKey(body.keyA, env.PASSKEY_RP_ID)
    if (!keyA) return json({ error: 'keyA' }, 400)
    where = { column: 'key_a_fingerprint', value: keyA.fingerprint }
  } else {
    return json({ error: 'device' }, 400)
  }

  const device = await env.DB.prepare(
    `SELECT id, state, key_a_fingerprint, key_b_fingerprint, node_key FROM device_keys WHERE ${where.column} = ? AND account_id = ?`,
  )
    .bind(where.value, account)
    .first<{ id: string; state: string; key_a_fingerprint: string; key_b_fingerprint: string | null; node_key: string | null }>()
  if (!device) return json({ error: 'not found' }, 404)
  if (device.state !== 'pending') return json({ error: 'state', state: device.state }, 409)

  const statement = certifyStatement(account, device.key_a_fingerprint, device.key_b_fingerprint, device.node_key)
  // The voucher signs with Ed25519 alone, even when the device it vouches
  // for is a passkey: a runner checks this certificate offline, in Rust.
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
    // The voucher is checked again inside the write: a device revoked, or
    // sent back to pending by R-52's cascade, between the read above and this
    // batch vouches for nothing (review R1).
    env.DB.prepare(
      `UPDATE device_keys SET state = 'certified', certified_by = ?1, certificate = ?2, certified_at = ?3
       WHERE id = ?4 AND state = 'pending'
         AND EXISTS (SELECT 1 FROM device_keys WHERE id = ?1 AND account_id = ?5 AND state IN ('trusted', 'certified'))`,
    ).bind(voucher.id, body.certificate, Date.now(), device.id, account),
    audit(
      env,
      { team: null, actor: account, action: 'device.certified', subject: device.id, detail: { by: voucher.id } },
      { sql: `EXISTS (SELECT 1 FROM device_keys WHERE id = ? AND certified_by = ?)`, binds: [device.id, voucher.id] },
    ),
  ])
  if ((changed.meta?.changes ?? 0) === 0) return json({ error: 'state' }, 409)
  return json({ device: device.id, state: 'certified' })
}

/// Everything `device` certified, then what those certified, and so on, back
/// to pending, each with a `device.uncertified` audit line naming `cause`
/// (`revoked`, R-52; or `recertify`, a node-key change, fix round 1 H1). Two
/// statements, audit first while the links still exist. UNION, not UNION
/// ALL, so a cycle ends. Run them BEFORE the statement that demotes or
/// revokes `device` itself; they don't depend on its state.
///
/// `except` is a phone's new key taking the device's place (ov-569): never
/// in the cascade, since it succeeds the device rather than depending on it.
function uncertifyVouched(
  env: Env,
  device: string,
  account: string,
  now: number,
  cause: 'revoked' | 'recertify',
  except: string | null = null,
): D1PreparedStatement[] {
  const vouched = `WITH RECURSIVE vouched(id) AS (
      SELECT id FROM device_keys WHERE certified_by = ?1 AND account_id = ?2 AND state = 'certified' AND id IS NOT ?5
      UNION
      SELECT d.id FROM device_keys d JOIN vouched v ON d.certified_by = v.id WHERE d.state = 'certified')`
  return [
    env.DB.prepare(
      `${vouched}
       INSERT INTO audit (team_id, actor, action, subject, detail, at)
       SELECT NULL, ?2, 'device.uncertified', id, json_object(?4, ?1), ?3 FROM vouched`,
    ).bind(device, account, now, cause, except),
    env.DB.prepare(
      `UPDATE device_keys SET state = 'pending', certified_by = NULL, certificate = NULL, certified_at = NULL
       WHERE id IN (${vouched} SELECT id FROM vouched)`,
    ).bind(device, account, null, null, except),
  ]
}

/// Revoke one of the caller's devices, and with it every certificate it
/// signed, transitively (R-52): each device it vouched for, and each one those
/// vouched for, goes back to pending, to be certified again from another
/// trusted device. Each one gets its own audit line. The revoked keys stay on
/// record, so they can't be registered again; taking them off runners is
/// phase 6.
///
/// A phone's "Make a New Key" (ov-569) names a `successor`: the new key's
/// device, which the old key certified on the phone before this. The devices
/// the old key vouched for then move to the successor instead of going back
/// to pending, so replacing a phone's key doesn't unconfirm the person's other
/// devices, and the successor takes the old device's place in the chain. A
/// hold (R-76) carries over to the same end, so a new key is no way around it. Only a device the old key itself certified can be its successor
/// (`certified_by`, checked again inside the write), and only its private key
/// could sign that certificate. A stolen session can't name a device of its
/// own; naming the real successor only does less than a plain revoke.
export async function revokeDeviceKeys(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  if (typeof body?.device !== 'string' || !body.device) return json({ error: 'device' }, 400)
  let successor: string | null = null
  if (body.successor !== undefined && body.successor !== null) {
    if (typeof body.successor !== 'string' || !body.successor || body.successor === body.device) {
      return json({ error: 'successor' }, 400)
    }
    successor = body.successor
  }

  const device = await env.DB.prepare(`SELECT state, label FROM device_keys WHERE id = ? AND account_id = ?`)
    .bind(body.device, account)
    .first<{ state: string; label: string }>()
  if (!device) return json({ error: 'not found' }, 404)
  if (device.state === 'revoked') return json({ ok: false })
  // Refused before anything is written, so a revoke that can't hand over
  // doesn't cascade instead: the phone asked to keep its chain.
  const SUCCEEDS = `EXISTS (SELECT 1 FROM device_keys WHERE id = ?4 AND account_id = ?2 AND state = 'certified' AND certified_by = ?1)`
  if (successor) {
    const ok = await env.DB.prepare(`SELECT ${SUCCEEDS} AS ok`).bind(body.device, account, null, successor).first<{ ok: number }>()
    if (!ok?.ok) return json({ error: 'successor' }, 409)
  }

  const now = Date.now()
  // Moved to the successor first, inside the same batch and only while it is
  // still the old device's successor, so the cascade below, which never
  // includes the successor, finds nothing.
  //
  // Then the successor takes the old device's place in the chain, so it and
  // everything moved to it still reach a trusted device (`hold.ts`): a trusted
  // old device's trust, with its time and whether a reset gave it, so a hold
  // runs on to the same end; or a certified one's voucher. Its own certificate
  // stays the old key's signature, the link that was actually made.
  const OLD = `(SELECT o.%s FROM device_keys o WHERE o.id = ?1 AND o.account_id = ?2 AND o.state IN ('trusted', 'certified'))`
  const old = (column: string) => OLD.replace('%s', column)
  const handover = successor
    ? [
        env.DB.prepare(
          `UPDATE device_keys SET certified_by = ?4
           WHERE certified_by = ?1 AND account_id = ?2 AND state = 'certified' AND id != ?4 AND ${SUCCEEDS}`,
        ).bind(body.device, account, null, successor),
        audit(
          env,
          { team: null, actor: account, action: 'device.replaced', subject: body.device, detail: { by: successor } },
          {
            sql: `EXISTS (SELECT 1 FROM device_keys WHERE id = ? AND account_id = ? AND state = 'certified' AND certified_by = ?)`,
            binds: [successor, account, body.device],
          },
        ),
        env.DB.prepare(
          `UPDATE device_keys SET
             certified_by = CASE ${old('state')} WHEN 'trusted' THEN NULL ELSE ${old('certified_by')} END,
             trusted_at = CASE ${old('state')} WHEN 'trusted' THEN ${old('trusted_at')} END,
             trust_reset = CASE ${old('state')} WHEN 'trusted' THEN ${old('trust_reset')} END,
             state = ${old('state')}
           WHERE id = ?4 AND account_id = ?2 AND state = 'certified' AND certified_by = ?1 AND ${old('state')} IS NOT NULL`,
        ).bind(body.device, account, null, successor),
      ]
    : []
  const results = await env.DB.batch([
    ...handover,
    ...uncertifyVouched(env, body.device, account, now, 'revoked', successor),
    env.DB.prepare(`UPDATE device_keys SET state = 'revoked', revoked_at = ?3 WHERE id = ?1 AND account_id = ?2`).bind(
      body.device,
      account,
      now,
    ),
    audit(env, { team: null, actor: account, action: 'device.revoked', subject: body.device }),
  ])
  const uncertified = results[handover.length + 1]
  // A stolen session can remove devices too, so a removal is never silent.
  await tellAccount(env, account, deviceNotice('removed', device.label))
  return json({
    ok: true,
    uncertified: uncertified.meta?.changes ?? 0,
    ...(successor ? { moved: results[0].meta?.changes ?? 0 } : {}),
  })
}
