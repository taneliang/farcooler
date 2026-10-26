/// The hold on a device trusted by a reset (ov-565, ruling R-76).
///
/// R-75 lets an account with no confirmed device trust its next one on first
/// use. A stolen session can bring that about by removing every confirmed
/// device and signing in, and its device would then be in the `keys` a team's
/// admins check a teammate's code against, as the person's own, at once
/// (ov-561 review M2). So a device trusted by the reset, and every device it
/// certifies after, is held: left out of `keys` until the hold ends, or until
/// an admin of the team, with the person in front of them, confirms it.
///
/// What this must not depend on: the device's own word (it chose its label
/// and its keys), anything an app's clock says (the relay's clock alone
/// decides when a hold ends), a push reaching the person (the session can
/// delete every push install first), or an admin of one team vouching for
/// another (a confirmation is per team). An admin can never confirm a device
/// of their own account: a stolen admin session would otherwise lift its own
/// hold.

import { json, type Env } from '../index'
import { parseDeviceKey } from '../sshsig'
import { audit, bodyOf, requireUser, teamFor } from './common'
import { deviceNotice, tellAccount } from './notices'

/// How long a device trusted by a reset waits before a team's admins are
/// sent its key: 72 hours.
///
/// The hold is the person's time to see the reset announced (the banner on
/// every app they open, from `directory/me`) and remove a device that isn't
/// theirs before any team admin's scan would take it as theirs. Seventy-two
/// hours covers a weekend: a reset late on a Friday is still held when the
/// person is back at work on Monday. Someone recovering their own account
/// needn't wait it out, because an admin who sees them in person can confirm
/// the device at once, so a longer hold would cost little; but a week would
/// strand teammates whose admins work elsewhere for days at a time, and a
/// day wouldn't outlast a weekend.
export const TRUST_RESET_HOLD_MS = 72 * 60 * 60 * 1000

/// The columns of a device row that decide its hold.
export interface HoldRow {
  id: string
  state: string
  trust_reset: number | null
  trusted_at: number | null
  certified_by: string | null
}

/// Each confirmed device's standing, by id:
/// - `since`: when the hold began, for a device trusted by a reset less than
///   `TRUST_RESET_HOLD_MS` before `now`, and for each certified device whose
///   chain of vouchers reaches one, inheriting its start;
/// - `unrooted`: a certified device whose chain doesn't reach a trusted
///   device: a voucher that's missing, pending or revoked, or a cycle
///   (fix round 1, H1). Its certificate vouches for nothing any more, so it
///   is listed nowhere, neither in `keys` nor in `held`.
///
/// A confirmed device in neither is listed. Fails closed: whatever the walk
/// can't account for is unrooted, never listed.
///
/// `rows` must hold every device a chain can pass through, in any state. A
/// certificate is always signed by a device of the same account
/// (`certifyDevice`), so one account's rows are enough.
export function holds(rows: HoldRow[], now: number): { since: Map<string, number>; unrooted: Set<string> } {
  const byId = new Map(rows.map(row => [row.id, row]))
  const since = new Map<string, number>()
  const unrooted = new Set<string>()
  for (const row of rows) {
    if (row.state !== 'trusted' && row.state !== 'certified') continue
    const seen = new Set<string>()
    let at: HoldRow | undefined = row
    for (;;) {
      if (!at || seen.has(at.id) || (at.state !== 'trusted' && at.state !== 'certified')) {
        unrooted.add(row.id)
        break
      }
      seen.add(at.id)
      if (at.state === 'trusted') {
        if (at.trust_reset === 1 && at.trusted_at !== null && at.trusted_at > now - TRUST_RESET_HOLD_MS) {
          since.set(row.id, at.trusted_at)
        }
        break
      }
      at = at.certified_by === null ? undefined : byId.get(at.certified_by)
    }
  }
  return { since, unrooted }
}

/// A team admin confirms, in person, that a held device is its person's
/// (R-76): from then on the team's roster lists its key.
///
/// Body: `team`, `user` (whose device it is) and `keyA` (the device's Key A,
/// as the admin's Mac read it from the code: the relay never names the key).
/// The caller must be an admin of `team`, and `user` an admin or member of it
/// who isn't the caller. A device that isn't held answers `{ok: true}` with
/// nothing written, so a scan that races the hold's end still goes through.
export async function confirmHeldDevice(request: Request, env: Env): Promise<Response> {
  const account = await requireUser(request, env)
  if (account instanceof Response) return account
  const body = await bodyOf(request)
  const found = await teamFor(env, account, body?.team, ['admin'])
  if (found instanceof Response) return found
  if (found.team.personal_of !== null) return json({ error: 'personal' }, 409)
  if (typeof body?.user !== 'string' || !body.user) return json({ error: 'user' }, 400)
  const user = body.user
  // Your own devices are confirmed by your own devices, never by your say-so
  // as an admin: that is the one person a stolen session can always be.
  if (user === account) return json({ error: 'own device' }, 403)
  const keyA = await parseDeviceKey(body?.keyA, env.PASSKEY_RP_ID)
  if (!keyA) return json({ error: 'keyA' }, 400)

  const team = found.team.id
  const member = await env.DB.prepare(
    `SELECT 1 AS ok FROM memberships WHERE team_id = ? AND account_id = ? AND role IN ('admin', 'member')`,
  )
    .bind(team, user)
    .first<{ ok: number }>()
  if (!member) return json({ error: 'not found' }, 404)

  const rows = await env.DB.prepare(
    `SELECT id, label, state, trust_reset, trusted_at, certified_by, key_a_fingerprint FROM device_keys WHERE account_id = ?`,
  )
    .bind(user)
    .all<HoldRow & { key_a_fingerprint: string; label: string }>()
  const device = rows.results.find(d => d.key_a_fingerprint === keyA.fingerprint)
  if (!device) return json({ error: 'not found' }, 404)
  if (device.state !== 'trusted' && device.state !== 'certified') return json({ error: 'state', state: device.state }, 409)
  const standing = holds(rows.results, Date.now())
  // A device whose chain is broken is listed nowhere, and a confirmation
  // can't list it either (fix round 1, H1).
  if (standing.unrooted.has(device.id)) return json({ error: 'state', state: 'unconfirmed' }, 409)
  const since = standing.since.get(device.id)
  if (since === undefined) return json({ ok: true })

  const now = Date.now()
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO device_hold_confirmations (device_id, team_id, confirmed_by, confirmed_at) VALUES (?, ?, ?, ?)
       ON CONFLICT (device_id, team_id) DO UPDATE SET confirmed_by = excluded.confirmed_by, confirmed_at = excluded.confirmed_at`,
    ).bind(device.id, team, account, now),
    audit(env, {
      team,
      actor: account,
      action: 'device.hold_confirmed',
      subject: device.id,
      detail: { user, keyA: keyA.fingerprint, heldSince: since },
    }),
  ])
  // Lifting a hold is news to the person (fix round 1, M1): it's what a
  // stolen session would want, so it's announced. The push is a hint; the
  // record is `me`'s `holdConfirmations`, which no route deletes.
  await tellAccount(env, user, deviceNotice('confirmed', device.label, found.team.name))
  return json({ ok: true })
}
