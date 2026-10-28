import { allowedDownload, newestBuild } from './appcast'

// GET /download/canary: a short-lived 302 to the newest canary .dmg.
//
// The page's Download buttons point here instead of at a build number, so the
// page never goes stale and a new canary needs no site deploy.

/** How long a browser or the edge may reuse a redirect. */
const REDIRECT_SECONDS = 60
/** How long the last good answer is kept for when the appcast cannot be read. */
const LAST_GOOD_SECONDS = 30 * 24 * 60 * 60
const FEED_TIMEOUT_MS = 4000
/** A cache key, not a real URL anyone requests. */
const LAST_GOOD_KEY = 'https://last-good.farcooler.invalid/canary'

/** The slice of Workers Analytics Engine this file uses. */
export interface DownloadCounter {
  writeDataPoint(event: { blobs?: string[]; doubles?: number[]; indexes?: string[] }): void
}

export interface DownloadEnv {
  APPCAST_URL: string
  /// Counts downloads per build (ov-504). Optional: a Worker without the
  /// binding just doesn't count.
  DOWNLOADS?: DownloadCounter
}

/// One aggregate row per download: the event and the build number, nothing
/// about the visitor. Never throws, so a counter can't break a download.
function count(env: DownloadEnv, build: number | null): void {
  try {
    env.DOWNLOADS?.writeDataPoint({ indexes: ['download.click'], blobs: ['download.click', build === null ? 'unknown' : String(build)], doubles: [1] })
  } catch {
    // Counting is best effort.
  }
}

/** Read the appcast; null on any failure (network, status, timeout). */
async function readFeed(appcastUrl: string): Promise<string | null> {
  try {
    const res = await fetch(appcastUrl, {
      signal: AbortSignal.timeout(FEED_TIMEOUT_MS),
      cf: { cacheTtl: REDIRECT_SECONDS, cacheEverything: true },
    })
    return res.ok ? await res.text() : null
  } catch {
    return null
  }
}

function redirect(url: string): Response {
  return new Response(null, {
    status: 302,
    headers: { location: url, 'cache-control': `public, max-age=${REDIRECT_SECONDS}` },
  })
}

/** What a person sees when there is genuinely nowhere to send them. */
function unavailable(): Response {
  const body =
    '<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width">' +
    '<title>Far Cooler</title>' +
    '<body style="font:17px/1.5 system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem">' +
    "<h1>The download isn't available right now.</h1>" +
    "<p>We couldn't find the latest build. Please try again in a few minutes.</p>" +
    '<p><a href="/">Back to Far Cooler</a></p>'
  return new Response(body, {
    status: 503,
    headers: { 'content-type': 'text/html; charset=utf-8', 'retry-after': '120', 'cache-control': 'no-store' },
  })
}

/**
 * Redirect to the newest canary build.
 *
 * When the appcast cannot be read (or has nothing we may use), the fallback is
 * the last build this Worker successfully read. The folder itself can't be
 * listed (the bucket's public host returns 404 for it), so "last known" has to
 * be remembered here. It is re-validated against the appcast's host on the way
 * out, so a poisoned cache entry is no more trusted than a poisoned feed. With
 * nothing remembered, a friendly 503.
 */
export async function canaryDownload(env: DownloadEnv, counted = true): Promise<Response> {
  const cache = await caches.open('site-download')
  const xml = await readFeed(env.APPCAST_URL)
  const build = xml === null ? null : newestBuild(xml, env.APPCAST_URL)
  if (build !== null) {
    await cache.put(
      new Request(LAST_GOOD_KEY),
      new Response(build.url, { headers: { 'cache-control': `max-age=${LAST_GOOD_SECONDS}` } }),
    )
    if (counted) count(env, build.version)
    return redirect(build.url)
  }
  const remembered = await cache.match(new Request(LAST_GOOD_KEY))
  const url = remembered === undefined ? null : allowedDownload(await remembered.text(), env.APPCAST_URL)
  console.error('canary appcast unusable', url === null ? 'no fallback' : 'serving last known build')
  if (url === null) return unavailable()
  if (counted) count(env, null)
  return redirect(url)
}
