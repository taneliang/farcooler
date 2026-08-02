/// Counting how much the product is used, without learning who used it.
///
/// Deliberately Analytics Engine rather than a table in D1, for two reasons
/// that matter more than the convenience:
///
///  1. A deletion request has to be answerable. Account rows cascade; a metrics
///     table would have to be swept too, and the sweep is the thing everyone
///     forgets. Nothing here is keyed by an account id, so there is nothing to
///     sweep.
///  2. Counters and identity have different lifetimes. DAU for last March is
///     worth keeping; the account that produced it may have asked to be
///     forgotten in April. Storing them together forces a choice between
///     lying to the auditor and losing the history.
///
/// So the account id never lands here. What lands is a MONTHLY-SALTED hash of
/// it, which supports "how many distinct users this month" and supports nothing
/// else: once the salt rotates, last month's hashes cannot be matched to this
/// month's, so the series cannot become a per-user history by accumulation.

export interface Metrics {
  writeDataPoint(event: {
    blobs?: string[]
    doubles?: number[]
    indexes?: string[]
  }): void
}

/// A stable-within-the-month, useless-across-months identifier.
export async function anonymousId(accountId: string, salt: string): Promise<string> {
  const month = new Date().toISOString().slice(0, 7) // YYYY-MM
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(`${salt}:${month}`),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  )
  const mac = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(accountId))
  return [...new Uint8Array(mac)].slice(0, 12).map(b => b.toString(16).padStart(2, '0')).join('')
}

export type EventName =
  // The first event any account ever produces, and the one DAU and MAU are
  // actually counted from: someone can open the app for a week without
  // registering a device or pairing a machine, and that is still a user.
  | 'signed_in'
  | 'device_registered'
  | 'daemon_paired'
  | 'notification_sent'
  | 'notification_failed'

/// One data point per interesting thing.
///
/// `indexes` is what Analytics Engine samples and groups by, so the anonymous
/// id goes there: DAU is `count(distinct index1)` over a day, MAU the same over
/// a month, and neither query needs anything the relay stores.
export async function record(
  metrics: Metrics,
  salt: string,
  event: EventName,
  accountId: string,
  extra: { platform?: string; ok?: boolean } = {},
): Promise<void> {
  metrics.writeDataPoint({
    indexes: [await anonymousId(accountId, salt)],
    blobs: [event, extra.platform ?? ''],
    doubles: [extra.ok === false ? 0 : 1],
  })
}
