// Reading the canary appcast: which build is newest, and is its URL one we may
// send a browser to.
//
// The appcast is a Sparkle feed (RSS) written by canary.yml. Only the
// `<enclosure>` tags matter here. The feed is fetched over the network, so
// nothing in it is trusted: a redirect is a thing an attacker would love to
// own, and "the newest build" is exactly the field a corrupted or substituted
// feed would lie in.

export interface Build {
  /** `sparkle:version`, the build number. Compared as a number. */
  version: number
  /** The .dmg URL, already checked by `allowedDownload`. */
  url: string
}

/** Pull every `name="value"` pair out of one tag's attribute text. */
function attributes(tag: string): Map<string, string> {
  const found = new Map<string, string>()
  for (const m of tag.matchAll(/([A-Za-z_:][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) {
    found.set(m[1], decodeEntities(m[2] ?? m[3] ?? ''))
  }
  return found
}

/** The five XML entities, which is all an attribute value in this feed can use. */
function decodeEntities(text: string): string {
  return text
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&')
}

/**
 * Whether `candidate` is a canary disk image on the appcast's own host.
 *
 * The rule is the whole defense: https, exactly the appcast's host (no
 * userinfo, no port, no lookalike subdomain), a path inside the appcast's own
 * folder, and a `.dmg` name. Anything else is refused rather than repaired.
 */
export function allowedDownload(candidate: string, appcastUrl: string): string | null {
  let url: URL
  let feed: URL
  try {
    url = new URL(candidate)
    feed = new URL(appcastUrl)
  } catch {
    return null
  }
  if (url.protocol !== 'https:' || feed.protocol !== 'https:') return null
  if (url.host !== feed.host) return null
  if (url.username !== '' || url.password !== '' || url.port !== '') return null
  const folder = feed.pathname.slice(0, feed.pathname.lastIndexOf('/') + 1)
  if (!url.pathname.startsWith(folder)) return null
  // `..` and encoded slashes could climb out of the folder after the check.
  if (/(^|\/)\.\.?(\/|$)/.test(url.pathname) || /%2f|%5c/i.test(url.pathname)) return null
  if (!url.pathname.toLowerCase().endsWith('.dmg')) return null
  url.hash = ''
  return url.href
}

/**
 * The newest build in the feed, or null if the feed has none we may use.
 *
 * "Newest" is the highest `sparkle:version`, not the first or last item, so
 * the feed's ordering does not matter. If the newest build's URL is refused the
 * answer is null, NOT the next-newest: a feed whose top entry points somewhere
 * foreign is a feed to distrust, and quietly serving an older build would hide
 * that.
 */
export function newestBuild(xml: string, appcastUrl: string): Build | null {
  let best: { version: number; url: string } | null = null
  for (const m of xml.matchAll(/<enclosure\b([^>]*)>/g)) {
    const attrs = attributes(m[1])
    const url = attrs.get('url')
    const version = Number(attrs.get('sparkle:version'))
    if (url === undefined || !Number.isFinite(version) || version <= 0) continue
    if (best === null || version > best.version) best = { version, url }
  }
  if (best === null) return null
  const safe = allowedDownload(best.url, appcastUrl)
  return safe === null ? null : { version: best.version, url: safe }
}
