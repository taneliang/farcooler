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

/// A label as a notice may quote it: every double quote mark and runs of
/// space dropped (an apostrophe stays, since “Bob’s iPad” can't close the
/// quotes), cut to 40 characters, and `a device` when nothing is left.
export function shownLabel(label: string): string {
  const plain = label.replace(/["`\u201c\u201d\u201e\u201f\u00ab\u00bb\u2039\u203a]/g, '').replace(/\s+/g, ' ').trim()
  const cut = [...plain].slice(0, 40).join('').trim()
  return cut || 'a device'
}

/// What `devices/register` and `devices/revoke` send, by event.
export function deviceNotice(event: 'added' | 'removed', label: string): { title: string; subtitle: string } {
  const name = shownLabel(label)
  return event === 'added'
    ? {
        title: 'A device was added to your account',
        subtitle: `If “${name}” isn’t yours, remove it from your device keys in Far Cooler’s settings.`,
      }
    : {
        title: 'A device was removed from your account',
        subtitle: `“${name}” is no longer one of your devices. If you didn’t remove it, check your device keys in Far Cooler’s settings.`,
      }
}

/// Push `notice` to every app install signed in to `account`, all at once:
/// one slow push service mustn't hold up the request that registered or
/// removed the device (review L3).
export async function tellAccount(env: Env, account: string, notice: { title: string; subtitle: string }): Promise<void> {
  const installs = await env.DB.prepare(`SELECT platform, push_token, environment FROM devices WHERE account_id = ?`)
    .bind(account)
    .all<{ platform: string; push_token: string; environment: string | null }>()
  // Settled, never rejected: a failed push names nothing worth keeping.
  await Promise.allSettled(
    (installs.results ?? []).map(install =>
      sendPush(
        env,
        install.platform,
        install.push_token,
        { title: notice.title, subtitle: notice.subtitle, terminal: '', kind: 'device', level: 'active' },
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
