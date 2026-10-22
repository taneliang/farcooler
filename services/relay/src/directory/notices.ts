/// Telling a person when their account gains or loses a device key (ov-550,
/// ov-549 review M1).
///
/// The first device a person registers is trusted on first use, so a stolen
/// session that registers first becomes their trusted device. Nothing can
/// tell that apart from the real first device but the person, so every new
/// device and every removal is pushed to each app install on the account,
/// and each app's Settings lists the account's device keys with a Remove
/// button (`devices/revoke`). The same notice makes a removal by a stolen
/// session visible, which used to be silent.
///
/// The push is a hint, not the record: a stolen session can delete the
/// account's push installs before it acts, and then nobody hears. The record
/// is `directory/me`'s `createdAt` and `revokedAt` for every device key, which
/// no route deletes, and each signed-in app shows a banner for any it hasn't
/// shown before (review M1, ov-550 fix round 1).
///
/// The push carries the device's label, which the registering session chose,
/// so it goes through `shownLabel` first: no quote marks to close the notice's
/// own quotes and go on with a sentence of the attacker's, and at most 40
/// characters (review M2). No key, no id. It has no terminal and no task, so a
/// tap opens the app and nothing in it. A push that fails is dropped: the
/// device is registered either way, and Settings lists it.

import type { Env } from '../index'
import { sendPush } from '../push'
import { withoutUnshown } from './common'

/// A label as a notice may quote it: no quote marks, lookalikes, bidi
/// controls or zero-width characters (`withoutUnshown`; the apostrophe stays,
/// since “Bob’s iPad” can't close the quotes), runs of space made one, cut to
/// 40 characters, and `a device` when nothing is left. `nameOf` drops the same
/// characters before a label is stored; this is for rows stored before it did
/// (ov-560 R1).
export function shownLabel(label: string): string {
  const plain = withoutUnshown(label).replace(/\s+/g, ' ').trim()
  const cut = [...plain].slice(0, 40).join('').trim()
  return cut || 'a device'
}

/// What `devices/register` and `devices/revoke` send, by event. `trusted` is
/// a device trusted on first use by R-75's reset (ov-561), because none of
/// the account's devices was confirmed any more: a new one, or one already on
/// the account signing in again.
///
/// `confirmed` is a team's admin lifting the hold on a device trusted by the
/// reset (R-76, ov-565 fix round 1 M1), with `team` the team's name.
export function deviceNotice(
  event: 'added' | 'removed' | 'trusted' | 'confirmed',
  label: string,
  team = '',
): { title: string; subtitle: string } {
  const name = shownLabel(label)
  switch (event) {
    case 'added':
      return {
        title: 'A device was added to your account',
        subtitle: `If “${name}” isn’t yours, remove it from your device keys in Far Cooler’s settings.`,
      }
    case 'removed':
      return {
        title: 'A device was removed from your account',
        subtitle: `“${name}” is no longer one of your devices. If you didn’t remove it, check your device keys in Far Cooler’s settings.`,
      }
    case 'trusted':
      return {
        title: 'A device is now confirmed on your account',
        subtitle: `None of your devices was confirmed, so “${name}” was confirmed when it signed in. If it isn’t yours, remove it from your device keys in Far Cooler’s settings.`,
      }
    case 'confirmed':
      return {
        title: 'A team admin confirmed one of your devices',
        subtitle: `An admin of “${shownLabel(team)}” confirmed “${name}”, so that team’s runners can add it now. If it isn’t yours, remove it from your device keys in Far Cooler’s settings.`,
      }
  }
}

/// Push `notice` to every app install signed in to `account`, all at once:
/// one slow push service mustn't hold up the request that registered or
/// removed the device (review L3).
///
/// Every install, whatever its notification settings say, unless `wants`
/// says otherwise: these are notices only the account's own person can act
/// on. A member's Needs You push (ov-633) passes `wants`, so it reaches only
/// the installs that keep its class on. `about` is what a tap opens: `kind`
/// and, for a runner's notice, the runner as a phone knows it
/// (`/v1/notify/member`, ov-646).
export async function tellAccount(
  env: Env,
  account: string,
  notice: { title: string; subtitle: string },
  about: { kind: string; runner?: string } = { kind: 'device' },
  wants: (notifyEvents: string | null) => boolean = () => true,
): Promise<void> {
  const installs = await env.DB.prepare(`SELECT platform, push_token, environment, notify_events FROM devices WHERE account_id = ?`)
    .bind(account)
    .all<{ platform: string; push_token: string; environment: string | null; notify_events: string | null }>()
  // Settled, never rejected: a failed push names nothing worth keeping.
  await Promise.allSettled(
    (installs.results ?? []).filter(install => wants(install.notify_events)).map(install =>
      sendPush(
        env,
        install.platform,
        install.push_token,
        { title: notice.title, subtitle: notice.subtitle, terminal: '', ...about, level: 'active' },
        install.environment,
      ),
    ),
  )
}

/// Sign-out's half (ov-550 review L1): drop the push row of the install that
/// is signing out, so a phone signed out and handed on stops hearing the
/// account's device notices. Possession of the token is the authority, as it
/// is for the pulse token. True when a token was given.
export async function forgetInstall(env: Env, token: unknown): Promise<boolean> {
  if (typeof token !== 'string' || !token || token.length > 4096) return false
  await env.DB.prepare(`DELETE FROM devices WHERE push_token = ?`).bind(token).run()
  return true
}
