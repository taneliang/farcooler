// Cloudflare Web Analytics, rendered only when a token is configured.

/** A Web Analytics site token is 32 hex characters. Anything else is not one. */
const TOKEN = /^[0-9a-f]{32}$/i

/** The beacon `<script>` for `token`, or null when there is no usable token. */
export function beaconTag(token: string | undefined): string | null {
  const t = (token ?? '').trim()
  if (t === '') return null
  if (!TOKEN.test(t)) {
    console.error('ANALYTICS_TOKEN is set but is not a Web Analytics token; no beacon rendered')
    return null
  }
  return `<script defer src="https://static.cloudflareinsights.com/beacon.min.js" data-cf-beacon='{"token": "${t}"}'></script>`
}

/** `response` with the beacon appended to <body>; untouched if no beacon or not HTML. */
export function withBeacon(response: Response, token: string | undefined): Response {
  const tag = beaconTag(token)
  if (tag === null || !(response.headers.get('content-type') ?? '').includes('text/html')) return response
  return new HTMLRewriter()
    .on('body', { element: (el) => void el.append(tag, { html: true }) })
    .transform(response)
}
