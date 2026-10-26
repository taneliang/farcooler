/// Who gets the web client (ov-764): owner-only, per channel, until the plan's
/// slice 15 widens it.
///
/// Fails closed. Only the exact value "owner" turns the client on, and then
/// only for an account id listed in WEB_ACCOUNTS. An empty list, a misspelled
/// value ("Owner", "everyone"), or a variable that's missing all mean no one.
/// The account id is the relay's (`user.id` from /v1/directory/me), which the
/// relay derives from the verified access token, never from the browser.

import type { WebEnv } from './env'

/// Whether the client is on for anyone on this channel. Off means every page
/// is "Not available yet" and there is no sign-in at all.
export function open(env: WebEnv): boolean {
  return env.WEB_CLIENT === 'owner' && accounts(env).length > 0
}

/// Whether this account may load the client.
export function admitted(env: WebEnv, account: unknown): boolean {
  if (!open(env)) return false
  return typeof account === 'string' && account !== '' && accounts(env).includes(account)
}

function accounts(env: WebEnv): string[] {
  return (env.WEB_ACCOUNTS ?? '').split(/[\s,]+/).filter(Boolean)
}
