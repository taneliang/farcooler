/// The ICE servers a WebRTC session is handed (ov-1534): Cloudflare's STUN
/// always, and Cloudflare TURN credentials minted for that one session when
/// this relay holds a TURN key.
///
/// **The key never leaves this file's one request.** `TURN_KEY_API_TOKEN` is a
/// worker secret the orchestrator sets; it goes in the Authorization header of
/// the call to Cloudflare's credential endpoint and nowhere else: not into a
/// reply, not into a message to a runner, not into a log line. What a browser
/// and a runner get is the short-lived username and credential that call
/// returns, good for `TURN_TTL_S` and for nothing after.
///
/// **Without the key, STUN only, and said so.** A relay with no key (a
/// self-hoster, or a channel the orchestrator hasn't keyed yet) still lets two
/// peers find each other's public addresses, so direct paths work; a session
/// that needs a relay fails. The `ice_servers` message carries `stun_only`
/// with why, so the page can tell a person their network needs what this relay
/// can't give, rather than timing out without a word. The relay logs the
/// same, without any value.

import type { Env } from './index'

/// How long minted TURN credentials work: an hour.
///
/// Short on purpose, against Cloudflare's 24-hour default: a credential lifted
/// from a page or a runner's memory relays someone else's traffic on our bill
/// for at most this long. An hour rather than minutes because a TURN
/// allocation refreshes with the same credential, and a relayed SSH session
/// should not drop at the ten-minute mark; one that outlives the hour opens a
/// new signaling session and restarts ICE with fresh credentials.
export const TURN_TTL_S = 3600

/// Cloudflare's STUN, keyless and free. Both ports, because some networks
/// that block 3478 leave 53 open.
export const STUN_URLS = ['stun:stun.cloudflare.com:3478', 'stun:stun.cloudflare.com:53'] as const

/// Cloudflare's credential endpoint. The key id is a var; it is not a secret.
export function turnEndpoint(keyId: string): string {
  return `https://rtc.live.cloudflare.com/v1/turn/keys/${encodeURIComponent(keyId)}/credentials/generate`
}

/// Why a session got STUN only: this relay holds no TURN key, or Cloudflare
/// refused or failed to mint.
export type StunOnly = 'no_key' | 'refused'

/// The `ice_servers` message of the signaling contract, less its `session`.
export interface IceServers {
  urls: string[]
  username?: string
  credential?: string
  ttl: number
  stun_only?: StunOnly
}

const ICE_URL = /^(stun|stuns|turn|turns):[^\s]{1,250}$/

function stunOnly(why: StunOnly): IceServers {
  return { urls: [...STUN_URLS], ttl: TURN_TTL_S, stun_only: why }
}

/// The servers for one new session: freshly minted TURN credentials, or STUN
/// only (`stun_only` says why). Never throws for Cloudflare's sake: a failed
/// mint is a session without a relay, not a session refused.
export async function iceServers(env: Env): Promise<IceServers> {
  const keyId = env.TURN_KEY_ID
  const token = env.TURN_KEY_API_TOKEN
  if (!keyId || !token) {
    console.warn('rtc: no TURN key on this relay; handing out STUN only')
    return stunOnly('no_key')
  }

  let response: Response
  try {
    response = await fetch(turnEndpoint(keyId), {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ ttl: TURN_TTL_S }),
    })
  } catch {
    // Not the error: a fetch failure can quote the request it was making.
    console.error('rtc: TURN credentials unreachable; handing out STUN only')
    return stunOnly('refused')
  }
  if (!response.ok) {
    // The status only. Cloudflare's error body is not ours to repeat, and
    // could echo what it was sent.
    console.error(`rtc: TURN credentials refused (${response.status}); handing out STUN only`)
    return stunOnly('refused')
  }

  const minted = await credentialsOf(response)
  if (!minted) {
    console.error('rtc: TURN credentials unreadable; handing out STUN only')
    return stunOnly('refused')
  }
  // Cloudflare's own list carries its STUN too; ours go first either way, so
  // a page that only reads the first URL still finds a server.
  const urls = [...new Set([...STUN_URLS, ...minted.urls])]
  return { urls, username: minted.username, credential: minted.credential, ttl: TURN_TTL_S }
}

/// `{iceServers: {urls, username, credential}}`, as the generate endpoint
/// answers, with only addresses an ICE agent would take kept.
async function credentialsOf(response: Response): Promise<{ urls: string[]; username: string; credential: string } | null> {
  let body: unknown
  try {
    body = await response.json()
  } catch {
    return null
  }
  const servers = (body as { iceServers?: unknown } | null)?.iceServers
  if (!servers || typeof servers !== 'object' || Array.isArray(servers)) return null
  const { urls, username, credential } = servers as Record<string, unknown>
  const list = (typeof urls === 'string' ? [urls] : Array.isArray(urls) ? urls : []).filter(
    (u): u is string => typeof u === 'string' && ICE_URL.test(u),
  )
  if (!list.some(u => u.startsWith('turn'))) return null
  if (typeof username !== 'string' || !username || username.length > 512) return null
  if (typeof credential !== 'string' || !credential || credential.length > 512) return null
  return { urls: list, username, credential }
}
