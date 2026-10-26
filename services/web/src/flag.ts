/// Who gets the web client (ov-764, ov-810), per channel.
///
/// Fails closed. Two exact values turn the client on. "owner" admits only an
/// account id listed in WEB_ACCOUNTS (an empty list means no one). "everyone"
/// admits any signed-in account and ignores WEB_ACCOUNTS. Anything else, such
/// as "Owner", "Everyone", "everyone " or a missing variable, means no one.
/// The account id is the relay's (`user.id` from /v1/directory/me), which the
/// relay derives from the verified access token, never from the browser.

import type { WebEnv } from './env'

/// Whether the client is on for anyone on this channel. Off means every page
/// is "Not available yet" and there is no sign-in at all.
export function open(env: WebEnv): boolean {
  if (env.WEB_CLIENT === 'everyone') return true
  return env.WEB_CLIENT === 'owner' && accounts(env).length > 0
}

/// Whether this account may load the client.
export function admitted(env: WebEnv, account: unknown): boolean {
  if (!open(env)) return false
  if (typeof account !== 'string' || account === '') return false
  return env.WEB_CLIENT === 'everyone' || accounts(env).includes(account)
}

function accounts(env: WebEnv): string[] {
  return (env.WEB_ACCOUNTS ?? '').split(/[\s,]+/).filter(Boolean)
}
